use std::collections::{BTreeMap, BTreeSet, VecDeque};

use anyhow::{Context, Result, anyhow};
use async_graphql::types::Json;
use serde_json::Value;

use super::{DevqlGraphqlContext, DevqlSchemaMode};
use crate::graphql::ResolverScope;
use crate::graphql::scope::SelectedRepository;
use crate::graphql::types::{
    ArchitectureContainer, ArchitectureGraph, ArchitectureGraphAssertionAction,
    ArchitectureGraphAssertionSummary, ArchitectureGraphEdge, ArchitectureGraphEdgeKind,
    ArchitectureGraphFilterInput, ArchitectureGraphFlow, ArchitectureGraphFlowStep,
    ArchitectureGraphNode, ArchitectureGraphNodeKind, ArchitectureGraphRepositoryRef,
    ArchitectureGraphTargetKind, ArchitectureSystem,
};
use crate::host::devql::esc_pg;

impl DevqlGraphqlContext {
    pub(crate) async fn list_architecture_graph(
        &self,
        scope: &ResolverScope,
        filter: Option<&ArchitectureGraphFilterInput>,
        first: Option<usize>,
        after: Option<&str>,
    ) -> Result<ArchitectureGraph> {
        if scope.temporal_scope().is_some() {
            return Err(anyhow!(
                "`architectureGraph` does not support historical or temporary `asOf(...)` scopes"
            ));
        }

        let repo_id = self.repo_id_for_scope(scope)?;
        let mut nodes = load_computed_nodes(self, &repo_id, scope, filter).await?;
        let mut edges = load_computed_edges(self, &repo_id, filter).await?;
        let assertions = load_assertions(self, &repo_id).await?;
        apply_assertions(&mut nodes, &mut edges, assertions);

        if filter.map_or(true, |filter| filter.effective_only) {
            nodes.retain(|_, node| node.effective);
            let node_ids = nodes.keys().cloned().collect::<BTreeSet<_>>();
            edges.retain(|_, edge| {
                edge.effective
                    && node_ids.contains(&edge.from_node_id)
                    && node_ids.contains(&edge.to_node_id)
            });
        }

        let mut node_values = nodes.into_values().collect::<Vec<_>>();
        node_values.sort_by(|left, right| left.id.cmp(&right.id));
        if let Some(after) = after {
            node_values = node_values
                .into_iter()
                .skip_while(|node| node.id != after)
                .skip(1)
                .collect();
        }
        if let Some(limit) = first {
            node_values.truncate(limit);
        }
        let included_node_ids = node_values
            .iter()
            .map(|node| node.id.clone())
            .collect::<BTreeSet<_>>();
        let mut edge_values = edges
            .into_values()
            .filter(|edge| {
                included_node_ids.contains(&edge.from_node_id)
                    && included_node_ids.contains(&edge.to_node_id)
            })
            .collect::<Vec<_>>();
        edge_values.sort_by(|left, right| left.id.cmp(&right.id));

        Ok(ArchitectureGraph {
            total_nodes: graph_count(node_values.len()),
            total_edges: graph_count(edge_values.len()),
            nodes: node_values,
            edges: edge_values,
        })
    }

    pub(crate) async fn list_architecture_entry_points(
        &self,
        scope: &ResolverScope,
        kind: Option<&str>,
        first: Option<usize>,
    ) -> Result<Vec<ArchitectureGraphNode>> {
        let filter = ArchitectureGraphFilterInput {
            node_kind: Some(ArchitectureGraphNodeKind::EntryPoint),
            edge_kind: None,
            path: None,
            source_kind: None,
            effective_only: true,
        };
        let graph = self
            .list_architecture_graph(scope, Some(&filter), None, None)
            .await?;
        let mut nodes = graph
            .nodes
            .into_iter()
            .filter(|node| {
                kind.map_or(true, |kind| {
                    node.entry_kind
                        .as_deref()
                        .is_some_and(|entry_kind| entry_kind.eq_ignore_ascii_case(kind))
                })
            })
            .collect::<Vec<_>>();
        nodes.sort_by(|left, right| {
            left.path
                .cmp(&right.path)
                .then_with(|| left.label.cmp(&right.label))
                .then_with(|| left.id.cmp(&right.id))
        });
        if let Some(limit) = first {
            nodes.truncate(limit);
        }
        Ok(nodes)
    }

    pub(crate) async fn list_architecture_flows(
        &self,
        scope: &ResolverScope,
        entry_point_id: Option<&str>,
        first: Option<usize>,
    ) -> Result<Vec<ArchitectureGraphFlow>> {
        let graph = self
            .list_architecture_graph(scope, None, None, None)
            .await?;
        let nodes_by_id = graph
            .nodes
            .into_iter()
            .map(|node| (node.id.clone(), node))
            .collect::<BTreeMap<_, _>>();
        let edges = graph.edges;
        let mut traverses_by_flow: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for edge in &edges {
            if edge.kind == ArchitectureGraphEdgeKind::Traverses {
                traverses_by_flow
                    .entry(edge.from_node_id.clone())
                    .or_default()
                    .push(edge.to_node_id.clone());
            }
        }

        let mut flows = Vec::new();
        for edge in &edges {
            if edge.kind != ArchitectureGraphEdgeKind::Triggers {
                continue;
            }
            if entry_point_id.is_some_and(|id| edge.from_node_id != id) {
                continue;
            }
            let (Some(entry_point), Some(flow)) = (
                nodes_by_id.get(&edge.from_node_id),
                nodes_by_id.get(&edge.to_node_id),
            ) else {
                continue;
            };
            let traversed_nodes = traverses_by_flow
                .get(&edge.to_node_id)
                .into_iter()
                .flat_map(|ids| ids.iter())
                .filter_map(|node_id| nodes_by_id.get(node_id).cloned())
                .collect::<Vec<_>>();
            let steps = flow_steps_for_entry(entry_point, &traversed_nodes, &edges);
            flows.push(ArchitectureGraphFlow {
                entry_point: entry_point.clone(),
                flow: flow.clone(),
                traversed_nodes,
                steps,
            });
        }
        flows.sort_by(|left, right| {
            left.entry_point
                .path
                .cmp(&right.entry_point.path)
                .then_with(|| left.entry_point.label.cmp(&right.entry_point.label))
        });
        if let Some(limit) = first {
            flows.truncate(limit);
        }
        Ok(flows)
    }

    pub(crate) async fn list_architecture_containers(
        &self,
        scope: &ResolverScope,
        system_key: Option<&str>,
        first: Option<usize>,
    ) -> Result<Vec<ArchitectureContainer>> {
        let repository = self.repository_selection_for_scope(scope)?;
        let repo_ref = repository_ref(&repository);
        let graph = self
            .list_architecture_graph(scope, None, None, None)
            .await?;
        let mut systems = systems_from_repo_graph(repo_ref, graph, system_key);
        let mut containers = systems
            .values_mut()
            .flat_map(|system| std::mem::take(&mut system.containers))
            .collect::<Vec<_>>();
        sort_containers(&mut containers);
        let mut seen = BTreeSet::new();
        containers.retain(|container| {
            seen.insert((container.repository.repo_id.clone(), container.id.clone()))
        });
        if let Some(limit) = first {
            containers.truncate(limit);
        }
        Ok(containers)
    }

    pub(crate) async fn list_architecture_systems(
        &self,
        system_key: Option<&str>,
        first: Option<usize>,
    ) -> Result<Vec<ArchitectureSystem>> {
        let mut repositories = self.list_known_repositories().await?;
        if self.schema_mode == DevqlSchemaMode::Slim && self.request_scope_present {
            let current = self.repository_selection_for_scope(&self.slim_root_scope())?;
            if !repositories
                .iter()
                .any(|repository| repository.repo_id() == current.repo_id())
            {
                repositories.push(current);
            }
        }
        if repositories.is_empty() {
            repositories.push(self.repository_selection_for_scope(&self.slim_root_scope())?);
        }
        let mut merged = BTreeMap::<String, ArchitectureSystem>::new();
        for repository in repositories {
            let scope = ResolverScope::default().with_repository(repository.clone());
            let graph = match self.list_architecture_graph(&scope, None, None, None).await {
                Ok(graph) => graph,
                Err(err) if is_missing_architecture_graph_table_error(&err) => continue,
                Err(err) => return Err(err),
            };
            for (key, mut system) in
                systems_from_repo_graph(repository_ref(&repository), graph, system_key)
            {
                merged
                    .entry(key)
                    .and_modify(|existing| {
                        merge_repository_refs(&mut existing.repositories, &system.repositories);
                        existing.containers.append(&mut system.containers);
                        sort_containers(&mut existing.containers);
                    })
                    .or_insert(system);
            }
        }
        let mut systems = merged.into_values().collect::<Vec<_>>();
        systems.sort_by(|left, right| {
            left.key
                .cmp(&right.key)
                .then_with(|| left.label.cmp(&right.label))
        });
        if let Some(limit) = first {
            systems.truncate(limit);
        }
        Ok(systems)
    }

    pub(crate) async fn architecture_system(
        &self,
        key: &str,
    ) -> Result<Option<ArchitectureSystem>> {
        Ok(self
            .list_architecture_systems(Some(key), Some(1))
            .await?
            .into_iter()
            .next())
    }

    pub(crate) async fn architecture_node_for_artefact(
        &self,
        scope: &ResolverScope,
        artefact_id: &str,
    ) -> Result<Option<ArchitectureGraphNode>> {
        let graph = self
            .list_architecture_graph(scope, None, None, None)
            .await?;
        Ok(graph
            .nodes
            .into_iter()
            .find(|node| node.artefact_id.as_deref() == Some(artefact_id)))
    }
}

fn flow_steps_for_entry(
    entry_point: &ArchitectureGraphNode,
    traversed_nodes: &[ArchitectureGraphNode],
    edges: &[ArchitectureGraphEdge],
) -> Vec<ArchitectureGraphFlowStep> {
    if traversed_nodes.is_empty() {
        return Vec::new();
    }

    let mut module_nodes = BTreeMap::<String, Vec<ArchitectureGraphNode>>::new();
    let mut node_modules = BTreeMap::<String, String>::new();
    for node in traversed_nodes {
        let module_key = flow_module_key(node);
        node_modules.insert(node.id.clone(), module_key.clone());
        module_nodes
            .entry(module_key)
            .or_default()
            .push(node.clone());
    }
    for nodes in module_nodes.values_mut() {
        sort_nodes_for_display(nodes);
    }

    let mut module_graph = module_nodes
        .keys()
        .map(|key| (key.clone(), BTreeSet::new()))
        .collect::<BTreeMap<_, _>>();
    let mut incoming_modules = module_nodes
        .keys()
        .map(|key| (key.clone(), BTreeSet::new()))
        .collect::<BTreeMap<_, _>>();
    let mut incoming_edge_kinds =
        BTreeMap::<String, BTreeMap<String, ArchitectureGraphEdgeKind>>::new();
    let mut self_loop_modules = BTreeSet::new();

    for edge in edges {
        if !is_flow_sequence_edge(edge.kind) {
            continue;
        }
        let (Some(from_module), Some(to_module)) = (
            node_modules.get(&edge.from_node_id),
            node_modules.get(&edge.to_node_id),
        ) else {
            continue;
        };
        incoming_edge_kinds
            .entry(to_module.clone())
            .or_default()
            .insert(edge.kind.as_db().to_string(), edge.kind);
        if from_module == to_module {
            self_loop_modules.insert(from_module.clone());
            continue;
        }
        module_graph
            .entry(from_module.clone())
            .or_default()
            .insert(to_module.clone());
        incoming_modules
            .entry(to_module.clone())
            .or_default()
            .insert(from_module.clone());
    }

    let mut start_modules = start_module_keys(entry_point, &module_nodes);
    if start_modules.is_empty() {
        start_modules.extend(
            incoming_modules
                .iter()
                .filter_map(|(module_key, incoming)| {
                    incoming.is_empty().then(|| module_key.clone())
                }),
        );
    }
    if start_modules.is_empty() {
        start_modules.extend(module_nodes.keys().cloned());
    }

    let depth_by_module = module_depths(&module_graph, &start_modules);
    let fallback_depth = depth_by_module
        .values()
        .copied()
        .max()
        .unwrap_or_default()
        .saturating_add(1);
    let (components, component_by_module) = strongly_connected_modules(&module_graph);
    let component_order = ordered_components(
        &components,
        &component_by_module,
        &module_graph,
        &depth_by_module,
        fallback_depth,
    );
    let component_is_cyclic = cyclic_components(
        &components,
        &component_by_module,
        &module_graph,
        &self_loop_modules,
    );

    let mut steps = Vec::new();
    for component_id in component_order {
        let mut modules = components.get(component_id).cloned().unwrap_or_default();
        modules.sort_by(|left, right| {
            flow_sort_depth(&depth_by_module, left, fallback_depth)
                .cmp(&flow_sort_depth(&depth_by_module, right, fallback_depth))
                .then_with(|| left.cmp(right))
        });
        for module_key in modules {
            let predecessor_module_keys = incoming_modules
                .get(&module_key)
                .map(|modules| modules.iter().cloned().collect())
                .unwrap_or_default();
            let edge_kinds = incoming_edge_kinds
                .get(&module_key)
                .map(|kinds| kinds.values().copied().collect())
                .unwrap_or_default();
            steps.push(ArchitectureGraphFlowStep {
                ordinal: graph_count(steps.len().saturating_add(1)),
                module_key: module_key.clone(),
                depth: flow_sort_depth(&depth_by_module, &module_key, fallback_depth),
                nodes: module_nodes.get(&module_key).cloned().unwrap_or_default(),
                predecessor_module_keys,
                edge_kinds,
                cyclic: component_is_cyclic
                    .get(component_id)
                    .copied()
                    .unwrap_or(false),
            });
        }
    }
    steps
}

fn is_flow_sequence_edge(kind: ArchitectureGraphEdgeKind) -> bool {
    matches!(
        kind,
        ArchitectureGraphEdgeKind::DependsOn
            | ArchitectureGraphEdgeKind::Calls
            | ArchitectureGraphEdgeKind::Reads
            | ArchitectureGraphEdgeKind::Writes
            | ArchitectureGraphEdgeKind::Emits
            | ArchitectureGraphEdgeKind::Stores
            | ArchitectureGraphEdgeKind::Modifies
    )
}

fn flow_module_key(node: &ArchitectureGraphNode) -> String {
    node.path
        .clone()
        .or_else(|| property_string(node, "module_key"))
        .or_else(|| property_string(node, "component_key"))
        .unwrap_or_else(|| node.id.clone())
}

fn sort_nodes_for_display(nodes: &mut [ArchitectureGraphNode]) {
    nodes.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| left.label.cmp(&right.label))
            .then_with(|| left.id.cmp(&right.id))
    });
}

fn start_module_keys(
    entry_point: &ArchitectureGraphNode,
    module_nodes: &BTreeMap<String, Vec<ArchitectureGraphNode>>,
) -> BTreeSet<String> {
    let mut starts = BTreeSet::new();
    for (module_key, nodes) in module_nodes {
        if entry_point.artefact_id.as_ref().is_some_and(|artefact_id| {
            nodes
                .iter()
                .any(|node| node.artefact_id.as_ref() == Some(artefact_id))
        }) || entry_point.symbol_id.as_ref().is_some_and(|symbol_id| {
            nodes
                .iter()
                .any(|node| node.symbol_id.as_ref() == Some(symbol_id))
        }) || entry_point
            .path
            .as_ref()
            .is_some_and(|path| nodes.iter().any(|node| node.path.as_ref() == Some(path)))
        {
            starts.insert(module_key.clone());
        }
    }
    starts
}

fn module_depths(
    module_graph: &BTreeMap<String, BTreeSet<String>>,
    start_modules: &BTreeSet<String>,
) -> BTreeMap<String, i32> {
    let mut depths = BTreeMap::<String, i32>::new();
    let mut queue = VecDeque::new();
    for module_key in start_modules {
        depths.insert(module_key.clone(), 0i32);
        queue.push_back(module_key.clone());
    }
    while let Some(module_key) = queue.pop_front() {
        let Some(depth) = depths.get(&module_key).copied() else {
            continue;
        };
        for next in module_graph.get(&module_key).into_iter().flatten() {
            if !depths.contains_key(next) {
                depths.insert(next.clone(), depth.saturating_add(1));
                queue.push_back(next.clone());
            }
        }
    }
    depths
}

fn flow_sort_depth(
    depth_by_module: &BTreeMap<String, i32>,
    module_key: &str,
    fallback_depth: i32,
) -> i32 {
    depth_by_module
        .get(module_key)
        .copied()
        .unwrap_or(fallback_depth)
}

fn strongly_connected_modules(
    module_graph: &BTreeMap<String, BTreeSet<String>>,
) -> (Vec<Vec<String>>, BTreeMap<String, usize>) {
    #[derive(Default)]
    struct TarjanState {
        index: usize,
        stack: Vec<String>,
        on_stack: BTreeSet<String>,
        indices: BTreeMap<String, usize>,
        lowlinks: BTreeMap<String, usize>,
        components: Vec<Vec<String>>,
    }

    fn connect(node: &str, graph: &BTreeMap<String, BTreeSet<String>>, state: &mut TarjanState) {
        let index = state.index;
        state.indices.insert(node.to_string(), index);
        state.lowlinks.insert(node.to_string(), index);
        state.index += 1;
        state.stack.push(node.to_string());
        state.on_stack.insert(node.to_string());

        let neighbours = graph.get(node).cloned().unwrap_or_default();
        for neighbour in neighbours {
            if !state.indices.contains_key(&neighbour) {
                connect(&neighbour, graph, state);
                let node_lowlink = state.lowlinks.get(node).copied().unwrap_or(index);
                let neighbour_lowlink = state.lowlinks.get(&neighbour).copied().unwrap_or(index);
                state
                    .lowlinks
                    .insert(node.to_string(), node_lowlink.min(neighbour_lowlink));
            } else if state.on_stack.contains(&neighbour) {
                let node_lowlink = state.lowlinks.get(node).copied().unwrap_or(index);
                let neighbour_index = state.indices.get(&neighbour).copied().unwrap_or(index);
                state
                    .lowlinks
                    .insert(node.to_string(), node_lowlink.min(neighbour_index));
            }
        }

        if state.lowlinks.get(node) == state.indices.get(node) {
            let mut component = Vec::new();
            while let Some(member) = state.stack.pop() {
                state.on_stack.remove(&member);
                let is_root = member == node;
                component.push(member);
                if is_root {
                    break;
                }
            }
            component.sort();
            state.components.push(component);
        }
    }

    let mut state = TarjanState::default();
    for module_key in module_graph.keys() {
        if !state.indices.contains_key(module_key) {
            connect(module_key, module_graph, &mut state);
        }
    }

    let mut component_by_module = BTreeMap::new();
    for (component_id, component) in state.components.iter().enumerate() {
        for module_key in component {
            component_by_module.insert(module_key.clone(), component_id);
        }
    }
    (state.components, component_by_module)
}

fn ordered_components(
    components: &[Vec<String>],
    component_by_module: &BTreeMap<String, usize>,
    module_graph: &BTreeMap<String, BTreeSet<String>>,
    depth_by_module: &BTreeMap<String, i32>,
    fallback_depth: i32,
) -> Vec<usize> {
    let mut component_graph = (0..components.len())
        .map(|component_id| (component_id, BTreeSet::new()))
        .collect::<BTreeMap<_, _>>();
    let mut incoming_counts = (0..components.len())
        .map(|component_id| (component_id, 0usize))
        .collect::<BTreeMap<_, _>>();

    for (from_module, targets) in module_graph {
        let Some(from_component) = component_by_module.get(from_module).copied() else {
            continue;
        };
        for target in targets {
            let Some(to_component) = component_by_module.get(target).copied() else {
                continue;
            };
            if from_component == to_component {
                continue;
            }
            if component_graph
                .entry(from_component)
                .or_default()
                .insert(to_component)
            {
                *incoming_counts.entry(to_component).or_default() += 1;
            }
        }
    }

    let mut ready = BTreeSet::new();
    for component_id in 0..components.len() {
        if incoming_counts
            .get(&component_id)
            .copied()
            .unwrap_or_default()
            == 0
        {
            ready.insert(component_sort_key(
                component_id,
                components,
                depth_by_module,
                fallback_depth,
            ));
        }
    }

    let mut ordered = Vec::new();
    while let Some(key) = ready.pop_first() {
        let component_id = key.2;
        ordered.push(component_id);
        for target in component_graph
            .get(&component_id)
            .cloned()
            .unwrap_or_default()
        {
            let Some(count) = incoming_counts.get_mut(&target) else {
                continue;
            };
            *count = count.saturating_sub(1);
            if *count == 0 {
                ready.insert(component_sort_key(
                    target,
                    components,
                    depth_by_module,
                    fallback_depth,
                ));
            }
        }
    }
    ordered
}

fn component_sort_key(
    component_id: usize,
    components: &[Vec<String>],
    depth_by_module: &BTreeMap<String, i32>,
    fallback_depth: i32,
) -> (i32, String, usize) {
    let component = components
        .get(component_id)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let depth = component
        .iter()
        .map(|module_key| flow_sort_depth(depth_by_module, module_key, fallback_depth))
        .min()
        .unwrap_or(fallback_depth);
    let key = component.first().cloned().unwrap_or_default();
    (depth, key, component_id)
}

fn cyclic_components(
    components: &[Vec<String>],
    component_by_module: &BTreeMap<String, usize>,
    module_graph: &BTreeMap<String, BTreeSet<String>>,
    self_loop_modules: &BTreeSet<String>,
) -> Vec<bool> {
    let mut cyclic = components
        .iter()
        .map(|component| component.len() > 1)
        .collect::<Vec<_>>();
    for module_key in self_loop_modules {
        if let Some(component_id) = component_by_module.get(module_key).copied()
            && let Some(component_cyclic) = cyclic.get_mut(component_id)
        {
            *component_cyclic = true;
        }
    }
    for (module_key, targets) in module_graph {
        if targets.contains(module_key)
            && let Some(component_id) = component_by_module.get(module_key).copied()
            && let Some(component_cyclic) = cyclic.get_mut(component_id)
        {
            *component_cyclic = true;
        }
    }
    cyclic
}

fn systems_from_repo_graph(
    repository: ArchitectureGraphRepositoryRef,
    graph: ArchitectureGraph,
    system_key_filter: Option<&str>,
) -> BTreeMap<String, ArchitectureSystem> {
    let nodes_by_id = graph
        .nodes
        .into_iter()
        .map(|node| (node.id.clone(), node))
        .collect::<BTreeMap<_, _>>();
    let mut system_ids_by_container = BTreeMap::<String, Vec<String>>::new();
    let mut components_by_container = BTreeMap::<String, Vec<String>>::new();
    let mut deployments_by_container = BTreeMap::<String, Vec<String>>::new();
    let mut entry_points_by_container = BTreeMap::<String, Vec<String>>::new();
    for edge in graph.edges {
        match edge.kind {
            ArchitectureGraphEdgeKind::Contains => {
                let Some(from) = nodes_by_id.get(&edge.from_node_id) else {
                    continue;
                };
                let Some(to) = nodes_by_id.get(&edge.to_node_id) else {
                    continue;
                };
                match (from.kind, to.kind) {
                    (ArchitectureGraphNodeKind::System, ArchitectureGraphNodeKind::Container) => {
                        system_ids_by_container
                            .entry(to.id.clone())
                            .or_default()
                            .push(from.id.clone());
                    }
                    (
                        ArchitectureGraphNodeKind::Container,
                        ArchitectureGraphNodeKind::Component,
                    ) => {
                        components_by_container
                            .entry(from.id.clone())
                            .or_default()
                            .push(to.id.clone());
                    }
                    _ => {}
                }
            }
            ArchitectureGraphEdgeKind::Realises => {
                let Some(from) = nodes_by_id.get(&edge.from_node_id) else {
                    continue;
                };
                let Some(to) = nodes_by_id.get(&edge.to_node_id) else {
                    continue;
                };
                if from.kind == ArchitectureGraphNodeKind::DeploymentUnit
                    && to.kind == ArchitectureGraphNodeKind::Container
                {
                    deployments_by_container
                        .entry(to.id.clone())
                        .or_default()
                        .push(from.id.clone());
                }
            }
            ArchitectureGraphEdgeKind::Exposes => {
                let Some(from) = nodes_by_id.get(&edge.from_node_id) else {
                    continue;
                };
                let Some(to) = nodes_by_id.get(&edge.to_node_id) else {
                    continue;
                };
                if from.kind == ArchitectureGraphNodeKind::Container
                    && to.kind == ArchitectureGraphNodeKind::EntryPoint
                {
                    entry_points_by_container
                        .entry(from.id.clone())
                        .or_default()
                        .push(to.id.clone());
                }
            }
            _ => {}
        }
    }

    let mut systems = BTreeMap::<String, ArchitectureSystem>::new();
    for container in nodes_by_id
        .values()
        .filter(|node| node.kind == ArchitectureGraphNodeKind::Container)
    {
        let Some(system_ids) = system_ids_by_container.get(&container.id) else {
            continue;
        };
        for system_id in system_ids {
            let Some(system) = nodes_by_id.get(system_id) else {
                continue;
            };
            let system_key =
                property_string(system, "system_key").unwrap_or_else(|| system.id.clone());
            if system_key_filter.is_some_and(|filter| filter != system_key) {
                continue;
            }
            let architecture_container = ArchitectureContainer {
                id: container.id.clone(),
                key: property_string(container, "container_key"),
                kind: property_string(container, "container_kind"),
                label: container.label.clone(),
                repository: repository.clone(),
                node: container.clone(),
                components: collect_nodes(&nodes_by_id, components_by_container.get(&container.id)),
                deployment_units: collect_nodes(
                    &nodes_by_id,
                    deployments_by_container.get(&container.id),
                ),
                entry_points: collect_nodes(
                    &nodes_by_id,
                    entry_points_by_container.get(&container.id),
                ),
            };
            systems
                .entry(system_key.clone())
                .and_modify(|existing| {
                    merge_repository_refs(
                        &mut existing.repositories,
                        std::slice::from_ref(&repository),
                    );
                    if !existing
                        .containers
                        .iter()
                        .any(|existing| existing.id == architecture_container.id)
                    {
                        existing.containers.push(architecture_container.clone());
                    }
                    sort_containers(&mut existing.containers);
                })
                .or_insert_with(|| ArchitectureSystem {
                    id: system.id.clone(),
                    key: system_key,
                    label: system.label.clone(),
                    repositories: vec![repository.clone()],
                    containers: vec![architecture_container],
                    node: system.clone(),
                });
        }
    }
    systems
}

fn collect_nodes(
    nodes_by_id: &BTreeMap<String, ArchitectureGraphNode>,
    ids: Option<&Vec<String>>,
) -> Vec<ArchitectureGraphNode> {
    let mut nodes = ids
        .into_iter()
        .flat_map(|ids| ids.iter())
        .filter_map(|id| nodes_by_id.get(id).cloned())
        .collect::<Vec<_>>();
    nodes.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| left.label.cmp(&right.label))
            .then_with(|| left.id.cmp(&right.id))
    });
    nodes
}

fn property_string(node: &ArchitectureGraphNode, key: &str) -> Option<String> {
    node.properties
        .0
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn repository_ref(repository: &SelectedRepository) -> ArchitectureGraphRepositoryRef {
    ArchitectureGraphRepositoryRef {
        repo_id: repository.repo_id().to_string(),
        name: repository.name().to_string(),
        provider: repository.provider().to_string(),
        organization: repository.organization().to_string(),
    }
}

fn merge_repository_refs(
    target: &mut Vec<ArchitectureGraphRepositoryRef>,
    source: &[ArchitectureGraphRepositoryRef],
) {
    for repository in source {
        if !target
            .iter()
            .any(|existing| existing.repo_id == repository.repo_id)
        {
            target.push(repository.clone());
        }
    }
    target.sort_by(|left, right| {
        left.repo_id
            .cmp(&right.repo_id)
            .then_with(|| left.name.cmp(&right.name))
    });
}

fn sort_containers(containers: &mut [ArchitectureContainer]) {
    containers.sort_by(|left, right| {
        left.repository
            .repo_id
            .cmp(&right.repository.repo_id)
            .then_with(|| left.label.cmp(&right.label))
            .then_with(|| left.id.cmp(&right.id))
    });
}

fn is_missing_architecture_graph_table_error(err: &anyhow::Error) -> bool {
    let message = err.to_string();
    message.contains("architecture_graph_nodes_current")
        || message.contains("architecture_graph_edges_current")
        || message.contains("architecture_graph_assertions")
}

async fn load_computed_nodes(
    context: &DevqlGraphqlContext,
    repo_id: &str,
    scope: &ResolverScope,
    filter: Option<&ArchitectureGraphFilterInput>,
) -> Result<BTreeMap<String, ArchitectureGraphNode>> {
    let mut clauses = vec![format!("repo_id = {}", sql_text(repo_id))];
    if let Some(node_kind) = filter.and_then(|filter| filter.node_kind) {
        clauses.push(format!("node_kind = {}", sql_text(node_kind.as_db())));
    }
    if let Some(source_kind) = filter.and_then(|filter| filter.source_kind.as_deref()) {
        clauses.push(format!("source_kind = {}", sql_text(source_kind)));
    }
    let sql = format!(
        "SELECT node_id, node_kind, label, artefact_id, symbol_id, path, entry_kind, source_kind, confidence, \
                provenance_json, evidence_json, properties_json \
         FROM architecture_graph_nodes_current WHERE {} ORDER BY node_id",
        clauses.join(" AND ")
    );
    let rows = context.query_devql_sqlite_rows(&sql).await?;
    let mut nodes = BTreeMap::new();
    for row in rows {
        let node = node_from_row(&row)?;
        if !node_path_in_scope(&node, scope, filter) {
            continue;
        }
        nodes.insert(node.id.clone(), node);
    }
    Ok(nodes)
}

async fn load_computed_edges(
    context: &DevqlGraphqlContext,
    repo_id: &str,
    filter: Option<&ArchitectureGraphFilterInput>,
) -> Result<BTreeMap<String, ArchitectureGraphEdge>> {
    let mut clauses = vec![format!("repo_id = {}", sql_text(repo_id))];
    if let Some(edge_kind) = filter.and_then(|filter| filter.edge_kind) {
        clauses.push(format!("edge_kind = {}", sql_text(edge_kind.as_db())));
    }
    if let Some(source_kind) = filter.and_then(|filter| filter.source_kind.as_deref()) {
        clauses.push(format!("source_kind = {}", sql_text(source_kind)));
    }
    let sql = format!(
        "SELECT edge_id, edge_kind, from_node_id, to_node_id, source_kind, confidence, \
                provenance_json, evidence_json, properties_json \
         FROM architecture_graph_edges_current WHERE {} ORDER BY edge_id",
        clauses.join(" AND ")
    );
    let rows = context.query_devql_sqlite_rows(&sql).await?;
    let mut edges = BTreeMap::new();
    for row in rows {
        let edge = edge_from_row(&row)?;
        edges.insert(edge.id.clone(), edge);
    }
    Ok(edges)
}

async fn load_assertions(
    context: &DevqlGraphqlContext,
    repo_id: &str,
) -> Result<Vec<AssertionRecord>> {
    let sql = format!(
        "SELECT assertion_id, action, target_kind, node_id, node_kind, edge_id, edge_kind, \
                from_node_id, to_node_id, label, artefact_id, symbol_id, path, entry_kind, \
                reason, source, confidence, provenance_json, evidence_json, properties_json \
         FROM architecture_graph_assertions \
         WHERE repo_id = {} AND revoked_at IS NULL ORDER BY created_at ASC, assertion_id ASC",
        sql_text(repo_id)
    );
    context
        .query_devql_sqlite_rows(&sql)
        .await?
        .into_iter()
        .map(|row| assertion_from_row(&row))
        .collect()
}

fn apply_assertions(
    nodes: &mut BTreeMap<String, ArchitectureGraphNode>,
    edges: &mut BTreeMap<String, ArchitectureGraphEdge>,
    assertions: Vec<AssertionRecord>,
) {
    for assertion in assertions {
        match (assertion.action, assertion.target_kind) {
            (ArchitectureGraphAssertionAction::Suppress, ArchitectureGraphTargetKind::Node) => {
                if let Some(node_id) = assertion.node_id.as_ref()
                    && let Some(node) = nodes.get_mut(node_id)
                {
                    node.suppressed = true;
                    node.effective = false;
                    node.annotations.push(assertion.summary());
                }
            }
            (ArchitectureGraphAssertionAction::Suppress, ArchitectureGraphTargetKind::Edge) => {
                if let Some(edge_id) = assertion.edge_id.as_ref()
                    && let Some(edge) = edges.get_mut(edge_id)
                {
                    edge.suppressed = true;
                    edge.effective = false;
                    edge.annotations.push(assertion.summary());
                }
            }
            (ArchitectureGraphAssertionAction::Annotate, ArchitectureGraphTargetKind::Node) => {
                if let Some(node_id) = assertion.node_id.as_ref()
                    && let Some(node) = nodes.get_mut(node_id)
                {
                    node.annotations.push(assertion.summary());
                }
            }
            (ArchitectureGraphAssertionAction::Annotate, ArchitectureGraphTargetKind::Edge) => {
                if let Some(edge_id) = assertion.edge_id.as_ref()
                    && let Some(edge) = edges.get_mut(edge_id)
                {
                    edge.annotations.push(assertion.summary());
                }
            }
            (ArchitectureGraphAssertionAction::Assert, ArchitectureGraphTargetKind::Node) => {
                apply_node_assertion(nodes, assertion);
            }
            (ArchitectureGraphAssertionAction::Assert, ArchitectureGraphTargetKind::Edge) => {
                apply_edge_assertion(edges, assertion);
            }
        }
    }
}

fn apply_node_assertion(
    nodes: &mut BTreeMap<String, ArchitectureGraphNode>,
    assertion: AssertionRecord,
) {
    let Some(node_id) = assertion.node_id.clone() else {
        return;
    };
    if let Some(node) = nodes.get_mut(&node_id) {
        node.asserted = true;
        node.asserted_provenance = Json(assertion.provenance.clone());
        node.provenance = Json(merge_provenance(
            &node.computed_provenance.0,
            &assertion.provenance,
        ));
        node.annotations.push(assertion.summary());
        return;
    }
    let Some(kind) = assertion.node_kind else {
        return;
    };
    let label = assertion
        .label
        .clone()
        .or_else(|| assertion.path.clone())
        .or_else(|| assertion.artefact_id.clone())
        .unwrap_or_else(|| node_id.clone());
    nodes.insert(
        node_id.clone(),
        ArchitectureGraphNode::assertion(
            node_id,
            kind,
            label,
            assertion.artefact_id,
            assertion.symbol_id,
            assertion.path,
            assertion.entry_kind,
            assertion.source.clone(),
            assertion.confidence.unwrap_or(0.85),
            assertion.provenance,
            assertion.evidence,
            assertion.properties,
        ),
    );
}

fn apply_edge_assertion(
    edges: &mut BTreeMap<String, ArchitectureGraphEdge>,
    assertion: AssertionRecord,
) {
    let Some(edge_id) = assertion.edge_id.clone() else {
        return;
    };
    if let Some(edge) = edges.get_mut(&edge_id) {
        edge.asserted = true;
        edge.asserted_provenance = Json(assertion.provenance.clone());
        edge.provenance = Json(merge_provenance(
            &edge.computed_provenance.0,
            &assertion.provenance,
        ));
        edge.annotations.push(assertion.summary());
        return;
    }
    let (Some(kind), Some(from_node_id), Some(to_node_id)) = (
        assertion.edge_kind,
        assertion.from_node_id.clone(),
        assertion.to_node_id.clone(),
    ) else {
        return;
    };
    edges.insert(
        edge_id.clone(),
        ArchitectureGraphEdge::assertion(
            edge_id,
            kind,
            from_node_id,
            to_node_id,
            assertion.source.clone(),
            assertion.confidence.unwrap_or(0.85),
            assertion.provenance,
            assertion.evidence,
            assertion.properties,
        ),
    );
}

fn node_from_row(row: &Value) -> Result<ArchitectureGraphNode> {
    let id = required_string(row, "node_id")?;
    let kind = ArchitectureGraphNodeKind::from_db(&required_string(row, "node_kind")?)
        .with_context(|| format!("unknown architecture graph node kind for `{id}`"))?;
    let provenance = json_column(row, "provenance_json")?;
    Ok(ArchitectureGraphNode {
        id,
        kind,
        label: required_string(row, "label")?,
        artefact_id: optional_string(row, "artefact_id"),
        symbol_id: optional_string(row, "symbol_id"),
        path: optional_string(row, "path"),
        entry_kind: optional_string(row, "entry_kind"),
        source_kind: required_string(row, "source_kind")?,
        confidence: number_field(row, "confidence").unwrap_or(1.0),
        computed: true,
        asserted: false,
        suppressed: false,
        effective: true,
        provenance: Json(provenance.clone()),
        computed_provenance: Json(provenance),
        asserted_provenance: Json(Value::Null),
        evidence: Json(json_column(row, "evidence_json")?),
        properties: Json(json_column(row, "properties_json")?),
        annotations: Vec::new(),
    })
}

fn edge_from_row(row: &Value) -> Result<ArchitectureGraphEdge> {
    let id = required_string(row, "edge_id")?;
    let kind = ArchitectureGraphEdgeKind::from_db(&required_string(row, "edge_kind")?)
        .with_context(|| format!("unknown architecture graph edge kind for `{id}`"))?;
    let provenance = json_column(row, "provenance_json")?;
    Ok(ArchitectureGraphEdge {
        id,
        kind,
        from_node_id: required_string(row, "from_node_id")?,
        to_node_id: required_string(row, "to_node_id")?,
        source_kind: required_string(row, "source_kind")?,
        confidence: number_field(row, "confidence").unwrap_or(1.0),
        computed: true,
        asserted: false,
        suppressed: false,
        effective: true,
        provenance: Json(provenance.clone()),
        computed_provenance: Json(provenance),
        asserted_provenance: Json(Value::Null),
        evidence: Json(json_column(row, "evidence_json")?),
        properties: Json(json_column(row, "properties_json")?),
        annotations: Vec::new(),
    })
}

fn assertion_from_row(row: &Value) -> Result<AssertionRecord> {
    let action = ArchitectureGraphAssertionAction::from_db(&required_string(row, "action")?)
        .context("unknown architecture graph assertion action")?;
    let target_kind = ArchitectureGraphTargetKind::from_db(&required_string(row, "target_kind")?)
        .context("unknown architecture graph assertion target kind")?;
    Ok(AssertionRecord {
        id: required_string(row, "assertion_id")?,
        action,
        target_kind,
        node_id: optional_string(row, "node_id"),
        node_kind: optional_string(row, "node_kind")
            .and_then(|kind| ArchitectureGraphNodeKind::from_db(&kind)),
        edge_id: optional_string(row, "edge_id"),
        edge_kind: optional_string(row, "edge_kind")
            .and_then(|kind| ArchitectureGraphEdgeKind::from_db(&kind)),
        from_node_id: optional_string(row, "from_node_id"),
        to_node_id: optional_string(row, "to_node_id"),
        label: optional_string(row, "label"),
        artefact_id: optional_string(row, "artefact_id"),
        symbol_id: optional_string(row, "symbol_id"),
        path: optional_string(row, "path"),
        entry_kind: optional_string(row, "entry_kind"),
        reason: required_string(row, "reason")?,
        source: required_string(row, "source")?,
        confidence: number_field(row, "confidence"),
        provenance: json_column(row, "provenance_json")?,
        evidence: json_column(row, "evidence_json")?,
        properties: json_column(row, "properties_json")?,
    })
}

#[derive(Debug, Clone)]
struct AssertionRecord {
    id: String,
    action: ArchitectureGraphAssertionAction,
    target_kind: ArchitectureGraphTargetKind,
    node_id: Option<String>,
    node_kind: Option<ArchitectureGraphNodeKind>,
    edge_id: Option<String>,
    edge_kind: Option<ArchitectureGraphEdgeKind>,
    from_node_id: Option<String>,
    to_node_id: Option<String>,
    label: Option<String>,
    artefact_id: Option<String>,
    symbol_id: Option<String>,
    path: Option<String>,
    entry_kind: Option<String>,
    reason: String,
    source: String,
    confidence: Option<f64>,
    provenance: Value,
    evidence: Value,
    properties: Value,
}

impl AssertionRecord {
    fn summary(&self) -> ArchitectureGraphAssertionSummary {
        ArchitectureGraphAssertionSummary {
            id: self.id.clone(),
            action: self.action,
            target_kind: self.target_kind,
            reason: self.reason.clone(),
            source: self.source.clone(),
            provenance: Json(self.provenance.clone()),
            evidence: Json(self.evidence.clone()),
            properties: Json(self.properties.clone()),
        }
    }
}

fn node_path_in_scope(
    node: &ArchitectureGraphNode,
    scope: &ResolverScope,
    filter: Option<&ArchitectureGraphFilterInput>,
) -> bool {
    let Some(path) = node.path.as_deref() else {
        return filter.and_then(|filter| filter.path.as_deref()).is_none();
    };
    if !scope.contains_repo_path(path) {
        return false;
    }
    let Some(filter_path) = filter.and_then(|filter| filter.path.as_deref()) else {
        return true;
    };
    path == filter_path
        || path
            .strip_prefix(filter_path)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn merge_provenance(computed: &Value, asserted: &Value) -> Value {
    serde_json::json!({
        "computed": computed,
        "asserted": asserted,
    })
}

fn required_string(row: &Value, key: &str) -> Result<String> {
    optional_string(row, key).ok_or_else(|| anyhow!("missing `{key}` in architecture graph row"))
}

fn optional_string(row: &Value, key: &str) -> Option<String> {
    row.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn number_field(row: &Value, key: &str) -> Option<f64> {
    row.get(key).and_then(Value::as_f64)
}

fn json_column(row: &Value, key: &str) -> Result<Value> {
    match row.get(key) {
        Some(Value::String(raw)) => {
            serde_json::from_str(raw).with_context(|| format!("parsing `{key}` JSON"))
        }
        Some(value) => Ok(value.clone()),
        None => Ok(Value::Null),
    }
}

fn sql_text(value: &str) -> String {
    format!("'{}'", esc_pg(value))
}

fn graph_count(value: usize) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn computed_node(id: &str) -> ArchitectureGraphNode {
        ArchitectureGraphNode {
            id: id.to_string(),
            kind: ArchitectureGraphNodeKind::Node,
            label: id.to_string(),
            artefact_id: Some(format!("{id}-artefact")),
            symbol_id: Some(format!("{id}-symbol")),
            path: Some("src/main.rs".to_string()),
            entry_kind: None,
            source_kind: "COMPUTED".to_string(),
            confidence: 1.0,
            computed: true,
            asserted: false,
            suppressed: false,
            effective: true,
            provenance: Json(serde_json::json!({ "computed": true })),
            computed_provenance: Json(serde_json::json!({ "computed": true })),
            asserted_provenance: Json(Value::Null),
            evidence: Json(serde_json::json!([])),
            properties: Json(serde_json::json!({})),
            annotations: Vec::new(),
        }
    }

    fn flow_code_node(id: &str, path: &str, artefact_id: &str) -> ArchitectureGraphNode {
        let mut node = computed_node(id);
        node.path = Some(path.to_string());
        node.artefact_id = Some(artefact_id.to_string());
        node
    }

    fn flow_entry_point(id: &str, path: &str, artefact_id: &str) -> ArchitectureGraphNode {
        let mut node = graph_node(id, ArchitectureGraphNodeKind::EntryPoint, id, Value::Null);
        node.path = Some(path.to_string());
        node.artefact_id = Some(artefact_id.to_string());
        node
    }

    fn graph_node(
        id: &str,
        kind: ArchitectureGraphNodeKind,
        label: &str,
        properties: Value,
    ) -> ArchitectureGraphNode {
        ArchitectureGraphNode {
            id: id.to_string(),
            kind,
            label: label.to_string(),
            artefact_id: None,
            symbol_id: None,
            path: None,
            entry_kind: None,
            source_kind: "COMPUTED".to_string(),
            confidence: 1.0,
            computed: true,
            asserted: false,
            suppressed: false,
            effective: true,
            provenance: Json(serde_json::json!({})),
            computed_provenance: Json(serde_json::json!({})),
            asserted_provenance: Json(Value::Null),
            evidence: Json(serde_json::json!([])),
            properties: Json(properties),
            annotations: Vec::new(),
        }
    }

    fn graph_edge(
        id: &str,
        kind: ArchitectureGraphEdgeKind,
        from_node_id: &str,
        to_node_id: &str,
    ) -> ArchitectureGraphEdge {
        ArchitectureGraphEdge {
            id: id.to_string(),
            kind,
            from_node_id: from_node_id.to_string(),
            to_node_id: to_node_id.to_string(),
            source_kind: "COMPUTED".to_string(),
            confidence: 1.0,
            computed: true,
            asserted: false,
            suppressed: false,
            effective: true,
            provenance: Json(serde_json::json!({})),
            computed_provenance: Json(serde_json::json!({})),
            asserted_provenance: Json(Value::Null),
            evidence: Json(serde_json::json!([])),
            properties: Json(serde_json::json!({})),
            annotations: Vec::new(),
        }
    }

    #[test]
    fn flow_steps_order_modules_from_entry_point_dependencies() {
        let entry = flow_entry_point("entry", "src/main.rs", "main-artefact");
        let traversed_nodes = vec![
            flow_code_node("main", "src/main.rs", "main-artefact"),
            flow_code_node("service", "src/service.rs", "service-artefact"),
            flow_code_node("repo", "src/repo.rs", "repo-artefact"),
        ];
        let edges = vec![
            graph_edge(
                "main-service",
                ArchitectureGraphEdgeKind::DependsOn,
                "main",
                "service",
            ),
            graph_edge(
                "service-repo",
                ArchitectureGraphEdgeKind::DependsOn,
                "service",
                "repo",
            ),
        ];

        let steps = flow_steps_for_entry(&entry, &traversed_nodes, &edges);

        assert_eq!(
            steps
                .iter()
                .map(|step| step.module_key.as_str())
                .collect::<Vec<_>>(),
            vec!["src/main.rs", "src/service.rs", "src/repo.rs"]
        );
        assert_eq!(
            steps.iter().map(|step| step.depth).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert!(steps[0].predecessor_module_keys.is_empty());
        assert_eq!(
            steps[1].predecessor_module_keys,
            vec!["src/main.rs".to_string()]
        );
        assert_eq!(
            steps[1].edge_kinds,
            vec![ArchitectureGraphEdgeKind::DependsOn]
        );
        assert!(steps.iter().all(|step| !step.cyclic));
    }

    #[test]
    fn flow_steps_flag_cyclic_module_groups() {
        let entry = flow_entry_point("entry", "src/a.rs", "a-artefact");
        let traversed_nodes = vec![
            flow_code_node("a", "src/a.rs", "a-artefact"),
            flow_code_node("b", "src/b.rs", "b-artefact"),
        ];
        let edges = vec![
            graph_edge("a-b", ArchitectureGraphEdgeKind::DependsOn, "a", "b"),
            graph_edge("b-a", ArchitectureGraphEdgeKind::DependsOn, "b", "a"),
        ];

        let steps = flow_steps_for_entry(&entry, &traversed_nodes, &edges);

        assert_eq!(
            steps
                .iter()
                .map(|step| step.module_key.as_str())
                .collect::<Vec<_>>(),
            vec!["src/a.rs", "src/b.rs"]
        );
        assert!(steps.iter().all(|step| step.cyclic));
        assert_eq!(
            steps[0].predecessor_module_keys,
            vec!["src/b.rs".to_string()]
        );
        assert_eq!(
            steps[1].predecessor_module_keys,
            vec!["src/a.rs".to_string()]
        );
    }

    fn assertion(
        action: ArchitectureGraphAssertionAction,
        target_kind: ArchitectureGraphTargetKind,
    ) -> AssertionRecord {
        AssertionRecord {
            id: format!("{action:?}-{target_kind:?}"),
            action,
            target_kind,
            node_id: Some("node-1".to_string()),
            node_kind: Some(ArchitectureGraphNodeKind::Node),
            edge_id: None,
            edge_kind: None,
            from_node_id: None,
            to_node_id: None,
            label: Some("Manual node".to_string()),
            artefact_id: None,
            symbol_id: None,
            path: Some("src/manual.rs".to_string()),
            entry_kind: None,
            reason: "manual correction".to_string(),
            source: "test".to_string(),
            confidence: Some(0.7),
            provenance: serde_json::json!({ "asserted": true }),
            evidence: serde_json::json!([]),
            properties: serde_json::json!({ "note": "manual" }),
        }
    }

    #[test]
    fn suppression_marks_computed_node_ineffective_and_keeps_provenance() {
        let mut nodes = BTreeMap::from([("node-1".to_string(), computed_node("node-1"))]);
        let mut edges = BTreeMap::new();

        apply_assertions(
            &mut nodes,
            &mut edges,
            vec![assertion(
                ArchitectureGraphAssertionAction::Suppress,
                ArchitectureGraphTargetKind::Node,
            )],
        );

        let node = nodes.get("node-1").unwrap();
        assert!(node.suppressed);
        assert!(!node.effective);
        assert_eq!(node.annotations.len(), 1);
        assert_eq!(node.computed_provenance.0["computed"], true);
    }

    #[test]
    fn assert_adds_manual_node_when_computed_fact_is_absent() {
        let mut nodes = BTreeMap::new();
        let mut edges = BTreeMap::new();

        apply_assertions(
            &mut nodes,
            &mut edges,
            vec![assertion(
                ArchitectureGraphAssertionAction::Assert,
                ArchitectureGraphTargetKind::Node,
            )],
        );

        let node = nodes.get("node-1").unwrap();
        assert!(node.asserted);
        assert!(node.effective);
        assert_eq!(node.label, "Manual node");
        assert_eq!(node.asserted_provenance.0["asserted"], true);
    }

    #[test]
    fn annotate_enriches_existing_fact_without_changing_effectiveness() {
        let mut nodes = BTreeMap::from([("node-1".to_string(), computed_node("node-1"))]);
        let mut edges = BTreeMap::new();

        apply_assertions(
            &mut nodes,
            &mut edges,
            vec![assertion(
                ArchitectureGraphAssertionAction::Annotate,
                ArchitectureGraphTargetKind::Node,
            )],
        );

        let node = nodes.get("node-1").unwrap();
        assert!(node.computed);
        assert!(node.effective);
        assert_eq!(node.annotations.len(), 1);
    }

    #[test]
    fn c4_projection_groups_containers_under_system_key() {
        let repo = ArchitectureGraphRepositoryRef {
            repo_id: "repo".to_string(),
            name: "repo".to_string(),
            provider: "local".to_string(),
            organization: "local".to_string(),
        };
        let graph = ArchitectureGraph {
            nodes: vec![
                graph_node(
                    "system-1",
                    ArchitectureGraphNodeKind::System,
                    "Platform",
                    serde_json::json!({ "system_key": "bitloops.platform" }),
                ),
                graph_node(
                    "container-1",
                    ArchitectureGraphNodeKind::Container,
                    "CLI",
                    serde_json::json!({
                        "container_key": "cli",
                        "container_kind": "cli",
                    }),
                ),
                graph_node(
                    "deployment-1",
                    ArchitectureGraphNodeKind::DeploymentUnit,
                    "CLI deployment",
                    serde_json::json!({ "deployment_kind": "cargo_bin" }),
                ),
                graph_node(
                    "component-1",
                    ArchitectureGraphNodeKind::Component,
                    "runtime",
                    serde_json::json!({ "component_key": "src/runtime" }),
                ),
                graph_node(
                    "entry-1",
                    ArchitectureGraphNodeKind::EntryPoint,
                    "main",
                    serde_json::json!({}),
                ),
            ],
            edges: vec![
                graph_edge(
                    "contains-container",
                    ArchitectureGraphEdgeKind::Contains,
                    "system-1",
                    "container-1",
                ),
                graph_edge(
                    "realises",
                    ArchitectureGraphEdgeKind::Realises,
                    "deployment-1",
                    "container-1",
                ),
                graph_edge(
                    "contains-component",
                    ArchitectureGraphEdgeKind::Contains,
                    "container-1",
                    "component-1",
                ),
                graph_edge(
                    "exposes",
                    ArchitectureGraphEdgeKind::Exposes,
                    "container-1",
                    "entry-1",
                ),
            ],
            total_nodes: 5,
            total_edges: 4,
        };

        let systems = systems_from_repo_graph(repo, graph, Some("bitloops.platform"));
        let system = systems.get("bitloops.platform").expect("system");

        assert_eq!(system.containers.len(), 1);
        let container = &system.containers[0];
        assert_eq!(container.key.as_deref(), Some("cli"));
        assert_eq!(container.deployment_units.len(), 1);
        assert_eq!(container.components.len(), 1);
        assert_eq!(container.entry_points.len(), 1);
    }

    #[test]
    fn c4_projection_keeps_multiple_system_memberships_for_container() {
        let repo = ArchitectureGraphRepositoryRef {
            repo_id: "repo".to_string(),
            name: "repo".to_string(),
            provider: "local".to_string(),
            organization: "local".to_string(),
        };
        let graph = ArchitectureGraph {
            nodes: vec![
                graph_node(
                    "fallback-system",
                    ArchitectureGraphNodeKind::System,
                    "Repository system",
                    serde_json::json!({ "system_key": "repo:repo" }),
                ),
                graph_node(
                    "shared-system",
                    ArchitectureGraphNodeKind::System,
                    "Shared Platform",
                    serde_json::json!({ "system_key": "bitloops.platform" }),
                ),
                graph_node(
                    "container-1",
                    ArchitectureGraphNodeKind::Container,
                    "CLI",
                    serde_json::json!({
                        "container_key": "cli",
                        "container_kind": "cli",
                    }),
                ),
            ],
            edges: vec![
                graph_edge(
                    "fallback-contains",
                    ArchitectureGraphEdgeKind::Contains,
                    "fallback-system",
                    "container-1",
                ),
                graph_edge(
                    "shared-contains",
                    ArchitectureGraphEdgeKind::Contains,
                    "shared-system",
                    "container-1",
                ),
            ],
            total_nodes: 3,
            total_edges: 2,
        };

        let systems = systems_from_repo_graph(repo, graph, Some("bitloops.platform"));
        let system = systems.get("bitloops.platform").expect("shared system");

        assert_eq!(system.containers.len(), 1);
        assert_eq!(system.containers[0].id, "container-1");
    }
}

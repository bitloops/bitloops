use super::*;

pub(super) fn flow_steps_for_entry(
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
                .filter(|(_, incoming)| incoming.is_empty())
                .map(|(module_key, _)| module_key.clone()),
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

pub(super) fn is_flow_sequence_edge(kind: ArchitectureGraphEdgeKind) -> bool {
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

pub(super) fn flow_module_key(node: &ArchitectureGraphNode) -> String {
    node.path
        .clone()
        .or_else(|| property_string(node, "module_key"))
        .or_else(|| property_string(node, "component_key"))
        .unwrap_or_else(|| node.id.clone())
}

pub(super) fn sort_nodes_for_display(nodes: &mut [ArchitectureGraphNode]) {
    nodes.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| left.label.cmp(&right.label))
            .then_with(|| left.id.cmp(&right.id))
    });
}

pub(super) fn start_module_keys(
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

pub(super) fn module_depths(
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

pub(super) fn flow_sort_depth(
    depth_by_module: &BTreeMap<String, i32>,
    module_key: &str,
    fallback_depth: i32,
) -> i32 {
    depth_by_module
        .get(module_key)
        .copied()
        .unwrap_or(fallback_depth)
}

pub(super) fn strongly_connected_modules(
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

pub(super) fn ordered_components(
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

pub(super) fn component_sort_key(
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

pub(super) fn cyclic_components(
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

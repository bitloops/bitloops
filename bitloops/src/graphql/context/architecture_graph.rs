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

mod flows;
mod storage;
mod systems;

use flows::*;
use storage::*;
use systems::*;

#[derive(Debug, Clone)]
pub(crate) struct ArchitectureGraphTargetOverview {
    pub(crate) available: bool,
    pub(crate) reason: Option<String>,
    pub(crate) selected_artefact_count: usize,
    pub(crate) matched_artefact_ids: Vec<String>,
    pub(crate) direct_node_count: usize,
    pub(crate) nodes: Vec<ArchitectureGraphNode>,
    pub(crate) edges: Vec<ArchitectureGraphEdge>,
}

impl ArchitectureGraphTargetOverview {
    pub(crate) fn unavailable(selected_artefact_count: usize, reason: &str) -> Self {
        Self {
            available: false,
            reason: Some(reason.to_string()),
            selected_artefact_count,
            matched_artefact_ids: Vec::new(),
            direct_node_count: 0,
            nodes: Vec::new(),
            edges: Vec::new(),
        }
    }
}

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

        if filter.is_none_or(|filter| filter.effective_only) {
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
                kind.is_none_or(|kind| {
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

    pub(crate) async fn architecture_overview_for_targets(
        &self,
        scope: &ResolverScope,
        artefact_ids: &[String],
        symbol_ids: &[String],
        paths: &[String],
    ) -> Result<ArchitectureGraphTargetOverview> {
        if scope.temporal_scope().is_some() {
            return Ok(ArchitectureGraphTargetOverview::unavailable(
                artefact_ids.len(),
                "unsupported_scope",
            ));
        }

        let graph = match self.list_architecture_graph(scope, None, None, None).await {
            Ok(graph) => graph,
            Err(err) if is_missing_architecture_graph_table_error(&err) => {
                return Ok(ArchitectureGraphTargetOverview::unavailable(
                    artefact_ids.len(),
                    "missing_architecture_graph_tables",
                ));
            }
            Err(err) => return Err(err),
        };

        Ok(architecture_target_overview_from_graph(
            graph,
            artefact_ids,
            symbol_ids,
            paths,
        ))
    }

    pub(crate) async fn architecture_graph_context_available_for_targets(
        &self,
        scope: &ResolverScope,
        artefact_ids: &[String],
        symbol_ids: &[String],
        paths: &[String],
    ) -> Result<bool> {
        if scope.temporal_scope().is_some() {
            return Ok(false);
        }

        let graph = match self.list_architecture_graph(scope, None, None, None).await {
            Ok(graph) => graph,
            Err(err) if is_missing_architecture_graph_table_error(&err) => return Ok(false),
            Err(err) => return Err(err),
        };

        Ok(graph_context_available_from_graph(
            &graph,
            artefact_ids,
            symbol_ids,
            paths,
        ))
    }
}

const ARCHITECTURE_OVERVIEW_RELATED_HOPS: usize = 2;

fn graph_context_available_from_graph(
    graph: &ArchitectureGraph,
    artefact_ids: &[String],
    symbol_ids: &[String],
    paths: &[String],
) -> bool {
    graph.nodes.iter().any(|node| {
        node.artefact_id
            .as_deref()
            .is_some_and(|id| artefact_ids.iter().any(|target| target == id))
            || node
                .symbol_id
                .as_deref()
                .is_some_and(|id| symbol_ids.iter().any(|target| target == id))
            || node
                .path
                .as_deref()
                .is_some_and(|path| paths.iter().any(|target| target == path))
    })
}

fn architecture_target_overview_from_graph(
    graph: ArchitectureGraph,
    artefact_ids: &[String],
    symbol_ids: &[String],
    paths: &[String],
) -> ArchitectureGraphTargetOverview {
    let selected_artefact_count = artefact_ids.len();
    if artefact_ids.is_empty() && symbol_ids.is_empty() && paths.is_empty() {
        return ArchitectureGraphTargetOverview::unavailable(
            selected_artefact_count,
            "empty_selection",
        );
    }

    let artefact_ids = artefact_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let symbol_ids = symbol_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let paths = paths.iter().map(String::as_str).collect::<BTreeSet<_>>();

    let mut nodes_by_id = graph
        .nodes
        .into_iter()
        .map(|node| (node.id.clone(), node))
        .collect::<BTreeMap<_, _>>();
    let mut edges = graph.edges;
    edges.sort_by(|left, right| left.id.cmp(&right.id));

    let mut direct_node_ids = BTreeSet::new();
    let mut matched_artefact_ids = BTreeSet::new();
    for node in nodes_by_id.values() {
        let artefact_match = node
            .artefact_id
            .as_deref()
            .is_some_and(|id| artefact_ids.contains(id));
        let symbol_match = node
            .symbol_id
            .as_deref()
            .is_some_and(|id| symbol_ids.contains(id));
        let path_match = node
            .path
            .as_deref()
            .is_some_and(|path| paths.contains(path));
        if artefact_match || symbol_match || path_match {
            direct_node_ids.insert(node.id.clone());
            if let Some(artefact_id) = node.artefact_id.as_ref()
                && artefact_ids.contains(artefact_id.as_str())
            {
                matched_artefact_ids.insert(artefact_id.clone());
            }
        }
    }

    if direct_node_ids.is_empty() {
        return ArchitectureGraphTargetOverview::unavailable(
            selected_artefact_count,
            "no_matching_architecture_facts",
        );
    }

    let mut included_node_ids = direct_node_ids.clone();
    for _ in 0..ARCHITECTURE_OVERVIEW_RELATED_HOPS {
        let mut next_node_ids = included_node_ids.clone();
        for edge in &edges {
            if included_node_ids.contains(&edge.from_node_id)
                || included_node_ids.contains(&edge.to_node_id)
            {
                next_node_ids.insert(edge.from_node_id.clone());
                next_node_ids.insert(edge.to_node_id.clone());
            }
        }
        if next_node_ids == included_node_ids {
            break;
        }
        included_node_ids = next_node_ids;
    }

    let mut nodes = included_node_ids
        .iter()
        .filter_map(|id| nodes_by_id.remove(id))
        .collect::<Vec<_>>();
    nodes.sort_by(|left, right| left.id.cmp(&right.id));

    let included_node_ids = nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<BTreeSet<_>>();
    let edges = edges
        .into_iter()
        .filter(|edge| {
            included_node_ids.contains(edge.from_node_id.as_str())
                && included_node_ids.contains(edge.to_node_id.as_str())
        })
        .collect::<Vec<_>>();

    ArchitectureGraphTargetOverview {
        available: true,
        reason: None,
        selected_artefact_count,
        matched_artefact_ids: matched_artefact_ids.into_iter().collect(),
        direct_node_count: direct_node_ids.len(),
        nodes,
        edges,
    }
}

#[cfg(test)]
mod tests;

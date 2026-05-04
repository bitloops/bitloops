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

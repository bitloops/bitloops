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
fn graph_context_available_detects_direct_selected_node() {
    let graph = ArchitectureGraph {
        nodes: vec![flow_code_node("code-main", "src/main.rs", "artefact-main")],
        edges: Vec::new(),
        total_nodes: 1,
        total_edges: 0,
    };

    assert!(graph_context_available_from_graph(
        &graph,
        &["artefact-main".to_string()],
        &[],
        &[]
    ));
    assert!(!graph_context_available_from_graph(
        &graph,
        &["artefact-other".to_string()],
        &[],
        &[]
    ));
}

#[test]
fn architecture_target_overview_includes_direct_and_nearby_nodes() {
    let code = flow_code_node("code-main", "src/main.rs", "artefact-main");
    let entry = flow_entry_point("entry-main", "src/main.rs", "artefact-main");
    let component = graph_node(
        "component-cli",
        ArchitectureGraphNodeKind::Component,
        "src/main",
        serde_json::json!({ "component_key": "src/main" }),
    );
    let container = graph_node(
        "container-cli",
        ArchitectureGraphNodeKind::Container,
        "bitloops-cli",
        serde_json::json!({ "container_key": "bitloops-cli" }),
    );
    let unrelated = flow_code_node("code-other", "src/other.rs", "artefact-other");
    let graph = ArchitectureGraph {
        nodes: vec![code, entry, component, container, unrelated],
        edges: vec![
            graph_edge(
                "code-component",
                ArchitectureGraphEdgeKind::Implements,
                "code-main",
                "component-cli",
            ),
            graph_edge(
                "container-component",
                ArchitectureGraphEdgeKind::Contains,
                "container-cli",
                "component-cli",
            ),
        ],
        total_nodes: 5,
        total_edges: 2,
    };

    let overview = architecture_target_overview_from_graph(
        graph,
        &["artefact-main".to_string()],
        &[],
        &["src/main.rs".to_string()],
    );

    assert!(overview.available);
    assert_eq!(overview.reason, None);
    assert_eq!(
        overview.matched_artefact_ids,
        vec!["artefact-main".to_string()]
    );
    assert_eq!(overview.direct_node_count, 2);
    assert_eq!(
        overview
            .nodes
            .iter()
            .map(|node| node.id.as_str())
            .collect::<Vec<_>>(),
        vec!["code-main", "component-cli", "container-cli", "entry-main"]
    );
    assert_eq!(
        overview
            .edges
            .iter()
            .map(|edge| edge.id.as_str())
            .collect::<Vec<_>>(),
        vec!["code-component", "container-component"]
    );
}

#[test]
fn architecture_target_overview_reports_no_matching_facts() {
    let graph = ArchitectureGraph {
        nodes: vec![flow_code_node(
            "code-other",
            "src/other.rs",
            "artefact-other",
        )],
        edges: Vec::new(),
        total_nodes: 1,
        total_edges: 0,
    };

    let overview = architecture_target_overview_from_graph(
        graph,
        &["artefact-main".to_string()],
        &[],
        &["src/main.rs".to_string()],
    );

    assert!(!overview.available);
    assert_eq!(
        overview.reason.as_deref(),
        Some("no_matching_architecture_facts")
    );
    assert!(overview.nodes.is_empty());
    assert!(overview.edges.is_empty());
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

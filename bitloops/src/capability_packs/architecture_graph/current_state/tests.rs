use super::*;

fn file(path: &str, language: &str) -> CurrentCanonicalFileRecord {
    CurrentCanonicalFileRecord {
        repo_id: "repo".to_string(),
        path: path.to_string(),
        analysis_mode: "parsed".to_string(),
        file_role: "source".to_string(),
        language: language.to_string(),
        resolved_language: language.to_string(),
        effective_content_id: format!("content:{path}"),
        parser_version: "test".to_string(),
        extractor_version: "test".to_string(),
        exists_in_head: true,
        exists_in_index: true,
        exists_in_worktree: true,
    }
}

fn entry_artefact(path: &str, name: &str, kind: &str) -> LanguageEntryPointArtefact {
    LanguageEntryPointArtefact {
        artefact_id: format!("{path}:{name}:artefact"),
        symbol_id: format!("{path}:{name}:symbol"),
        path: path.to_string(),
        name: name.to_string(),
        canonical_kind: Some(kind.to_string()),
        language_kind: Some("function_item".to_string()),
        symbol_fqn: Some(format!("{path}::{name}")),
        signature: None,
        modifiers: Vec::new(),
        start_line: 1,
        end_line: 3,
    }
}

fn current_artefact(path: &str, name: &str) -> CurrentCanonicalArtefactRecord {
    CurrentCanonicalArtefactRecord {
        repo_id: "repo".to_string(),
        path: path.to_string(),
        content_id: format!("content:{path}"),
        symbol_id: format!("{path}:{name}:symbol"),
        artefact_id: format!("{path}:{name}:artefact"),
        language: "rust".to_string(),
        extraction_fingerprint: "fingerprint".to_string(),
        canonical_kind: Some("function".to_string()),
        language_kind: Some("function_item".to_string()),
        symbol_fqn: Some(format!("{path}::{name}")),
        parent_symbol_id: None,
        parent_artefact_id: None,
        start_line: 1,
        end_line: 3,
        start_byte: 0,
        end_byte: 10,
        signature: None,
        modifiers: "[]".to_string(),
        docstring: None,
    }
}

fn synthesis_request() -> CurrentStateConsumerRequest {
    CurrentStateConsumerRequest {
        run_id: Some("run".to_string()),
        repo_id: "repo".to_string(),
        repo_root: std::path::PathBuf::from("/tmp/repo"),
        active_branch: Some("main".to_string()),
        head_commit_sha: Some("abc123".to_string()),
        from_generation_seq_exclusive: 0,
        to_generation_seq_inclusive: 7,
        reconcile_mode: crate::host::capability_host::ReconcileMode::MergedDelta,
        file_upserts: Vec::new(),
        file_removals: Vec::new(),
        affected_paths: Vec::new(),
        artefact_upserts: Vec::new(),
        artefact_removals: Vec::new(),
    }
}

#[test]
fn dependency_adjacency_keeps_resolved_edges_only() {
    let edges = vec![
        CurrentCanonicalEdgeRecord {
            repo_id: "repo".to_string(),
            edge_id: "edge-1".to_string(),
            path: "src/lib.rs".to_string(),
            content_id: "content".to_string(),
            from_symbol_id: "a".to_string(),
            from_artefact_id: "a-art".to_string(),
            to_symbol_id: Some("b".to_string()),
            to_artefact_id: Some("b-art".to_string()),
            to_symbol_ref: None,
            edge_kind: "call".to_string(),
            language: "rust".to_string(),
            start_line: None,
            end_line: None,
            metadata: "{}".to_string(),
        },
        CurrentCanonicalEdgeRecord {
            repo_id: "repo".to_string(),
            edge_id: "edge-2".to_string(),
            path: "src/lib.rs".to_string(),
            content_id: "content".to_string(),
            from_symbol_id: "b".to_string(),
            from_artefact_id: "b-art".to_string(),
            to_symbol_id: None,
            to_artefact_id: None,
            to_symbol_ref: Some("external".to_string()),
            edge_kind: "call".to_string(),
            language: "rust".to_string(),
            start_line: None,
            end_line: None,
            metadata: "{}".to_string(),
        },
    ];

    let adjacency = dependency_adjacency(&edges);

    assert_eq!(
        adjacency
            .get("a-art")
            .and_then(|targets| targets.iter().next())
            .map(String::as_str),
        Some("b-art")
    );
    assert!(!adjacency.contains_key("b-art"));
}

#[test]
fn config_entry_points_include_cargo_package_binary() {
    let temp = tempfile::tempdir().expect("tempdir");
    let cargo_path = temp.path().join("crates/bitloops-inference/Cargo.toml");
    std::fs::create_dir_all(cargo_path.parent().expect("parent")).expect("create dirs");
    std::fs::write(
        &cargo_path,
        "[package]\nname = \"bitloops-inference\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("write Cargo.toml");
    std::fs::create_dir_all(temp.path().join("crates/bitloops-inference/src")).expect("create src");
    std::fs::write(
        temp.path().join("crates/bitloops-inference/src/main.rs"),
        "fn main() {}\n",
    )
    .expect("write main");

    let files = vec![
        file("crates/bitloops-inference/Cargo.toml", "toml"),
        file("crates/bitloops-inference/src/main.rs", "rust"),
    ];
    let artefacts = vec![entry_artefact(
        "crates/bitloops-inference/src/main.rs",
        "main",
        "function",
    )];
    let grouped = group_artefacts_for_test(artefacts);

    let candidates = detect_config_entry_points(temp.path(), &files, &grouped);

    let cargo_bin = candidates
        .iter()
        .find(|candidate| candidate.entry_kind == "cargo_bin")
        .expect("cargo bin entry point");
    assert_eq!(cargo_bin.path, "crates/bitloops-inference/src/main.rs");
    assert_eq!(cargo_bin.name, "bitloops-inference");
    assert!(cargo_bin.artefact_id.is_some());
}

#[test]
fn config_entry_points_include_package_json_bin_and_runtime_script() {
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        temp.path().join("package.json"),
        r#"{
  "name": "sample-cli",
  "bin": { "sample": "./src/cli.ts" },
  "scripts": { "start": "tsx ./src/server.ts" }
}
"#,
    )
    .expect("write package.json");
    std::fs::create_dir_all(temp.path().join("src")).expect("create src");
    std::fs::write(
        temp.path().join("src/cli.ts"),
        "export function main() {}\n",
    )
    .expect("write cli");
    std::fs::write(
        temp.path().join("src/server.ts"),
        "export function startServer() {}\n",
    )
    .expect("write server");

    let files = vec![
        file("package.json", "json"),
        file("src/cli.ts", "typescript"),
        file("src/server.ts", "typescript"),
    ];
    let grouped = group_artefacts_for_test(vec![
        entry_artefact("src/cli.ts", "main", "function"),
        entry_artefact("src/server.ts", "startServer", "function"),
    ]);

    let candidates = detect_config_entry_points(temp.path(), &files, &grouped);

    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.entry_kind == "npm_bin" && candidate.path == "src/cli.ts")
    );
    assert!(candidates.iter().any(|candidate| {
        candidate.entry_kind == "npm_script" && candidate.path == "src/server.ts"
    }));
}

#[test]
fn config_entry_points_promote_docusaurus_and_vscode_extension_packages() {
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(temp.path().join("documentation/src/pages"))
        .expect("create documentation dirs");
    std::fs::write(
        temp.path().join("documentation/package.json"),
        r#"{
  "name": "documentation",
  "scripts": {
    "start": "docusaurus start",
    "build": "docusaurus build",
    "deploy": "docusaurus deploy",
    "serve": "docusaurus serve"
  },
  "dependencies": { "@docusaurus/core": "^3.9.2" }
}
"#,
    )
    .expect("write documentation package");
    std::fs::write(
        temp.path().join("documentation/docusaurus.config.ts"),
        "export default {};\n",
    )
    .expect("write docusaurus config");
    std::fs::write(
        temp.path().join("documentation/src/pages/index.tsx"),
        "export default function Home() { return null; }\n",
    )
    .expect("write docs page");

    std::fs::create_dir_all(temp.path().join("vscode-extension/src"))
        .expect("create extension dirs");
    std::fs::write(
        temp.path().join("vscode-extension/package.json"),
        r#"{
  "name": "bitloops",
  "displayName": "Bitloops",
  "engines": { "vscode": "^1.89.0" },
  "main": "./out/extension.js",
  "contributes": {
    "commands": [
      { "command": "bitloops.searchArtefacts", "title": "Bitloops: Search Artefacts" }
    ]
  }
}
"#,
    )
    .expect("write extension package");
    std::fs::write(
        temp.path().join("vscode-extension/src/extension.ts"),
        "export function activate() {}\n",
    )
    .expect("write extension source");

    let files = vec![
        file("documentation/package.json", "json"),
        file("documentation/docusaurus.config.ts", "typescript"),
        file("documentation/src/pages/index.tsx", "typescript"),
        file("vscode-extension/package.json", "json"),
        file("vscode-extension/src/extension.ts", "typescript"),
    ];
    let grouped = group_artefacts_for_test(vec![
        entry_artefact("documentation/src/pages/index.tsx", "Home", "function"),
        entry_artefact("vscode-extension/src/extension.ts", "activate", "function"),
    ]);

    let candidates = detect_config_entry_points(temp.path(), &files, &grouped);

    assert!(candidates.iter().any(|candidate| {
        candidate.entry_kind == "docusaurus_site"
            && candidate.name == "documentation"
            && candidate.path == "documentation/docusaurus.config.ts"
    }));
    assert!(candidates.iter().any(|candidate| {
        candidate.entry_kind == "docusaurus_script"
            && candidate.name == "documentation build"
            && candidate.path == "documentation/docusaurus.config.ts"
    }));
    assert!(candidates.iter().any(|candidate| {
        candidate.entry_kind == "vscode_extension"
            && candidate.name == "Bitloops VS Code extension"
            && candidate.path == "vscode-extension/src/extension.ts"
    }));
    assert!(candidates.iter().any(|candidate| {
        candidate.entry_kind == "vscode_command"
            && candidate.name == "bitloops.searchArtefacts"
            && candidate.path == "vscode-extension/src/extension.ts"
    }));
}

#[test]
fn config_entry_points_include_clap_commands_and_devql_routes() {
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(temp.path().join("bitloops/src/api")).expect("create api dirs");
    std::fs::create_dir_all(temp.path().join("bitloops/src/cli/devql")).expect("create cli dirs");
    std::fs::write(
        temp.path().join("bitloops/Cargo.toml"),
        "[package]\nname = \"bitloops\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("write Cargo.toml");
    std::fs::write(temp.path().join("bitloops/src/main.rs"), "fn main() {}\n").expect("write main");
    std::fs::write(
        temp.path().join("bitloops/src/cli.rs"),
        r#"
#[derive(Subcommand)]
pub enum Commands {
    /// Start the daemon.
    Start(StartArgs),
    /// DevQL ingestion and querying.
    Devql(DevqlArgs),
    /// Hidden internal command.
    #[command(hide = true)]
    Internal(InternalArgs),
}
"#,
    )
    .expect("write cli");
    std::fs::write(
        temp.path().join("bitloops/src/cli/devql/args.rs"),
        r#"
#[derive(Subcommand)]
pub enum DevqlCommand {
    /// Execute a DevQL query.
    Query(QueryArgs),
    /// Print schema SDL.
    Schema(SchemaArgs),
}
"#,
    )
    .expect("write devql args");
    std::fs::write(
        temp.path().join("bitloops/src/api/router.rs"),
        r#"
fn router() {
    Router::new()
        .route("/devql", post(handler))
        .route("/devql/playground", get(handler))
        .route(
            "/devql/global",
            post(handler),
        )
        .route("/devql/dashboard/blobs/{repo_id}/{blob_sha}", get(handler));
}
"#,
    )
    .expect("write router");

    let files = vec![
        file("bitloops/Cargo.toml", "toml"),
        file("bitloops/src/main.rs", "rust"),
        file("bitloops/src/cli.rs", "rust"),
        file("bitloops/src/cli/devql/args.rs", "rust"),
        file("bitloops/src/api/router.rs", "rust"),
    ];
    let grouped = group_artefacts_for_test(vec![entry_artefact(
        "bitloops/src/main.rs",
        "main",
        "function",
    )]);

    let candidates = detect_config_entry_points(temp.path(), &files, &grouped);

    assert!(candidates.iter().any(|candidate| {
        candidate.entry_kind == "rust_clap_command" && candidate.name == "bitloops start"
    }));
    assert!(candidates.iter().any(|candidate| {
        candidate.entry_kind == "rust_clap_command" && candidate.name == "bitloops devql query"
    }));
    assert!(
        !candidates
            .iter()
            .any(|candidate| candidate.name.contains("internal"))
    );
    assert!(
        candidates.iter().any(|candidate| {
            candidate.entry_kind == "http_route" && candidate.name == "/devql"
        })
    );
    assert!(candidates.iter().any(|candidate| {
        candidate.entry_kind == "http_route" && candidate.name == "/devql/global"
    }));
    assert!(!candidates.iter().any(|candidate| {
        candidate.entry_kind == "http_route" && candidate.name == "/devql/playground"
    }));
}

#[test]
fn repo_structure_creates_fallback_system_without_deployment_unit() {
    let mut builder = GraphBuilder::new("repo", 7, "run");
    builder.seed_repo_structure();

    let facts = builder.finish();

    assert!(facts.nodes.iter().any(|node| {
        node.node_kind == ArchitectureGraphNodeKind::System.as_str()
            && node.properties["system_key"] == "repo:repo"
    }));
    assert!(
        !facts
            .nodes
            .iter()
            .any(|node| node.node_kind == ArchitectureGraphNodeKind::DeploymentUnit.as_str()),
        "repo root alone must not be a deployment unit"
    );
}

#[test]
fn config_candidate_creates_deployment_container_and_realises_edge() {
    let mut builder = GraphBuilder::new("repo", 7, "run");
    builder.seed_repo_structure();
    let candidate = LanguageEntryPointCandidate {
        path: "crates/cli/src/main.rs".to_string(),
        artefact_id: Some("main-art".to_string()),
        symbol_id: Some("main-symbol".to_string()),
        name: "cli".to_string(),
        entry_kind: "cargo_bin".to_string(),
        confidence: 0.94,
        reason: "Cargo binary target".to_string(),
        evidence: vec![
            "crates/cli/Cargo.toml".to_string(),
            "crates/cli/src/main.rs".to_string(),
        ],
    };

    builder.ensure_deployment_container_for_candidate(&candidate);
    let facts = builder.finish();

    assert!(facts.nodes.iter().any(|node| {
        node.node_kind == ArchitectureGraphNodeKind::DeploymentUnit.as_str()
            && node.properties["deployment_kind"] == "cargo_bin"
    }));
    assert!(facts.nodes.iter().any(|node| {
        node.node_kind == ArchitectureGraphNodeKind::Container.as_str()
            && node.properties["container_kind"] == "cli"
            && node.properties["system_key"] == "repo:repo"
    }));
    assert!(
        facts
            .edges
            .iter()
            .any(|edge| edge.edge_kind == ArchitectureGraphEdgeKind::Realises.as_str())
    );
    assert!(
        facts
            .edges
            .iter()
            .any(|edge| edge.edge_kind == ArchitectureGraphEdgeKind::Produces.as_str())
    );
}

#[test]
fn components_are_inferred_inside_detected_container() {
    let mut builder = GraphBuilder::new("repo", 7, "run");
    builder.seed_repo_structure();
    let artefacts = vec![
        current_artefact("crates/cli/src/main.rs", "main"),
        current_artefact("crates/cli/src/runtime.rs", "run"),
    ];
    builder.add_code_nodes(&artefacts);
    let candidate = LanguageEntryPointCandidate {
        path: "crates/cli/src/main.rs".to_string(),
        artefact_id: Some("crates/cli/src/main.rs:main:artefact".to_string()),
        symbol_id: Some("crates/cli/src/main.rs:main:symbol".to_string()),
        name: "cli".to_string(),
        entry_kind: "cargo_bin".to_string(),
        confidence: 0.94,
        reason: "Cargo binary target".to_string(),
        evidence: vec![
            "crates/cli/Cargo.toml".to_string(),
            "crates/cli/src/main.rs".to_string(),
        ],
    };
    builder.ensure_deployment_container_for_candidate(&candidate);

    let component_inputs = artefacts
        .iter()
        .map(|artefact| ComponentArtefactInput {
            artefact_id: artefact.artefact_id.clone(),
            path: artefact.path.clone(),
        })
        .collect::<Vec<_>>();
    builder.add_components_for_containers(&component_inputs);
    let facts = builder.finish();

    assert!(facts.nodes.iter().any(|node| {
        node.node_kind == ArchitectureGraphNodeKind::Component.as_str()
            && node.properties["component_key"] == "src/runtime"
    }));
    assert!(
        facts
            .edges
            .iter()
            .any(|edge| edge.edge_kind == ArchitectureGraphEdgeKind::Implements.as_str())
    );
}

#[test]
fn synthesised_structured_output_is_validated_before_merge() {
    let mut builder = GraphBuilder::new("repo", 7, "run");
    builder.seed_repo_structure();
    let system_id = builder.fallback_system_id();

    let error = builder
        .add_synthesised_facts(json!({
            "nodes": [{
                "kind": "DOMAIN",
                "identity": "payments",
                "label": "Payments",
                "confidence": 0.82
            }],
            "edges": [{
                "kind": "CONTAINS",
                "from": { "node_id": "missing" },
                "to": { "node_id": system_id },
                "confidence": 0.7
            }]
        }))
        .expect_err("unknown edge endpoint should reject the whole response");

    assert!(error.to_string().contains("known node"));
    let facts = builder.finish();
    assert!(
        !facts.nodes.iter().any(|node| node.label == "Payments"),
        "malformed structured output must not be partially merged"
    );
}

#[test]
fn synthesised_structured_output_adds_valid_facts() {
    let mut builder = GraphBuilder::new("repo", 7, "run");
    builder.seed_repo_structure();
    let system_id = builder.fallback_system_id();

    let (nodes, edges) = builder
        .add_synthesised_facts(json!({
            "nodes": [{
                "kind": "DOMAIN",
                "identity": "payments",
                "label": "Payments",
                "confidence": 0.82,
                "properties": { "bounded_context": true }
            }],
            "edges": [{
                "kind": "CONTAINS",
                "from": { "node_id": system_id },
                "to": { "kind": "DOMAIN", "identity": "payments" },
                "confidence": 0.74
            }]
        }))
        .expect("valid structured response should merge");

    assert_eq!((nodes, edges), (1, 1));
    let facts = builder.finish();
    assert!(facts.nodes.iter().any(|node| {
        node.node_kind == ArchitectureGraphNodeKind::Domain.as_str()
            && node.label == "Payments"
            && node.source_kind == "AGENT_SYNTHESIS"
    }));
    assert!(facts.edges.iter().any(|edge| {
        edge.edge_kind == ArchitectureGraphEdgeKind::Contains.as_str()
            && edge.to_node_id == node_id("repo", ArchitectureGraphNodeKind::Domain, "payments")
            && edge.source_kind == "AGENT_SYNTHESIS"
    }));
}

#[test]
fn synthesised_structured_output_rejects_ambiguous_edge_endpoint() {
    let mut builder = GraphBuilder::new("repo", 7, "run");
    builder.seed_repo_structure();
    let system_id = builder.fallback_system_id();

    let error = builder
        .add_synthesised_facts(json!({
            "nodes": [],
            "edges": [{
                "kind": "CONTAINS",
                "from": {
                    "node_id": system_id,
                    "kind": "SYSTEM",
                    "identity": "repo"
                },
                "to": { "node_id": system_id },
                "confidence": 0.7
            }]
        }))
        .expect_err("ambiguous endpoint should be rejected");

    assert!(
        error
            .to_string()
            .contains("either node_id or kind plus identity")
    );
}

#[test]
fn architecture_fact_synthesis_prompt_limits_nodes_without_snapshot_edges() {
    let mut builder = GraphBuilder::new("repo", 7, "run");
    builder.seed_repo_structure();
    let artefacts = (0..120)
        .map(|index| current_artefact(&format!("src/file_{index}.rs"), &format!("symbol_{index}")))
        .collect::<Vec<_>>();
    builder.add_code_nodes(&artefacts);
    builder.upsert_edge_by_kind(
        ArchitectureGraphEdgeKind::Contains,
        builder.fallback_system_id(),
        node_id(
            "repo",
            ArchitectureGraphNodeKind::Node,
            "src/file_0.rs:symbol_0:artefact",
        ),
        "COMPUTED",
        0.9,
        builder.provenance("test"),
        json!([]),
        json!({}),
    );

    let prompt = architecture_fact_synthesis_user_prompt(&synthesis_request(), &builder);
    let prompt: Value = serde_json::from_str(&prompt).expect("prompt is JSON");
    let existing_nodes = prompt["existing_nodes"]
        .as_array()
        .expect("existing nodes are an array");

    assert_eq!(existing_nodes.len(), 80);
    assert!(
        prompt.get("existing_edges").is_none(),
        "prompt should not serialise edge snapshots"
    );
}

#[test]
fn change_unit_impacts_edges_only_keep_target_matched_paths() {
    let mut builder = GraphBuilder::new("repo", 7, "run");
    builder.seed_repo_structure();

    let first = current_artefact("src/a.rs", "first");
    let second = current_artefact("src/b.rs", "second");
    builder.add_code_nodes(&[first.clone(), second.clone()]);

    let first_node_id = builder
        .artefact_nodes
        .get(&first.artefact_id)
        .cloned()
        .expect("first node id");
    builder
        .path_nodes
        .entry("src/alias.rs".to_string())
        .or_default()
        .push(first_node_id.clone());

    let mut request = synthesis_request();
    request.affected_paths = vec![
        "src/a.rs".to_string(),
        "src/alias.rs".to_string(),
        "src/b.rs".to_string(),
    ];

    let metrics = builder.add_change_unit(&request);
    assert_eq!(metrics.affected_paths, 3);
    assert_eq!(metrics.impacted_nodes, 2);

    let facts = builder.finish();
    let change_node = facts
        .nodes
        .iter()
        .find(|node| node.node_kind == ArchitectureGraphNodeKind::ChangeUnit.as_str())
        .expect("change unit node");
    let stored_paths = change_node.properties["affected_paths"]
        .as_array()
        .expect("change unit paths array")
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert_eq!(stored_paths, vec!["src/a.rs", "src/alias.rs", "src/b.rs"]);

    let impacts_edges = facts
        .edges
        .iter()
        .filter(|edge| edge.edge_kind == ArchitectureGraphEdgeKind::Impacts.as_str())
        .collect::<Vec<_>>();
    assert_eq!(impacts_edges.len(), 2);

    let first_edge = impacts_edges
        .iter()
        .find(|edge| edge.to_node_id == first_node_id)
        .expect("edge for first node");
    assert!(
        first_edge.evidence.get("affectedPaths").is_none(),
        "legacy affectedPaths payload should be removed"
    );
    let first_paths = first_edge
        .evidence
        .as_array()
        .expect("first edge evidence array")
        .iter()
        .filter_map(|item| item.get("path").and_then(Value::as_str))
        .collect::<Vec<_>>();
    assert_eq!(first_paths, vec!["src/a.rs", "src/alias.rs"]);

    let second_node_id = builder_node_id_for_artefact("repo", &second.artefact_id);
    let second_edge = impacts_edges
        .iter()
        .find(|edge| edge.to_node_id == second_node_id)
        .expect("edge for second node");
    let second_paths = second_edge
        .evidence
        .as_array()
        .expect("second edge evidence array")
        .iter()
        .filter_map(|item| item.get("path").and_then(Value::as_str))
        .collect::<Vec<_>>();
    assert_eq!(second_paths, vec!["src/b.rs"]);
}

#[tokio::test]
async fn reconcile_streams_current_state_and_persists_metrics() -> Result<()> {
    let temp = tempfile::tempdir().expect("tempdir");
    let db_path = temp.path().join("runtime.sqlite");
    crate::storage::init::init_database(&db_path, false, "seed-commit")?;
    let conn = rusqlite::Connection::open(&db_path).expect("open sqlite");
    conn.execute_batch(
        crate::capability_packs::architecture_graph::schema::architecture_graph_sqlite_schema_sql(),
    )
    .expect("create architecture graph schema");
    let storage = std::sync::Arc::new(crate::host::devql::RelationalStorage::local_only(db_path));

    let mut request = synthesis_request();
    request.repo_root = temp.path().to_path_buf();
    request.affected_paths = vec!["src/lib.rs".to_string()];

    let context = CurrentStateConsumerContext {
        config_root: json!({}),
        storage: storage.clone(),
        relational: std::sync::Arc::new(StreamingOnlyRelationalGateway {
            files: vec![file("src/lib.rs", "rust")],
            artefacts: vec![current_artefact("src/lib.rs", "main")],
            edges: Vec::new(),
        }),
        language_services: std::sync::Arc::new(
            crate::host::capability_host::gateways::EmptyLanguageServicesGateway,
        ),
        git_history: std::sync::Arc::new(
            crate::host::capability_host::gateways::EmptyGitHistoryGateway,
        ),
        inference: std::sync::Arc::new(crate::host::inference::EmptyInferenceGateway),
        host_services: std::sync::Arc::new(
            crate::host::capability_host::gateways::DefaultHostServicesGateway::new("repo"),
        ),
        workplane: std::sync::Arc::new(NoopArchitectureWorkplaneGateway),
        test_harness: None,
        init_session_id: None,
        parent_pid: None,
    };

    let result = ArchitectureGraphCurrentStateConsumer
        .reconcile(&request, &context)
        .await?;

    let metrics = result.metrics.expect("metrics payload");
    assert_eq!(metrics["files"], json!(1));
    assert_eq!(metrics["artefacts"], json!(1));
    assert_eq!(metrics["dependency_edges"], json!(0));
    assert_eq!(metrics["affected_paths"], json!(1));
    assert_eq!(metrics["impacted_nodes"], json!(1));

    let node_count = storage
        .query_rows(
            "SELECT COUNT(*) AS count FROM architecture_graph_nodes_current WHERE repo_id = 'repo'",
        )
        .await?;
    assert_eq!(node_count[0]["count"], json!(3));

    let impacts = storage
        .query_rows(
            "SELECT evidence_json FROM architecture_graph_edges_current
             WHERE repo_id = 'repo' AND edge_kind = 'IMPACTS'",
        )
        .await?;
    assert_eq!(impacts.len(), 1);
    assert_eq!(
        serde_json::from_str::<Value>(
            impacts[0]["evidence_json"]
                .as_str()
                .expect("stored evidence JSON string"),
        )
        .expect("parse impacts evidence"),
        json!([{ "path": "src/lib.rs" }])
    );

    Ok(())
}

#[tokio::test]
async fn reconcile_rejects_persistence_when_worker_parent_is_gone() {
    let temp = tempfile::tempdir().expect("tempdir");
    let db_path = temp.path().join("runtime.sqlite");
    crate::storage::init::init_database(&db_path, false, "seed-commit")
        .expect("init sqlite database");
    let conn = rusqlite::Connection::open(&db_path).expect("open sqlite");
    conn.execute_batch(
        crate::capability_packs::architecture_graph::schema::architecture_graph_sqlite_schema_sql(),
    )
    .expect("create architecture graph schema");
    let storage = std::sync::Arc::new(crate::host::devql::RelationalStorage::local_only(db_path));

    let mut request = synthesis_request();
    request.repo_root = temp.path().to_path_buf();

    let context = CurrentStateConsumerContext {
        config_root: json!({}),
        storage: storage.clone(),
        relational: std::sync::Arc::new(StreamingOnlyRelationalGateway {
            files: vec![file("src/lib.rs", "rust")],
            artefacts: vec![current_artefact("src/lib.rs", "main")],
            edges: Vec::new(),
        }),
        language_services: std::sync::Arc::new(
            crate::host::capability_host::gateways::EmptyLanguageServicesGateway,
        ),
        git_history: std::sync::Arc::new(
            crate::host::capability_host::gateways::EmptyGitHistoryGateway,
        ),
        inference: std::sync::Arc::new(crate::host::inference::EmptyInferenceGateway),
        host_services: std::sync::Arc::new(
            crate::host::capability_host::gateways::DefaultHostServicesGateway::new("repo"),
        ),
        workplane: std::sync::Arc::new(NoopArchitectureWorkplaneGateway),
        test_harness: None,
        init_session_id: None,
        parent_pid: Some(u32::MAX),
    };

    let err = ArchitectureGraphCurrentStateConsumer
        .reconcile(&request, &context)
        .await
        .expect_err("missing parent should prevent persistence");

    assert!(
        err.to_string().contains("parent process"),
        "unexpected error: {err:#}"
    );

    let node_count = storage
        .query_rows(
            "SELECT COUNT(*) AS count FROM architecture_graph_nodes_current WHERE repo_id = 'repo'",
        )
        .await
        .expect("query persisted nodes");
    assert_eq!(node_count[0]["count"], json!(0));
}

fn group_artefacts_for_test(
    artefacts: Vec<LanguageEntryPointArtefact>,
) -> BTreeMap<String, Vec<LanguageEntryPointArtefact>> {
    let mut grouped = BTreeMap::new();
    for artefact in artefacts {
        grouped
            .entry(artefact.path.clone())
            .or_insert_with(Vec::new)
            .push(artefact);
    }
    grouped
}

fn builder_node_id_for_artefact(repo_id: &str, artefact_id: &str) -> String {
    node_id(repo_id, ArchitectureGraphNodeKind::Node, artefact_id)
}

#[derive(Clone)]
struct StreamingOnlyRelationalGateway {
    files: Vec<CurrentCanonicalFileRecord>,
    artefacts: Vec<CurrentCanonicalArtefactRecord>,
    edges: Vec<CurrentCanonicalEdgeRecord>,
}

impl crate::host::capability_host::gateways::RelationalGateway for StreamingOnlyRelationalGateway {
    fn resolve_checkpoint_id(&self, _repo_id: &str, checkpoint_ref: &str) -> Result<String> {
        Ok(checkpoint_ref.to_string())
    }

    fn artefact_exists(&self, _repo_id: &str, _artefact_id: &str) -> Result<bool> {
        Ok(false)
    }

    fn load_repo_id_for_commit(&self, _commit_sha: &str) -> Result<String> {
        Ok("repo".to_string())
    }

    fn load_current_canonical_files(
        &self,
        _repo_id: &str,
    ) -> Result<Vec<CurrentCanonicalFileRecord>> {
        Ok(self.files.clone())
    }

    fn load_current_canonical_artefacts(
        &self,
        _repo_id: &str,
    ) -> Result<Vec<CurrentCanonicalArtefactRecord>> {
        panic!("architecture_graph consumer should stream artefacts via visitor");
    }

    fn visit_current_canonical_artefacts(
        &self,
        _repo_id: &str,
        visitor: &mut dyn FnMut(CurrentCanonicalArtefactRecord) -> Result<()>,
    ) -> Result<()> {
        for artefact in self.artefacts.clone() {
            visitor(artefact)?;
        }
        Ok(())
    }

    fn load_current_canonical_edges(
        &self,
        _repo_id: &str,
    ) -> Result<Vec<CurrentCanonicalEdgeRecord>> {
        panic!("architecture_graph consumer should stream dependency edges via visitor");
    }

    fn visit_current_canonical_edges(
        &self,
        _repo_id: &str,
        visitor: &mut dyn FnMut(CurrentCanonicalEdgeRecord) -> Result<()>,
    ) -> Result<()> {
        for edge in self.edges.clone() {
            visitor(edge)?;
        }
        Ok(())
    }

    fn load_current_production_artefacts(
        &self,
        _repo_id: &str,
    ) -> Result<Vec<crate::models::ProductionArtefact>> {
        Ok(Vec::new())
    }

    fn load_production_artefacts(
        &self,
        _commit_sha: &str,
    ) -> Result<Vec<crate::models::ProductionArtefact>> {
        Ok(Vec::new())
    }

    fn load_artefacts_for_file_lines(
        &self,
        _commit_sha: &str,
        _file_path: &str,
    ) -> Result<Vec<(String, i64, i64)>> {
        Ok(Vec::new())
    }
}

struct NoopArchitectureWorkplaneGateway;

impl crate::host::capability_host::gateways::CapabilityWorkplaneGateway
    for NoopArchitectureWorkplaneGateway
{
    fn enqueue_jobs(
        &self,
        _jobs: Vec<crate::host::capability_host::gateways::CapabilityWorkplaneJob>,
    ) -> Result<crate::host::capability_host::gateways::CapabilityWorkplaneEnqueueResult> {
        Ok(crate::host::capability_host::gateways::CapabilityWorkplaneEnqueueResult::default())
    }

    fn mailbox_status(
        &self,
    ) -> Result<BTreeMap<String, crate::host::capability_host::gateways::CapabilityMailboxStatus>>
    {
        Ok(BTreeMap::new())
    }
}

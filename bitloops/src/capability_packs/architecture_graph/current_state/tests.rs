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

    builder.add_components_for_containers(&artefacts);
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

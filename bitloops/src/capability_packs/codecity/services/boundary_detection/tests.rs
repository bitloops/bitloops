use std::collections::BTreeSet;

use tempfile::tempdir;

use super::detect_boundaries;
use crate::capability_packs::codecity::services::config::CodeCityConfig;
use crate::capability_packs::codecity::services::source_graph::{
    CodeCitySourceArtefact, CodeCitySourceEdge, CodeCitySourceFile, CodeCitySourceGraph,
};
use crate::capability_packs::codecity::types::CODECITY_ROOT_BOUNDARY_ID;

fn graph(files: &[&str], edges: &[(&str, &str)]) -> CodeCitySourceGraph {
    CodeCitySourceGraph {
        project_path: None,
        files: files
            .iter()
            .map(|path| CodeCitySourceFile {
                path: (*path).to_string(),
                language: "typescript".to_string(),
                effective_content_id: format!("content::{path}"),
                included: true,
                exclusion_reason: None,
            })
            .collect(),
        artefacts: files
            .iter()
            .map(|path| CodeCitySourceArtefact {
                artefact_id: format!("artefact::{path}"),
                symbol_id: format!("symbol::{path}"),
                path: (*path).to_string(),
                symbol_fqn: Some(format!("{path}::file")),
                canonical_kind: Some("file".to_string()),
                language_kind: Some("fixture".to_string()),
                parent_artefact_id: None,
                parent_symbol_id: None,
                signature: None,
                start_line: 1,
                end_line: 1,
            })
            .collect(),
        edges: edges
            .iter()
            .enumerate()
            .map(|(index, (from, to))| CodeCitySourceEdge {
                edge_id: format!("edge-{index}"),
                from_path: (*from).to_string(),
                to_path: (*to).to_string(),
                from_symbol_id: format!("symbol::{from}"),
                from_artefact_id: format!("artefact::{from}"),
                to_symbol_id: Some(format!("symbol::{to}")),
                to_artefact_id: Some(format!("artefact::{to}")),
                to_symbol_ref: Some(format!("{to}::file")),
                edge_kind: "imports".to_string(),
                language: "typescript".to_string(),
                start_line: Some(1),
                end_line: Some(1),
                metadata: "{}".to_string(),
            })
            .collect(),
        external_dependency_hints: Vec::new(),
        diagnostics: Vec::new(),
    }
}

fn indexed_manifest(path: &str) -> CodeCitySourceFile {
    CodeCitySourceFile {
        path: path.to_string(),
        language: "json".to_string(),
        effective_content_id: format!("content::{path}"),
        included: false,
        exclusion_reason: Some("file_role".to_string()),
    }
}

#[test]
fn falls_back_to_root_boundary_when_no_manifest_exists() {
    let temp = tempdir().expect("tempdir");
    let source = graph(
        &["src/main.ts", "src/core.ts"],
        &[("src/main.ts", "src/core.ts")],
    );
    std::fs::create_dir_all(temp.path().join("src")).expect("mkdir");
    std::fs::write(temp.path().join("src/main.ts"), "export {}").expect("write");
    std::fs::write(temp.path().join("src/core.ts"), "export {}").expect("write");

    let result = detect_boundaries(&source, &CodeCityConfig::default(), temp.path());

    assert_eq!(result.boundaries.len(), 1);
    assert_eq!(result.boundaries[0].id, CODECITY_ROOT_BOUNDARY_ID);
}

#[test]
fn skips_implicit_split_for_large_interactive_boundaries() {
    let temp = tempdir().expect("tempdir");
    let files = (0..=super::implicit::MAX_INTERACTIVE_IMPLICIT_BOUNDARY_FILES)
        .map(|index| format!("src/file_{index}.ts"))
        .collect::<Vec<_>>();
    let file_refs = files.iter().map(String::as_str).collect::<Vec<_>>();
    let source = graph(&file_refs, &[]);

    let result = detect_boundaries(&source, &CodeCityConfig::default(), temp.path());

    assert_eq!(result.boundaries.len(), 1);
    assert_eq!(result.boundaries[0].id, CODECITY_ROOT_BOUNDARY_ID);
    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "codecity.boundary.implicit_split_too_large")
    );
}

#[test]
fn implicit_boundary_ids_are_unique_when_names_repeat() {
    let temp = tempdir().expect("tempdir");
    let files = [
        "src/module/a_one.ts",
        "src/module/a_two.ts",
        "src/module/b_one.ts",
        "src/module/b_two.ts",
    ];
    let edges = [
        ("src/module/a_one.ts", "src/module/a_two.ts"),
        ("src/module/a_two.ts", "src/module/a_one.ts"),
        ("src/module/b_one.ts", "src/module/b_two.ts"),
        ("src/module/b_two.ts", "src/module/b_one.ts"),
    ];
    let source = graph(&files, &edges);
    let mut config = CodeCityConfig::default();
    config.boundaries.min_implicit_boundary_files = 2;
    config.boundaries.community_modularity_threshold = 0.0;

    let result = detect_boundaries(&source, &config, temp.path());
    let unique_count = result
        .boundaries
        .iter()
        .map(|boundary| boundary.id.as_str())
        .collect::<BTreeSet<_>>()
        .len();

    assert!(result.boundaries.len() > 1);
    assert_eq!(result.boundaries.len(), unique_count);
}

#[test]
fn detects_manifest_boundaries_from_indexed_files_without_checkout() {
    let temp = tempdir().expect("tempdir");
    let mut source = graph(
        &["packages/api/src/main.ts", "packages/web/src/app.ts"],
        &[],
    );
    source
        .files
        .push(indexed_manifest("packages/api/package.json"));
    source
        .files
        .push(indexed_manifest("packages/web/package.json"));

    let result = detect_boundaries(&source, &CodeCityConfig::default(), temp.path());
    let boundary_roots = result
        .boundaries
        .iter()
        .map(|boundary| boundary.root_path.as_str())
        .collect::<BTreeSet<_>>();

    assert_eq!(result.boundaries.len(), 2);
    assert!(boundary_roots.contains("packages/api"));
    assert!(boundary_roots.contains("packages/web"));
    assert_eq!(
        result.file_to_boundary["packages/api/src/main.ts"],
        "boundary:packages/api"
    );
    assert_eq!(
        result.file_to_boundary["packages/web/src/app.ts"],
        "boundary:packages/web"
    );
    assert!(
        !result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "codecity.boundary.fallback_root")
    );
}

#[test]
fn detects_explicit_manifest_boundaries_within_scope() {
    let temp = tempdir().expect("tempdir");
    std::fs::create_dir_all(temp.path().join("packages/api/src")).expect("mkdir");
    std::fs::write(
        temp.path().join("packages/api/package.json"),
        r#"{ "name": "@demo/api" }"#,
    )
    .expect("write manifest");
    std::fs::write(temp.path().join("packages/api/src/main.ts"), "export {}").expect("write");
    std::fs::write(temp.path().join("packages/api/src/core.ts"), "export {}").expect("write");

    let mut source = graph(
        &["packages/api/src/main.ts", "packages/api/src/core.ts"],
        &[("packages/api/src/main.ts", "packages/api/src/core.ts")],
    );
    source.project_path = Some("packages/api".to_string());

    let result = detect_boundaries(&source, &CodeCityConfig::default(), temp.path());

    assert_eq!(result.boundaries.len(), 1);
    assert_eq!(result.boundaries[0].id, "boundary:packages/api");
    assert_eq!(result.boundaries[0].root_path, "packages/api");
    assert_eq!(
        result.file_to_boundary["packages/api/src/main.ts"],
        "boundary:packages/api"
    );
    assert_eq!(
        result.file_to_boundary["packages/api/src/core.ts"],
        "boundary:packages/api"
    );
}

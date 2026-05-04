use std::collections::BTreeSet;

use tempfile::tempdir;

use super::detect_boundaries;
use crate::capability_packs::codecity::services::config::CodeCityConfig;
use crate::capability_packs::codecity::services::source_graph::{
    CodeCitySourceArtefact, CodeCitySourceEdge, CodeCitySourceFile, CodeCitySourceGraph,
};
use crate::capability_packs::codecity::types::{
    CODECITY_ROOT_BOUNDARY_ID, CodeCityBoundaryKind, CodeCityBoundarySource,
};

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
    config.boundaries.small_cluster_collapse_file_limit = 0;

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
fn collapses_small_implicit_communities_into_independent_boundary() {
    let temp = tempdir().expect("tempdir");
    let files = [
        "src/a/one.ts",
        "src/a/two.ts",
        "src/a/three.ts",
        "src/b/one.ts",
        "src/b/two.ts",
        "src/b/three.ts",
    ];
    let edges = [
        ("src/a/one.ts", "src/a/two.ts"),
        ("src/a/two.ts", "src/a/one.ts"),
        ("src/a/two.ts", "src/a/three.ts"),
        ("src/a/three.ts", "src/a/two.ts"),
        ("src/b/one.ts", "src/b/two.ts"),
        ("src/b/two.ts", "src/b/one.ts"),
        ("src/b/two.ts", "src/b/three.ts"),
        ("src/b/three.ts", "src/b/two.ts"),
    ];
    let source = graph(&files, &edges);
    let mut config = CodeCityConfig::default();
    config.boundaries.community_modularity_threshold = 0.0;

    let result = detect_boundaries(&source, &config, temp.path());

    assert_eq!(result.boundaries.len(), 2);
    let root = result
        .boundaries
        .iter()
        .find(|boundary| boundary.id == CODECITY_ROOT_BOUNDARY_ID)
        .expect("root parent boundary");
    assert_eq!(root.kind, CodeCityBoundaryKind::RootFallback);
    assert!(!root.atomic);

    let boundary = result
        .boundaries
        .iter()
        .find(|boundary| boundary.name == "independent")
        .expect("independent boundary");
    assert_eq!(boundary.id, "boundary:root:implicit:independent");
    assert_eq!(boundary.name, "independent");
    assert_eq!(boundary.kind, CodeCityBoundaryKind::Implicit);
    assert_eq!(boundary.source, CodeCityBoundarySource::CommunityDetection);
    assert_eq!(
        boundary.parent_boundary_id.as_deref(),
        Some(CODECITY_ROOT_BOUNDARY_ID)
    );
    assert_eq!(boundary.file_count, files.len());
    assert!(!boundary.atomic);
    for file in files {
        assert_eq!(result.file_to_boundary[file], boundary.id.as_str());
    }
}

#[test]
fn keeps_large_implicit_communities_and_collapses_only_small_ones() {
    let temp = tempdir().expect("tempdir");
    let files = [
        "src/a/one.ts",
        "src/a/two.ts",
        "src/a/three.ts",
        "src/b/one.ts",
        "src/b/two.ts",
        "src/b/three.ts",
        "src/misc/one.ts",
        "src/misc/two.ts",
    ];
    let edges = [
        ("src/a/one.ts", "src/a/two.ts"),
        ("src/a/two.ts", "src/a/one.ts"),
        ("src/a/two.ts", "src/a/three.ts"),
        ("src/a/three.ts", "src/a/two.ts"),
        ("src/b/one.ts", "src/b/two.ts"),
        ("src/b/two.ts", "src/b/one.ts"),
        ("src/b/two.ts", "src/b/three.ts"),
        ("src/b/three.ts", "src/b/two.ts"),
    ];
    let source = graph(&files, &edges);
    let mut config = CodeCityConfig::default();
    config.boundaries.community_modularity_threshold = 0.0;
    config.boundaries.small_cluster_collapse_file_limit = 2;

    let result = detect_boundaries(&source, &config, temp.path());

    assert_eq!(result.boundaries.len(), 5);
    let root = result
        .boundaries
        .iter()
        .find(|boundary| boundary.id == CODECITY_ROOT_BOUNDARY_ID)
        .expect("root parent boundary");
    assert!(!root.atomic);

    let source_group = result
        .boundaries
        .iter()
        .find(|boundary| boundary.id == "boundary:src")
        .expect("source parent boundary");
    assert_eq!(source_group.kind, CodeCityBoundaryKind::Group);
    assert_eq!(source_group.source, CodeCityBoundarySource::Hierarchy);
    assert_eq!(
        source_group.parent_boundary_id.as_deref(),
        Some(CODECITY_ROOT_BOUNDARY_ID)
    );
    assert!(!source_group.atomic);

    let independent = result
        .boundaries
        .iter()
        .find(|boundary| boundary.name == "independent")
        .expect("independent boundary");
    assert_eq!(independent.file_count, 2);
    assert!(!independent.atomic);
    assert_eq!(
        result.file_to_boundary["src/misc/one.ts"],
        independent.id.as_str()
    );
    assert_eq!(
        result.file_to_boundary["src/misc/two.ts"],
        independent.id.as_str()
    );

    let retained = result
        .boundaries
        .iter()
        .filter(|boundary| {
            boundary.name != "independent"
                && boundary.kind == CodeCityBoundaryKind::Implicit
                && boundary.atomic
        })
        .collect::<Vec<_>>();
    assert_eq!(retained.len(), 2);
    assert!(retained.iter().all(|boundary| boundary.file_count == 3));
    assert!(
        retained
            .iter()
            .all(|boundary| { boundary.parent_boundary_id.as_deref() == Some("boundary:src") })
    );
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

    assert_eq!(result.boundaries.len(), 3);
    let packages = result
        .boundaries
        .iter()
        .find(|boundary| boundary.id == "boundary:packages")
        .expect("packages parent boundary");
    assert_eq!(packages.kind, CodeCityBoundaryKind::Group);
    assert_eq!(packages.source, CodeCityBoundarySource::Hierarchy);
    assert!(!packages.atomic);
    assert!(boundary_roots.contains("packages/api"));
    assert!(boundary_roots.contains("packages/web"));
    assert!(boundary_roots.contains("packages"));
    for boundary in result
        .boundaries
        .iter()
        .filter(|boundary| matches!(boundary.root_path.as_str(), "packages/api" | "packages/web"))
    {
        assert_eq!(
            boundary.parent_boundary_id.as_deref(),
            Some("boundary:packages")
        );
    }
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

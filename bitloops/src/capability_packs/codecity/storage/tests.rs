use std::path::Path;

use anyhow::Result;
use tempfile::TempDir;

use super::{SqliteCodeCityRepository, codecity_sqlite_schema_sql};
use crate::capability_packs::codecity::services::config::CodeCityConfig;
use crate::capability_packs::codecity::services::health::apply_health_overlay;
use crate::capability_packs::codecity::services::source_graph::{
    CodeCitySourceArtefact, CodeCitySourceFile, CodeCitySourceGraph,
};
use crate::capability_packs::codecity::services::world::build_codecity_world;
use crate::host::capability_host::gateways::EmptyGitHistoryGateway;
use crate::storage::SqliteConnectionPool;

#[test]
fn schema_contains_health_snapshot_tables() {
    let sql = codecity_sqlite_schema_sql();
    assert!(sql.contains("codecity_floor_health_current"));
    assert!(sql.contains("codecity_file_health_current"));
    assert!(sql.contains("codecity_health_runs_current"));
    assert!(sql.contains("codecity_dependency_evidence_current"));
    assert!(sql.contains("codecity_file_dependency_arcs_current"));
    assert!(sql.contains("codecity_architecture_violations_current"));
    assert!(sql.contains("codecity_render_arcs_current"));
}

#[test]
fn repository_replaces_and_loads_current_snapshot() -> Result<()> {
    let temp = TempDir::new()?;
    let sqlite = SqliteConnectionPool::connect(temp.path().join("devql.sqlite"))?;
    let repo = SqliteCodeCityRepository::from_sqlite(sqlite);
    repo.initialise_schema()?;

    let config = CodeCityConfig::default();
    let source = CodeCitySourceGraph {
        project_path: None,
        files: vec![CodeCitySourceFile {
            path: "src/lib.rs".to_string(),
            language: "rust".to_string(),
            effective_content_id: "content-1".to_string(),
            included: true,
            exclusion_reason: None,
        }],
        artefacts: vec![CodeCitySourceArtefact {
            artefact_id: "artefact-1".to_string(),
            symbol_id: "symbol-1".to_string(),
            path: "src/lib.rs".to_string(),
            symbol_fqn: Some("crate::work".to_string()),
            canonical_kind: Some("function".to_string()),
            language_kind: None,
            parent_artefact_id: None,
            parent_symbol_id: None,
            signature: None,
            start_line: 1,
            end_line: 5,
        }],
        edges: Vec::new(),
        external_dependency_hints: Vec::new(),
        diagnostics: Vec::new(),
    };
    let mut world = build_codecity_world(
        &source,
        "repo-1",
        Some("commit-1".to_string()),
        config.clone(),
        Path::new("."),
    )?;
    apply_health_overlay(
        &mut world,
        &source,
        &config,
        Path::new("."),
        &EmptyGitHistoryGateway,
        None,
    )?;

    repo.replace_current_snapshot(&world)?;
    let mut loaded = build_codecity_world(
        &source,
        "repo-1",
        Some("commit-1".to_string()),
        config,
        Path::new("."),
    )?;
    assert!(repo.try_apply_current_snapshot(&mut loaded)?);
    assert_eq!(
        loaded.buildings[0].floors[0].health_status,
        world.buildings[0].floors[0].health_status
    );
    Ok(())
}

#[test]
fn repository_replaces_and_loads_phase4_snapshot() -> Result<()> {
    use crate::capability_packs::codecity::types::{
        CodeCityArcGeometry, CodeCityArcKind, CodeCityArcVisibility, CodeCityArchitectureViolation,
        CodeCityDependencyEvidence, CodeCityFileDependencyArc, CodeCityPhase4Snapshot,
        CodeCityRenderArc, CodeCityViolationEvidence, CodeCityViolationPattern,
        CodeCityViolationRule, CodeCityViolationSeverity,
    };

    let temp = TempDir::new()?;
    let sqlite = SqliteConnectionPool::connect(temp.path().join("devql.sqlite"))?;
    let repo = SqliteCodeCityRepository::from_sqlite(sqlite);
    repo.initialise_schema()?;

    let evidence = CodeCityDependencyEvidence {
        evidence_id: "evidence-1".to_string(),
        run_id: "run-1".to_string(),
        commit_sha: Some("commit-1".to_string()),
        from_path: "src/domain.rs".to_string(),
        to_path: Some("src/api.rs".to_string()),
        to_symbol_ref: Some("crate::api".to_string()),
        from_boundary_id: Some("boundary:root".to_string()),
        to_boundary_id: Some("boundary:root".to_string()),
        from_zone: Some("core".to_string()),
        to_zone: Some("edge".to_string()),
        from_symbol_id: Some("from".to_string()),
        from_artefact_id: Some("artefact-from".to_string()),
        to_symbol_id: Some("to".to_string()),
        to_artefact_id: Some("artefact-to".to_string()),
        edge_id: Some("edge-1".to_string()),
        edge_kind: "imports".to_string(),
        language: Some("rust".to_string()),
        start_line: Some(3),
        end_line: Some(3),
        metadata_json: "{}".to_string(),
        resolved: true,
        cross_boundary: false,
    };
    let file_arc = CodeCityFileDependencyArc {
        arc_id: "arc-1".to_string(),
        run_id: "run-1".to_string(),
        commit_sha: Some("commit-1".to_string()),
        from_path: "src/domain.rs".to_string(),
        to_path: "src/api.rs".to_string(),
        from_boundary_id: Some("boundary:root".to_string()),
        to_boundary_id: Some("boundary:root".to_string()),
        from_zone: Some("core".to_string()),
        to_zone: Some("edge".to_string()),
        edge_count: 1,
        import_count: 1,
        call_count: 0,
        reference_count: 0,
        export_count: 0,
        inheritance_count: 0,
        weight: 1.0,
        cross_boundary: false,
        has_violation: true,
        highest_severity: Some(CodeCityViolationSeverity::High),
        evidence_ids: vec!["evidence-1".to_string()],
    };
    let violation_evidence = CodeCityViolationEvidence {
        evidence_id: "evidence-1".to_string(),
        edge_id: Some("edge-1".to_string()),
        edge_kind: "imports".to_string(),
        from_symbol_id: Some("from".to_string()),
        to_symbol_id: Some("to".to_string()),
        from_artefact_id: Some("artefact-from".to_string()),
        to_artefact_id: Some("artefact-to".to_string()),
        start_line: Some(3),
        end_line: Some(3),
        to_symbol_ref: Some("crate::api".to_string()),
    };
    let violation = CodeCityArchitectureViolation {
        id: "violation-1".to_string(),
        run_id: "run-1".to_string(),
        commit_sha: Some("commit-1".to_string()),
        boundary_id: Some("boundary:root".to_string()),
        boundary_root: Some(".".to_string()),
        pattern: CodeCityViolationPattern::Layered,
        rule: CodeCityViolationRule::LayeredUpwardDependency,
        severity: CodeCityViolationSeverity::High,
        from_path: "src/domain.rs".to_string(),
        to_path: Some("src/api.rs".to_string()),
        from_zone: Some("core".to_string()),
        to_zone: Some("edge".to_string()),
        from_boundary_id: Some("boundary:root".to_string()),
        to_boundary_id: Some("boundary:root".to_string()),
        arc_id: Some("arc-1".to_string()),
        message: "Core depends on edge.".to_string(),
        explanation: "Layered dependency points upward.".to_string(),
        recommendation: Some("Invert the dependency.".to_string()),
        evidence_ids: vec!["evidence-1".to_string()],
        evidence: vec![violation_evidence],
        confidence: 1.0,
        suppressed: false,
    };
    let render_arc = CodeCityRenderArc {
        id: "render-1".to_string(),
        kind: CodeCityArcKind::Violation,
        visibility: CodeCityArcVisibility::VisibleAtMediumZoom,
        severity: Some(CodeCityViolationSeverity::High),
        from_path: Some("src/domain.rs".to_string()),
        to_path: Some("src/api.rs".to_string()),
        from_boundary_id: Some("boundary:root".to_string()),
        to_boundary_id: Some("boundary:root".to_string()),
        source_arc_id: Some("arc-1".to_string()),
        violation_id: Some("violation-1".to_string()),
        weight: 1.0,
        label: Some("layered_upward_dependency".to_string()),
        tooltip: Some("Core depends on edge.".to_string()),
        geometry: CodeCityArcGeometry {
            from_x: 0.0,
            from_y: 1.0,
            from_z: 0.0,
            to_x: 2.0,
            to_y: 1.0,
            to_z: 0.0,
            control_y: 6.0,
        },
        metadata_json: "{}".to_string(),
    };
    let snapshot = CodeCityPhase4Snapshot {
        repo_id: "repo-1".to_string(),
        run_id: "run-1".to_string(),
        commit_sha: Some("commit-1".to_string()),
        evidence: vec![evidence],
        file_arcs: vec![file_arc],
        violations: vec![violation],
        render_arcs: vec![render_arc],
        diagnostics: Vec::new(),
    };

    repo.replace_phase4_snapshot(&snapshot)?;
    let loaded = repo.load_phase4_snapshot("repo-1")?;

    assert_eq!(loaded.evidence.len(), 1);
    assert_eq!(
        loaded.file_arcs[0].highest_severity,
        Some(CodeCityViolationSeverity::High)
    );
    assert_eq!(
        loaded.violations[0].rule,
        CodeCityViolationRule::LayeredUpwardDependency
    );
    assert_eq!(loaded.render_arcs[0].kind, CodeCityArcKind::Violation);
    Ok(())
}

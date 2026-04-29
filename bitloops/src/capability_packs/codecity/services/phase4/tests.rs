use super::render::geometry_between_buildings;
use super::*;
use crate::capability_packs::codecity::services::config::CodeCityConfig;
use crate::capability_packs::codecity::types::{
    CODECITY_CAPABILITY_ID, CODECITY_ROOT_BOUNDARY_ID, CodeCityBoundaryArchitectureReport,
    CodeCityBoundaryArchitectureSummary, CodeCityBoundaryGraphMetrics, CodeCityBoundaryKind,
    CodeCityBoundarySource, CodeCityGeometry, CodeCityHealthOverview, CodeCityHealthWeights,
    CodeCityLayoutSummary, CodeCityMacroGraph, CodeCityMacroTopology, CodeCitySummary,
};

fn world(buildings: Vec<CodeCityBuilding>) -> CodeCityWorldPayload {
    CodeCityWorldPayload {
        capability: CODECITY_CAPABILITY_ID.to_string(),
        stage: "codecity_world".to_string(),
        status: "ok".to_string(),
        repo_id: "repo-1".to_string(),
        commit_sha: Some("commit-1".to_string()),
        config_fingerprint: "fingerprint".to_string(),
        summary: CodeCitySummary::default(),
        health: CodeCityHealthOverview::not_requested(
            6,
            CodeCityHealthWeights {
                churn: 1.0,
                complexity: 1.0,
                bugs: 1.0,
                coverage: 1.0,
                author_concentration: 1.0,
            },
        ),
        legends: CodeCityLegends::default(),
        layout: CodeCityLayoutSummary::default(),
        boundaries: vec![boundary(
            CODECITY_ROOT_BOUNDARY_ID,
            CodeCityArchitecturePattern::Layered,
        )],
        macro_graph: None,
        architecture: None,
        boundary_layouts: Vec::new(),
        buildings,
        arcs: Vec::new(),
        dependency_arcs: Vec::new(),
        diagnostics: Vec::new(),
    }
}

fn boundary(id: &str, pattern: CodeCityArchitecturePattern) -> CodeCityBoundary {
    CodeCityBoundary {
        id: id.to_string(),
        name: id.to_string(),
        root_path: ".".to_string(),
        kind: CodeCityBoundaryKind::RootFallback,
        ecosystem: None,
        parent_boundary_id: None,
        source: CodeCityBoundarySource::Fallback,
        file_count: 2,
        artefact_count: 2,
        dependency_count: 1,
        entry_points: Vec::new(),
        shared_library: false,
        atomic: true,
        architecture: Some(CodeCityBoundaryArchitectureSummary {
            primary_pattern: pattern,
            primary_score: 0.8,
            secondary_pattern: None,
            mud_score: 0.0,
            modularity: 0.0,
        }),
        layout: None,
        violation_summary: Default::default(),
        diagnostics: Vec::new(),
    }
}

fn building(path: &str, zone: &str, x: f64) -> CodeCityBuilding {
    CodeCityBuilding {
        path: path.to_string(),
        language: "rust".to_string(),
        boundary_id: CODECITY_ROOT_BOUNDARY_ID.to_string(),
        zone: zone.to_string(),
        inferred_zone: Some(zone.to_string()),
        convention_zone: Some(zone.to_string()),
        architecture_role: None,
        importance: Default::default(),
        size: Default::default(),
        geometry: CodeCityGeometry {
            x,
            y: 0.0,
            z: 0.0,
            width: 2.0,
            depth: 2.0,
            side_length: 2.0,
            footprint_area: 4.0,
            height: 4.0,
        },
        health_risk: None,
        health_status: "insufficient_data".to_string(),
        health_confidence: 0.0,
        colour: "#888888".to_string(),
        health_summary: Default::default(),
        diagnostic_badges: Vec::new(),
        floors: Vec::new(),
    }
}

fn analysis(pattern: CodeCityArchitecturePattern) -> CodeCityArchitectureAnalysis {
    let boundary = boundary(CODECITY_ROOT_BOUNDARY_ID, pattern);
    CodeCityArchitectureAnalysis {
        boundaries: vec![boundary.clone()],
        file_to_boundary: BTreeMap::from([
            (
                "src/domain.rs".to_string(),
                CODECITY_ROOT_BOUNDARY_ID.to_string(),
            ),
            (
                "src/api.rs".to_string(),
                CODECITY_ROOT_BOUNDARY_ID.to_string(),
            ),
        ]),
        macro_graph: CodeCityMacroGraph {
            topology: CodeCityMacroTopology::SingleBoundary,
            boundary_count: 1,
            edge_count: 0,
            density: 0.0,
            modularity: None,
            edges: Vec::new(),
        },
        boundary_reports: vec![CodeCityBoundaryArchitectureReport {
            boundary_id: CODECITY_ROOT_BOUNDARY_ID.to_string(),
            primary_pattern: pattern,
            primary_score: 0.8,
            secondary_pattern: None,
            secondary_score: None,
            scores: Default::default(),
            metrics: CodeCityBoundaryGraphMetrics::default(),
            evidence: Vec::new(),
            diagnostics: Vec::new(),
        }],
        zone_assignments: BTreeMap::from([
            (
                "src/domain.rs".to_string(),
                crate::capability_packs::codecity::types::CodeCityZoneAssignment {
                    path: "src/domain.rs".to_string(),
                    boundary_id: CODECITY_ROOT_BOUNDARY_ID.to_string(),
                    zone: CodeCityZone::Core,
                    convention_zone: Some(CodeCityZone::Core),
                    inferred_zone: Some(CodeCityZone::Core),
                    depth_score: None,
                    confidence: 1.0,
                    disagreement: false,
                    reason: "fixture".to_string(),
                },
            ),
            (
                "src/api.rs".to_string(),
                crate::capability_packs::codecity::types::CodeCityZoneAssignment {
                    path: "src/api.rs".to_string(),
                    boundary_id: CODECITY_ROOT_BOUNDARY_ID.to_string(),
                    zone: CodeCityZone::Edge,
                    convention_zone: Some(CodeCityZone::Edge),
                    inferred_zone: Some(CodeCityZone::Edge),
                    depth_score: None,
                    confidence: 1.0,
                    disagreement: false,
                    reason: "fixture".to_string(),
                },
            ),
        ]),
        summary_report: crate::capability_packs::codecity::types::CodeCityArchitectureReport {
            macro_topology: CodeCityMacroTopology::SingleBoundary,
            primary_pattern: pattern,
            primary_score: 0.8,
            secondary_pattern: None,
            secondary_score: None,
            mud_score: 0.0,
            mud_warning: false,
            boundary_reports: Vec::new(),
            diagnostics: Vec::new(),
        },
        diagnostics: Vec::new(),
    }
}

fn source() -> CodeCitySourceGraph {
    CodeCitySourceGraph {
        project_path: None,
        files: Vec::new(),
        artefacts: Vec::new(),
        edges: vec![CodeCitySourceEdge {
            edge_id: "edge-1".to_string(),
            from_path: "src/domain.rs".to_string(),
            to_path: "src/api.rs".to_string(),
            from_symbol_id: "from".to_string(),
            from_artefact_id: "artefact-from".to_string(),
            to_symbol_id: Some("to".to_string()),
            to_artefact_id: Some("artefact-to".to_string()),
            to_symbol_ref: Some("crate::api".to_string()),
            edge_kind: "imports".to_string(),
            language: "rust".to_string(),
            start_line: Some(7),
            end_line: Some(7),
            metadata: "{}".to_string(),
        }],
        external_dependency_hints: Vec::new(),
        diagnostics: Vec::new(),
    }
}

#[test]
fn layered_upward_dependency_creates_high_violation_and_arc() {
    let mut world = world(vec![
        building("src/domain.rs", "core", 0.0),
        building("src/api.rs", "edge", 10.0),
    ]);
    let snapshot = enrich_world_with_phase4(
        &source(),
        &analysis(CodeCityArchitecturePattern::Layered),
        &mut world,
        &CodeCityConfig::default(),
    );

    assert_eq!(snapshot.violations.len(), 1);
    assert_eq!(
        snapshot.violations[0].rule,
        CodeCityViolationRule::LayeredUpwardDependency
    );
    assert_eq!(
        snapshot.violations[0].severity,
        CodeCityViolationSeverity::High
    );
    assert!(
        snapshot
            .render_arcs
            .iter()
            .any(|arc| arc.kind == CodeCityArcKind::Violation)
    );
    assert_eq!(world.summary.violation_count, 1);
    assert_eq!(world.buildings[0].diagnostic_badges.len(), 1);
}

#[test]
fn arc_geometry_lifts_with_distance() {
    let config = CodeCityConfig::default();
    let from = building("a.rs", "core", 0.0);
    let to = building("b.rs", "edge", 20.0);
    let geometry = geometry_between_buildings(&from, &to, &config);

    assert!(geometry.control_y > geometry.from_y);
    assert!(geometry.control_y > geometry.to_y);
}

#[test]
fn legends_include_violation_rule_explanations() {
    let legends = codecity_legends();
    assert!(
        legends
            .violation_rules
            .iter()
            .any(|rule| rule.rule == CodeCityViolationRule::LayeredUpwardDependency)
    );
}

#[test]
fn duplicate_violation_ids_merge_evidence() {
    let merged = merge_duplicate_violations(vec![
        violation_fixture("violation-1", "evidence-1", 0.4, false),
        violation_fixture("violation-1", "evidence-2", 0.8, true),
    ]);

    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].evidence_ids, vec!["evidence-1", "evidence-2"]);
    assert_eq!(
        merged[0]
            .evidence
            .iter()
            .map(|row| row.evidence_id.as_str())
            .collect::<Vec<_>>(),
        vec!["evidence-1", "evidence-2"]
    );
    assert_eq!(merged[0].confidence, 0.8);
    assert!(merged[0].suppressed);
}

fn violation_fixture(
    violation_id: &str,
    evidence_id: &str,
    confidence: f64,
    suppressed: bool,
) -> CodeCityArchitectureViolation {
    CodeCityArchitectureViolation {
        id: violation_id.to_string(),
        run_id: "run-1".to_string(),
        commit_sha: Some("commit-1".to_string()),
        boundary_id: Some(CODECITY_ROOT_BOUNDARY_ID.to_string()),
        boundary_root: Some(".".to_string()),
        pattern: CodeCityViolationPattern::Layered,
        rule: CodeCityViolationRule::LayeredUpwardDependency,
        severity: CodeCityViolationSeverity::High,
        from_path: "src/domain.rs".to_string(),
        to_path: Some("src/api.rs".to_string()),
        from_zone: Some("core".to_string()),
        to_zone: Some("edge".to_string()),
        from_boundary_id: Some(CODECITY_ROOT_BOUNDARY_ID.to_string()),
        to_boundary_id: Some(CODECITY_ROOT_BOUNDARY_ID.to_string()),
        arc_id: Some("arc-1".to_string()),
        message: "message".to_string(),
        explanation: "explanation".to_string(),
        recommendation: None,
        evidence_ids: vec![evidence_id.to_string()],
        evidence: vec![CodeCityViolationEvidence {
            evidence_id: evidence_id.to_string(),
            edge_id: Some(format!("edge-{evidence_id}")),
            edge_kind: "imports".to_string(),
            from_symbol_id: Some("from".to_string()),
            to_symbol_id: Some("to".to_string()),
            from_artefact_id: Some("artefact-from".to_string()),
            to_artefact_id: Some("artefact-to".to_string()),
            start_line: Some(1),
            end_line: Some(1),
            to_symbol_ref: Some("crate::api".to_string()),
        }],
        confidence,
        suppressed,
    }
}

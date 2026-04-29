use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

use super::architecture::CodeCityArchitectureAnalysis;
use super::community_detection::detect_communities;
use super::config::CodeCityConfig;
use super::graph_metrics::{FileGraph, strongly_connected_components};
use super::source_graph::{CodeCitySourceEdge, CodeCitySourceGraph};
use crate::capability_packs::codecity::types::{
    CODECITY_UNCLASSIFIED_ZONE, CodeCityArcConnectionEdgePayload, CodeCityArcConnectionPayload,
    CodeCityArcFilter, CodeCityArcGeometry, CodeCityArcKind, CodeCityArcKindLegend,
    CodeCityArcVisibility, CodeCityArchitecturePattern, CodeCityArchitectureViolation,
    CodeCityBoundary, CodeCityBoundaryArchitectureReport, CodeCityBuilding,
    CodeCityDependencyConnectionEdgePayload, CodeCityDependencyConnectionPayload,
    CodeCityDependencyDirection, CodeCityDependencyEvidence, CodeCityDiagnostic,
    CodeCityDiagnosticBadge, CodeCityDiagnosticBadgeKind, CodeCityFileArchitectureContext,
    CodeCityFileDependencyArc, CodeCityFileDetailPayload, CodeCityLegends, CodeCityPageInfo,
    CodeCityPhase4Snapshot, CodeCityRenderArc, CodeCitySeverityLegend,
    CodeCityViolationConnectionEdgePayload, CodeCityViolationConnectionPayload,
    CodeCityViolationEvidence, CodeCityViolationFilter, CodeCityViolationPattern,
    CodeCityViolationRule, CodeCityViolationRuleCount, CodeCityViolationRuleLegend,
    CodeCityViolationSeverity, CodeCityViolationSummary, CodeCityWorldPayload, CodeCityZone,
};

const MAX_EVIDENCE_IDS_PER_ARC: usize = 20;

mod evidence;
mod legends;
mod query;
mod render;
mod rules;
#[cfg(test)]
mod tests;

use evidence::{aggregate_file_arcs, build_dependency_evidence};
use render::{
    apply_boundary_violation_summaries, apply_diagnostic_badges,
    apply_violation_state_to_file_arcs, build_render_arcs, world_arcs,
};
use rules::evaluate_violations;

pub use legends::codecity_legends;
pub use query::{
    arcs_connection, file_detail, filter_arcs, filter_violations, violations_connection,
};

type EvidenceById<'data> = BTreeMap<String, &'data CodeCityDependencyEvidence>;
type BoundaryReports<'data> = BTreeMap<String, &'data CodeCityBoundaryArchitectureReport>;
type BoundariesById<'data> = BTreeMap<String, &'data CodeCityBoundary>;

struct RuleEvaluationContext<'ctx, 'data> {
    evidence_by_id: &'ctx EvidenceById<'data>,
    reports: &'ctx BoundaryReports<'data>,
    boundaries: &'ctx BoundariesById<'data>,
    run_id: &'ctx str,
    world: &'ctx CodeCityWorldPayload,
    config: &'ctx CodeCityConfig,
}

struct ArcViolationSpec {
    pattern: CodeCityViolationPattern,
    rule: CodeCityViolationRule,
    severity: CodeCityViolationSeverity,
    message: String,
    explanation: String,
    recommendation: Option<String>,
}

pub fn enrich_world_with_phase4(
    source: &CodeCitySourceGraph,
    analysis: &CodeCityArchitectureAnalysis,
    world: &mut CodeCityWorldPayload,
    config: &CodeCityConfig,
) -> CodeCityPhase4Snapshot {
    let mut snapshot = build_phase4_snapshot(source, analysis, world, config);

    apply_violation_state_to_file_arcs(&mut snapshot.file_arcs, &snapshot.violations);
    let render_arcs = build_render_arcs(&snapshot.file_arcs, &snapshot.violations, world, config);
    snapshot.render_arcs = render_arcs;

    world.legends = codecity_legends();
    world.summary.violation_count = snapshot.violations.len();
    world.summary.high_severity_violation_count = snapshot
        .violations
        .iter()
        .filter(|violation| violation.severity == CodeCityViolationSeverity::High)
        .count();
    world.summary.visible_arc_count = snapshot
        .render_arcs
        .iter()
        .filter(|arc| arc.visibility != CodeCityArcVisibility::HiddenByDefault)
        .count();
    world.summary.cross_boundary_arc_count = snapshot
        .render_arcs
        .iter()
        .filter(|arc| arc.kind == CodeCityArcKind::CrossBoundary)
        .count();

    apply_diagnostic_badges(
        &mut world.buildings,
        &snapshot.violations,
        &snapshot.file_arcs,
    );
    apply_boundary_violation_summaries(&mut world.boundaries, &snapshot.violations);

    world.arcs = world_arcs(&snapshot.render_arcs, config);
    world.diagnostics.extend(snapshot.diagnostics.clone());
    world.diagnostics.sort_by(|left, right| {
        left.severity
            .cmp(&right.severity)
            .then_with(|| left.code.cmp(&right.code))
            .then_with(|| left.path.cmp(&right.path))
            .then_with(|| left.boundary_id.cmp(&right.boundary_id))
    });

    snapshot
}

pub fn build_phase4_snapshot(
    source: &CodeCitySourceGraph,
    analysis: &CodeCityArchitectureAnalysis,
    world: &CodeCityWorldPayload,
    config: &CodeCityConfig,
) -> CodeCityPhase4Snapshot {
    let run_id = stable_id(
        "codecity-phase4-run",
        &[
            world.repo_id.as_str(),
            world.config_fingerprint.as_str(),
            world.commit_sha.as_deref().unwrap_or(""),
        ],
    );
    let building_by_path = world
        .buildings
        .iter()
        .map(|building| (building.path.clone(), building))
        .collect::<BTreeMap<_, _>>();
    let boundary_by_id = analysis
        .boundaries
        .iter()
        .map(|boundary| (boundary.id.clone(), boundary))
        .collect::<BTreeMap<_, _>>();
    let report_by_boundary = analysis
        .boundary_reports
        .iter()
        .map(|report| (report.boundary_id.clone(), report))
        .collect::<BTreeMap<_, _>>();

    let mut diagnostics = Vec::new();
    let evidence = build_dependency_evidence(
        source,
        analysis,
        world.repo_id.as_str(),
        &run_id,
        world.commit_sha.clone(),
    );
    let mut file_arcs = aggregate_file_arcs(&evidence, world.repo_id.as_str(), &run_id);
    let mut violations = if config.violations.enabled {
        evaluate_violations(
            source,
            analysis,
            &file_arcs,
            &evidence,
            &building_by_path,
            &boundary_by_id,
            &report_by_boundary,
            &run_id,
            world,
            config,
            &mut diagnostics,
        )
    } else {
        Vec::new()
    };
    violations.sort_by(compare_violations);
    apply_violation_state_to_file_arcs(&mut file_arcs, &violations);

    CodeCityPhase4Snapshot {
        repo_id: world.repo_id.clone(),
        run_id,
        commit_sha: world.commit_sha.clone(),
        evidence,
        file_arcs,
        violations,
        render_arcs: Vec::new(),
        diagnostics,
    }
}

pub(super) fn compare_violations(
    left: &CodeCityArchitectureViolation,
    right: &CodeCityArchitectureViolation,
) -> std::cmp::Ordering {
    left.severity
        .rank()
        .cmp(&right.severity.rank())
        .then_with(|| left.rule.cmp(&right.rule))
        .then_with(|| left.from_path.cmp(&right.from_path))
        .then_with(|| left.to_path.cmp(&right.to_path))
        .then_with(|| left.id.cmp(&right.id))
}

pub(super) fn compare_render_arcs(
    left: &CodeCityRenderArc,
    right: &CodeCityRenderArc,
) -> std::cmp::Ordering {
    left.kind
        .cmp(&right.kind)
        .then_with(|| {
            left.severity
                .map(CodeCityViolationSeverity::rank)
                .cmp(&right.severity.map(CodeCityViolationSeverity::rank))
        })
        .then_with(|| {
            right
                .weight
                .partial_cmp(&left.weight)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .then_with(|| left.id.cmp(&right.id))
}

fn violation_cursor(violation: &CodeCityArchitectureViolation) -> String {
    violation.id.clone()
}

fn arc_cursor(arc: &CodeCityRenderArc) -> String {
    arc.id.clone()
}

pub(super) fn stable_id(prefix: &str, parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update(b"\0");
    }
    let encoded = hex::encode(hasher.finalize());
    format!("{prefix}:{}", &encoded[..20])
}

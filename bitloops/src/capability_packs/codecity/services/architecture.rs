use std::collections::BTreeMap;

use super::architecture_classifiers::classify_boundary_architecture;
use super::boundary_detection::{CodeCityBoundaryDetectionResult, detect_boundaries};
use super::config::CodeCityConfig;
use super::graph_metrics::{build_graph_from_paths, compute_boundary_metrics};
use super::macro_graph::{apply_shared_library_flags, build_macro_graph};
use super::source_graph::CodeCitySourceGraph;
use super::zone_assignment::assign_zones;
use crate::capability_packs::codecity::types::{
    CodeCityArchitecturePattern, CodeCityArchitectureReport, CodeCityBoundary,
    CodeCityBoundaryArchitectureReport, CodeCityBoundaryArchitectureSummary, CodeCityDiagnostic,
    CodeCityMacroGraph, CodeCityZoneAssignment,
};

const MAX_INTERACTIVE_ARCHITECTURE_FILES: usize = 2048;

#[derive(Debug, Clone, PartialEq)]
pub struct CodeCityArchitectureAnalysis {
    pub boundaries: Vec<CodeCityBoundary>,
    pub file_to_boundary: BTreeMap<String, String>,
    pub macro_graph: CodeCityMacroGraph,
    pub boundary_reports: Vec<CodeCityBoundaryArchitectureReport>,
    pub zone_assignments: BTreeMap<String, CodeCityZoneAssignment>,
    pub summary_report: CodeCityArchitectureReport,
    pub diagnostics: Vec<CodeCityDiagnostic>,
}

pub fn analyse_codecity_architecture(
    source: &CodeCitySourceGraph,
    config: &CodeCityConfig,
    repo_root: &std::path::Path,
) -> CodeCityArchitectureAnalysis {
    let boundary_result = detect_boundaries(source, config, repo_root);
    analyse_codecity_architecture_from_boundary_result(source, boundary_result, config)
}

pub fn analyse_codecity_architecture_from_boundary_result(
    source: &CodeCitySourceGraph,
    boundary_result: CodeCityBoundaryDetectionResult,
    config: &CodeCityConfig,
) -> CodeCityArchitectureAnalysis {
    let mut boundaries = boundary_result.boundaries.clone();
    let file_to_boundary = boundary_result.file_to_boundary.clone();
    let macro_graph = build_macro_graph(source, &boundaries, &file_to_boundary);
    apply_shared_library_flags(&mut boundaries, &macro_graph, config);

    let mut boundary_reports = Vec::new();
    for boundary in &boundaries {
        let files = file_to_boundary
            .iter()
            .filter_map(|(path, boundary_id)| (boundary_id == &boundary.id).then_some(path.clone()))
            .collect::<Vec<_>>();
        let graph = build_graph_from_paths(&files, &source.edges);
        if graph.paths.len() > MAX_INTERACTIVE_ARCHITECTURE_FILES {
            boundary_reports.push(large_boundary_report(boundary, &graph));
            continue;
        }
        let communities = super::community_detection::detect_communities(
            &graph,
            config.boundaries.community_max_iterations,
        );
        let metrics = compute_boundary_metrics(&graph, &communities);
        let report = if !config.architecture.enabled || files.len() < 2 {
            CodeCityBoundaryArchitectureReport {
                boundary_id: boundary.id.clone(),
                primary_pattern: CodeCityArchitecturePattern::Unclassified,
                primary_score: 0.0,
                secondary_pattern: None,
                secondary_score: None,
                scores: Default::default(),
                metrics: metrics.clone(),
                evidence: Vec::new(),
                diagnostics: vec![CodeCityDiagnostic {
                    code: if config.architecture.enabled {
                        "codecity.architecture.too_small".to_string()
                    } else {
                        "codecity.architecture.disabled".to_string()
                    },
                    severity: "info".to_string(),
                    message: if config.architecture.enabled {
                        format!(
                            "Boundary `{}` is too small for architecture classification.",
                            boundary.id
                        )
                    } else {
                        format!(
                            "Architecture classification is disabled for boundary `{}`.",
                            boundary.id
                        )
                    },
                    path: None,
                    boundary_id: Some(boundary.id.clone()),
                }],
            }
        } else {
            classify_boundary_architecture(boundary, &graph, &metrics, &communities, source, config)
        };
        boundary_reports.push(report);
    }
    boundary_reports.sort_by(|left, right| left.boundary_id.cmp(&right.boundary_id));

    for boundary in &mut boundaries {
        if let Some(report) = boundary_reports
            .iter()
            .find(|report| report.boundary_id == boundary.id)
        {
            boundary.architecture = Some(CodeCityBoundaryArchitectureSummary {
                primary_pattern: report.primary_pattern,
                primary_score: report.primary_score,
                secondary_pattern: report.secondary_pattern,
                mud_score: report.scores.ball_of_mud,
                modularity: report.metrics.modularity,
            });
        }
    }

    let zone_assignments = assign_zones(
        source,
        &boundaries,
        &boundary_reports,
        &file_to_boundary,
        config,
    );
    let mut diagnostics = boundary_result.diagnostics;
    diagnostics.extend(
        boundary_reports
            .iter()
            .flat_map(|report| report.diagnostics.clone())
            .collect::<Vec<_>>(),
    );
    if config.include_zone_diagnostics {
        diagnostics.extend(zone_assignments.values().filter_map(|assignment| {
            assignment.disagreement.then_some(CodeCityDiagnostic {
                code: "codecity.zone.disagreement".to_string(),
                severity: "warning".to_string(),
                message: format!(
                    "Path `{}` had a convention/dependency zone disagreement.",
                    assignment.path
                ),
                path: Some(assignment.path.clone()),
                boundary_id: Some(assignment.boundary_id.clone()),
            })
        }));
    }
    diagnostics.sort_by(|left, right| {
        left.severity
            .cmp(&right.severity)
            .then_with(|| left.code.cmp(&right.code))
            .then_with(|| left.path.cmp(&right.path))
    });

    let summary_report =
        summarise_architecture(&macro_graph, &boundary_reports, &diagnostics, config);

    CodeCityArchitectureAnalysis {
        boundaries,
        file_to_boundary,
        macro_graph,
        boundary_reports,
        zone_assignments,
        summary_report,
        diagnostics,
    }
}

fn large_boundary_report(
    boundary: &CodeCityBoundary,
    graph: &super::graph_metrics::FileGraph,
) -> CodeCityBoundaryArchitectureReport {
    let node_count = graph.paths.len();
    let edge_count = graph.edges.len();
    let density = if node_count <= 1 {
        0.0
    } else {
        edge_count as f64 / (node_count * (node_count - 1)) as f64
    };

    CodeCityBoundaryArchitectureReport {
        boundary_id: boundary.id.clone(),
        primary_pattern: CodeCityArchitecturePattern::Unclassified,
        primary_score: 0.0,
        secondary_pattern: None,
        secondary_score: None,
        scores: Default::default(),
        metrics: crate::capability_packs::codecity::types::CodeCityBoundaryGraphMetrics {
            node_count,
            edge_count,
            density,
            community_count: 1,
            ..Default::default()
        },
        evidence: Vec::new(),
        diagnostics: vec![CodeCityDiagnostic {
            code: "codecity.architecture.too_large".to_string(),
            severity: "info".to_string(),
            message: format!(
                "Boundary `{}` has {node_count} files, so detailed architecture classification was skipped for interactive rendering.",
                boundary.id
            ),
            path: None,
            boundary_id: Some(boundary.id.clone()),
        }],
    }
}

fn summarise_architecture(
    macro_graph: &CodeCityMacroGraph,
    boundary_reports: &[CodeCityBoundaryArchitectureReport],
    diagnostics: &[CodeCityDiagnostic],
    config: &CodeCityConfig,
) -> CodeCityArchitectureReport {
    let boundary_count = boundary_reports.len().max(1) as f64;
    let average = |selector: fn(
        &crate::capability_packs::codecity::types::CodeCityArchitectureScores,
    ) -> f64| {
        boundary_reports
            .iter()
            .map(|report| selector(&report.scores))
            .sum::<f64>()
            / boundary_count
    };

    let layered = average(|scores| scores.layered);
    let hexagonal = average(|scores| scores.hexagonal);
    let modular = average(|scores| scores.modular);
    let event_driven = average(|scores| scores.event_driven);
    let pipe_and_filter = average(|scores| scores.pipe_and_filter);
    let ball_of_mud = average(|scores| scores.ball_of_mud);

    let mut ranked = [
        (CodeCityArchitecturePattern::Layered, layered),
        (CodeCityArchitecturePattern::Hexagonal, hexagonal),
        (CodeCityArchitecturePattern::Modular, modular),
        (CodeCityArchitecturePattern::EventDriven, event_driven),
        (CodeCityArchitecturePattern::PipeAndFilter, pipe_and_filter),
    ];
    ranked.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let (primary_pattern, primary_score) = if ranked[0].1 < 0.3 {
        if ball_of_mud > config.architecture.mud_warning_threshold {
            (CodeCityArchitecturePattern::BallOfMud, ball_of_mud)
        } else {
            (CodeCityArchitecturePattern::Unclassified, 0.0)
        }
    } else {
        (ranked[0].0, ranked[0].1)
    };
    let secondary_pattern = ranked
        .iter()
        .skip(1)
        .find(|(_, score)| *score > config.architecture.secondary_pattern_threshold)
        .map(|(pattern, _)| *pattern);
    let secondary_score = ranked
        .iter()
        .skip(1)
        .find(|(_, score)| *score > config.architecture.secondary_pattern_threshold)
        .map(|(_, score)| *score);

    CodeCityArchitectureReport {
        macro_topology: macro_graph.topology,
        primary_pattern,
        primary_score,
        secondary_pattern,
        secondary_score,
        mud_score: ball_of_mud,
        mud_warning: ball_of_mud > config.architecture.mud_warning_threshold,
        boundary_reports: boundary_reports.to_vec(),
        diagnostics: diagnostics.to_vec(),
    }
}

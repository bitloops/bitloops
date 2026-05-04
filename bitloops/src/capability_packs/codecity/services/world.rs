use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::Result;

use super::architecture::{CodeCityArchitectureAnalysis, analyse_codecity_architecture};
use super::config::CodeCityConfig;
use super::graph_metrics::{build_file_graph, compute_importance};
use super::height::{build_floors_for_file, building_loc, total_height};
use super::layout::{apply_architecture_layout, apply_grid_treemap_layout};
use super::source_graph::CodeCitySourceGraph;
use crate::capability_packs::codecity::types::{
    CODECITY_CAPABILITY_ID, CODECITY_ROOT_BOUNDARY_ID, CODECITY_WORLD_STAGE_ID, CodeCityBoundary,
    CodeCityBoundaryLayoutPreview, CodeCityBoundaryLayoutSummary, CodeCityBuilding,
    CodeCityBuildingHealthSummary, CodeCityDiagnostic, CodeCityGeometry, CodeCityHealthOverview,
    CodeCityHealthWeights, CodeCityImportance, CodeCityLayoutStrategy, CodeCityLayoutSummary,
    CodeCityLegends, CodeCitySize, CodeCitySnapshotStatus, CodeCitySummary, CodeCityWorldPayload,
};

pub fn build_codecity_world(
    source: &CodeCitySourceGraph,
    repo_id: &str,
    commit_sha: Option<String>,
    config: CodeCityConfig,
    repo_root: &Path,
) -> Result<CodeCityWorldPayload> {
    let analysis = analyse_codecity_architecture(source, &config, repo_root);
    build_codecity_world_from_analysis(source, repo_id, commit_sha, &config, &analysis)
}

pub fn build_codecity_world_from_analysis(
    source: &CodeCitySourceGraph,
    repo_id: &str,
    commit_sha: Option<String>,
    config: &CodeCityConfig,
    analysis: &CodeCityArchitectureAnalysis,
) -> Result<CodeCityWorldPayload> {
    let config_fingerprint = config.fingerprint()?;
    let mut diagnostics = source.diagnostics.clone();
    let included_files = source
        .files
        .iter()
        .filter(|file| file.included)
        .cloned()
        .collect::<Vec<_>>();
    let artefact_count = source
        .artefacts
        .iter()
        .filter(|artefact| !is_file_artefact(artefact))
        .count();

    diagnostics.push(CodeCityDiagnostic {
        code: "codecity.loc.line_span_fallback".to_string(),
        severity: "info".to_string(),
        message:
            "Architecture analysis still approximates floor size with artefact line spans rather than semantic LoC."
                .to_string(),
        path: None,
        boundary_id: None,
    });

    if source.files.is_empty() {
        diagnostics.push(CodeCityDiagnostic {
            code: "codecity.source.no_current_files".to_string(),
            severity: "info".to_string(),
            message: "No current DevQL file rows were available for this repository.".to_string(),
            path: None,
            boundary_id: None,
        });
    }

    if included_files.is_empty() {
        return Ok(CodeCityWorldPayload {
            capability: CODECITY_CAPABILITY_ID.to_string(),
            stage: CODECITY_WORLD_STAGE_ID.to_string(),
            status: "empty".to_string(),
            repo_id: repo_id.to_string(),
            commit_sha,
            config_fingerprint,
            snapshot_status: CodeCitySnapshotStatus::default(),
            summary: CodeCitySummary {
                file_count: source.files.len(),
                artefact_count,
                dependency_count: source.edges.len(),
                boundary_count: 0,
                macro_edge_count: 0,
                included_file_count: 0,
                excluded_file_count: source.files.len(),
                unhealthy_floor_count: 0,
                insufficient_health_data_count: 0,
                coverage_available: false,
                git_history_available: false,
                violation_count: 0,
                high_severity_violation_count: 0,
                visible_arc_count: 0,
                cross_boundary_arc_count: 0,
                max_importance: 0.0,
                max_height: 0.0,
            },
            health: CodeCityHealthOverview::not_requested(
                config.health.analysis_window_months,
                CodeCityHealthWeights::from(&config.health),
            ),
            legends: CodeCityLegends::default(),
            layout: CodeCityLayoutSummary::default(),
            boundaries: Vec::new(),
            macro_graph: None,
            architecture: None,
            boundary_layouts: Vec::new(),
            buildings: Vec::new(),
            arcs: Vec::new(),
            dependency_arcs: Vec::new(),
            diagnostics,
        });
    }

    let file_graph = build_file_graph(&included_files, &source.edges);
    let importance_by_path = compute_importance(&file_graph, &config.importance);
    let mut artefacts_by_path = BTreeMap::<String, Vec<_>>::new();
    for artefact in source.artefacts.clone() {
        artefacts_by_path
            .entry(artefact.path.clone())
            .or_default()
            .push(artefact);
    }

    if source.edges.is_empty() {
        diagnostics.push(CodeCityDiagnostic {
            code: "codecity.dependencies.empty".to_string(),
            severity: "info".to_string(),
            message: "No resolved cross-file dependencies were available for CodeCity scoring."
                .to_string(),
            path: None,
            boundary_id: None,
        });
    }

    let mut buildings = included_files
        .into_iter()
        .map(|file| {
            let artefacts = artefacts_by_path.remove(&file.path).unwrap_or_default();
            let floors = build_floors_for_file(&file, &artefacts, &config.height, &config.colours);
            let importance = importance_by_path
                .get(&file.path)
                .cloned()
                .unwrap_or_else(CodeCityImportance::default);
            let total_height = total_height(&floors, &config.height);
            let side_length = config.importance.min_footprint
                + (config.importance.max_footprint - config.importance.min_footprint)
                    * importance.score.clamp(0.0, 1.0).sqrt();

            CodeCityBuilding {
                path: file.path,
                language: file.language,
                boundary_id: CODECITY_ROOT_BOUNDARY_ID.to_string(),
                zone: "unclassified".to_string(),
                inferred_zone: None,
                convention_zone: None,
                architecture_role: None,
                importance,
                size: CodeCitySize {
                    loc: building_loc(&floors),
                    artefact_count: floors.len(),
                    total_height,
                },
                geometry: CodeCityGeometry {
                    side_length,
                    footprint_area: side_length * side_length,
                    height: total_height,
                    width: side_length,
                    depth: side_length,
                    ..CodeCityGeometry::default()
                },
                health_risk: None,
                health_status: "insufficient_data".to_string(),
                health_confidence: 0.0,
                colour: config.colours.no_data.clone(),
                health_summary: CodeCityBuildingHealthSummary {
                    floor_count: floors.len(),
                    insufficient_data_floor_count: floors.len(),
                    ..CodeCityBuildingHealthSummary::default()
                },
                diagnostic_badges: Vec::new(),
                floors,
            }
        })
        .collect::<Vec<_>>();

    buildings.sort_by(|left, right| {
        right
            .importance
            .score
            .partial_cmp(&left.importance.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| {
                right
                    .size
                    .total_height
                    .partial_cmp(&left.size.total_height)
                    .unwrap_or(Ordering::Equal)
            })
            .then_with(|| left.path.cmp(&right.path))
    });

    diagnostics.extend(analysis.diagnostics.clone());
    let report_by_boundary = analysis
        .boundary_reports
        .iter()
        .map(|report| (report.boundary_id.clone(), report))
        .collect::<BTreeMap<_, _>>();

    for building in &mut buildings {
        if let Some(assignment) = analysis.zone_assignments.get(&building.path) {
            building.boundary_id = assignment.boundary_id.clone();
            building.zone = assignment.zone.as_str().to_string();
            building.inferred_zone = assignment
                .inferred_zone
                .map(|zone| zone.as_str().to_string());
            building.convention_zone = assignment
                .convention_zone
                .map(|zone| zone.as_str().to_string());
            if let Some(report) = report_by_boundary.get(&assignment.boundary_id) {
                building.architecture_role = Some(format!(
                    "{}_{}",
                    report.primary_pattern.as_str(),
                    assignment.zone.as_str()
                ));
            }
        }
    }

    let is_single_boundary_fallback = analysis.boundaries.len() == 1
        && analysis.boundaries[0].id == CODECITY_ROOT_BOUNDARY_ID
        && analysis.boundaries[0].kind
            == crate::capability_packs::codecity::types::CodeCityBoundaryKind::RootFallback;

    let (layout, mut boundary_layouts, mut boundaries) = if is_single_boundary_fallback {
        let layout = apply_grid_treemap_layout(&mut buildings, &config.layout);
        let boundary_layouts = vec![CodeCityBoundaryLayoutSummary {
            boundary_id: CODECITY_ROOT_BOUNDARY_ID.to_string(),
            strategy: CodeCityLayoutStrategy::GridTreemap,
            zone_count: 1,
            width: layout.width,
            depth: layout.depth,
            x: 0.0,
            z: 0.0,
        }];
        let mut boundaries = analysis.boundaries.clone();
        if let Some(boundary) = boundaries.first_mut() {
            boundary.layout = Some(
                crate::capability_packs::codecity::types::CodeCityBoundaryLayoutPreview {
                    strategy: CodeCityLayoutStrategy::GridTreemap,
                    zone_count: 1,
                },
            );
        }
        (layout, boundary_layouts, boundaries)
    } else {
        let mut boundaries = analysis.boundaries.clone();
        let parent_ids = boundary_parent_ids(&boundaries);
        let mut layout_boundaries = boundaries
            .iter()
            .filter(|boundary| !parent_ids.contains(&boundary.id))
            .cloned()
            .collect::<Vec<_>>();
        let (layout, boundary_layouts) = apply_architecture_layout(
            &mut buildings,
            &mut layout_boundaries,
            &analysis.boundary_reports,
            &analysis.macro_graph,
            &analysis.zone_assignments,
            &config.layout,
        );
        let layout_by_boundary = layout_boundaries
            .into_iter()
            .map(|boundary| (boundary.id.clone(), boundary.layout))
            .collect::<BTreeMap<_, _>>();
        for boundary in &mut boundaries {
            if let Some(layout) = layout_by_boundary.get(&boundary.id) {
                boundary.layout = layout.clone();
            }
        }
        let mut boundary_layouts = boundary_layouts;
        apply_parent_boundary_layouts(&mut boundaries, &mut boundary_layouts);
        (layout, boundary_layouts, boundaries)
    };

    boundaries.sort_by(|left, right| {
        left.root_path
            .cmp(&right.root_path)
            .then_with(|| left.id.cmp(&right.id))
    });
    boundary_layouts.sort_by(|left, right| left.boundary_id.cmp(&right.boundary_id));

    let max_importance = buildings
        .iter()
        .map(|building| building.importance.score)
        .fold(0.0_f64, f64::max);
    let max_height = buildings
        .iter()
        .map(|building| building.geometry.height)
        .fold(0.0_f64, f64::max);

    let dependency_arcs = if config.include_dependency_arcs {
        super::source_graph::build_dependency_arcs(&source.edges)
    } else {
        Vec::new()
    };

    let macro_graph = config.include_macro_edges.then(|| {
        let mut macro_graph = analysis.macro_graph.clone();
        macro_graph.edges.sort_by(|left, right| {
            left.from_boundary_id
                .cmp(&right.from_boundary_id)
                .then_with(|| left.to_boundary_id.cmp(&right.to_boundary_id))
        });
        macro_graph
    });

    Ok(CodeCityWorldPayload {
        capability: CODECITY_CAPABILITY_ID.to_string(),
        stage: CODECITY_WORLD_STAGE_ID.to_string(),
        status: "ok".to_string(),
        repo_id: repo_id.to_string(),
        commit_sha,
        config_fingerprint,
        snapshot_status: CodeCitySnapshotStatus::default(),
        summary: CodeCitySummary {
            file_count: source.files.len(),
            artefact_count,
            dependency_count: source.edges.len(),
            boundary_count: analysis.boundaries.len(),
            macro_edge_count: analysis.macro_graph.edge_count,
            included_file_count: buildings.len(),
            excluded_file_count: source.files.len().saturating_sub(buildings.len()),
            unhealthy_floor_count: 0,
            insufficient_health_data_count: buildings
                .iter()
                .map(|building| building.floors.len())
                .sum(),
            coverage_available: false,
            git_history_available: false,
            violation_count: 0,
            high_severity_violation_count: 0,
            visible_arc_count: 0,
            cross_boundary_arc_count: 0,
            max_importance,
            max_height,
        },
        health: CodeCityHealthOverview::not_requested(
            config.health.analysis_window_months,
            CodeCityHealthWeights::from(&config.health),
        ),
        legends: CodeCityLegends::default(),
        layout,
        boundaries: if config.include_boundaries {
            boundaries
        } else {
            Vec::new()
        },
        macro_graph,
        architecture: config
            .include_architecture
            .then_some(analysis.summary_report.clone()),
        boundary_layouts,
        buildings,
        arcs: Vec::new(),
        dependency_arcs,
        diagnostics,
    })
}

fn boundary_parent_ids(boundaries: &[CodeCityBoundary]) -> BTreeSet<String> {
    boundaries
        .iter()
        .filter_map(|boundary| boundary.parent_boundary_id.clone())
        .collect()
}

fn apply_parent_boundary_layouts(
    boundaries: &mut [CodeCityBoundary],
    boundary_layouts: &mut Vec<CodeCityBoundaryLayoutSummary>,
) {
    let mut children_by_parent = BTreeMap::<String, Vec<String>>::new();
    for boundary in boundaries.iter() {
        if let Some(parent_id) = &boundary.parent_boundary_id {
            children_by_parent
                .entry(parent_id.clone())
                .or_default()
                .push(boundary.id.clone());
        }
    }

    let mut layout_by_boundary = boundary_layouts
        .iter()
        .cloned()
        .map(|layout| (layout.boundary_id.clone(), layout))
        .collect::<BTreeMap<_, _>>();

    loop {
        let mut changed = false;
        for boundary in boundaries.iter_mut() {
            if layout_by_boundary.contains_key(&boundary.id) {
                continue;
            }
            let Some(child_ids) = children_by_parent.get(&boundary.id) else {
                continue;
            };
            let child_layouts = child_ids
                .iter()
                .filter_map(|child_id| layout_by_boundary.get(child_id))
                .collect::<Vec<_>>();
            if child_layouts.len() != child_ids.len() || child_layouts.is_empty() {
                continue;
            }

            let min_x = child_layouts
                .iter()
                .map(|layout| layout.x)
                .fold(f64::INFINITY, f64::min);
            let min_z = child_layouts
                .iter()
                .map(|layout| layout.z)
                .fold(f64::INFINITY, f64::min);
            let max_x = child_layouts
                .iter()
                .map(|layout| layout.x + layout.width)
                .fold(0.0_f64, f64::max);
            let max_z = child_layouts
                .iter()
                .map(|layout| layout.z + layout.depth)
                .fold(0.0_f64, f64::max);

            let zone_count = child_layouts.len();
            let summary = CodeCityBoundaryLayoutSummary {
                boundary_id: boundary.id.clone(),
                strategy: CodeCityLayoutStrategy::GridTreemap,
                zone_count,
                width: max_x - min_x,
                depth: max_z - min_z,
                x: min_x,
                z: min_z,
            };
            boundary.layout = Some(CodeCityBoundaryLayoutPreview {
                strategy: CodeCityLayoutStrategy::GridTreemap,
                zone_count,
            });
            layout_by_boundary.insert(boundary.id.clone(), summary.clone());
            boundary_layouts.push(summary);
            changed = true;
        }

        if !changed {
            break;
        }
    }
}

fn is_file_artefact(
    artefact: &crate::capability_packs::codecity::services::source_graph::CodeCitySourceArtefact,
) -> bool {
    artefact
        .canonical_kind
        .as_deref()
        .unwrap_or("")
        .trim()
        .eq_ignore_ascii_case("file")
}

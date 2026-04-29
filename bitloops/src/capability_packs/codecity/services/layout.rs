use std::collections::BTreeMap;

use crate::capability_packs::codecity::types::{
    CodeCityArchitecturePattern, CodeCityBoundary, CodeCityBoundaryArchitectureReport,
    CodeCityBoundaryLayoutPreview, CodeCityBoundaryLayoutSummary, CodeCityBuilding,
    CodeCityLayoutStrategy, CodeCityLayoutSummary, CodeCityMacroGraph, CodeCityZone,
    CodeCityZoneAssignment,
};

use super::config::LayoutConfig;

pub fn apply_phase1_layout(
    buildings: &mut [CodeCityBuilding],
    layout: &LayoutConfig,
) -> CodeCityLayoutSummary {
    if buildings.is_empty() {
        return CodeCityLayoutSummary {
            gap: layout.building_gap,
            ..CodeCityLayoutSummary::default()
        };
    }

    let columns = ((buildings.len() as f64 * layout.target_aspect_ratio)
        .sqrt()
        .ceil() as usize)
        .max(1);
    let mut current_x = layout.building_padding;
    let mut current_z = layout.building_padding;
    let mut row_item_count = 0usize;
    let mut row_max_side = 0.0_f64;
    let mut max_width = 0.0_f64;
    let mut max_depth = 0.0_f64;

    for building in buildings {
        if row_item_count == columns {
            current_x = layout.building_padding;
            current_z += row_max_side + layout.building_gap;
            row_item_count = 0;
            row_max_side = 0.0;
        }

        let side = building.geometry.side_length;
        building.geometry.x = current_x;
        building.geometry.y = 0.0;
        building.geometry.z = current_z;
        building.geometry.width = side;
        building.geometry.depth = side;
        building.geometry.footprint_area = side * side;
        building.geometry.height = building.size.total_height;

        max_width = max_width.max(current_x + side + layout.building_padding);
        max_depth = max_depth.max(current_z + side + layout.building_padding);
        row_max_side = row_max_side.max(side);
        current_x += side + layout.building_gap;
        row_item_count += 1;
    }

    CodeCityLayoutSummary {
        layout_kind: "phase1_grid_treemap".to_string(),
        width: max_width,
        depth: max_depth,
        gap: layout.building_gap,
    }
}

pub fn apply_architecture_layout(
    buildings: &mut [CodeCityBuilding],
    boundaries: &mut [CodeCityBoundary],
    reports: &[CodeCityBoundaryArchitectureReport],
    macro_graph: &CodeCityMacroGraph,
    zone_assignments: &BTreeMap<String, CodeCityZoneAssignment>,
    layout: &LayoutConfig,
) -> (CodeCityLayoutSummary, Vec<CodeCityBoundaryLayoutSummary>) {
    let report_by_boundary = reports
        .iter()
        .map(|report| (report.boundary_id.clone(), report))
        .collect::<BTreeMap<_, _>>();

    let mut building_indices_by_boundary = BTreeMap::<String, Vec<usize>>::new();
    for (index, building) in buildings.iter().enumerate() {
        building_indices_by_boundary
            .entry(building.boundary_id.clone())
            .or_default()
            .push(index);
    }

    let mut boundary_layouts = Vec::new();
    let mut local_frames = BTreeMap::<String, (f64, f64)>::new();
    for boundary in boundaries.iter_mut() {
        let strategy = select_strategy(boundary, report_by_boundary.get(&boundary.id).copied());
        let indices = building_indices_by_boundary
            .get(&boundary.id)
            .cloned()
            .unwrap_or_default();
        let (width, depth, zone_count) =
            layout_boundary_local(buildings, &indices, strategy, zone_assignments, layout);
        local_frames.insert(boundary.id.clone(), (width, depth));
        boundary.layout = Some(CodeCityBoundaryLayoutPreview {
            strategy,
            zone_count,
        });
        boundary_layouts.push(CodeCityBoundaryLayoutSummary {
            boundary_id: boundary.id.clone(),
            strategy,
            zone_count,
            width,
            depth,
            x: 0.0,
            z: 0.0,
        });
    }

    let positions = position_boundaries(
        &boundary_layouts,
        macro_graph,
        layout.world_gap.max(layout.building_gap),
    );
    let mut world_width = 0.0_f64;
    let mut world_depth = 0.0_f64;

    for boundary_layout in &mut boundary_layouts {
        let (x, z) = positions
            .get(&boundary_layout.boundary_id)
            .copied()
            .unwrap_or((0.0, 0.0));
        boundary_layout.x = x;
        boundary_layout.z = z;
        world_width = world_width.max(x + boundary_layout.width);
        world_depth = world_depth.max(z + boundary_layout.depth);

        if let Some(indices) = building_indices_by_boundary.get(&boundary_layout.boundary_id) {
            for &index in indices {
                buildings[index].geometry.x += x;
                buildings[index].geometry.z += z;
            }
        }
    }

    (
        CodeCityLayoutSummary {
            layout_kind: macro_graph.topology.as_layout_kind().to_string(),
            width: world_width,
            depth: world_depth,
            gap: layout.world_gap,
        },
        boundary_layouts,
    )
}

fn layout_boundary_local(
    buildings: &mut [CodeCityBuilding],
    indices: &[usize],
    strategy: CodeCityLayoutStrategy,
    zone_assignments: &BTreeMap<String, CodeCityZoneAssignment>,
    layout: &LayoutConfig,
) -> (f64, f64, usize) {
    if indices.is_empty() {
        return (0.0, 0.0, 0);
    }

    let mut zone_to_indices = BTreeMap::<String, Vec<usize>>::new();
    for &index in indices {
        let zone = zone_assignments
            .get(&buildings[index].path)
            .map(|assignment| assignment.zone.as_str().to_string())
            .unwrap_or_else(|| buildings[index].zone.clone());
        zone_to_indices.entry(zone).or_default().push(index);
    }

    let zone_order = CodeCityZone::ordered()
        .iter()
        .map(|zone| zone.as_str().to_string())
        .filter(|zone| zone_to_indices.contains_key(zone))
        .collect::<Vec<_>>();
    let mut zone_frames = Vec::new();
    for zone in zone_order {
        let mut zone_indices = zone_to_indices.remove(&zone).unwrap_or_default();
        zone_indices.sort_by(|left, right| buildings[*left].path.cmp(&buildings[*right].path));
        let (width, depth) = layout_group(buildings, &zone_indices, layout);
        zone_frames.push((zone, zone_indices, width, depth));
    }
    let zone_count = zone_frames.len();

    let mut current_x = 0.0_f64;
    let mut current_z = 0.0_f64;
    let mut max_width = 0.0_f64;
    let mut max_depth = 0.0_f64;

    for (ordinal, (_zone, zone_indices, width, depth)) in zone_frames.into_iter().enumerate() {
        let (offset_x, offset_z) = match strategy {
            CodeCityLayoutStrategy::LayeredBands => (0.0, current_z),
            CodeCityLayoutStrategy::PipeAndFilterStrip => (current_x, 0.0),
            _ => {
                let column = ordinal % 2;
                let row = ordinal / 2;
                (
                    column as f64 * (width + layout.zone_gap),
                    row as f64 * (depth + layout.zone_gap),
                )
            }
        };

        for index in zone_indices {
            buildings[index].geometry.x += offset_x;
            buildings[index].geometry.z += offset_z;
        }

        max_width = max_width.max(offset_x + width);
        max_depth = max_depth.max(offset_z + depth);
        if matches!(strategy, CodeCityLayoutStrategy::LayeredBands) {
            current_z += depth + layout.zone_gap;
        } else if matches!(strategy, CodeCityLayoutStrategy::PipeAndFilterStrip) {
            current_x += width + layout.zone_gap;
        }
    }

    (max_width, max_depth, zone_count)
}

fn layout_group(
    buildings: &mut [CodeCityBuilding],
    indices: &[usize],
    layout: &LayoutConfig,
) -> (f64, f64) {
    if indices.is_empty() {
        return (0.0, 0.0);
    }

    let columns = ((indices.len() as f64 * layout.target_aspect_ratio)
        .sqrt()
        .ceil() as usize)
        .max(1);
    let mut current_x = layout.building_padding;
    let mut current_z = layout.building_padding;
    let mut row_item_count = 0usize;
    let mut row_max_side = 0.0_f64;
    let mut max_width = 0.0_f64;
    let mut max_depth = 0.0_f64;

    for &index in indices {
        if row_item_count == columns {
            current_x = layout.building_padding;
            current_z += row_max_side + layout.building_gap;
            row_item_count = 0;
            row_max_side = 0.0;
        }

        let side = buildings[index].geometry.side_length;
        buildings[index].geometry.x = current_x;
        buildings[index].geometry.y = 0.0;
        buildings[index].geometry.z = current_z;
        buildings[index].geometry.width = side;
        buildings[index].geometry.depth = side;
        buildings[index].geometry.footprint_area = side * side;
        buildings[index].geometry.height = buildings[index].size.total_height;

        max_width = max_width.max(current_x + side + layout.building_padding);
        max_depth = max_depth.max(current_z + side + layout.building_padding);
        row_max_side = row_max_side.max(side);
        current_x += side + layout.building_gap;
        row_item_count += 1;
    }

    (max_width, max_depth)
}

fn select_strategy(
    boundary: &CodeCityBoundary,
    report: Option<&CodeCityBoundaryArchitectureReport>,
) -> CodeCityLayoutStrategy {
    if boundary.shared_library {
        return CodeCityLayoutStrategy::PlainTreemap;
    }
    match report
        .map(|report| report.primary_pattern)
        .unwrap_or(CodeCityArchitecturePattern::Unclassified)
    {
        CodeCityArchitecturePattern::Hexagonal => CodeCityLayoutStrategy::HexagonalRings,
        CodeCityArchitecturePattern::Layered => CodeCityLayoutStrategy::LayeredBands,
        CodeCityArchitecturePattern::Modular => CodeCityLayoutStrategy::ModularIslands,
        CodeCityArchitecturePattern::PipeAndFilter => CodeCityLayoutStrategy::PipeAndFilterStrip,
        CodeCityArchitecturePattern::BallOfMud => CodeCityLayoutStrategy::MudForceDirected,
        _ => CodeCityLayoutStrategy::PlainTreemap,
    }
}

fn position_boundaries(
    layouts: &[CodeCityBoundaryLayoutSummary],
    macro_graph: &CodeCityMacroGraph,
    gap: f64,
) -> BTreeMap<String, (f64, f64)> {
    let mut positions = BTreeMap::new();
    if layouts.is_empty() {
        return positions;
    }

    match macro_graph.topology {
        crate::capability_packs::codecity::types::CodeCityMacroTopology::SingleBoundary => {
            positions.insert(layouts[0].boundary_id.clone(), (0.0, 0.0));
        }
        crate::capability_packs::codecity::types::CodeCityMacroTopology::Star => {
            let central = macro_graph
                .edges
                .iter()
                .fold(BTreeMap::<String, usize>::new(), |mut counts, edge| {
                    *counts.entry(edge.to_boundary_id.clone()).or_insert(0) += edge.weight;
                    counts
                })
                .into_iter()
                .max_by_key(|(_, count)| *count)
                .map(|(id, _)| id)
                .unwrap_or_else(|| layouts[0].boundary_id.clone());
            positions.insert(central.clone(), (0.0, 0.0));
            let orbit = layouts
                .iter()
                .filter(|layout| layout.boundary_id != central)
                .collect::<Vec<_>>();
            let radius = layouts
                .iter()
                .map(|layout| layout.width.max(layout.depth))
                .fold(0.0_f64, f64::max)
                + gap * 2.0;
            for (index, layout) in orbit.into_iter().enumerate() {
                let angle = (index as f64 / layouts.len().max(1) as f64) * std::f64::consts::TAU;
                positions.insert(
                    layout.boundary_id.clone(),
                    (radius * angle.cos() + radius, radius * angle.sin() + radius),
                );
            }
        }
        crate::capability_packs::codecity::types::CodeCityMacroTopology::Layered => {
            let mut indegree = BTreeMap::<String, usize>::new();
            for layout in layouts {
                indegree.insert(layout.boundary_id.clone(), 0);
            }
            for edge in &macro_graph.edges {
                *indegree.entry(edge.to_boundary_id.clone()).or_insert(0) += 1;
            }
            let mut ordered = layouts.iter().collect::<Vec<_>>();
            ordered.sort_by(|left, right| {
                indegree[&left.boundary_id]
                    .cmp(&indegree[&right.boundary_id])
                    .then_with(|| left.boundary_id.cmp(&right.boundary_id))
            });
            let mut current_z = 0.0_f64;
            for layout in ordered {
                positions.insert(layout.boundary_id.clone(), (0.0, current_z));
                current_z += layout.depth + gap;
            }
        }
        _ => {
            let columns = ((layouts.len() as f64).sqrt().ceil() as usize).max(1);
            let mut current_x = 0.0_f64;
            let mut current_z = 0.0_f64;
            let mut row_count = 0usize;
            let mut row_depth = 0.0_f64;
            for layout in layouts {
                if row_count == columns {
                    current_x = 0.0;
                    current_z += row_depth + gap;
                    row_count = 0;
                    row_depth = 0.0;
                }
                positions.insert(layout.boundary_id.clone(), (current_x, current_z));
                current_x += layout.width + gap;
                row_depth = row_depth.max(layout.depth);
                row_count += 1;
            }
        }
    }

    positions
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{apply_architecture_layout, apply_phase1_layout};
    use crate::capability_packs::codecity::services::config::CodeCityConfig;
    use crate::capability_packs::codecity::types::{
        CodeCityArchitecturePattern, CodeCityBoundary, CodeCityBoundaryArchitectureReport,
        CodeCityBoundaryKind, CodeCityBoundarySource, CodeCityBuilding,
        CodeCityBuildingHealthSummary, CodeCityFloor, CodeCityGeometry, CodeCityHealthEvidence,
        CodeCityHealthMetrics, CodeCityImportance, CodeCityMacroGraph, CodeCityMacroTopology,
        CodeCitySize, CodeCityZone, CodeCityZoneAssignment,
    };

    fn building(path: &str, boundary_id: &str, zone: &str, side_length: f64) -> CodeCityBuilding {
        CodeCityBuilding {
            path: path.to_string(),
            language: "typescript".to_string(),
            boundary_id: boundary_id.to_string(),
            zone: zone.to_string(),
            inferred_zone: Some(zone.to_string()),
            convention_zone: Some(zone.to_string()),
            architecture_role: None,
            importance: CodeCityImportance::default(),
            size: CodeCitySize {
                loc: 10,
                artefact_count: 1,
                total_height: 1.0,
            },
            geometry: CodeCityGeometry {
                side_length,
                ..CodeCityGeometry::default()
            },
            health_risk: None,
            health_status: "insufficient_data".to_string(),
            health_confidence: 0.0,
            colour: "#888888".to_string(),
            health_summary: CodeCityBuildingHealthSummary {
                floor_count: 1,
                insufficient_data_floor_count: 1,
                ..CodeCityBuildingHealthSummary::default()
            },
            diagnostic_badges: Vec::new(),
            floors: vec![CodeCityFloor {
                artefact_id: None,
                symbol_id: None,
                name: "fixture".to_string(),
                canonical_kind: Some("function".to_string()),
                language_kind: None,
                start_line: 1,
                end_line: 10,
                loc: 10,
                floor_index: 0,
                floor_height: 1.0,
                health_risk: None,
                colour: "#888888".to_string(),
                health_status: "insufficient_data".to_string(),
                health_confidence: 0.0,
                health_metrics: CodeCityHealthMetrics::default(),
                health_evidence: CodeCityHealthEvidence::default(),
            }],
        }
    }

    fn boundary(id: &str) -> CodeCityBoundary {
        CodeCityBoundary {
            id: id.to_string(),
            name: id.to_string(),
            root_path: id.trim_start_matches("boundary:").to_string(),
            kind: CodeCityBoundaryKind::Explicit,
            ecosystem: Some("node".to_string()),
            parent_boundary_id: None,
            source: CodeCityBoundarySource::Manifest,
            file_count: 2,
            artefact_count: 2,
            dependency_count: 1,
            entry_points: Vec::new(),
            shared_library: false,
            atomic: true,
            architecture: None,
            layout: None,
            violation_summary: Default::default(),
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn layout_is_deterministic_for_the_same_input() {
        let config = CodeCityConfig::default();
        let mut first = vec![
            building("a", "boundary:root", "core", 4.0),
            building("b", "boundary:root", "core", 3.0),
            building("c", "boundary:root", "core", 2.5),
        ];
        let mut second = first.clone();

        let first_layout = apply_phase1_layout(&mut first, &config.layout);
        let second_layout = apply_phase1_layout(&mut second, &config.layout);

        assert_eq!(first_layout, second_layout);
        assert_eq!(first, second);
    }

    #[test]
    fn layered_boundary_places_zones_in_different_bands() {
        let config = CodeCityConfig::default();
        let mut buildings = vec![
            building(
                "apps/api/src/domain/user.ts",
                "boundary:apps/api",
                "core",
                2.0,
            ),
            building(
                "apps/api/src/controllers/user.ts",
                "boundary:apps/api",
                "edge",
                2.0,
            ),
        ];
        let mut boundaries = vec![boundary("boundary:apps/api")];
        let reports = vec![CodeCityBoundaryArchitectureReport {
            boundary_id: "boundary:apps/api".to_string(),
            primary_pattern: CodeCityArchitecturePattern::Layered,
            primary_score: 0.8,
            secondary_pattern: None,
            secondary_score: None,
            scores: Default::default(),
            metrics: Default::default(),
            evidence: Vec::new(),
            diagnostics: Vec::new(),
        }];
        let macro_graph = CodeCityMacroGraph {
            topology: CodeCityMacroTopology::SingleBoundary,
            boundary_count: 1,
            edge_count: 0,
            density: 0.0,
            modularity: Some(0.0),
            edges: Vec::new(),
        };
        let assignments = BTreeMap::from([
            (
                "apps/api/src/domain/user.ts".to_string(),
                CodeCityZoneAssignment {
                    path: "apps/api/src/domain/user.ts".to_string(),
                    boundary_id: "boundary:apps/api".to_string(),
                    zone: CodeCityZone::Core,
                    convention_zone: Some(CodeCityZone::Core),
                    inferred_zone: Some(CodeCityZone::Core),
                    depth_score: Some(0.8),
                    confidence: 1.0,
                    disagreement: false,
                    reason: "fixture".to_string(),
                },
            ),
            (
                "apps/api/src/controllers/user.ts".to_string(),
                CodeCityZoneAssignment {
                    path: "apps/api/src/controllers/user.ts".to_string(),
                    boundary_id: "boundary:apps/api".to_string(),
                    zone: CodeCityZone::Edge,
                    convention_zone: Some(CodeCityZone::Edge),
                    inferred_zone: Some(CodeCityZone::Edge),
                    depth_score: Some(0.1),
                    confidence: 1.0,
                    disagreement: false,
                    reason: "fixture".to_string(),
                },
            ),
        ]);

        let (_, boundary_layouts) = apply_architecture_layout(
            &mut buildings,
            &mut boundaries,
            &reports,
            &macro_graph,
            &assignments,
            &config.layout,
        );

        assert_eq!(
            boundary_layouts[0].strategy,
            super::CodeCityLayoutStrategy::LayeredBands
        );
        assert!(buildings[0].geometry.z < buildings[1].geometry.z);
    }
}

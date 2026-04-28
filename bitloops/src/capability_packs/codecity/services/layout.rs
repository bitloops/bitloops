use crate::capability_packs::codecity::types::{CodeCityBuilding, CodeCityLayoutSummary};

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

#[cfg(test)]
mod tests {
    use super::apply_phase1_layout;
    use crate::capability_packs::codecity::services::config::CodeCityConfig;
    use crate::capability_packs::codecity::types::{
        CodeCityBuilding, CodeCityFloor, CodeCityGeometry, CodeCityImportance, CodeCitySize,
    };

    fn building(path: &str, score: f64, side_length: f64, total_height: f64) -> CodeCityBuilding {
        CodeCityBuilding {
            path: path.to_string(),
            language: "typescript".to_string(),
            boundary_id: "root".to_string(),
            zone: "unclassified".to_string(),
            importance: CodeCityImportance {
                score,
                ..CodeCityImportance::default()
            },
            size: CodeCitySize {
                loc: 10,
                artefact_count: 1,
                total_height,
            },
            geometry: CodeCityGeometry {
                side_length,
                ..CodeCityGeometry::default()
            },
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
                floor_height: total_height,
                health_risk: None,
                colour: "#888888".to_string(),
                health_status: "insufficient_data".to_string(),
            }],
        }
    }

    fn overlaps(left: &CodeCityBuilding, right: &CodeCityBuilding) -> bool {
        let left_x2 = left.geometry.x + left.geometry.width;
        let left_z2 = left.geometry.z + left.geometry.depth;
        let right_x2 = right.geometry.x + right.geometry.width;
        let right_z2 = right.geometry.z + right.geometry.depth;

        left.geometry.x < right_x2
            && left_x2 > right.geometry.x
            && left.geometry.z < right_z2
            && left_z2 > right.geometry.z
    }

    #[test]
    fn layout_is_deterministic_for_the_same_input() {
        let config = CodeCityConfig::default();
        let mut first = vec![
            building("a", 1.0, 4.0, 2.0),
            building("b", 0.8, 3.0, 2.0),
            building("c", 0.7, 2.5, 1.5),
        ];
        let mut second = first.clone();

        let first_layout = apply_phase1_layout(&mut first, &config.layout);
        let second_layout = apply_phase1_layout(&mut second, &config.layout);

        assert_eq!(first_layout, second_layout);
        assert_eq!(first, second);
    }

    #[test]
    fn layout_avoids_overlaps() {
        let config = CodeCityConfig::default();
        let mut buildings = vec![
            building("a", 1.0, 4.0, 2.0),
            building("b", 0.8, 3.0, 2.0),
            building("c", 0.7, 2.5, 1.5),
            building("d", 0.5, 2.0, 1.0),
        ];

        apply_phase1_layout(&mut buildings, &config.layout);

        for left_index in 0..buildings.len() {
            for right_index in (left_index + 1)..buildings.len() {
                assert!(
                    !overlaps(&buildings[left_index], &buildings[right_index]),
                    "buildings {} and {} overlap",
                    buildings[left_index].path,
                    buildings[right_index].path
                );
            }
        }
    }

    #[test]
    fn layout_summary_is_positive_when_buildings_exist() {
        let config = CodeCityConfig::default();
        let mut buildings = vec![building("a", 1.0, 4.0, 2.0)];

        let summary = apply_phase1_layout(&mut buildings, &config.layout);

        assert!(summary.width > 0.0);
        assert!(summary.depth > 0.0);
    }

    #[test]
    fn empty_layout_summary_is_zeroed() {
        let config = CodeCityConfig::default();
        let mut buildings = Vec::new();

        let summary = apply_phase1_layout(&mut buildings, &config.layout);

        assert_eq!(summary.width, 0.0);
        assert_eq!(summary.depth, 0.0);
    }
}

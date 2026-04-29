use std::collections::BTreeMap;

use super::config::CodeCityConfig;
use super::graph_metrics::{build_graph_from_paths, compute_depth_scores};
use super::source_graph::CodeCitySourceGraph;
use crate::capability_packs::codecity::types::{
    CodeCityBoundary, CodeCityBoundaryArchitectureReport, CodeCityZone, CodeCityZoneAssignment,
};

pub fn assign_zones(
    source: &CodeCitySourceGraph,
    boundaries: &[CodeCityBoundary],
    _reports: &[CodeCityBoundaryArchitectureReport],
    file_to_boundary: &BTreeMap<String, String>,
    config: &CodeCityConfig,
) -> BTreeMap<String, CodeCityZoneAssignment> {
    let boundary_by_id = boundaries
        .iter()
        .map(|boundary| (boundary.id.clone(), boundary))
        .collect::<BTreeMap<_, _>>();

    let depth_by_file = boundary_depth_scores(source, file_to_boundary);
    let mut assignments = BTreeMap::new();

    for file in source.files.iter().filter(|file| file.included) {
        let Some(boundary_id) = file_to_boundary.get(&file.path) else {
            continue;
        };
        let Some(boundary) = boundary_by_id.get(boundary_id) else {
            continue;
        };

        let manual_zone = config
            .zones
            .zone_overrides
            .iter()
            .find_map(|override_rule| {
                path_matches_override(&file.path, &override_rule.pattern)
                    .then_some(override_rule.zone)
            });
        let convention_zone = match_convention_zone(&file.path, config);
        let inferred_depth = depth_by_file.get(&file.path).copied();
        let inferred_zone =
            inferred_depth.and_then(|depth| infer_zone_from_depth(depth, source, &file.path));

        let disagreement = convention_zone.is_some()
            && inferred_zone.is_some()
            && convention_zone != inferred_zone;

        let (zone, reason, confidence) = if let Some(zone) = manual_zone {
            (zone, "manual_override".to_string(), 1.0)
        } else if let Some(zone) = convention_zone {
            let confidence = if disagreement { 0.65 } else { 0.9 };
            (zone, "path_convention".to_string(), confidence)
        } else if let Some(zone) = inferred_zone {
            (zone, "dependency_depth".to_string(), 0.75)
        } else if boundary.shared_library {
            (CodeCityZone::Shared, "shared_library".to_string(), 0.7)
        } else {
            (CodeCityZone::Unclassified, "fallback".to_string(), 0.5)
        };

        assignments.insert(
            file.path.clone(),
            CodeCityZoneAssignment {
                path: file.path.clone(),
                boundary_id: boundary_id.clone(),
                zone,
                convention_zone,
                inferred_zone,
                depth_score: inferred_depth,
                confidence,
                disagreement,
                reason,
            },
        );
    }

    assignments
}

fn boundary_depth_scores(
    source: &CodeCitySourceGraph,
    file_to_boundary: &BTreeMap<String, String>,
) -> BTreeMap<String, f64> {
    let mut boundary_files = BTreeMap::<String, Vec<String>>::new();
    for (path, boundary_id) in file_to_boundary {
        boundary_files
            .entry(boundary_id.clone())
            .or_default()
            .push(path.clone());
    }

    let mut depth_by_file = BTreeMap::new();
    for files in boundary_files.values() {
        let graph = build_graph_from_paths(files, &source.edges);
        depth_by_file.extend(compute_depth_scores(&graph));
    }
    depth_by_file
}

fn path_matches_override(path: &str, pattern: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix("/**") {
        path == prefix || path.starts_with(&format!("{prefix}/"))
    } else if let Some(suffix) = pattern.strip_prefix("**/") {
        path == suffix || path.ends_with(&format!("/{suffix}"))
    } else {
        path == pattern
    }
}

fn match_convention_zone(path: &str, config: &CodeCityConfig) -> Option<CodeCityZone> {
    let segments = path.split('/').collect::<Vec<_>>();
    let mut best = None::<(usize, CodeCityZone)>;

    for (depth, segment) in segments.iter().enumerate() {
        let zone = if config
            .zones
            .conventions
            .core
            .iter()
            .any(|value| value == segment)
        {
            Some(CodeCityZone::Core)
        } else if config
            .zones
            .conventions
            .application
            .iter()
            .any(|value| value == segment)
        {
            Some(CodeCityZone::Application)
        } else if config
            .zones
            .conventions
            .periphery
            .iter()
            .any(|value| value == segment)
        {
            Some(CodeCityZone::Periphery)
        } else if config
            .zones
            .conventions
            .edge
            .iter()
            .any(|value| value == segment)
        {
            Some(CodeCityZone::Edge)
        } else if config
            .zones
            .conventions
            .ports
            .iter()
            .any(|value| value == segment)
        {
            Some(CodeCityZone::Ports)
        } else {
            None
        };

        if let Some(zone) = zone {
            best = Some((depth, zone));
        }
    }

    best.map(|(_, zone)| zone)
}

fn infer_zone_from_depth(
    depth: f64,
    source: &CodeCitySourceGraph,
    path: &str,
) -> Option<CodeCityZone> {
    let has_edge = source
        .edges
        .iter()
        .any(|edge| edge.from_path == path || edge.to_path == path);
    if !has_edge {
        return None;
    }

    Some(if depth > 0.7 {
        CodeCityZone::Core
    } else if depth >= 0.5 {
        CodeCityZone::Application
    } else if depth >= 0.3 {
        CodeCityZone::Periphery
    } else {
        CodeCityZone::Edge
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::assign_zones;
    use crate::capability_packs::codecity::services::config::{
        CodeCityConfig, CodeCityZoneOverride,
    };
    use crate::capability_packs::codecity::services::source_graph::{
        CodeCitySourceFile, CodeCitySourceGraph,
    };
    use crate::capability_packs::codecity::types::{
        CodeCityBoundary, CodeCityBoundaryKind, CodeCityBoundarySource, CodeCityZone,
    };

    fn source(paths: &[&str]) -> CodeCitySourceGraph {
        CodeCitySourceGraph {
            project_path: None,
            files: paths
                .iter()
                .map(|path| CodeCitySourceFile {
                    path: (*path).to_string(),
                    language: "typescript".to_string(),
                    effective_content_id: format!("content::{path}"),
                    included: true,
                    exclusion_reason: None,
                })
                .collect(),
            artefacts: Vec::new(),
            edges: Vec::new(),
            external_dependency_hints: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    fn boundary(id: &str, shared_library: bool) -> CodeCityBoundary {
        CodeCityBoundary {
            id: id.to_string(),
            name: id.to_string(),
            root_path: id.trim_start_matches("boundary:").to_string(),
            kind: CodeCityBoundaryKind::Explicit,
            ecosystem: None,
            parent_boundary_id: None,
            source: CodeCityBoundarySource::Manifest,
            file_count: 1,
            artefact_count: 0,
            dependency_count: 0,
            entry_points: Vec::new(),
            shared_library,
            atomic: true,
            architecture: None,
            layout: None,
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn manual_zone_override_wins() {
        let mut config = CodeCityConfig::default();
        config.zones.zone_overrides.push(CodeCityZoneOverride {
            pattern: "apps/api/src/domain/**".to_string(),
            zone: CodeCityZone::Edge,
        });

        let source = source(&["apps/api/src/domain/user.ts"]);
        let boundaries = vec![boundary("boundary:apps/api", false)];
        let file_to_boundary = BTreeMap::from([(
            "apps/api/src/domain/user.ts".to_string(),
            "boundary:apps/api".to_string(),
        )]);

        let assignments = assign_zones(&source, &boundaries, &[], &file_to_boundary, &config);

        assert_eq!(
            assignments["apps/api/src/domain/user.ts"].zone,
            CodeCityZone::Edge
        );
    }

    #[test]
    fn shared_library_files_default_to_shared_zone() {
        let source = source(&["libs/shared/src/lib.ts"]);
        let boundaries = vec![boundary("boundary:libs/shared", true)];
        let file_to_boundary = BTreeMap::from([(
            "libs/shared/src/lib.ts".to_string(),
            "boundary:libs/shared".to_string(),
        )]);

        let assignments = assign_zones(
            &source,
            &boundaries,
            &[],
            &file_to_boundary,
            &CodeCityConfig::default(),
        );

        assert_eq!(
            assignments["libs/shared/src/lib.ts"].zone,
            CodeCityZone::Shared
        );
    }
}

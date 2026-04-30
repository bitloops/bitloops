use std::ffi::OsStr;
use std::path::Path;

use crate::capability_packs::codecity::services::community_detection::{
    CodeCityCommunity, detect_communities,
};
use crate::capability_packs::codecity::services::config::CodeCityConfig;
use crate::capability_packs::codecity::services::graph_metrics::build_graph_from_paths;
use crate::capability_packs::codecity::services::source_graph::CodeCitySourceGraph;
use crate::capability_packs::codecity::types::{
    CodeCityBoundaryKind, CodeCityBoundarySource, CodeCityDiagnostic,
};

use super::builder::build_boundary;
use super::model::{BoundaryBuildSpec, BoundarySplitResult, ResolvedBoundary};
use super::naming::slugify;

pub(super) const MAX_INTERACTIVE_IMPLICIT_BOUNDARY_FILES: usize = 2048;
const INDEPENDENT_BOUNDARY_NAME: &str = "independent";

pub(super) fn split_implicit_boundaries(
    source: &CodeCitySourceGraph,
    boundary: &ResolvedBoundary,
    config: &CodeCityConfig,
) -> BoundarySplitResult {
    if boundary.files.len() < config.boundaries.min_implicit_boundary_files {
        return BoundarySplitResult {
            boundaries: Vec::new(),
            diagnostics: Vec::new(),
        };
    }
    if boundary.files.len() > MAX_INTERACTIVE_IMPLICIT_BOUNDARY_FILES {
        return BoundarySplitResult {
            boundaries: Vec::new(),
            diagnostics: vec![CodeCityDiagnostic {
                code: "codecity.boundary.implicit_split_too_large".to_string(),
                severity: "info".to_string(),
                message: format!(
                    "Boundary `{}` has {} files, so implicit community splitting was skipped for interactive rendering.",
                    boundary.boundary.id,
                    boundary.files.len()
                ),
                path: None,
                boundary_id: Some(boundary.boundary.id.clone()),
            }],
        };
    }

    let graph = build_graph_from_paths(&boundary.files, &source.edges);
    let communities = detect_communities(&graph, config.boundaries.community_max_iterations);
    if communities.communities.len() < 2 {
        return BoundarySplitResult {
            boundaries: Vec::new(),
            diagnostics: Vec::new(),
        };
    }
    if communities.modularity < config.boundaries.community_modularity_threshold {
        let diagnostics = if communities.modularity >= 0.2 {
            vec![CodeCityDiagnostic {
                code: "codecity.boundary.community_weak_structure".to_string(),
                severity: "info".to_string(),
                message: format!(
                    "Boundary `{}` had weak implicit community structure (modularity {:.2}).",
                    boundary.boundary.id, communities.modularity
                ),
                path: None,
                boundary_id: Some(boundary.boundary.id.clone()),
            }]
        } else {
            Vec::new()
        };
        return BoundarySplitResult {
            boundaries: Vec::new(),
            diagnostics,
        };
    }

    let mut retained_communities = Vec::new();
    let mut independent_files = Vec::new();
    let collapse_limit = config.boundaries.small_cluster_collapse_file_limit;
    for (index, community) in communities.communities.iter().enumerate() {
        if collapse_limit > 0 && community.paths.len() <= collapse_limit {
            independent_files.extend(community.paths.clone());
        } else {
            retained_communities.push((index, community));
        }
    }

    independent_files.sort();
    independent_files.dedup();

    let mut boundaries = retained_communities
        .into_iter()
        .map(|(index, community)| build_implicit_boundary(source, boundary, community, index))
        .collect::<Vec<_>>();

    if !independent_files.is_empty() {
        let mut independent = build_boundary(
            source,
            BoundaryBuildSpec {
                root_path: boundary.boundary.root_path.clone(),
                id: format!("{}:implicit:independent", boundary.boundary.id),
                name: INDEPENDENT_BOUNDARY_NAME.to_string(),
                kind: CodeCityBoundaryKind::Implicit,
                ecosystem: boundary.boundary.ecosystem.clone(),
                parent_boundary_id: Some(boundary.boundary.id.clone()),
                source_kind: CodeCityBoundarySource::CommunityDetection,
                files: independent_files,
                entry_points: Vec::new(),
                diagnostics: Vec::new(),
            },
        );
        independent.boundary.atomic = false;
        boundaries.push(independent);
    }

    BoundarySplitResult {
        boundaries,
        diagnostics: Vec::new(),
    }
}

fn build_implicit_boundary(
    source: &CodeCitySourceGraph,
    parent: &ResolvedBoundary,
    community: &CodeCityCommunity,
    index: usize,
) -> ResolvedBoundary {
    let name = common_directory_prefix(&community.paths)
        .map(|prefix| {
            Path::new(&prefix)
                .file_name()
                .and_then(OsStr::to_str)
                .unwrap_or("community")
                .to_string()
        })
        .unwrap_or_else(|| format!("community_{}", index + 1));

    build_boundary(
        source,
        BoundaryBuildSpec {
            root_path: parent.boundary.root_path.clone(),
            id: format!(
                "{}:implicit:{}:{}",
                parent.boundary.id,
                slugify(&name),
                index + 1
            ),
            name,
            kind: CodeCityBoundaryKind::Implicit,
            ecosystem: parent.boundary.ecosystem.clone(),
            parent_boundary_id: Some(parent.boundary.id.clone()),
            source_kind: CodeCityBoundarySource::CommunityDetection,
            files: community.paths.clone(),
            entry_points: Vec::new(),
            diagnostics: Vec::new(),
        },
    )
}

fn common_directory_prefix(paths: &[String]) -> Option<String> {
    let segments = paths
        .iter()
        .map(|path| {
            Path::new(path)
                .parent()
                .map(|parent| {
                    parent
                        .components()
                        .map(|component| component.as_os_str().to_string_lossy().to_string())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        })
        .collect::<Vec<_>>();
    let first = segments.first()?.clone();
    let mut length = first.len();
    for other in segments.iter().skip(1) {
        length = length.min(other.len());
        for index in 0..length {
            if first[index] != other[index] {
                length = index;
                break;
            }
        }
    }
    (length > 0).then_some(first[..length].join("/"))
}

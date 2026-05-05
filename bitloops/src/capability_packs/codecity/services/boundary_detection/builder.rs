use std::collections::BTreeSet;

use crate::capability_packs::codecity::services::source_graph::CodeCitySourceGraph;
use crate::capability_packs::codecity::types::CodeCityBoundary;

use super::model::{BoundaryBuildSpec, ResolvedBoundary};

pub(super) fn build_boundary(
    source: &CodeCitySourceGraph,
    spec: BoundaryBuildSpec,
) -> ResolvedBoundary {
    let BoundaryBuildSpec {
        root_path,
        id,
        name,
        kind,
        ecosystem,
        parent_boundary_id,
        source_kind,
        files,
        entry_points,
        diagnostics,
    } = spec;

    let file_set = files.iter().cloned().collect::<BTreeSet<_>>();
    let artefact_count = source
        .artefacts
        .iter()
        .filter(|artefact| file_set.contains(&artefact.path))
        .count();
    let dependency_count = source
        .edges
        .iter()
        .filter(|edge| file_set.contains(&edge.from_path) && file_set.contains(&edge.to_path))
        .count();

    ResolvedBoundary {
        boundary: CodeCityBoundary {
            id,
            name,
            root_path,
            kind,
            ecosystem,
            parent_boundary_id,
            source: source_kind,
            file_count: file_set.len(),
            artefact_count,
            dependency_count,
            entry_points,
            shared_library: false,
            atomic: true,
            architecture: None,
            layout: None,
            violation_summary: Default::default(),
            diagnostics,
        },
        files,
    }
}

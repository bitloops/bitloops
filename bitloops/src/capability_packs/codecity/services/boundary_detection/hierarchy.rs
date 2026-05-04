use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::Path;

use crate::capability_packs::codecity::services::source_graph::CodeCitySourceGraph;
use crate::capability_packs::codecity::types::{CodeCityBoundaryKind, CodeCityBoundarySource};

use super::builder::build_boundary;
use super::manifest::boundary_id_for_root;
use super::model::{BoundaryBuildSpec, ResolvedBoundary};

pub(super) fn materialise_directory_parent_boundaries(
    source: &CodeCitySourceGraph,
    mut resolved: Vec<ResolvedBoundary>,
) -> Vec<ResolvedBoundary> {
    let existing_ids = resolved
        .iter()
        .map(|boundary| boundary.boundary.id.clone())
        .collect::<BTreeSet<_>>();
    let existing_roots = resolved
        .iter()
        .map(|boundary| boundary.boundary.root_path.clone())
        .collect::<BTreeSet<_>>();
    let parent_ids = parent_boundary_ids(&resolved);
    let mut groups = BTreeMap::<(Option<String>, String), Vec<usize>>::new();

    for (index, boundary) in resolved.iter().enumerate() {
        if parent_ids.contains(&boundary.boundary.id) {
            continue;
        }
        let Some(group_root) = parent_group_root(&boundary.boundary.root_path) else {
            continue;
        };
        if existing_roots.contains(&group_root) {
            continue;
        }
        groups
            .entry((boundary.boundary.parent_boundary_id.clone(), group_root))
            .or_default()
            .push(index);
    }

    for ((parent_boundary_id, group_root), child_indices) in groups {
        if child_indices.len() < 2 {
            continue;
        }

        let group_id = boundary_id_for_root(&group_root);
        if existing_ids.contains(&group_id) {
            continue;
        }

        let mut files = child_indices
            .iter()
            .flat_map(|index| resolved[*index].files.clone())
            .collect::<Vec<_>>();
        files.sort();
        files.dedup();

        for index in &child_indices {
            resolved[*index].boundary.parent_boundary_id = Some(group_id.clone());
        }

        let mut group = build_boundary(
            source,
            BoundaryBuildSpec {
                root_path: group_root.clone(),
                id: group_id,
                name: boundary_name_for_root(&group_root),
                kind: CodeCityBoundaryKind::Group,
                ecosystem: None,
                parent_boundary_id,
                source_kind: CodeCityBoundarySource::Hierarchy,
                files,
                entry_points: Vec::new(),
                diagnostics: Vec::new(),
            },
        );
        group.boundary.atomic = false;
        resolved.push(group);
    }

    resolved
}

pub(super) fn parent_boundary_ids(boundaries: &[ResolvedBoundary]) -> BTreeSet<String> {
    boundaries
        .iter()
        .filter_map(|boundary| boundary.boundary.parent_boundary_id.clone())
        .collect()
}

fn parent_group_root(root_path: &str) -> Option<String> {
    if root_path == "." {
        return None;
    }

    let parent = Path::new(root_path).parent()?;
    if parent.as_os_str().is_empty() {
        return None;
    }

    let root = parent.to_string_lossy().to_string();
    (!root.is_empty() && root != ".").then_some(root)
}

fn boundary_name_for_root(root_path: &str) -> String {
    Path::new(root_path)
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or(root_path)
        .to_string()
}

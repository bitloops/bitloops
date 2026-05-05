use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::Path;

use crate::capability_packs::codecity::services::config::CodeCityConfig;
use crate::capability_packs::codecity::services::source_graph::CodeCitySourceGraph;
use crate::capability_packs::codecity::types::{
    CodeCityBoundaryKind, CodeCityBoundarySource, CodeCityDiagnostic, CodeCityEntryPoint,
};

use super::builder::build_boundary;
use super::entry_points::{
    closure_overlap, dependency_closure, detect_entry_points, infer_entry_kind,
};
use super::model::{BoundaryBuildSpec, BoundarySplitResult, ResolvedBoundary};
use super::naming::slugify;

pub(super) fn split_runtime_boundaries(
    source: &CodeCitySourceGraph,
    boundary: &ResolvedBoundary,
    config: &CodeCityConfig,
) -> BoundarySplitResult {
    let entry_candidates = detect_entry_points(&boundary.files, &source.artefacts);
    if entry_candidates.len() < 2 {
        return BoundarySplitResult {
            boundaries: Vec::new(),
            diagnostics: Vec::new(),
        };
    }

    let closures = entry_candidates
        .iter()
        .filter_map(|entry| {
            let closure = dependency_closure(entry, &boundary.files, source);
            (closure.len() >= config.boundaries.min_runtime_boundary_files)
                .then_some((entry.clone(), closure))
        })
        .collect::<Vec<_>>();
    if closures.len() < 2 {
        return BoundarySplitResult {
            boundaries: Vec::new(),
            diagnostics: Vec::new(),
        };
    }

    for left in 0..closures.len() {
        for right in (left + 1)..closures.len() {
            let overlap = closure_overlap(&closures[left].1, &closures[right].1);
            if overlap >= config.boundaries.overlap_split_threshold
                && overlap <= config.boundaries.overlap_merge_threshold
            {
                return BoundarySplitResult {
                    boundaries: Vec::new(),
                    diagnostics: vec![CodeCityDiagnostic {
                        code: "codecity.boundary.runtime_overlap_ambiguous".to_string(),
                        severity: "warning".to_string(),
                        message: format!(
                            "Runtime entry points `{}` and `{}` had ambiguous overlap {:.2}.",
                            closures[left].0, closures[right].0, overlap
                        ),
                        path: Some(closures[left].0.clone()),
                        boundary_id: Some(boundary.boundary.id.clone()),
                    }],
                };
            }
        }
    }

    let mut groups = Vec::<(Vec<String>, BTreeSet<String>)>::new();
    for (entry, closure) in closures {
        let mut merged = false;
        for (entries, current) in &mut groups {
            if closure_overlap(current, &closure) > config.boundaries.overlap_merge_threshold {
                entries.push(entry.clone());
                current.extend(closure.clone());
                merged = true;
                break;
            }
        }
        if !merged {
            groups.push((vec![entry], closure));
        }
    }

    if groups.len() < 2 {
        return BoundarySplitResult {
            boundaries: Vec::new(),
            diagnostics: Vec::new(),
        };
    }

    let mut ownership = BTreeMap::<String, usize>::new();
    for (index, (_, closure)) in groups.iter().enumerate() {
        for path in closure {
            ownership.entry(path.clone()).or_insert(index);
        }
    }
    for path in &boundary.files {
        ownership.entry(path.clone()).or_insert(0usize);
    }

    let boundaries = groups
        .into_iter()
        .enumerate()
        .filter_map(|(index, (entries, _closure))| {
            let boundary_files = ownership
                .iter()
                .filter_map(|(path, owner)| (*owner == index).then_some(path.clone()))
                .collect::<Vec<_>>();
            if boundary_files.is_empty() {
                return None;
            }
            let entry_points = entries
                .iter()
                .map(|entry| CodeCityEntryPoint {
                    path: entry.clone(),
                    entry_kind: infer_entry_kind(entry),
                    closure_file_count: boundary_files.len(),
                })
                .collect::<Vec<_>>();

            Some(build_boundary(
                source,
                BoundaryBuildSpec {
                    root_path: boundary.boundary.root_path.clone(),
                    id: format!("{}:runtime:{}", boundary.boundary.id, slugify(&entries[0])),
                    name: Path::new(&entries[0])
                        .file_stem()
                        .and_then(OsStr::to_str)
                        .unwrap_or("runtime")
                        .to_string(),
                    kind: CodeCityBoundaryKind::Runtime,
                    ecosystem: boundary.boundary.ecosystem.clone(),
                    parent_boundary_id: Some(boundary.boundary.id.clone()),
                    source_kind: CodeCityBoundarySource::EntryPoint,
                    files: boundary_files,
                    entry_points,
                    diagnostics: Vec::new(),
                },
            ))
        })
        .collect::<Vec<_>>();

    BoundarySplitResult {
        boundaries,
        diagnostics: Vec::new(),
    }
}

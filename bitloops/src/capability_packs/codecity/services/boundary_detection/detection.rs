use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::capability_packs::codecity::services::config::CodeCityConfig;
use crate::capability_packs::codecity::services::source_graph::CodeCitySourceGraph;
use crate::capability_packs::codecity::types::{
    CODECITY_ROOT_BOUNDARY_ID, CodeCityBoundaryKind, CodeCityBoundarySource, CodeCityDiagnostic,
};

use super::builder::build_boundary;
use super::implicit::split_implicit_boundaries;
use super::manifest::{
    ManifestDescriptor, assign_files_to_explicit_roots, discover_manifest_roots,
};
use super::model::{BoundaryBuildSpec, CodeCityBoundaryDetectionResult, ResolvedBoundary};
use super::runtime::split_runtime_boundaries;

pub fn detect_boundaries(
    source: &CodeCitySourceGraph,
    config: &CodeCityConfig,
    repo_root: &Path,
) -> CodeCityBoundaryDetectionResult {
    let included_files = source
        .files
        .iter()
        .filter(|file| file.included)
        .map(|file| file.path.clone())
        .collect::<Vec<_>>();

    if included_files.is_empty() {
        return CodeCityBoundaryDetectionResult {
            boundaries: Vec::new(),
            file_to_boundary: BTreeMap::new(),
            diagnostics: Vec::new(),
        };
    }

    let analysis_root = source.project_path.as_deref().unwrap_or(".");
    let mut diagnostics = Vec::new();
    let manifest_roots =
        discover_manifest_roots(repo_root, analysis_root, &included_files, &mut diagnostics);

    let explicit_assignment = assign_files_to_explicit_roots(&included_files, &manifest_roots);
    let mut resolved = Vec::<ResolvedBoundary>::new();

    if manifest_roots.is_empty() {
        diagnostics.push(CodeCityDiagnostic {
            code: "codecity.boundary.fallback_root".to_string(),
            severity: "info".to_string(),
            message: "No manifest boundary was found; using the root fallback boundary."
                .to_string(),
            path: None,
            boundary_id: Some(CODECITY_ROOT_BOUNDARY_ID.to_string()),
        });
    }

    let roots = explicit_assignment
        .values()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    for root_path in roots {
        let files = explicit_assignment
            .iter()
            .filter_map(|(path, assigned_root)| {
                (assigned_root == &root_path).then_some(path.clone())
            })
            .collect::<Vec<_>>();
        if files.is_empty() {
            continue;
        }

        let descriptor = manifest_roots
            .get(&root_path)
            .cloned()
            .unwrap_or_else(ManifestDescriptor::fallback);
        let base_boundary = build_boundary(
            source,
            BoundaryBuildSpec {
                root_path: root_path.clone(),
                id: descriptor.boundary_id(&root_path),
                name: descriptor.boundary_name(&root_path),
                kind: descriptor.kind,
                ecosystem: descriptor.ecosystem.clone(),
                parent_boundary_id: None,
                source_kind: descriptor.source,
                files,
                entry_points: Vec::new(),
                diagnostics: Vec::new(),
            },
        );

        let runtime_split = split_runtime_boundaries(source, &base_boundary, config);
        diagnostics.extend(runtime_split.diagnostics);
        let runtime_boundaries = if runtime_split.boundaries.is_empty() {
            vec![base_boundary]
        } else {
            runtime_split.boundaries
        };

        for candidate in runtime_boundaries {
            let implicit_split = split_implicit_boundaries(source, &candidate, config);
            diagnostics.extend(implicit_split.diagnostics);
            if implicit_split.boundaries.is_empty() {
                resolved.push(candidate);
            } else {
                resolved.extend(implicit_split.boundaries);
            }
        }
    }

    if resolved.is_empty() {
        resolved.push(build_boundary(
            source,
            BoundaryBuildSpec {
                root_path: ".".to_string(),
                id: CODECITY_ROOT_BOUNDARY_ID.to_string(),
                name: "root".to_string(),
                kind: CodeCityBoundaryKind::RootFallback,
                ecosystem: None,
                parent_boundary_id: None,
                source_kind: CodeCityBoundarySource::Fallback,
                files: included_files,
                entry_points: Vec::new(),
                diagnostics: Vec::new(),
            },
        ));
    }

    let mut file_to_boundary = BTreeMap::new();
    let mut boundaries = Vec::new();
    for boundary in resolved {
        for path in &boundary.files {
            file_to_boundary.insert(path.clone(), boundary.boundary.id.clone());
        }
        boundaries.push(boundary.boundary);
    }

    boundaries.sort_by(|left, right| {
        left.root_path
            .cmp(&right.root_path)
            .then_with(|| left.id.cmp(&right.id))
    });
    diagnostics.sort_by(|left, right| {
        left.severity
            .cmp(&right.severity)
            .then_with(|| left.code.cmp(&right.code))
            .then_with(|| left.path.cmp(&right.path))
    });

    CodeCityBoundaryDetectionResult {
        boundaries,
        file_to_boundary,
        diagnostics,
    }
}

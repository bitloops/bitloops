use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::community_detection::detect_communities;
use super::config::CodeCityConfig;
use super::graph_metrics::build_graph_from_paths;
use super::source_graph::{CodeCitySourceArtefact, CodeCitySourceGraph};
use crate::capability_packs::codecity::types::{
    CODECITY_ROOT_BOUNDARY_ID, CodeCityBoundary, CodeCityBoundaryKind, CodeCityBoundarySource,
    CodeCityDiagnostic, CodeCityEntryPoint,
};

const MAX_INTERACTIVE_IMPLICIT_BOUNDARY_FILES: usize = 2048;

#[derive(Debug, Clone, PartialEq)]
pub struct CodeCityBoundaryDetectionResult {
    pub boundaries: Vec<CodeCityBoundary>,
    pub file_to_boundary: BTreeMap<String, String>,
    pub diagnostics: Vec<CodeCityDiagnostic>,
}

#[derive(Debug, Clone)]
struct ResolvedBoundary {
    boundary: CodeCityBoundary,
    files: Vec<String>,
}

#[derive(Debug, Clone)]
struct BoundarySplitResult {
    boundaries: Vec<ResolvedBoundary>,
    diagnostics: Vec<CodeCityDiagnostic>,
}

#[derive(Debug, Clone)]
struct BoundaryBuildSpec {
    root_path: String,
    id: String,
    name: String,
    kind: CodeCityBoundaryKind,
    ecosystem: Option<String>,
    parent_boundary_id: Option<String>,
    source_kind: CodeCityBoundarySource,
    files: Vec<String>,
    entry_points: Vec<CodeCityEntryPoint>,
    diagnostics: Vec<CodeCityDiagnostic>,
}

#[derive(Debug, Clone)]
struct ManifestDescriptor {
    ecosystem: Option<String>,
    source: CodeCityBoundarySource,
    kind: CodeCityBoundaryKind,
}

impl ManifestDescriptor {
    fn fallback() -> Self {
        Self {
            ecosystem: None,
            source: CodeCityBoundarySource::Fallback,
            kind: CodeCityBoundaryKind::RootFallback,
        }
    }

    fn boundary_id(&self, root_path: &str) -> String {
        if root_path == "." {
            CODECITY_ROOT_BOUNDARY_ID.to_string()
        } else {
            format!("boundary:{root_path}")
        }
    }

    fn boundary_name(&self, root_path: &str) -> String {
        if root_path == "." {
            "root".to_string()
        } else {
            Path::new(root_path)
                .file_name()
                .and_then(OsStr::to_str)
                .unwrap_or(root_path)
                .to_string()
        }
    }
}

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

fn discover_manifest_roots(
    repo_root: &Path,
    analysis_root: &str,
    files: &[String],
    diagnostics: &mut Vec<CodeCityDiagnostic>,
) -> BTreeMap<String, ManifestDescriptor> {
    let mut manifests = BTreeMap::new();
    let candidate_directories = files
        .iter()
        .flat_map(|file| ancestor_directories_within_scope(file, analysis_root))
        .collect::<BTreeSet<_>>();

    for ancestor in candidate_directories {
        let absolute = repo_root.join(&ancestor);
        if let Some((manifest_name, ecosystem)) = find_manifest_in_dir(&absolute) {
            let source =
                infer_manifest_source(&absolute.join(&manifest_name), &manifest_name, diagnostics);
            manifests.insert(
                ancestor,
                ManifestDescriptor {
                    ecosystem,
                    source,
                    kind: CodeCityBoundaryKind::Explicit,
                },
            );
        }
    }
    manifests
}

fn assign_files_to_explicit_roots(
    files: &[String],
    roots: &BTreeMap<String, ManifestDescriptor>,
) -> BTreeMap<String, String> {
    let root_names = roots.keys().collect::<Vec<_>>();
    files
        .iter()
        .map(|path| {
            let root = deepest_manifest_root(path, root_names.iter().copied())
                .unwrap_or_else(|| ".".to_string());
            (path.clone(), root)
        })
        .collect()
}

fn ancestor_directories_within_scope(path: &str, analysis_root: &str) -> Vec<String> {
    let path = Path::new(path);
    let scope_root = if analysis_root == "." {
        PathBuf::new()
    } else {
        PathBuf::from(analysis_root)
    };
    let mut directories = Vec::new();
    let mut current = path.parent().unwrap_or(Path::new(""));

    loop {
        let relative = if current.as_os_str().is_empty() {
            ".".to_string()
        } else {
            current.to_string_lossy().to_string()
        };
        directories.push(relative.clone());
        if current == scope_root || current.as_os_str().is_empty() {
            break;
        }
        current = current.parent().unwrap_or(Path::new(""));
    }

    directories
}

fn find_manifest_in_dir(dir: &Path) -> Option<(String, Option<String>)> {
    let candidates = [
        "package.json",
        "go.mod",
        "Cargo.toml",
        "pom.xml",
        "build.gradle",
        "build.gradle.kts",
        "pyproject.toml",
        "setup.py",
        "setup.cfg",
        "BUILD",
        "BUILD.bazel",
        "project.json",
        "lerna.json",
        "nx.json",
        "pnpm-workspace.yaml",
    ];

    for candidate in candidates {
        if dir.join(candidate).exists() {
            return Some((candidate.to_string(), infer_ecosystem(candidate)));
        }
    }

    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.ends_with(".csproj") || name.ends_with(".fsproj") || name.ends_with(".sln") {
            return Some((name.clone(), infer_ecosystem(&name)));
        }
    }

    None
}

fn infer_manifest_source(
    manifest_path: &Path,
    manifest_name: &str,
    diagnostics: &mut Vec<CodeCityDiagnostic>,
) -> CodeCityBoundarySource {
    let raw = match std::fs::read_to_string(manifest_path) {
        Ok(raw) => raw,
        Err(_) => return CodeCityBoundarySource::Manifest,
    };

    match manifest_name {
        "package.json" | "lerna.json" => match serde_json::from_str::<Value>(&raw) {
            Ok(json) if json.get("workspaces").is_some() || json.get("packages").is_some() => {
                CodeCityBoundarySource::WorkspaceManifest
            }
            Ok(_) => CodeCityBoundarySource::Manifest,
            Err(err) => {
                diagnostics.push(CodeCityDiagnostic {
                    code: "codecity.boundary.workspace_parse_skipped".to_string(),
                    severity: "info".to_string(),
                    message: format!(
                        "Skipped workspace parsing for `{}`: {err}",
                        manifest_path.display()
                    ),
                    path: None,
                    boundary_id: None,
                });
                CodeCityBoundarySource::Manifest
            }
        },
        "Cargo.toml" => match toml_edit::de::from_str::<Value>(&raw) {
            Ok(value) if value.get("workspace").is_some() => {
                CodeCityBoundarySource::WorkspaceManifest
            }
            Ok(_) => CodeCityBoundarySource::Manifest,
            Err(err) => {
                diagnostics.push(CodeCityDiagnostic {
                    code: "codecity.boundary.workspace_parse_skipped".to_string(),
                    severity: "info".to_string(),
                    message: format!(
                        "Skipped workspace parsing for `{}`: {err}",
                        manifest_path.display()
                    ),
                    path: None,
                    boundary_id: None,
                });
                CodeCityBoundarySource::Manifest
            }
        },
        "pnpm-workspace.yaml" => {
            diagnostics.push(CodeCityDiagnostic {
                code: "codecity.boundary.workspace_parse_skipped".to_string(),
                severity: "info".to_string(),
                message: format!(
                    "Skipped workspace parsing for `{}` because YAML parsing is not enabled in phase 2.",
                    manifest_path.display()
                ),
                path: None,
                boundary_id: None,
            });
            CodeCityBoundarySource::Manifest
        }
        _ => CodeCityBoundarySource::Manifest,
    }
}

fn infer_ecosystem(manifest_name: &str) -> Option<String> {
    Some(
        match manifest_name {
            "package.json" | "project.json" | "lerna.json" | "nx.json" | "pnpm-workspace.yaml" => {
                "node"
            }
            "Cargo.toml" => "rust",
            "go.mod" => "go",
            "pyproject.toml" | "setup.py" | "setup.cfg" => "python",
            "pom.xml" | "build.gradle" | "build.gradle.kts" => "jvm",
            name if name.ends_with(".csproj")
                || name.ends_with(".fsproj")
                || name.ends_with(".sln") =>
            {
                "dotnet"
            }
            "BUILD" | "BUILD.bazel" => "bazel",
            _ => return None,
        }
        .to_string(),
    )
}

fn deepest_manifest_root<'a>(
    path: &str,
    roots: impl Iterator<Item = &'a String>,
) -> Option<String> {
    roots
        .filter(|root| {
            root.as_str() == "."
                || path == root.as_str()
                || path
                    .strip_prefix(root.as_str())
                    .is_some_and(|suffix| suffix.starts_with('/'))
        })
        .max_by_key(|root| root.len())
        .cloned()
}

fn split_runtime_boundaries(
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

fn split_implicit_boundaries(
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

    let boundaries = communities
        .communities
        .iter()
        .enumerate()
        .map(|(index, community)| {
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
                    root_path: boundary.boundary.root_path.clone(),
                    id: format!("{}:implicit:{}", boundary.boundary.id, slugify(&name)),
                    name,
                    kind: CodeCityBoundaryKind::Implicit,
                    ecosystem: boundary.boundary.ecosystem.clone(),
                    parent_boundary_id: Some(boundary.boundary.id.clone()),
                    source_kind: CodeCityBoundarySource::CommunityDetection,
                    files: community.paths.clone(),
                    entry_points: Vec::new(),
                    diagnostics: Vec::new(),
                },
            )
        })
        .collect::<Vec<_>>();

    BoundarySplitResult {
        boundaries,
        diagnostics: Vec::new(),
    }
}

fn build_boundary(source: &CodeCitySourceGraph, spec: BoundaryBuildSpec) -> ResolvedBoundary {
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

fn detect_entry_points(files: &[String], artefacts: &[CodeCitySourceArtefact]) -> Vec<String> {
    let mut entries = BTreeSet::new();
    for path in files {
        let basename = Path::new(path)
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or("");
        if matches!(
            basename,
            "main.go" | "main.py" | "index.ts" | "index.js" | "app.py" | "App.java" | "Program.cs"
        ) || path.ends_with("src/main.rs")
            || path.contains("/bin/")
            || path.starts_with("bin/")
            || (path.contains("/cmd/") && path.ends_with("/main.go"))
        {
            entries.insert(path.clone());
        }
    }

    for artefact in artefacts {
        let is_main = artefact
            .symbol_fqn
            .as_deref()
            .is_some_and(|symbol| symbol.ends_with("::main"))
            || artefact.symbol_id.eq_ignore_ascii_case("main")
            || artefact
                .signature
                .as_deref()
                .is_some_and(|signature| signature.contains(" main("));
        if is_main && files.contains(&artefact.path) {
            entries.insert(artefact.path.clone());
        }
    }

    entries.into_iter().collect()
}

fn infer_entry_kind(path: &str) -> String {
    let basename = Path::new(path)
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("");
    match basename {
        "index.ts" | "index.js" => "node_index".to_string(),
        "main.go" | "main.py" => "main".to_string(),
        _ => "entry_point".to_string(),
    }
}

fn dependency_closure(
    entry: &str,
    files: &[String],
    source: &CodeCitySourceGraph,
) -> BTreeSet<String> {
    let file_set = files.iter().cloned().collect::<BTreeSet<_>>();
    let adjacency = source
        .edges
        .iter()
        .filter(|edge| file_set.contains(&edge.from_path) && file_set.contains(&edge.to_path))
        .fold(BTreeMap::<String, Vec<String>>::new(), |mut map, edge| {
            map.entry(edge.from_path.clone())
                .or_default()
                .push(edge.to_path.clone());
            map
        });

    let mut closure = BTreeSet::from([entry.to_string()]);
    let mut stack = vec![entry.to_string()];
    while let Some(path) = stack.pop() {
        if let Some(targets) = adjacency.get(&path) {
            for target in targets {
                if closure.insert(target.clone()) {
                    stack.push(target.clone());
                }
            }
        }
    }
    closure
}

fn closure_overlap(left: &BTreeSet<String>, right: &BTreeSet<String>) -> f64 {
    let denominator = left.len().min(right.len());
    if denominator == 0 {
        return 0.0;
    }
    left.intersection(right).count() as f64 / denominator as f64
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

fn slugify(value: &str) -> String {
    value
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect()
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::detect_boundaries;
    use crate::capability_packs::codecity::services::config::CodeCityConfig;
    use crate::capability_packs::codecity::services::source_graph::{
        CodeCitySourceArtefact, CodeCitySourceEdge, CodeCitySourceFile, CodeCitySourceGraph,
    };
    use crate::capability_packs::codecity::types::CODECITY_ROOT_BOUNDARY_ID;

    fn graph(files: &[&str], edges: &[(&str, &str)]) -> CodeCitySourceGraph {
        CodeCitySourceGraph {
            project_path: None,
            files: files
                .iter()
                .map(|path| CodeCitySourceFile {
                    path: (*path).to_string(),
                    language: "typescript".to_string(),
                    effective_content_id: format!("content::{path}"),
                    included: true,
                    exclusion_reason: None,
                })
                .collect(),
            artefacts: files
                .iter()
                .map(|path| CodeCitySourceArtefact {
                    artefact_id: format!("artefact::{path}"),
                    symbol_id: format!("symbol::{path}"),
                    path: (*path).to_string(),
                    symbol_fqn: Some(format!("{path}::file")),
                    canonical_kind: Some("file".to_string()),
                    language_kind: Some("fixture".to_string()),
                    parent_artefact_id: None,
                    parent_symbol_id: None,
                    signature: None,
                    start_line: 1,
                    end_line: 1,
                })
                .collect(),
            edges: edges
                .iter()
                .enumerate()
                .map(|(index, (from, to))| CodeCitySourceEdge {
                    edge_id: format!("edge-{index}"),
                    from_path: (*from).to_string(),
                    to_path: (*to).to_string(),
                    from_symbol_id: format!("symbol::{from}"),
                    from_artefact_id: format!("artefact::{from}"),
                    to_symbol_id: Some(format!("symbol::{to}")),
                    to_artefact_id: Some(format!("artefact::{to}")),
                    to_symbol_ref: Some(format!("{to}::file")),
                    edge_kind: "imports".to_string(),
                    language: "typescript".to_string(),
                    start_line: Some(1),
                    end_line: Some(1),
                    metadata: "{}".to_string(),
                })
                .collect(),
            external_dependency_hints: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn falls_back_to_root_boundary_when_no_manifest_exists() {
        let temp = tempdir().expect("tempdir");
        let source = graph(
            &["src/main.ts", "src/core.ts"],
            &[("src/main.ts", "src/core.ts")],
        );
        std::fs::create_dir_all(temp.path().join("src")).expect("mkdir");
        std::fs::write(temp.path().join("src/main.ts"), "export {}").expect("write");
        std::fs::write(temp.path().join("src/core.ts"), "export {}").expect("write");

        let result = detect_boundaries(&source, &CodeCityConfig::default(), temp.path());

        assert_eq!(result.boundaries.len(), 1);
        assert_eq!(result.boundaries[0].id, CODECITY_ROOT_BOUNDARY_ID);
    }

    #[test]
    fn skips_implicit_split_for_large_interactive_boundaries() {
        let temp = tempdir().expect("tempdir");
        let files = (0..=super::MAX_INTERACTIVE_IMPLICIT_BOUNDARY_FILES)
            .map(|index| format!("src/file_{index}.ts"))
            .collect::<Vec<_>>();
        let file_refs = files.iter().map(String::as_str).collect::<Vec<_>>();
        let source = graph(&file_refs, &[]);

        let result = detect_boundaries(&source, &CodeCityConfig::default(), temp.path());

        assert_eq!(result.boundaries.len(), 1);
        assert_eq!(result.boundaries[0].id, CODECITY_ROOT_BOUNDARY_ID);
        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "codecity.boundary.implicit_split_too_large"
            })
        );
    }

    #[test]
    fn detects_explicit_manifest_boundaries_within_scope() {
        let temp = tempdir().expect("tempdir");
        std::fs::create_dir_all(temp.path().join("packages/api/src")).expect("mkdir");
        std::fs::write(
            temp.path().join("packages/api/package.json"),
            r#"{ "name": "@demo/api" }"#,
        )
        .expect("write manifest");
        std::fs::write(temp.path().join("packages/api/src/main.ts"), "export {}").expect("write");
        std::fs::write(temp.path().join("packages/api/src/core.ts"), "export {}").expect("write");

        let mut source = graph(
            &["packages/api/src/main.ts", "packages/api/src/core.ts"],
            &[("packages/api/src/main.ts", "packages/api/src/core.ts")],
        );
        source.project_path = Some("packages/api".to_string());

        let result = detect_boundaries(&source, &CodeCityConfig::default(), temp.path());

        assert_eq!(result.boundaries.len(), 1);
        assert_eq!(result.boundaries[0].id, "boundary:packages/api");
        assert_eq!(result.boundaries[0].root_path, "packages/api");
        assert_eq!(
            result.file_to_boundary["packages/api/src/main.ts"],
            "boundary:packages/api"
        );
        assert_eq!(
            result.file_to_boundary["packages/api/src/core.ts"],
            "boundary:packages/api"
        );
    }
}

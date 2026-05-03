use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::capability_packs::codecity::types::{
    CODECITY_ROOT_BOUNDARY_ID, CodeCityBoundaryKind, CodeCityBoundarySource, CodeCityDiagnostic,
};

const MANIFEST_FILE_NAMES: &[&str] = &[
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

#[derive(Debug, Clone)]
pub(super) struct ManifestDescriptor {
    pub(super) ecosystem: Option<String>,
    pub(super) source: CodeCityBoundarySource,
    pub(super) kind: CodeCityBoundaryKind,
}

impl ManifestDescriptor {
    pub(super) fn fallback() -> Self {
        Self {
            ecosystem: None,
            source: CodeCityBoundarySource::Fallback,
            kind: CodeCityBoundaryKind::RootFallback,
        }
    }

    pub(super) fn boundary_id(&self, root_path: &str) -> String {
        boundary_id_for_root(root_path)
    }

    pub(super) fn boundary_name(&self, root_path: &str) -> String {
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

pub(super) fn boundary_id_for_root(root_path: &str) -> String {
    if root_path == "." {
        CODECITY_ROOT_BOUNDARY_ID.to_string()
    } else {
        format!("boundary:{root_path}")
    }
}

pub(super) fn discover_manifest_roots(
    repo_root: &Path,
    analysis_root: &str,
    files: &[String],
    indexed_files: &[String],
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

    for indexed_file in indexed_files {
        let Some((root_path, _manifest_name, ecosystem)) = indexed_manifest_root(indexed_file)
        else {
            continue;
        };
        if !root_is_within_scope(&root_path, analysis_root) {
            continue;
        }
        manifests
            .entry(root_path)
            .or_insert_with(|| ManifestDescriptor {
                ecosystem,
                source: CodeCityBoundarySource::Manifest,
                kind: CodeCityBoundaryKind::Explicit,
            });
    }

    manifests
}

pub(super) fn assign_files_to_explicit_roots(
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
    for candidate in MANIFEST_FILE_NAMES {
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

fn indexed_manifest_root(path: &str) -> Option<(String, String, Option<String>)> {
    let path = Path::new(path);
    let manifest_name = path.file_name().and_then(OsStr::to_str)?;
    if !is_manifest_name(manifest_name) {
        return None;
    }

    let root = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(|parent| parent.to_string_lossy().to_string())
        .unwrap_or_else(|| ".".to_string());

    Some((
        root,
        manifest_name.to_string(),
        infer_ecosystem(manifest_name),
    ))
}

fn is_manifest_name(name: &str) -> bool {
    MANIFEST_FILE_NAMES.contains(&name)
        || name.ends_with(".csproj")
        || name.ends_with(".fsproj")
        || name.ends_with(".sln")
}

fn root_is_within_scope(root_path: &str, analysis_root: &str) -> bool {
    analysis_root == "."
        || root_path == analysis_root
        || root_path
            .strip_prefix(analysis_root)
            .is_some_and(|suffix| suffix.starts_with('/'))
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
                    "Skipped workspace parsing for `{}` because YAML parsing is not enabled in architecture analysis.",
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

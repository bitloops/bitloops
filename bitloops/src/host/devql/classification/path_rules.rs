use std::path::{Component, Path, PathBuf};

use anyhow::{Result, bail};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::host::devql::normalize_repo_path;

pub(super) fn normalise_candidate_paths<I, S>(candidate_paths: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut out = candidate_paths
        .into_iter()
        .map(|value| normalize_repo_path(value.as_ref()))
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    out.sort();
    out.dedup();
    out
}

pub(super) fn compute_sha256_json(value: &Value) -> String {
    let mut hasher = Sha256::new();
    hasher.update(serde_json::to_vec(value).unwrap_or_else(|_| value.to_string().into_bytes()));
    hex::encode(hasher.finalize())
}

pub(super) fn join_dir_file(dir: &str, file_name: &str) -> String {
    if dir.is_empty() {
        file_name.to_string()
    } else {
        format!("{dir}/{file_name}")
    }
}

pub(super) fn parent_dir(path: &str) -> String {
    Path::new(path)
        .parent()
        .and_then(|value| value.to_str())
        .map(normalize_repo_path)
        .unwrap_or_default()
}

pub(super) fn ancestor_dirs(path: &str) -> Vec<String> {
    let mut dirs = Vec::new();
    let mut current = parent_dir(path);
    loop {
        dirs.push(current.clone());
        if current.is_empty() {
            break;
        }
        current = parent_dir(&current);
    }
    dirs
}

pub(super) fn relative_to_root(path: &str, root_rel: &str) -> Option<String> {
    let path = normalize_repo_path(path);
    if root_rel.is_empty() {
        return Some(path);
    }
    if path == root_rel {
        return Some(String::new());
    }
    let prefix = format!("{root_rel}/");
    path.strip_prefix(&prefix).map(str::to_string)
}

pub(super) fn lower_extension(path: &str) -> Option<String> {
    Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
}

pub(super) fn file_name(path: &str) -> &str {
    Path::new(path)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
}

pub(super) fn path_depth(path: &str) -> usize {
    path.split('/')
        .filter(|segment| !segment.is_empty())
        .count()
}

pub(super) fn matching_entries<F>(entries: &[String], predicate: F) -> Vec<String>
where
    F: Fn(&str) -> bool,
{
    entries
        .iter()
        .filter(|entry| predicate(entry))
        .cloned()
        .collect()
}

pub(super) fn is_tsconfig_file(entry: &str) -> bool {
    entry == "tsconfig.json" || (entry.starts_with("tsconfig.") && entry.ends_with(".json"))
}

pub(super) fn is_jsconfig_file(entry: &str) -> bool {
    entry == "jsconfig.json" || (entry.starts_with("jsconfig.") && entry.ends_with(".json"))
}

pub(super) fn is_requirements_file(entry: &str) -> bool {
    entry == "requirements.txt" || (entry.starts_with("requirements") && entry.ends_with(".txt"))
}

pub(super) fn is_auto_text_path(path: &str) -> bool {
    let path = normalize_repo_path(path);
    let file_name = Path::new(&path)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let file_name_lower = file_name.to_ascii_lowercase();
    if file_name.starts_with("README")
        || file_name.starts_with("CHANGELOG")
        || file_name.starts_with("LICENSE")
        || file_name_lower.starts_with(".env")
    {
        return !is_lockfile_name(file_name);
    }
    if matches!(
        file_name,
        "Cargo.toml"
            | "package.json"
            | "pyproject.toml"
            | "setup.py"
            | "setup.cfg"
            | "Pipfile"
            | "go.mod"
            | "pom.xml"
    ) {
        return true;
    }
    if is_tsconfig_file(file_name) || is_jsconfig_file(file_name) || is_requirements_file(file_name)
    {
        return true;
    }
    if file_name.starts_with("build.gradle") {
        return true;
    }
    if is_lockfile_name(file_name) {
        return false;
    }
    matches!(
        lower_extension(&path).as_deref(),
        Some(
            "md" | "mdx"
                | "txt"
                | "rst"
                | "adoc"
                | "toml"
                | "yaml"
                | "yml"
                | "json"
                | "jsonc"
                | "ini"
                | "cfg"
                | "conf"
        )
    )
}

pub(super) fn is_lockfile_name(file_name: &str) -> bool {
    matches!(
        file_name,
        "Cargo.lock"
            | "package-lock.json"
            | "pnpm-lock.yaml"
            | "yarn.lock"
            | "poetry.lock"
            | "Pipfile.lock"
    ) || file_name.ends_with(".lock")
}

pub(super) fn auto_ignored_dirs_for_kind(kind: &str) -> Vec<String> {
    match kind {
        "rust" => vec!["target".to_string()],
        "node" | "typescript" => vec![
            "node_modules".to_string(),
            ".next".to_string(),
            "dist".to_string(),
            "build".to_string(),
            "coverage".to_string(),
        ],
        "python" => vec![
            ".venv".to_string(),
            "venv".to_string(),
            "__pycache__".to_string(),
            ".pytest_cache".to_string(),
            ".mypy_cache".to_string(),
        ],
        "go" => vec!["vendor".to_string(), "bin".to_string()],
        "java" => vec![
            "target".to_string(),
            "build".to_string(),
            ".gradle".to_string(),
        ],
        _ => Vec::new(),
    }
}

pub(super) fn resolve_policy_relative_path(
    repo_root_abs: &Path,
    policy_root_abs: &Path,
    raw_value: &str,
) -> Result<PathBuf> {
    let candidate = PathBuf::from(raw_value);
    let resolved = if candidate.is_absolute() {
        candidate
    } else {
        policy_root_abs.join(candidate)
    };
    let resolved = resolved.canonicalize().unwrap_or(resolved);
    if !resolved.starts_with(repo_root_abs) {
        bail!(
            "path `{}` resolves outside repo root {}",
            raw_value,
            repo_root_abs.display()
        );
    }
    Ok(resolved)
}

pub(super) fn pathbuf_to_repo_relative(path: &Path) -> String {
    normalize_lexical_path(path)
        .to_string_lossy()
        .replace('\\', "/")
        .trim_start_matches("./")
        .trim_start_matches('/')
        .to_string()
}

pub(super) fn display_root(root_rel: &str) -> String {
    if root_rel.is_empty() {
        ".".to_string()
    } else {
        root_rel.to_string()
    }
}

fn normalize_lexical_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

pub(super) fn sorted_dedup(mut values: Vec<String>) -> Vec<String> {
    values.sort();
    values.dedup();
    values
}

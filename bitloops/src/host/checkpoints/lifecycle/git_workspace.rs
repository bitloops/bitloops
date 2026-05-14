use std::collections::HashSet;
use std::path::Path;

/// Returns (modified, new_files, deleted) relative to repo_root. Used by handle_lifecycle_turn_end.
pub(super) fn detect_file_changes_for_turn_end(
    repo_root: &Path,
    previously_untracked: Option<&[String]>,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    use std::collections::{BTreeSet, HashSet};
    use std::process::Command;

    let output = match Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=all"])
        .current_dir(repo_root)
        .output()
    {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => return (Vec::new(), Vec::new(), Vec::new()),
    };

    let pre: HashSet<&str> = previously_untracked
        .unwrap_or(&[])
        .iter()
        .map(String::as_str)
        .collect();
    let mut modified = BTreeSet::new();
    let mut new_files = BTreeSet::new();
    let mut deleted = BTreeSet::new();

    for line in output.lines() {
        if line.len() < 3 {
            continue;
        }
        let status = &line[..2];
        let mut path = line[3..].trim().to_string();
        if let Some(idx) = path.rfind(" -> ") {
            path = path[idx + 4..].to_string();
        }
        if path.is_empty()
            || path.ends_with('/')
            || crate::utils::paths::is_infrastructure_path(&path)
            || crate::utils::paths::is_protected_path(&path)
        {
            continue;
        }
        if status == "??" {
            if previously_untracked.is_none() || !pre.contains(path.as_str()) {
                new_files.insert(path);
            }
            continue;
        }
        let x = status.as_bytes().first().copied().unwrap_or(b' ');
        let y = status.as_bytes().get(1).copied().unwrap_or(b' ');
        if x == b'D' || y == b'D' {
            deleted.insert(path);
            continue;
        }
        if x != b' ' || y != b' ' {
            modified.insert(path);
        }
    }

    let base = repo_root.to_string_lossy();
    let normalize = |paths: BTreeSet<String>| {
        paths
            .into_iter()
            .map(|p| crate::utils::paths::to_relative_path(&p, &base))
            .filter(|p| !p.is_empty() && !p.starts_with(".."))
            .collect::<Vec<_>>()
    };
    (
        normalize(modified),
        normalize(new_files),
        normalize(deleted),
    )
}

pub(super) fn filter_and_normalize_paths_for_turn_end(
    files: &[String],
    repo_root: &Path,
) -> Vec<String> {
    let base = repo_root.to_string_lossy();
    files
        .iter()
        .map(|p| crate::utils::paths::to_relative_path(p, &base))
        .filter(|p| {
            !p.is_empty()
                && !p.starts_with("..")
                && !crate::utils::paths::is_infrastructure_path(p)
                && !crate::utils::paths::is_protected_path(p)
        })
        .collect()
}

pub(super) fn merge_unique_for_turn_end(mut base: Vec<String>, extra: Vec<String>) -> Vec<String> {
    if extra.is_empty() {
        return base;
    }
    let mut seen: HashSet<String> = base.iter().cloned().collect();
    for path in extra {
        if seen.insert(path.clone()) {
            base.push(path);
        }
    }
    base
}

pub(super) fn filter_to_uncommitted_files_for_turn_end(
    repo_root: &Path,
    files: Vec<String>,
) -> Vec<String> {
    if files.is_empty() {
        return files;
    }

    let head_probe = std::process::Command::new("git")
        .args(["rev-parse", "--verify", "HEAD"])
        .current_dir(repo_root)
        .output();
    let Ok(head_probe) = head_probe else {
        return files;
    };
    if !head_probe.status.success() {
        return files;
    }

    let mut filtered = Vec::with_capacity(files.len());
    for rel_path in files {
        let head_spec = format!("HEAD:{rel_path}");
        let head_has_file = std::process::Command::new("git")
            .args(["cat-file", "-e", &head_spec])
            .current_dir(repo_root)
            .output();
        let Ok(head_has_file) = head_has_file else {
            filtered.push(rel_path);
            continue;
        };
        if !head_has_file.status.success() {
            filtered.push(rel_path);
            continue;
        }

        let working_content = std::fs::read(repo_root.join(&rel_path));
        let Ok(working_content) = working_content else {
            filtered.push(rel_path);
            continue;
        };

        let head_content = std::process::Command::new("git")
            .args(["show", &head_spec])
            .current_dir(repo_root)
            .output();
        let Ok(head_content) = head_content else {
            filtered.push(rel_path);
            continue;
        };
        if !head_content.status.success() {
            filtered.push(rel_path);
            continue;
        }

        if working_content != head_content.stdout {
            filtered.push(rel_path);
        }
    }

    filtered
}

pub(super) fn collect_untracked_files_for_lifecycle(repo_root: &Path) -> Vec<String> {
    let output = std::process::Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=all"])
        .current_dir(repo_root)
        .output();

    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.strip_prefix("?? "))
        .map(str::trim)
        .filter(|path| !path.is_empty() && !crate::utils::paths::is_infrastructure_path(path))
        .map(ToOwned::to_owned)
        .collect()
}

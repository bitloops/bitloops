use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use crate::host::checkpoints::strategy::manual_commit::{
    is_missing_head_error, run_git, try_head_hash,
};

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StagedChange {
    Added(String),
    Modified(String),
    Deleted,
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspaceState {
    pub(crate) head_commit_sha: Option<String>,
    pub(crate) head_tree_sha: Option<String>,
    pub(crate) active_branch: Option<String>,
    pub(crate) head_tree: HashMap<String, String>,
    pub(crate) staged_changes: HashMap<String, StagedChange>,
    pub(crate) dirty_files: Vec<String>,
    pub(crate) untracked_files: Vec<String>,
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn inspect_workspace(repo_root: &Path) -> Result<WorkspaceState> {
    let head_commit_sha =
        try_head_hash(repo_root).context("resolving HEAD for workspace inspection")?;
    let has_head = head_commit_sha.is_some();
    let head_tree_sha = match head_commit_sha.as_deref() {
        Some(_) => Some(
            run_git(repo_root, &["rev-parse", "HEAD^{tree}"])
                .context("resolving HEAD tree for workspace inspection")?,
        ),
        None => None,
    };
    let head_tree = if head_tree_sha.is_some() {
        read_head_tree(repo_root)?
    } else {
        HashMap::new()
    };

    Ok(WorkspaceState {
        head_commit_sha,
        head_tree_sha,
        active_branch: read_active_branch(repo_root)?,
        head_tree,
        staged_changes: read_staged_changes(repo_root, has_head)?,
        dirty_files: read_dirty_files(repo_root)?,
        untracked_files: read_untracked_files(repo_root)?,
    })
}

#[cfg_attr(not(test), allow(dead_code))]
fn read_active_branch(repo_root: &Path) -> Result<Option<String>> {
    let branch = run_git(repo_root, &["branch", "--show-current"])
        .context("reading active branch for workspace inspection")?;
    Ok((!branch.is_empty()).then_some(branch))
}

#[cfg_attr(not(test), allow(dead_code))]
fn read_head_tree(repo_root: &Path) -> Result<HashMap<String, String>> {
    let output = match run_git(repo_root, &["ls-tree", "-rz", "--full-tree", "HEAD"]) {
        Ok(output) => output,
        Err(err) if is_missing_head_error(&err) => return Ok(HashMap::new()),
        Err(err) => return Err(err).context("listing HEAD tree for workspace inspection"),
    };

    let mut entries = HashMap::new();
    for record in output.split('\0').filter(|value| !value.is_empty()) {
        let Some((metadata, raw_path)) = record.split_once('\t') else {
            continue;
        };
        let mut metadata_parts = metadata.split_whitespace();
        let Some(mode) = metadata_parts.next() else {
            continue;
        };
        let Some(object_type) = metadata_parts.next() else {
            continue;
        };
        let Some(oid) = metadata_parts.next() else {
            continue;
        };

        if should_skip_git_entry(mode, object_type) {
            continue;
        }

        if raw_path.is_empty() {
            continue;
        }

        entries.insert(raw_path.to_string(), oid.to_string());
    }

    Ok(entries)
}

#[cfg_attr(not(test), allow(dead_code))]
fn read_staged_changes(repo_root: &Path, has_head: bool) -> Result<HashMap<String, StagedChange>> {
    if has_head {
        let output = run_git(
            repo_root,
            &[
                "diff-index",
                "--cached",
                "--raw",
                "--no-renames",
                "--diff-filter=ADM",
                "-z",
                "HEAD",
            ],
        )
        .context("listing staged changes for workspace inspection")?;
        return parse_staged_changes_from_diff_index(&output);
    }

    let output = run_git(repo_root, &["ls-files", "--stage", "-z"])
        .context("listing staged additions for unborn HEAD workspace inspection")?;
    parse_staged_changes_from_ls_files(&output)
}

#[cfg_attr(not(test), allow(dead_code))]
fn read_dirty_files(repo_root: &Path) -> Result<Vec<String>> {
    let output = run_git(repo_root, &["diff", "--name-only", "-z"])
        .context("listing dirty files for workspace inspection")?;
    collect_paths(repo_root, &output, false)
}

#[cfg_attr(not(test), allow(dead_code))]
fn read_untracked_files(repo_root: &Path) -> Result<Vec<String>> {
    let output = run_git(
        repo_root,
        &["ls-files", "--others", "--exclude-standard", "-z"],
    )
    .context("listing untracked files for workspace inspection")?;
    collect_paths(repo_root, &output, true)
}

#[cfg_attr(not(test), allow(dead_code))]
fn collect_paths(
    repo_root: &Path,
    output: &str,
    require_existing_file: bool,
) -> Result<Vec<String>> {
    let mut paths = Vec::new();
    for raw_path in output.split('\0').filter(|value| !value.is_empty()) {
        if raw_path.is_empty() {
            continue;
        }
        if should_skip_workspace_path(repo_root, raw_path, require_existing_file)? {
            continue;
        }
        paths.push(raw_path.to_string());
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

#[cfg_attr(not(test), allow(dead_code))]
fn should_skip_workspace_path(
    repo_root: &Path,
    path: &str,
    require_existing_file: bool,
) -> Result<bool> {
    let full_path = repo_root.join(path);
    let metadata = match fs::symlink_metadata(&full_path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => {
            return Err(anyhow::Error::from(err))
                .with_context(|| format!("reading metadata for `{path}`"));
        }
    };

    if metadata.file_type().is_symlink() {
        return Ok(true);
    }

    if require_existing_file && !metadata.file_type().is_file() {
        return Ok(true);
    }

    Ok(false)
}

#[cfg_attr(not(test), allow(dead_code))]
fn should_skip_git_entry(mode: &str, object_type: &str) -> bool {
    mode == "120000" || mode == "160000" || object_type != "blob"
}

#[cfg_attr(not(test), allow(dead_code))]
fn parse_staged_changes_from_diff_index(output: &str) -> Result<HashMap<String, StagedChange>> {
    let mut changes = HashMap::new();
    let mut parts = output.split('\0').filter(|value| !value.is_empty());

    while let Some(metadata) = parts.next() {
        let Some(path) = parts.next() else {
            break;
        };
        if path.is_empty() {
            continue;
        }

        let mut fields = metadata.split_whitespace();
        let Some(_old_mode) = fields.next() else {
            continue;
        };
        let Some(new_mode) = fields.next() else {
            continue;
        };
        let Some(_old_sha) = fields.next() else {
            continue;
        };
        let Some(new_sha) = fields.next() else {
            continue;
        };
        let Some(status) = fields.next() else {
            continue;
        };

        if should_skip_git_entry(new_mode, "blob") && status != "D" {
            continue;
        }

        let change = match status {
            "A" => StagedChange::Added(new_sha.to_string()),
            "M" => StagedChange::Modified(new_sha.to_string()),
            "D" => StagedChange::Deleted,
            _ => continue,
        };
        changes.insert(path.to_string(), change);
    }

    Ok(changes)
}

#[cfg_attr(not(test), allow(dead_code))]
fn parse_staged_changes_from_ls_files(output: &str) -> Result<HashMap<String, StagedChange>> {
    let mut changes = HashMap::new();

    for record in output.split('\0').filter(|value| !value.is_empty()) {
        let Some((metadata, path)) = record.split_once('\t') else {
            continue;
        };
        if path.is_empty() {
            continue;
        }

        let mut fields = metadata.split_whitespace();
        let Some(mode) = fields.next() else {
            continue;
        };
        let Some(sha) = fields.next() else {
            continue;
        };
        let Some(stage) = fields.next() else {
            continue;
        };

        if stage != "0" || should_skip_git_entry(mode, "blob") {
            continue;
        }

        changes.insert(path.to_string(), StagedChange::Added(sha.to_string()));
    }

    Ok(changes)
}

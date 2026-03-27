use anyhow::{Result, anyhow};
use std::fs;
use std::path::{Path, PathBuf};

use super::repo::repo_root;

pub fn is_inside_worktree() -> bool {
    let Ok(root) = repo_root() else {
        return false;
    };

    get_worktree_id(&root)
        .map(|worktree_id| !worktree_id.is_empty())
        .unwrap_or(false)
}

pub fn get_main_repo_root() -> Result<PathBuf> {
    let worktree_root = repo_root()?;
    let git_path = worktree_root.join(".git");
    let git_meta = fs::metadata(&git_path).map_err(|err| anyhow!("failed to stat .git: {err}"))?;

    if git_meta.is_dir() {
        return Ok(worktree_root);
    }

    let content =
        fs::read_to_string(&git_path).map_err(|err| anyhow!("failed to read .git file: {err}"))?;
    let line = content.trim();
    let gitdir = line
        .strip_prefix("gitdir: ")
        .ok_or_else(|| anyhow!("invalid .git file format: {line}"))?;

    let gitdir_path = if Path::new(gitdir).is_absolute() {
        PathBuf::from(gitdir)
    } else {
        worktree_root.join(gitdir)
    };
    let normalized = gitdir_path.to_string_lossy().replace('\\', "/");

    let Some((main_root, _)) = normalized.rsplit_once("/.git/") else {
        return Err(anyhow!("unexpected gitdir format: {gitdir}"));
    };

    Ok(PathBuf::from(main_root))
}

pub fn get_worktree_id(worktree_path: &Path) -> Result<String> {
    let git_path = worktree_path.join(".git");
    let info = fs::metadata(&git_path).map_err(|err| anyhow!("failed to stat .git: {err}"))?;

    if info.is_dir() {
        return Ok(String::new());
    }

    let content =
        fs::read_to_string(&git_path).map_err(|err| anyhow!("failed to read .git file: {err}"))?;
    let line = content.trim();
    if !line.starts_with("gitdir: ") {
        return Err(anyhow!("invalid .git file format: {line}"));
    }

    let gitdir = line.trim_start_matches("gitdir: ");
    let normalized_gitdir = gitdir.replace('\\', "/");
    let marker = ".git/worktrees/";
    let Some((_, worktree_id)) = normalized_gitdir.split_once(marker) else {
        return Err(anyhow!("unexpected gitdir format (no worktrees): {gitdir}"));
    };
    Ok(worktree_id.trim_end_matches('/').to_string())
}

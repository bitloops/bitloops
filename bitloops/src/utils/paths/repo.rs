use anyhow::{Result, anyhow};
use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{OnceLock, RwLock};

use super::constants::BITLOOPS_DIR;

#[derive(Clone)]
struct RepoRootCache {
    cwd: PathBuf,
    root: PathBuf,
}

fn repo_root_cache() -> &'static RwLock<Option<RepoRootCache>> {
    static CACHE: OnceLock<RwLock<Option<RepoRootCache>>> = OnceLock::new();
    CACHE.get_or_init(|| RwLock::new(None))
}

pub fn repo_root() -> Result<PathBuf> {
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::new());

    if !cwd.as_os_str().is_empty() {
        let cache = repo_root_cache().read().expect("repo root cache poisoned");
        if let Some(entry) = cache.as_ref()
            && entry.cwd == cwd
        {
            return Ok(entry.root.clone());
        }
    }

    let mut cmd = Command::new("git");
    cmd.args(["rev-parse", "--show-toplevel"])
        .stdin(Stdio::null());
    if !cwd.as_os_str().is_empty() {
        cmd.current_dir(&cwd);
    }
    let output = cmd
        .output()
        .map_err(|err| anyhow!("failed to get git repository root: {err}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            return Err(anyhow!("failed to get git repository root"));
        }
        return Err(anyhow!("failed to get git repository root: {stderr}"));
    }

    let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if root.is_empty() {
        return Err(anyhow!(
            "failed to get git repository root: git returned empty output"
        ));
    }
    let root = PathBuf::from(root);

    if !cwd.as_os_str().is_empty() {
        let mut cache = repo_root_cache().write().expect("repo root cache poisoned");
        *cache = Some(RepoRootCache {
            cwd,
            root: root.clone(),
        });
    }

    Ok(root)
}

pub fn open_repository() -> Result<PathBuf> {
    repo_root()
}

pub fn bitloops_project_root(start: &Path) -> Result<PathBuf> {
    let mut dir = if start.is_absolute() {
        start.to_path_buf()
    } else {
        env::current_dir()
            .map_err(|e| anyhow!("cannot determine current directory: {e}"))?
            .join(start)
    };

    let mut search = dir.clone();
    loop {
        if search.join(BITLOOPS_DIR).is_dir() {
            return Ok(search);
        }
        match search.parent() {
            Some(parent) if parent != search => search = parent.to_path_buf(),
            _ => break,
        }
    }

    loop {
        if dir.join(".git").exists() {
            return Ok(dir);
        }
        match dir.parent() {
            Some(parent) if parent != dir => dir = parent.to_path_buf(),
            _ => {
                return Err(anyhow!(
                    "not inside a git repository (no .git directory found)"
                ));
            }
        }
    }
}

pub fn clear_repo_root_cache() {
    let mut cache = repo_root_cache().write().expect("repo root cache poisoned");
    *cache = None;
}

pub fn abs_path(path: &str) -> Result<PathBuf> {
    let path = Path::new(path);
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    Ok(repo_root()?.join(path))
}

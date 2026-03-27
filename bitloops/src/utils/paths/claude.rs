use anyhow::{Result, anyhow};
use std::env;
use std::path::{Path, PathBuf};

pub fn sanitize_path_for_claude(path: &str) -> String {
    path.chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect()
}

pub fn get_claude_project_dir(repo_path: &str) -> Result<PathBuf> {
    let override_path = env::var("BITLOOPS_TEST_CLAUDE_PROJECT_DIR").unwrap_or_default();
    if !override_path.is_empty() {
        return Ok(PathBuf::from(override_path));
    }

    let home_dir = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .ok_or_else(|| anyhow!("failed to get home directory"))?;

    let project_dir = sanitize_path_for_claude(repo_path);
    Ok(Path::new(&home_dir)
        .join(".claude")
        .join("projects")
        .join(project_dir))
}

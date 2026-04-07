use std::path::Path;

use super::constants::{BITLOOPS_DIR, BITLOOPS_TEST_STATE_DIR};

pub fn is_infrastructure_path(path: &str) -> bool {
    let normalized = path.replace('\\', "/");
    is_dir_or_descendant(&normalized, BITLOOPS_DIR)
        || is_dir_or_descendant(&normalized, BITLOOPS_TEST_STATE_DIR)
}

pub fn is_protected_path(path: &str) -> bool {
    let normalized = path
        .replace('\\', "/")
        .trim_start_matches("./")
        .trim_end_matches('/')
        .to_string();

    [
        ".git",
        ".worktrees",
        BITLOOPS_DIR,
        BITLOOPS_TEST_STATE_DIR,
        ".claude",
        ".github/hooks",
        ".codex",
        ".cursor",
        ".gemini",
    ]
    .iter()
    .any(|dir| is_dir_or_descendant(&normalized, dir))
}

pub fn to_relative_path(abs_path: &str, cwd: &str) -> String {
    let abs = Path::new(abs_path);
    if !abs.is_absolute() {
        return abs_path.to_string();
    }
    match abs.strip_prefix(Path::new(cwd)) {
        Ok(rel) if rel.as_os_str().is_empty() => ".".to_string(),
        Ok(rel) => rel.to_string_lossy().into_owned(),
        Err(_) => String::new(),
    }
}

fn is_dir_or_descendant(path: &str, dir: &str) -> bool {
    path == dir || path.starts_with(&format!("{dir}/"))
}

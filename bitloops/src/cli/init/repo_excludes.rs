use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::adapters::agents::{
    AGENT_NAME_CLAUDE_CODE, AGENT_NAME_CODEX, AGENT_NAME_COPILOT, AGENT_NAME_GEMINI,
    AGENT_NAME_OPEN_CODE,
};
use crate::config::REPO_POLICY_LOCAL_FILE_NAME;

pub(crate) fn ensure_repo_init_files_excluded(
    git_root: &Path,
    project_root: &Path,
    selected_agents: &[String],
) -> Result<()> {
    let exclude_path = git_root.join(".git").join("info").join("exclude");
    if let Some(parent) = exclude_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating git exclude directory {}", parent.display()))?;
    }

    let mut content = std::fs::read_to_string(&exclude_path).unwrap_or_default();
    let mut excluded_paths = vec![project_root.join(REPO_POLICY_LOCAL_FILE_NAME)];
    excluded_paths.extend(repo_managed_skill_paths(project_root, selected_agents));

    for path in excluded_paths {
        let entry = path
            .strip_prefix(git_root)
            .unwrap_or(path.as_path())
            .to_string_lossy()
            .replace('\\', "/");
        if content.lines().any(|line| line.trim() == entry) {
            continue;
        }
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(&entry);
        content.push('\n');
    }

    std::fs::write(&exclude_path, content)
        .with_context(|| format!("writing {}", exclude_path.display()))?;
    Ok(())
}

fn repo_managed_skill_paths(project_root: &Path, selected_agents: &[String]) -> Vec<PathBuf> {
    let mut paths = Vec::new();

    for agent in selected_agents {
        let path = match agent.as_str() {
            AGENT_NAME_CLAUDE_CODE => {
                Some(crate::adapters::agents::claude_code::skills::repo_skill_path(project_root))
            }
            AGENT_NAME_CODEX => Some(crate::adapters::agents::codex::skills::repo_skill_path(
                project_root,
            )),
            AGENT_NAME_GEMINI => Some(crate::adapters::agents::gemini::skills::repo_skill_path(
                project_root,
            )),
            AGENT_NAME_COPILOT => Some(crate::adapters::agents::copilot::skills::repo_skill_path(
                project_root,
            )),
            AGENT_NAME_OPEN_CODE => Some(
                crate::adapters::agents::open_code::skills::repo_skill_path(project_root),
            ),
            _ => None,
        };
        if let Some(path) = path {
            paths.push(path);
        }
    }

    paths
}

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
    let selected_agents = selected_agents
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let exclude_path = git_root.join(".git").join("info").join("exclude");
    if let Some(parent) = exclude_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating git exclude directory {}", parent.display()))?;
    }

    let mut content = std::fs::read_to_string(&exclude_path).unwrap_or_default();
    for entry in repo_init_exclude_entries(git_root, project_root, &selected_agents) {
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

pub(crate) fn clear_repo_init_files_excluded(git_root: &Path, project_root: &Path) -> Result<bool> {
    let exclude_path = git_root.join(".git").join("info").join("exclude");
    let content = match std::fs::read_to_string(&exclude_path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => {
            return Err(err).with_context(|| format!("reading {}", exclude_path.display()));
        }
    };

    let managed_entries =
        repo_init_exclude_entries(git_root, project_root, &managed_repo_skill_agents());
    let mut changed = false;
    let mut retained_lines = Vec::new();
    for line in content.lines() {
        if managed_entries.iter().any(|entry| line.trim() == entry) {
            changed = true;
            continue;
        }
        retained_lines.push(line);
    }

    if !changed {
        return Ok(false);
    }

    let mut updated = retained_lines.join("\n");
    if !updated.is_empty() {
        updated.push('\n');
    }
    std::fs::write(&exclude_path, updated)
        .with_context(|| format!("writing {}", exclude_path.display()))?;
    Ok(true)
}

fn repo_init_exclude_entries(
    git_root: &Path,
    project_root: &Path,
    selected_agents: &[&str],
) -> Vec<String> {
    let mut excluded_paths = vec![project_root.join(REPO_POLICY_LOCAL_FILE_NAME)];
    excluded_paths.extend(repo_managed_skill_paths(project_root, selected_agents));
    excluded_paths
        .into_iter()
        .map(|path| {
            path.strip_prefix(git_root)
                .unwrap_or(path.as_path())
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect()
}

fn managed_repo_skill_agents() -> [&'static str; 5] {
    [
        AGENT_NAME_CLAUDE_CODE,
        AGENT_NAME_CODEX,
        AGENT_NAME_GEMINI,
        AGENT_NAME_COPILOT,
        AGENT_NAME_OPEN_CODE,
    ]
}

fn repo_managed_skill_paths(project_root: &Path, selected_agents: &[&str]) -> Vec<PathBuf> {
    let mut paths = Vec::new();

    for agent in selected_agents {
        let path = match *agent {
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

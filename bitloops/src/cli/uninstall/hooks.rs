use std::io::Write;
use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::adapters::agents::claude_code::git_hooks;

struct ProjectRepoCleanup {
    removed_policy_files: usize,
    cleared_exclude_entries: bool,
}

impl ProjectRepoCleanup {
    fn changed(&self) -> bool {
        self.removed_policy_files > 0 || self.cleared_exclude_entries
    }
}

pub(super) fn uninstall_agent_hooks(project_roots: &[PathBuf], out: &mut dyn Write) -> Result<()> {
    if project_roots.is_empty() {
        writeln!(
            out,
            "  No known Bitloops projects found for agent hook removal."
        )?;
        return Ok(());
    }

    let mut cleaned_projects = 0usize;
    for project_root in project_roots {
        let configured =
            crate::config::settings::supported_agents(project_root).unwrap_or_default();
        let mut project_out = Vec::new();
        let attempted = crate::cli::agent_surfaces::cleanup_project_agent_surfaces(
            project_root,
            &configured,
            &mut project_out,
        )?;
        let repo_cleanup = cleanup_project_repo_state(project_root)?;
        if attempted == 0 && !repo_cleanup.changed() {
            continue;
        }

        writeln!(out, "  Agent hooks: {}", project_root.display())?;
        write!(out, "{}", String::from_utf8(project_out).unwrap())?;
        if repo_cleanup.removed_policy_files > 0 {
            writeln!(
                out,
                "    Removed {} repo policy file(s).",
                repo_cleanup.removed_policy_files
            )?;
        }
        if repo_cleanup.cleared_exclude_entries {
            writeln!(out, "    Cleared managed .git/info/exclude entries.")?;
        }
        cleaned_projects += 1;
    }

    if cleaned_projects == 0 {
        writeln!(out, "  No agent hooks found.")?;
    }

    Ok(())
}

pub(super) fn uninstall_git_hooks(repo_roots: &[PathBuf], out: &mut dyn Write) -> Result<()> {
    if repo_roots.is_empty() {
        writeln!(out, "  No known repositories found for git hook removal.")?;
        return Ok(());
    }

    let mut removed = 0usize;
    for repo_root in repo_roots {
        let count = git_hooks::uninstall_git_hooks(repo_root).unwrap_or(0);
        if count == 0 {
            continue;
        }

        writeln!(
            out,
            "  Removed {count} git hook(s) from {}",
            repo_root.display()
        )?;
        removed += count;
    }

    if removed == 0 {
        writeln!(out, "  No git hooks found.")?;
    }

    Ok(())
}

fn cleanup_project_repo_state(project_root: &std::path::Path) -> Result<ProjectRepoCleanup> {
    let removed_policy_files = remove_repo_policy_files(project_root)?;
    let cleared_exclude_entries = match crate::cli::enable::find_repo_root(project_root) {
        Ok(git_root) => crate::cli::init::clear_repo_init_files_excluded(&git_root, project_root)?,
        Err(_) => false,
    };

    Ok(ProjectRepoCleanup {
        removed_policy_files,
        cleared_exclude_entries,
    })
}

fn remove_repo_policy_files(project_root: &std::path::Path) -> Result<usize> {
    let mut removed = 0usize;
    for file_name in [
        crate::config::REPO_POLICY_FILE_NAME,
        crate::config::REPO_POLICY_LOCAL_FILE_NAME,
    ] {
        let path = project_root.join(file_name);
        match std::fs::remove_file(&path) {
            Ok(()) => removed += 1,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(err).with_context(|| format!("removing {}", path.display()));
            }
        }
    }
    Ok(removed)
}

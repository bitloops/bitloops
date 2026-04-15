use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;

use crate::adapters::agents::claude_code::git_hooks;

pub(super) fn uninstall_agent_hooks(project_roots: &[PathBuf], out: &mut dyn Write) -> Result<()> {
    if project_roots.is_empty() {
        writeln!(
            out,
            "  No known Bitloops projects found for agent hook removal."
        )?;
        return Ok(());
    }

    let mut removed_projects = 0usize;
    for project_root in project_roots {
        let configured =
            crate::config::settings::supported_agents(project_root).unwrap_or_default();
        let mut project_out = Vec::new();
        let attempted = crate::cli::agent_surfaces::cleanup_project_agent_surfaces(
            project_root,
            &configured,
            &mut project_out,
        )?;
        if attempted == 0 {
            continue;
        }

        writeln!(out, "  Agent hooks: {}", project_root.display())?;
        write!(out, "{}", String::from_utf8(project_out).unwrap())?;
        removed_projects += 1;
    }

    if removed_projects == 0 {
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

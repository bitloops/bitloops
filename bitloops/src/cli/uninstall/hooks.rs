use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;

use crate::adapters::agents::claude_code::git_hooks;
use crate::cli::enable;

pub(super) fn uninstall_agent_hooks(repo_roots: &[PathBuf], out: &mut dyn Write) -> Result<()> {
    if repo_roots.is_empty() {
        writeln!(out, "  No known repositories found for agent hook removal.")?;
        return Ok(());
    }

    let registry = crate::adapters::agents::AgentAdapterRegistry::builtin();
    let mut removed = 0usize;

    for repo_root in repo_roots {
        let installed = registry.installed_agents(repo_root);
        if installed.is_empty() {
            continue;
        }

        writeln!(out, "  Agent hooks: {}", repo_root.display())?;
        enable::remove_agent_hooks(repo_root, out)?;
        removed += installed.len();
    }

    if removed == 0 {
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

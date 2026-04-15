use std::collections::BTreeSet;
use std::env;
use std::path::PathBuf;

use anyhow::{Context, Result};

use super::targets::{UninstallArgs, UninstallTarget};
use crate::cli::enable;
use crate::devql_transport::load_repo_path_registry;
use crate::utils::platform_dirs::bitloops_state_dir;

pub(super) struct ResolvedScope {
    pub(super) hook_repo_roots: Vec<PathBuf>,
    pub(super) agent_project_roots: Vec<PathBuf>,
    pub(super) repo_data_roots: Vec<PathBuf>,
}

pub(super) fn resolve_scope(
    args: &UninstallArgs,
    targets: &BTreeSet<UninstallTarget>,
) -> Result<ResolvedScope> {
    let hook_repo_roots = if targets.contains(&UninstallTarget::GitHooks) {
        if args.only_current_project {
            vec![
                current_repo_root()?
                    .context("`--only-current-project` requires running inside a git repository")?,
            ]
        } else {
            discover_known_repo_roots()?
        }
    } else {
        Vec::new()
    };

    let agent_project_roots = if targets.contains(&UninstallTarget::AgentHooks) {
        if args.only_current_project {
            vec![current_bitloops_project_root()?
                .or(current_repo_root()?)
                .context(
                    "`--only-current-project` requires running inside a git repository or Bitloops project",
                )?]
        } else {
            discover_bitloops_project_roots()?
        }
    } else {
        Vec::new()
    };

    let repo_data_roots = if targets.contains(&UninstallTarget::Data) {
        discover_known_repo_roots()?
    } else {
        Vec::new()
    };

    Ok(ResolvedScope {
        hook_repo_roots,
        agent_project_roots,
        repo_data_roots,
    })
}

fn discover_known_repo_roots() -> Result<Vec<PathBuf>> {
    let mut roots = BTreeSet::new();

    if let Some(current) = current_repo_root()? {
        roots.insert(current);
    }

    if let Some(registry_path) = repo_registry_path() {
        let registry = load_repo_path_registry(&registry_path).unwrap_or_default();
        for entry in registry.entries {
            let repo_root = entry
                .repo_root
                .canonicalize()
                .unwrap_or_else(|_| entry.repo_root.clone());
            if repo_root.join(".git").exists() {
                roots.insert(repo_root);
            }
        }
    }

    Ok(roots.into_iter().collect())
}

fn current_repo_root() -> Result<Option<PathBuf>> {
    let cwd = env::current_dir().context("getting current directory")?;
    Ok(enable::find_repo_root(&cwd).ok().map(|repo_root| {
        repo_root
            .canonicalize()
            .unwrap_or_else(|_| repo_root.to_path_buf())
    }))
}

fn current_bitloops_project_root() -> Result<Option<PathBuf>> {
    let cwd = env::current_dir().context("getting current directory")?;
    let snapshot = crate::config::discover_repo_policy_optional(&cwd)?;
    Ok(snapshot
        .root
        .map(|root| root.canonicalize().unwrap_or(root)))
}

fn should_skip_project_discovery_dir(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | "node_modules"
            | "target"
            | "vendor"
            | ".venv"
            | "venv"
            | "__pycache__"
            | ".next"
            | "dist"
            | "build"
    )
}

fn discover_bitloops_project_roots() -> Result<Vec<PathBuf>> {
    let mut roots = BTreeSet::new();
    let registry = crate::adapters::agents::AgentAdapterRegistry::builtin();

    if let Some(current) = current_bitloops_project_root()? {
        roots.insert(current);
    }

    for git_root in discover_known_repo_roots()? {
        if !registry.installed_agents(&git_root).is_empty() {
            roots.insert(git_root.clone());
        }
        for entry in walkdir::WalkDir::new(&git_root)
            .into_iter()
            .filter_entry(|entry| {
                if !entry.file_type().is_dir() {
                    return true;
                }
                let name = entry.file_name().to_string_lossy();
                !should_skip_project_discovery_dir(name.as_ref())
            })
        {
            let Ok(entry) = entry else {
                continue;
            };
            if !entry.file_type().is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy();
            if matches!(
                name.as_ref(),
                crate::config::REPO_POLICY_FILE_NAME | crate::config::REPO_POLICY_LOCAL_FILE_NAME
            ) && let Some(parent) = entry.path().parent()
            {
                roots.insert(
                    parent
                        .canonicalize()
                        .unwrap_or_else(|_| parent.to_path_buf()),
                );
            }
        }
    }

    Ok(roots.into_iter().collect())
}

fn repo_registry_path() -> Option<PathBuf> {
    bitloops_state_dir()
        .ok()
        .map(|state_dir| state_dir.join("daemon").join("repo-path-registry.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_discovery_skips_known_heavy_directories() {
        for name in [
            ".git",
            "node_modules",
            "target",
            "vendor",
            ".venv",
            "venv",
            "__pycache__",
            ".next",
            "dist",
            "build",
        ] {
            assert!(
                should_skip_project_discovery_dir(name),
                "{name} should be skipped"
            );
        }
    }

    #[test]
    fn project_discovery_keeps_normal_source_directories() {
        for name in ["packages", "apps", "src", "services", "repo"] {
            assert!(
                !should_skip_project_discovery_dir(name),
                "{name} should remain discoverable"
            );
        }
    }
}

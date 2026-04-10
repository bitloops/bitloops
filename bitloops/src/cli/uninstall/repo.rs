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
    pub(super) repo_data_roots: Vec<PathBuf>,
}

pub(super) fn resolve_scope(
    args: &UninstallArgs,
    targets: &BTreeSet<UninstallTarget>,
) -> Result<ResolvedScope> {
    let hook_repo_roots = if targets.contains(&UninstallTarget::AgentHooks)
        || targets.contains(&UninstallTarget::GitHooks)
    {
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

    let repo_data_roots = if targets.contains(&UninstallTarget::Data) {
        discover_known_repo_roots()?
    } else {
        Vec::new()
    };

    Ok(ResolvedScope {
        hook_repo_roots,
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

fn repo_registry_path() -> Option<PathBuf> {
    bitloops_state_dir()
        .ok()
        .map(|state_dir| state_dir.join("daemon").join("repo-path-registry.json"))
}

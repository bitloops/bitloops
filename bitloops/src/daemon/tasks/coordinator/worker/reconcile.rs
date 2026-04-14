use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::config::resolve_store_backend_config_for_repo;
use crate::daemon::types::{DevqlTaskSource, DevqlTaskSpec, SyncTaskMode, SyncTaskSpec};
use crate::host::devql::{DevqlConfig, RelationalStorage};

use super::super::DevqlTaskCoordinator;

impl DevqlTaskCoordinator {
    pub(super) async fn ensure_scope_exclusion_reconciles(
        self: &Arc<Self>,
        config_root: &Path,
        repo_registry_path: Option<&Path>,
    ) -> Result<bool> {
        let mut blocked = false;
        for repo_root in
            self.scope_exclusion_reconcile_repo_roots(config_root, repo_registry_path)?
        {
            match self
                .ensure_scope_exclusion_reconcile_for_repo(&repo_root)
                .await
            {
                Ok(repo_blocked) => blocked |= repo_blocked,
                Err(err) => log::warn!(
                    "daemon DevQL exclusion reconcile error for {}: {err:#}",
                    repo_root.display()
                ),
            }
        }
        Ok(blocked)
    }

    pub(super) fn scope_exclusion_reconcile_repo_roots(
        &self,
        config_root: &Path,
        repo_registry_path: Option<&Path>,
    ) -> Result<Vec<PathBuf>> {
        let current_root = config_root
            .canonicalize()
            .unwrap_or_else(|_| config_root.to_path_buf());
        let current_daemon_config_root =
            crate::config::resolve_bound_daemon_config_root_for_repo(&current_root)
                .unwrap_or_else(|_| current_root.clone());
        let mut repo_roots = Vec::new();
        let mut seen = HashSet::new();

        if git_repo_root(&current_root)
            .as_deref()
            .is_some_and(|repo_root| repo_root == current_root.as_path())
            && seen.insert(current_root.clone())
        {
            repo_roots.push(current_root.clone());
        }

        let Some(repo_registry_path) = repo_registry_path else {
            return Ok(repo_roots);
        };
        let registry = crate::devql_transport::load_repo_path_registry(repo_registry_path)?;
        for entry in registry.entries {
            let repo_root = entry
                .repo_root
                .canonicalize()
                .unwrap_or(entry.repo_root.clone());
            let Ok(bound_daemon_config_root) =
                crate::config::resolve_bound_daemon_config_root_for_repo(&repo_root)
            else {
                continue;
            };
            if bound_daemon_config_root != current_daemon_config_root {
                continue;
            }
            if seen.insert(repo_root.clone()) {
                repo_roots.push(repo_root);
            }
        }

        Ok(repo_roots)
    }

    pub(super) async fn ensure_scope_exclusion_reconcile_for_repo(
        self: &Arc<Self>,
        repo_root: &Path,
    ) -> Result<bool> {
        let repo = crate::host::devql::resolve_repo_identity(repo_root)
            .context("resolving repo identity for exclusion reconciliation")?;
        let cfg = DevqlConfig::from_env(repo_root.to_path_buf(), repo)
            .context("building DevQL config for exclusion reconciliation")?;
        let backends = resolve_store_backend_config_for_repo(&cfg.daemon_config_root)
            .context("resolving backend config for exclusion reconciliation")?;
        let relational = RelationalStorage::connect(
            &cfg,
            &backends.relational,
            "daemon exclusion reconciliation",
        )
        .await?;
        let blocking = self.has_blocking_scope_exclusion_reconcile(&cfg.repo.repo_id)?;
        let needs_reconcile =
            crate::host::devql::scope_exclusion_reconcile_needed(&cfg, &relational)
                .await?
                .is_some();
        if blocking || needs_reconcile {
            self.prune_excluded_path_sync_tasks_for_repo(&cfg)?;
        }
        if blocking {
            return Ok(true);
        }
        if needs_reconcile {
            self.enqueue(
                &cfg,
                DevqlTaskSource::RepoPolicyChange,
                DevqlTaskSpec::Sync(SyncTaskSpec {
                    mode: SyncTaskMode::Full,
                }),
            )?;
            return Ok(true);
        }
        Ok(blocking)
    }
}

pub(super) async fn persist_scope_exclusions_fingerprint(
    cfg: &DevqlConfig,
    relational: Option<&RelationalStorage>,
    fingerprint_override: Option<&str>,
) -> Result<()> {
    let fingerprint = match fingerprint_override {
        Some(fingerprint) => fingerprint.to_string(),
        None => crate::host::devql::current_scope_exclusions_fingerprint(&cfg.repo_root)
            .context("loading current scope exclusions fingerprint")?,
    };

    if let Some(relational) = relational {
        return crate::host::devql::sync::state::write_scope_exclusions_fingerprint(
            relational,
            &cfg.repo.repo_id,
            &fingerprint,
        )
        .await;
    }

    let backends = resolve_store_backend_config_for_repo(&cfg.daemon_config_root)
        .context("resolving backend config for scope exclusion fingerprint persistence")?;
    let relational = RelationalStorage::connect(
        cfg,
        &backends.relational,
        "persisting queued DevQL scope exclusions fingerprint",
    )
    .await?;
    crate::host::devql::sync::state::write_scope_exclusions_fingerprint(
        &relational,
        &cfg.repo.repo_id,
        &fingerprint,
    )
    .await
}

fn git_repo_root(path: &Path) -> Option<PathBuf> {
    crate::host::checkpoints::strategy::manual_commit::run_git(
        path,
        &["rev-parse", "--show-toplevel"],
    )
    .ok()
    .map(PathBuf::from)
    .map(|repo_root| repo_root.canonicalize().unwrap_or(repo_root))
}

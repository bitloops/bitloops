use std::collections::BTreeSet;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use super::super::DevqlConfig;
use super::row_access::{is_missing_table_error, optional_row_string, row_string};
use super::types::{AnalyticsRepoScope, AnalyticsRepository};
use crate::config::StoreBackendConfig;
use crate::host::relational_store::{DefaultRelationalStore, RelationalStore};

fn current_repository(cfg: &DevqlConfig) -> AnalyticsRepository {
    AnalyticsRepository {
        repo_id: cfg.repo.repo_id.clone(),
        repo_root: Some(cfg.repo_root.clone()),
        provider: cfg.repo.provider.clone(),
        organization: cfg.repo.organization.clone(),
        name: cfg.repo.name.clone(),
        identity: cfg.repo.identity.clone(),
        default_branch: None,
    }
}

pub(super) async fn resolve_analytics_scope(
    cfg: &DevqlConfig,
    backends: &StoreBackendConfig,
    scope: AnalyticsRepoScope,
) -> Result<Vec<AnalyticsRepository>> {
    let known = list_known_repositories(cfg, backends).await?;
    match scope {
        AnalyticsRepoScope::CurrentRepo => Ok(vec![current_repository(cfg)]),
        AnalyticsRepoScope::AllKnown => {
            if known.is_empty() {
                Ok(vec![current_repository(cfg)])
            } else {
                Ok(known)
            }
        }
        AnalyticsRepoScope::Explicit(selectors) => {
            let current = current_repository(cfg);
            let mut resolved = Vec::new();
            let mut seen = BTreeSet::new();
            for selector in selectors {
                let selector = selector.trim();
                if selector.is_empty() {
                    continue;
                }
                let repository = resolve_repository_selector(&known, &current, selector)?;
                if seen.insert(repository.repo_id.clone()) {
                    resolved.push(repository);
                }
            }
            if resolved.is_empty() {
                bail!("analytics scope resolved to zero repositories")
            }
            Ok(resolved)
        }
    }
}

async fn list_known_repositories(
    cfg: &DevqlConfig,
    backends: &StoreBackendConfig,
) -> Result<Vec<AnalyticsRepository>> {
    let relational =
        DefaultRelationalStore::open_local_for_backend_config(&cfg.repo_root, &backends.relational)
            .context("opening local relational store for analytics repository catalogue")?;
    let sql = "SELECT r.repo_id, \
                      COALESCE(s.repo_root, '') AS repo_root, \
                      COALESCE(r.provider, '') AS provider, \
                      COALESCE(r.organization, '') AS organization, \
                      COALESCE(r.name, '') AS name, \
                      (COALESCE(r.provider, '') || '://' || COALESCE(r.organization, '') || '/' || COALESCE(r.name, '')) AS identity, \
                      COALESCE(r.default_branch, '') AS default_branch \
               FROM repositories AS r \
               LEFT JOIN repo_sync_state AS s ON s.repo_id = r.repo_id \
               ORDER BY r.name ASC, r.provider ASC, r.organization ASC";
    let rows = match relational.query_rows(sql).await {
        Ok(rows) => rows,
        Err(err) if is_missing_table_error(&err) => Vec::new(),
        Err(err) => return Err(err).context("querying analytics repository catalogue"),
    };

    let mut repositories = rows
        .into_iter()
        .filter_map(|row| repository_from_row(&row))
        .collect::<Vec<_>>();
    repositories.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.provider.cmp(&right.provider))
            .then_with(|| left.organization.cmp(&right.organization))
    });
    repositories.dedup_by(|left, right| left.repo_id == right.repo_id);
    Ok(repositories)
}

fn repository_from_row(row: &Value) -> Option<AnalyticsRepository> {
    let repo_id = row_string(row, "repo_id");
    if repo_id.is_empty() {
        return None;
    }
    Some(AnalyticsRepository {
        repo_id,
        repo_root: optional_row_string(row, "repo_root").map(PathBuf::from),
        provider: row_string(row, "provider"),
        organization: row_string(row, "organization"),
        name: row_string(row, "name"),
        identity: row_string(row, "identity"),
        default_branch: optional_row_string(row, "default_branch"),
    })
}

fn resolve_repository_selector(
    known: &[AnalyticsRepository],
    current: &AnalyticsRepository,
    selector: &str,
) -> Result<AnalyticsRepository> {
    if matches_repository_selector(current, selector) {
        return Ok(current.clone());
    }

    if let Some(repository) = known
        .iter()
        .find(|repository| repository.repo_id == selector)
    {
        return Ok(repository.clone());
    }
    if let Some(repository) = known
        .iter()
        .find(|repository| repository.identity == selector)
    {
        return Ok(repository.clone());
    }

    let matches = known
        .iter()
        .filter(|repository| repository.name == selector)
        .cloned()
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [repository] => Ok(repository.clone()),
        [] => bail!("unknown repository `{selector}`"),
        _ => {
            bail!("repository name `{selector}` is ambiguous; use the repo id or identity instead")
        }
    }
}

fn matches_repository_selector(repository: &AnalyticsRepository, selector: &str) -> bool {
    repository.repo_id == selector || repository.identity == selector || repository.name == selector
}

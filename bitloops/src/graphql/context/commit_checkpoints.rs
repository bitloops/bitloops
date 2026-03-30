use super::DevqlGraphqlContext;
use crate::adapters::agents::canonical_agent_key;
use crate::config::resolve_store_backend_config_for_repo;
use crate::graphql::ResolverScope;
use crate::graphql::types::{Checkpoint, DateTimeScalar};
use crate::host::checkpoints::strategy::manual_commit::{list_committed, read_committed_info};
use crate::host::devql::resolve_repo_identity;
use crate::storage::SqliteConnectionPool;
use anyhow::{Context, Result};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use tokio::task;

impl DevqlGraphqlContext {
    pub(crate) async fn list_committed_checkpoints(
        &self,
        scope: &ResolverScope,
        agent: Option<&str>,
        since: Option<&DateTimeScalar>,
    ) -> Result<Vec<Checkpoint>> {
        let repo_root = self.repo_root_for_scope(scope)?;
        let scope = scope.clone();
        let agent = agent.map(str::to_string);
        let since = since.cloned();

        task::spawn_blocking(move || -> Result<Vec<Checkpoint>> {
            let committed = match list_committed(repo_root.as_path()) {
                Ok(committed) => committed,
                Err(err) if is_missing_sqlite_store_error(&err) => return Ok(Vec::new()),
                Err(err) => return Err(err).context("reading committed checkpoints"),
            };
            let checkpoint_commits =
                match read_latest_checkpoint_commit_mappings(repo_root.as_path()) {
                    Ok(mappings) => mappings,
                    Err(err) if is_missing_sqlite_store_error(&err) => HashMap::new(),
                    Err(err) => {
                        return Err(err).context("reading latest commit mappings for checkpoints");
                    }
                };

            let mut checkpoints = Vec::new();
            for info in committed {
                if !committed_checkpoint_matches_scope(&info, &scope) {
                    continue;
                }
                if !committed_checkpoint_matches_agent(&info, agent.as_deref()) {
                    continue;
                }

                let checkpoint = Checkpoint::from_ingested(
                    &info,
                    checkpoint_commits
                        .get(&info.checkpoint_id)
                        .map(String::as_str),
                );
                if !committed_checkpoint_matches_since(&checkpoint, since.as_ref()) {
                    continue;
                }
                checkpoints.push(checkpoint);
            }
            Ok(checkpoints)
        })
        .await
        .context("joining committed checkpoint query task")?
    }

    pub(crate) async fn list_commit_checkpoints(
        &self,
        scope: &ResolverScope,
        commit_sha: &str,
    ) -> Result<Vec<Checkpoint>> {
        let repo_root = self.repo_root_for_scope(scope)?;
        let repo_id = self.repo_id_for_scope(scope)?;
        let commit_sha = commit_sha.to_string();
        let sqlite_path = self
            .backend_config
            .as_ref()
            .context("store backend configuration unavailable")?
            .relational
            .resolve_sqlite_db_path_for_repo(&self.config_root)
            .context("resolving SQLite path for commit checkpoints")?;

        task::spawn_blocking(move || -> Result<Vec<Checkpoint>> {
            let sqlite = match SqliteConnectionPool::connect_existing(sqlite_path) {
                Ok(sqlite) => sqlite,
                Err(err) if is_missing_sqlite_store_error(&err) => return Ok(Vec::new()),
                Err(err) => return Err(err).context("opening checkpoint SQLite store"),
            };
            sqlite
                .initialise_checkpoint_schema()
                .context("initialising checkpoint schema for GraphQL commit checkpoints")?;
            let checkpoint_ids = sqlite.with_connection(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT checkpoint_id
                     FROM commit_checkpoints
                     WHERE repo_id = ?1 AND commit_sha = ?2
                     ORDER BY created_at DESC, checkpoint_id DESC",
                )?;
                let mut rows =
                    stmt.query(rusqlite::params![repo_id.as_str(), commit_sha.as_str()])?;
                let mut ids = Vec::new();
                while let Some(row) = rows.next()? {
                    ids.push(row.get::<_, String>(0)?);
                }
                Ok(ids)
            })?;

            let mut checkpoints = Vec::new();
            for checkpoint_id in checkpoint_ids {
                if let Some(info) = read_committed_info(repo_root.as_path(), &checkpoint_id)? {
                    checkpoints.push(Checkpoint::from_committed(&commit_sha, &info));
                }
            }
            Ok(checkpoints)
        })
        .await
        .context("joining commit checkpoint query task")?
    }
}

pub(super) fn read_commit_checkpoint_mappings_all(
    repo_root: &Path,
) -> Result<BTreeMap<String, Vec<String>>> {
    let cfg = resolve_store_backend_config_for_repo(repo_root)?;
    let sqlite_path = cfg.relational.resolve_sqlite_db_path_for_repo(repo_root)?;
    let sqlite = SqliteConnectionPool::connect_existing(sqlite_path)
        .context("opening SQLite database for commit-checkpoint mappings")?;
    sqlite
        .initialise_checkpoint_schema()
        .context("initialising checkpoint schema for commit-checkpoint mappings")?;
    let repo_id = resolve_repo_identity(repo_root)?.repo_id;

    sqlite.with_connection(|conn| {
        let mut stmt = conn.prepare(
            "SELECT commit_sha, checkpoint_id
             FROM commit_checkpoints
             WHERE repo_id = ?1
             ORDER BY created_at DESC, checkpoint_id DESC",
        )?;
        let mut rows = stmt.query(rusqlite::params![repo_id.as_str()])?;
        let mut out = BTreeMap::<String, Vec<String>>::new();
        while let Some(row) = rows.next()? {
            let commit_sha = row.get::<_, String>(0)?.trim().to_string();
            let checkpoint_id = row.get::<_, String>(1)?.trim().to_string();
            if commit_sha.is_empty() || checkpoint_id.is_empty() {
                continue;
            }
            out.entry(commit_sha).or_default().push(checkpoint_id);
        }
        Ok(out)
    })
}

fn read_latest_checkpoint_commit_mappings(repo_root: &Path) -> Result<HashMap<String, String>> {
    let cfg = resolve_store_backend_config_for_repo(repo_root)?;
    let sqlite_path = cfg.relational.resolve_sqlite_db_path_for_repo(repo_root)?;
    let sqlite = SqliteConnectionPool::connect_existing(sqlite_path)
        .context("opening SQLite database for latest checkpoint commit mappings")?;
    sqlite
        .initialise_checkpoint_schema()
        .context("initialising checkpoint schema for latest checkpoint commit mappings")?;
    let repo_id = resolve_repo_identity(repo_root)?.repo_id;

    sqlite.with_connection(|conn| {
        let mut stmt = conn.prepare(
            "SELECT checkpoint_id, commit_sha
             FROM commit_checkpoints
             WHERE repo_id = ?1
             ORDER BY created_at DESC, checkpoint_id DESC",
        )?;
        let mut rows = stmt.query(rusqlite::params![repo_id.as_str()])?;
        let mut out = HashMap::<String, String>::new();
        while let Some(row) = rows.next()? {
            let checkpoint_id = row.get::<_, String>(0)?.trim().to_string();
            let commit_sha = row.get::<_, String>(1)?.trim().to_string();
            if checkpoint_id.is_empty() || commit_sha.is_empty() {
                continue;
            }
            out.entry(checkpoint_id).or_insert(commit_sha);
        }
        Ok(out)
    })
}

fn committed_checkpoint_matches_scope(
    checkpoint: &crate::host::checkpoints::strategy::manual_commit::CommittedInfo,
    scope: &ResolverScope,
) -> bool {
    let Some(project_path) = scope.project_path() else {
        return true;
    };
    checkpoint.files_touched.iter().any(|path| {
        let trimmed = path.trim();
        !trimmed.is_empty()
            && (trimmed == project_path || trimmed.starts_with(&format!("{project_path}/")))
    })
}

fn committed_checkpoint_matches_agent(
    checkpoint: &crate::host::checkpoints::strategy::manual_commit::CommittedInfo,
    agent_filter: Option<&str>,
) -> bool {
    let Some(agent_filter) = agent_filter
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return true;
    };
    let canonical_filter = canonical_agent_key(agent_filter);
    let matches = |candidate: &str| {
        let trimmed = candidate.trim();
        if trimmed.is_empty() {
            return false;
        }
        if trimmed.eq_ignore_ascii_case(agent_filter) {
            return true;
        }
        !canonical_filter.is_empty() && canonical_agent_key(trimmed) == canonical_filter
    };

    checkpoint.agents.iter().any(|agent| matches(agent)) || matches(&checkpoint.agent)
}

fn committed_checkpoint_matches_since(
    checkpoint: &Checkpoint,
    since: Option<&DateTimeScalar>,
) -> bool {
    since
        .map(|since| since <= &checkpoint.event_time)
        .unwrap_or(true)
}

pub(super) fn is_missing_sqlite_store_error(err: &anyhow::Error) -> bool {
    format!("{err:#}").contains("SQLite database file not found")
}

use super::DevqlGraphqlContext;
use crate::adapters::agents::canonical_agent_key;
use crate::graphql::ResolverScope;
use crate::graphql::types::{
    ArtefactCopyLineage, Checkpoint, CheckpointFileRelation, DateTimeScalar,
};
use crate::host::checkpoints::strategy::manual_commit::{list_committed, read_committed_info};
use crate::host::devql::resolve_repo_identity;
use crate::host::relational_store::{DefaultRelationalStore, RelationalStore};
use anyhow::{Context, Result};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use tokio::task;

impl DevqlGraphqlContext {
    pub(crate) async fn list_selected_symbol_checkpoints(
        &self,
        scope: &ResolverScope,
        symbol_ids: &[String],
        agent: Option<&str>,
        since: Option<&DateTimeScalar>,
    ) -> Result<Vec<Checkpoint>> {
        if symbol_ids.is_empty() {
            return Ok(Vec::new());
        }

        let repo_id = self.repo_id_for_scope(scope)?;
        let repo_root = self.repo_root_for_scope(scope)?;
        let relational_store =
            crate::host::relational_store::DefaultRelationalStore::open_local_for_repo_root(
                &repo_root,
            )?;
        let sqlite_path = relational_store.sqlite_path().to_path_buf();
        if !sqlite_path.is_file() {
            return Ok(Vec::new());
        }
        relational_store
            .initialise_local_relational_checkpoint_schema()
            .context("initialising relational checkpoint schema for selected symbol checkpoints")?;

        let relational = relational_store.to_local_inner();
        let matches =
            crate::host::devql::checkpoint_provenance::CheckpointFileGateway::new(&relational)
                .list_checkpoint_ids_for_symbol_ids(
                    &repo_id,
                    symbol_ids,
                    crate::host::devql::checkpoint_provenance::CheckpointFileActivityFilter {
                        agent,
                        since: since.map(DateTimeScalar::as_str),
                    },
                )
                .await?;
        if matches.is_empty() {
            return Ok(Vec::new());
        }

        let checkpoint_commits = read_latest_checkpoint_commit_mappings(repo_root.as_path())
            .unwrap_or_else(|_| HashMap::new());
        let mut checkpoints = Vec::new();
        for checkpoint_match in matches {
            let Some(info) =
                read_committed_info(repo_root.as_path(), &checkpoint_match.checkpoint_id)?
            else {
                continue;
            };
            let checkpoint = Checkpoint::from_ingested(
                &info,
                checkpoint_commits
                    .get(&checkpoint_match.checkpoint_id)
                    .map(String::as_str),
            )
            .with_scope(scope.clone());
            checkpoints.push(checkpoint);
        }
        Ok(checkpoints)
    }

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
                )
                .with_scope(scope.clone());
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
        let scope = scope.clone();
        let commit_sha = commit_sha.to_string();
        let relational = DefaultRelationalStore::open_local_for_repo_root(repo_root.as_path())
            .context("opening relational store for commit checkpoints")?;
        let sqlite_path = relational.sqlite_path().to_path_buf();

        task::spawn_blocking(move || -> Result<Vec<Checkpoint>> {
            if !sqlite_path.is_file() {
                return Ok(Vec::new());
            }
            let relational = DefaultRelationalStore::local_only(sqlite_path);
            relational
                .initialise_local_relational_checkpoint_schema()
                .context(
                    "initialising relational checkpoint schema for GraphQL commit checkpoints",
                )?;
            let sqlite = RelationalStore::local_sqlite_pool(&relational)
                .context("opening checkpoint SQLite store")?;
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
                    checkpoints.push(
                        Checkpoint::from_committed(&commit_sha, &info).with_scope(scope.clone()),
                    );
                }
            }
            Ok(checkpoints)
        })
        .await
        .context("joining commit checkpoint query task")?
    }
}

impl DevqlGraphqlContext {
    pub(crate) async fn list_checkpoint_file_relations(
        &self,
        checkpoint_id: &str,
        scope: &ResolverScope,
    ) -> Result<Vec<CheckpointFileRelation>> {
        let repo_id = self.repo_id_for_scope(scope)?;
        let repo_root = self.repo_root_for_scope(scope)?;
        let relational_store =
            crate::host::relational_store::DefaultRelationalStore::open_local_for_repo_root(
                &repo_root,
            )?;
        let sqlite_path = relational_store.sqlite_path().to_path_buf();
        if !sqlite_path.is_file() {
            return Ok(Vec::new());
        }
        let relational = relational_store.to_local_inner();
        let rows =
            crate::host::devql::checkpoint_provenance::CheckpointFileGateway::new(&relational)
                .list_checkpoint_files(&repo_id, checkpoint_id)
                .await?;
        Ok(rows
            .into_iter()
            .map(|row| CheckpointFileRelation {
                filepath: crate::host::devql::checkpoint_provenance::checkpoint_display_path(
                    row.path_before.as_deref(),
                    row.path_after.as_deref(),
                ),
                change_kind: row.change_kind.as_str().to_string(),
                path_before: row.path_before,
                path_after: row.path_after,
                blob_sha_before: row.blob_sha_before,
                blob_sha_after: row.blob_sha_after,
                copied_from_path: row.copy_source_path,
                copied_from_blob_sha: row.copy_source_blob_sha,
            })
            .collect())
    }

    pub(crate) async fn list_artefact_copy_lineage(
        &self,
        artefact_id: &str,
        scope: &ResolverScope,
    ) -> Result<Vec<ArtefactCopyLineage>> {
        let repo_id = self.repo_id_for_scope(scope)?;
        let repo_root = self.repo_root_for_scope(scope)?;
        let relational_store =
            crate::host::relational_store::DefaultRelationalStore::open_local_for_repo_root(
                &repo_root,
            )?;
        let sqlite_path = relational_store.sqlite_path().to_path_buf();
        if !sqlite_path.is_file() {
            return Ok(Vec::new());
        }
        let relational = relational_store.to_local_inner();
        let rows =
            crate::host::devql::checkpoint_provenance::CheckpointFileGateway::new(&relational)
                .list_artefact_copy_lineage(&repo_id, artefact_id, 100)
                .await?;
        Ok(rows
            .into_iter()
            .map(|row| ArtefactCopyLineage {
                checkpoint_id: row.checkpoint_id,
                event_time: DateTimeScalar::from_rfc3339(row.event_time)
                    .or_else(|_| DateTimeScalar::from_rfc3339("1970-01-01T00:00:00+00:00"))
                    .expect("static epoch timestamp must parse"),
                commit_sha: row.commit_sha,
                source_symbol_id: row.source_symbol_id,
                source_artefact_id: row.source_artefact_id.into(),
                dest_symbol_id: row.dest_symbol_id,
                dest_artefact_id: row.dest_artefact_id.into(),
                scope: scope.clone(),
            })
            .collect())
    }
}

pub(super) fn read_commit_checkpoint_mappings_all(
    repo_root: &Path,
) -> Result<BTreeMap<String, Vec<String>>> {
    let relational =
        crate::host::relational_store::DefaultRelationalStore::open_local_for_repo_root(repo_root)
            .context("opening relational store for commit-checkpoint mappings")?;
    relational
        .initialise_local_relational_checkpoint_schema()
        .context("initialising relational checkpoint schema for commit-checkpoint mappings")?;
    let sqlite = crate::host::relational_store::RelationalStore::local_sqlite_pool(&relational)
        .context("opening SQLite database for commit-checkpoint mappings")?;
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
    let relational = DefaultRelationalStore::open_local_for_repo_root(repo_root)
        .context("opening relational store for latest checkpoint commit mappings")?;
    relational
        .initialise_local_relational_checkpoint_schema()
        .context(
            "initialising relational checkpoint schema for latest checkpoint commit mappings",
        )?;
    let sqlite = RelationalStore::local_sqlite_pool(&relational)
        .context("opening SQLite database for latest checkpoint commit mappings")?;
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

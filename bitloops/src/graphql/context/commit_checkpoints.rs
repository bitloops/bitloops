use super::DevqlGraphqlContext;
use crate::adapters::agents::canonical_agent_key;
use crate::graphql::ResolverScope;
use crate::graphql::types::{
    ArtefactCopyLineage, Checkpoint, CheckpointFileRelation, DateTimeScalar,
};
use crate::host::checkpoints::strategy::manual_commit::{
    checkpoint_row_text, list_committed, open_checkpoint_relational_store,
    query_checkpoint_metadata_rows, read_committed_info,
};
use crate::host::relational_store::DefaultRelationalStore;
use anyhow::{Context, Result};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use tokio::task;

impl DevqlGraphqlContext {
    pub(crate) async fn list_selected_checkpoints(
        &self,
        scope: &ResolverScope,
        symbol_ids: &[String],
        paths: &[String],
        agent: Option<&str>,
        since: Option<&DateTimeScalar>,
    ) -> Result<Vec<Checkpoint>> {
        if symbol_ids.is_empty() && paths.is_empty() {
            return Ok(Vec::new());
        }

        let repo_id = self.repo_id_for_scope(scope)?;
        let repo_root = self.repo_root_for_scope(scope)?;
        let relational_store =
            crate::host::relational_store::DefaultRelationalStore::open_primary_for_repo_root_preferring_bound_config(
                &repo_root,
            )?;
        if relational_store
            .backend_for_role(crate::host::devql::RelationalStorageRole::SharedRelational)
            == crate::host::devql::RelationalRoleBackend::LocalSqlite
            && !relational_store.sqlite_path().is_file()
        {
            return Ok(Vec::new());
        }
        let relational = relational_store.into_inner();
        let matches =
            crate::host::devql::checkpoint_provenance::CheckpointFileGateway::new(&relational)
                .list_checkpoint_ids_for_selection(
                    &repo_id,
                    symbol_ids,
                    paths,
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
        let relational =
            DefaultRelationalStore::open_primary_for_repo_root_preferring_bound_config(
                repo_root.as_path(),
            )
            .context("opening relational store for commit checkpoints")?;
        if relational.backend_for_role(crate::host::devql::RelationalStorageRole::SharedRelational)
            == crate::host::devql::RelationalRoleBackend::LocalSqlite
            && !relational.sqlite_path().is_file()
        {
            return Ok(Vec::new());
        }

        task::spawn_blocking(move || -> Result<Vec<Checkpoint>> {
            let checkpoint_ids_sql = format!(
                "SELECT checkpoint_id
                 FROM commit_checkpoints
                 WHERE repo_id = '{}' AND commit_sha = '{}'
                 ORDER BY created_at DESC, checkpoint_id DESC",
                crate::host::devql::esc_pg(repo_id.as_str()),
                crate::host::devql::esc_pg(commit_sha.as_str()),
            );
            let checkpoint_ids = query_checkpoint_metadata_rows(&relational, &checkpoint_ids_sql)?
                .into_iter()
                .filter_map(|row| checkpoint_row_text(&row, "checkpoint_id"))
                .collect::<Vec<_>>();

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
            crate::host::relational_store::DefaultRelationalStore::open_primary_for_repo_root_preferring_bound_config(
                &repo_root,
            )?;
        if relational_store
            .backend_for_role(crate::host::devql::RelationalStorageRole::SharedRelational)
            == crate::host::devql::RelationalRoleBackend::LocalSqlite
            && !relational_store.sqlite_path().is_file()
        {
            return Ok(Vec::new());
        }
        let relational = relational_store.into_inner();
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
            crate::host::relational_store::DefaultRelationalStore::open_primary_for_repo_root_preferring_bound_config(
                &repo_root,
            )?;
        if relational_store
            .backend_for_role(crate::host::devql::RelationalStorageRole::SharedRelational)
            == crate::host::devql::RelationalRoleBackend::LocalSqlite
            && !relational_store.sqlite_path().is_file()
        {
            return Ok(Vec::new());
        }
        let relational = relational_store.into_inner();
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
    let (relational, repo_id) = open_checkpoint_relational_store(repo_root)
        .context("opening relational store for commit-checkpoint mappings")?;
    let sql = format!(
        "SELECT commit_sha, checkpoint_id
         FROM commit_checkpoints
         WHERE repo_id = '{}'
         ORDER BY created_at DESC, checkpoint_id DESC",
        crate::host::devql::esc_pg(repo_id.as_str()),
    );
    let mut out = BTreeMap::<String, Vec<String>>::new();
    for row in query_checkpoint_metadata_rows(&relational, &sql)? {
        let Some(commit_sha) = checkpoint_row_text(&row, "commit_sha") else {
            continue;
        };
        let Some(checkpoint_id) = checkpoint_row_text(&row, "checkpoint_id") else {
            continue;
        };
        out.entry(commit_sha).or_default().push(checkpoint_id);
    }
    Ok(out)
}

fn read_latest_checkpoint_commit_mappings(repo_root: &Path) -> Result<HashMap<String, String>> {
    let (relational, repo_id) = open_checkpoint_relational_store(repo_root)
        .context("opening relational store for latest checkpoint commit mappings")?;
    let sql = format!(
        "SELECT checkpoint_id, commit_sha
         FROM commit_checkpoints
         WHERE repo_id = '{}'
         ORDER BY created_at DESC, checkpoint_id DESC",
        crate::host::devql::esc_pg(repo_id.as_str()),
    );
    let mut out = HashMap::<String, String>::new();
    for row in query_checkpoint_metadata_rows(&relational, &sql)? {
        let Some(checkpoint_id) = checkpoint_row_text(&row, "checkpoint_id") else {
            continue;
        };
        let Some(commit_sha) = checkpoint_row_text(&row, "commit_sha") else {
            continue;
        };
        out.entry(checkpoint_id).or_insert(commit_sha);
    }
    Ok(out)
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

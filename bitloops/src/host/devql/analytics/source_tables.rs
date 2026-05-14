use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result};
use serde_json::{Value, json};

use super::super::{
    DevqlConfig, RelationalStorage, clickhouse_query_data, duckdb_query_rows_path, esc_pg,
    sqlite_value_to_json,
};
use super::row_access::{ignore_missing_table, row_string, set_row_string, sql_in_list};
use super::specs::{
    CACHE_INTERACTION_EVENTS_SPEC, CACHE_INTERACTION_SESSIONS_SPEC, CACHE_INTERACTION_TURNS_SPEC,
    CACHE_REPO_SYNC_STATE_SPEC, CACHE_REPOSITORIES_SPEC,
};
use super::types::{AnalyticsRepository, AnalyticsSourceTables};
use crate::config::StoreBackendConfig;
use crate::host::interactions::db_store::SqliteInteractionSpool;
use crate::host::runtime_store::RepoSqliteRuntimeStore;

pub(super) async fn load_source_tables(
    cfg: &DevqlConfig,
    backends: &StoreBackendConfig,
    relational: &RelationalStorage,
    repositories: &[AnalyticsRepository],
) -> Result<AnalyticsSourceTables> {
    let repo_ids = repositories
        .iter()
        .map(|repository| repository.repo_id.clone())
        .collect::<Vec<_>>();

    let local_repositories = relational
        .query_rows(&local_repositories_sql(&repo_ids))
        .await
        .or_else(ignore_missing_table)?;
    let local_repo_sync_state = relational
        .query_rows(&local_repo_sync_state_sql(&repo_ids))
        .await
        .or_else(ignore_missing_table)?;
    let local_current_file_state = relational
        .query_rows(&local_current_file_state_sql(&repo_ids))
        .await
        .or_else(ignore_missing_table)?;

    let remote_repositories = if backends.relational.has_postgres() {
        relational
            .query_rows_remote(&remote_repositories_sql(&repo_ids))
            .await
            .or_else(ignore_missing_table)?
    } else {
        Vec::new()
    };
    let remote_repo_sync_state = if backends.relational.has_postgres() {
        relational
            .query_rows_remote(&remote_repo_sync_state_sql(&repo_ids))
            .await
            .or_else(ignore_missing_table)?
    } else {
        Vec::new()
    };
    let (repositories_rows, repo_sync_state_rows, current_file_state_rows) =
        merge_relational_source_rows(
            repositories,
            remote_repositories,
            local_repositories,
            remote_repo_sync_state,
            local_repo_sync_state,
            local_current_file_state,
        );

    let (interaction_sessions, interaction_turns, interaction_events) =
        if backends.events.has_clickhouse() {
            (
                clickhouse_rows(cfg, &clickhouse_interaction_sessions_sql(&repo_ids))
                    .await
                    .or_else(ignore_missing_table)?,
                clickhouse_rows(cfg, &clickhouse_interaction_turns_sql(&repo_ids))
                    .await
                    .or_else(ignore_missing_table)?,
                clickhouse_rows(cfg, &clickhouse_interaction_events_sql(&repo_ids))
                    .await
                    .or_else(ignore_missing_table)?,
            )
        } else {
            let duckdb_path = backends
                .events
                .resolve_duckdb_db_path_for_repo(&cfg.repo_root);
            (
                duckdb_query_rows_path(&duckdb_path, &local_interaction_sessions_sql(&repo_ids))
                    .await
                    .or_else(ignore_missing_table)?,
                duckdb_query_rows_path(&duckdb_path, &local_interaction_turns_sql(&repo_ids))
                    .await
                    .or_else(ignore_missing_table)?,
                duckdb_query_rows_path(&duckdb_path, &local_interaction_events_sql(&repo_ids))
                    .await
                    .or_else(ignore_missing_table)?,
            )
        };
    let runtime_overlay = load_runtime_interaction_overlay(cfg, repositories);
    let interaction_sessions = merge_rows(
        interaction_sessions,
        runtime_overlay.interaction_sessions,
        CACHE_INTERACTION_SESSIONS_SPEC.key_columns,
    );
    let interaction_turns = merge_rows(
        interaction_turns,
        runtime_overlay.interaction_turns,
        CACHE_INTERACTION_TURNS_SPEC.key_columns,
    );
    let interaction_events = merge_rows(
        interaction_events,
        runtime_overlay.interaction_events,
        CACHE_INTERACTION_EVENTS_SPEC.key_columns,
    );

    Ok(AnalyticsSourceTables {
        repositories: repositories_rows,
        repo_sync_state: repo_sync_state_rows,
        current_file_state: current_file_state_rows,
        interaction_sessions,
        interaction_turns,
        interaction_events,
    })
}

#[derive(Default)]
struct RuntimeInteractionOverlay {
    interaction_sessions: Vec<Value>,
    interaction_turns: Vec<Value>,
    interaction_events: Vec<Value>,
}

pub(super) fn load_runtime_interaction_watermarks(
    cfg: &DevqlConfig,
    repositories: &[AnalyticsRepository],
) -> BTreeMap<String, String> {
    let mut watermarks = BTreeMap::new();
    for repository in repositories {
        let Some(spool) = open_runtime_spool(cfg, repository) else {
            continue;
        };
        let repo_ids = vec![repository.repo_id.clone()];
        let rows = query_runtime_spool_rows(&spool, &runtime_interaction_watermark_sql(&repo_ids))
            .unwrap_or_default();
        let watermark = rows
            .first()
            .map(|row| row_string(row, "watermark"))
            .unwrap_or_default();
        if !watermark.is_empty() {
            watermarks.insert(repository.repo_id.clone(), watermark);
        }
    }
    watermarks
}

pub(super) fn ensure_repository_catalog_rows(
    rows: &mut Vec<Value>,
    repositories: &[AnalyticsRepository],
) {
    let mut existing = rows
        .iter()
        .filter_map(|row| row.get("repo_id").and_then(Value::as_str))
        .map(str::to_string)
        .collect::<BTreeSet<_>>();

    for repository in repositories {
        if existing.contains(&repository.repo_id) {
            for row in rows.iter_mut() {
                if row_string(row, "repo_id") == repository.repo_id {
                    let repo_root = row_string(row, "repo_root");
                    if repo_root.is_empty() {
                        set_row_string(
                            row,
                            "repo_root",
                            repository
                                .repo_root
                                .as_ref()
                                .map(|path| path.to_string_lossy().to_string())
                                .unwrap_or_default(),
                        );
                    }
                    if row_string(row, "identity").is_empty() {
                        set_row_string(row, "identity", repository.identity.clone());
                    }
                }
            }
            continue;
        }
        existing.insert(repository.repo_id.clone());
        rows.push(json!({
            "repo_id": repository.repo_id,
            "repo_root": repository.repo_root.as_ref().map(|path| path.to_string_lossy().to_string()).unwrap_or_default(),
            "provider": repository.provider,
            "organization": repository.organization,
            "name": repository.name,
            "identity": repository.identity,
            "default_branch": repository.default_branch.clone().unwrap_or_default(),
            "metadata_json": "",
            "created_at": "",
        }));
    }
}

fn merge_relational_source_rows(
    repositories: &[AnalyticsRepository],
    remote_repositories: Vec<Value>,
    local_repositories: Vec<Value>,
    remote_repo_sync_state: Vec<Value>,
    local_repo_sync_state: Vec<Value>,
    local_current_file_state: Vec<Value>,
) -> (Vec<Value>, Vec<Value>, Vec<Value>) {
    let mut repositories_rows = merge_rows(
        remote_repositories,
        local_repositories,
        CACHE_REPOSITORIES_SPEC.key_columns,
    );
    ensure_repository_catalog_rows(&mut repositories_rows, repositories);
    let repo_sync_state_rows = merge_rows(
        remote_repo_sync_state,
        local_repo_sync_state,
        CACHE_REPO_SYNC_STATE_SPEC.key_columns,
    );

    // current_file_state is the local current projection authority.
    (
        repositories_rows,
        repo_sync_state_rows,
        local_current_file_state,
    )
}

fn merge_rows(base: Vec<Value>, overlay: Vec<Value>, key_columns: &[&str]) -> Vec<Value> {
    let mut merged = BTreeMap::<String, Value>::new();
    for row in base {
        let key = row_key(&row, key_columns);
        if !key.is_empty() {
            merged.insert(key, row);
        }
    }
    for row in overlay {
        let key = row_key(&row, key_columns);
        if !key.is_empty() {
            merged.insert(key, row);
        }
    }
    merged.into_values().collect()
}

fn row_key(row: &Value, columns: &[&str]) -> String {
    columns
        .iter()
        .map(|column| row_string(row, column))
        .collect::<Vec<_>>()
        .join("\u{1f}")
}

fn local_repositories_sql(repo_ids: &[String]) -> String {
    format!(
        "SELECT r.repo_id, \
                COALESCE(s.repo_root, '') AS repo_root, \
                COALESCE(r.provider, '') AS provider, \
                COALESCE(r.organization, '') AS organization, \
                COALESCE(r.name, '') AS name, \
                (COALESCE(r.provider, '') || '://' || COALESCE(r.organization, '') || '/' || COALESCE(r.name, '')) AS identity, \
                COALESCE(r.default_branch, '') AS default_branch, \
                COALESCE(r.metadata_json, '') AS metadata_json, \
                COALESCE(r.created_at, '') AS created_at \
         FROM repositories AS r \
         LEFT JOIN repo_sync_state AS s ON s.repo_id = r.repo_id \
         WHERE r.repo_id IN ({})",
        sql_in_list(repo_ids, esc_pg)
    )
}

fn load_runtime_interaction_overlay(
    cfg: &DevqlConfig,
    repositories: &[AnalyticsRepository],
) -> RuntimeInteractionOverlay {
    let mut overlay = RuntimeInteractionOverlay::default();
    for repository in repositories {
        let Some(spool) = open_runtime_spool(cfg, repository) else {
            continue;
        };
        let repo_ids = vec![repository.repo_id.clone()];
        overlay.interaction_sessions.extend(
            query_runtime_spool_rows(&spool, &local_interaction_sessions_sql(&repo_ids))
                .unwrap_or_default(),
        );
        overlay.interaction_turns.extend(
            query_runtime_spool_rows(&spool, &local_interaction_turns_sql(&repo_ids))
                .unwrap_or_default(),
        );
        overlay.interaction_events.extend(
            query_runtime_spool_rows(&spool, &local_interaction_events_sql(&repo_ids))
                .unwrap_or_default(),
        );
    }
    overlay
}

fn open_runtime_spool(
    cfg: &DevqlConfig,
    repository: &AnalyticsRepository,
) -> Option<SqliteInteractionSpool> {
    let repo_root = repository_runtime_root(cfg, repository)?;
    let store = RepoSqliteRuntimeStore::open_for_roots(&cfg.daemon_config_root, repo_root).ok()?;
    store.interaction_spool().ok()
}

fn repository_runtime_root<'a>(
    cfg: &'a DevqlConfig,
    repository: &'a AnalyticsRepository,
) -> Option<&'a std::path::Path> {
    repository.repo_root.as_deref().or_else(|| {
        if repository.repo_id == cfg.repo.repo_id {
            Some(cfg.repo_root.as_path())
        } else {
            None
        }
    })
}

fn query_runtime_spool_rows(spool: &SqliteInteractionSpool, sql: &str) -> Result<Vec<Value>> {
    spool.with_connection(|conn| sqlite_query_rows(conn, sql))
}

fn sqlite_query_rows(conn: &rusqlite::Connection, sql: &str) -> Result<Vec<Value>> {
    let mut stmt = conn
        .prepare(sql)
        .context("preparing runtime interaction spool query")?;
    let column_names = stmt
        .column_names()
        .iter()
        .map(|name| (*name).to_string())
        .collect::<Vec<_>>();
    let mut rows = stmt
        .query([])
        .context("executing runtime interaction spool query")?;
    let mut out = Vec::new();

    while let Some(row) = rows
        .next()
        .context("iterating runtime interaction spool rows")?
    {
        let mut object = serde_json::Map::new();
        for (index, column_name) in column_names.iter().enumerate() {
            let value = row.get_ref(index).with_context(|| {
                format!(
                    "reading runtime interaction spool value for column index {index} (`{column_name}`)"
                )
            })?;
            object.insert(column_name.clone(), sqlite_value_to_json(value));
        }
        out.push(Value::Object(object));
    }

    Ok(out)
}

fn runtime_interaction_watermark_sql(repo_ids: &[String]) -> String {
    format!(
        "SELECT COALESCE(MAX(changed_at), '') AS watermark
         FROM (
            SELECT COALESCE(NULLIF(updated_at, ''), NULLIF(last_event_at, ''), NULLIF(started_at, ''), '') AS changed_at
            FROM interaction_sessions
            WHERE repo_id IN ({repo_ids})
            UNION ALL
            SELECT COALESCE(NULLIF(updated_at, ''), NULLIF(ended_at, ''), NULLIF(started_at, ''), '') AS changed_at
            FROM interaction_turns
            WHERE repo_id IN ({repo_ids})
            UNION ALL
            SELECT COALESCE(event_time, '') AS changed_at
            FROM interaction_events
            WHERE repo_id IN ({repo_ids})
         ) AS runtime_changes
         WHERE changed_at <> ''",
        repo_ids = sql_in_list(repo_ids, esc_pg),
    )
}

fn remote_repositories_sql(repo_ids: &[String]) -> String {
    format!(
        "SELECT r.repo_id, \
                COALESCE(s.repo_root, '') AS repo_root, \
                COALESCE(r.provider, '') AS provider, \
                COALESCE(r.organization, '') AS organization, \
                COALESCE(r.name, '') AS name, \
                (COALESCE(r.provider, '') || '://' || COALESCE(r.organization, '') || '/' || COALESCE(r.name, '')) AS identity, \
                COALESCE(r.default_branch, '') AS default_branch, \
                COALESCE(r.metadata_json::text, '') AS metadata_json, \
                COALESCE(r.created_at::text, '') AS created_at \
         FROM repositories AS r \
         LEFT JOIN repo_sync_state AS s ON s.repo_id = r.repo_id \
         WHERE r.repo_id IN ({})",
        sql_in_list(repo_ids, esc_pg)
    )
}

fn local_repo_sync_state_sql(repo_ids: &[String]) -> String {
    format!(
        "SELECT repo_id, COALESCE(repo_root, '') AS repo_root, COALESCE(active_branch, '') AS active_branch, \
                COALESCE(head_commit_sha, '') AS head_commit_sha, COALESCE(head_tree_sha, '') AS head_tree_sha, \
                COALESCE(parser_version, '') AS parser_version, COALESCE(extractor_version, '') AS extractor_version, \
                COALESCE(scope_exclusions_fingerprint, '') AS scope_exclusions_fingerprint, \
                COALESCE(last_sync_started_at, '') AS last_sync_started_at, \
                COALESCE(last_sync_completed_at, '') AS last_sync_completed_at, \
                COALESCE(last_sync_status, '') AS last_sync_status, COALESCE(last_sync_reason, '') AS last_sync_reason \
         FROM repo_sync_state WHERE repo_id IN ({})",
        sql_in_list(repo_ids, esc_pg)
    )
}

fn remote_repo_sync_state_sql(repo_ids: &[String]) -> String {
    local_repo_sync_state_sql(repo_ids)
}

fn local_current_file_state_sql(repo_ids: &[String]) -> String {
    format!(
        "SELECT repo_id, path, COALESCE(analysis_mode, '') AS analysis_mode, COALESCE(file_role, '') AS file_role, \
                COALESCE(text_index_mode, '') AS text_index_mode, COALESCE(language, '') AS language, \
                COALESCE(resolved_language, '') AS resolved_language, COALESCE(dialect, '') AS dialect, \
                COALESCE(primary_context_id, '') AS primary_context_id, \
                COALESCE(secondary_context_ids_json, '[]') AS secondary_context_ids_json, \
                COALESCE(frameworks_json, '[]') AS frameworks_json, \
                COALESCE(runtime_profile, '') AS runtime_profile, COALESCE(classification_reason, '') AS classification_reason, \
                COALESCE(context_fingerprint, '') AS context_fingerprint, COALESCE(extraction_fingerprint, '') AS extraction_fingerprint, \
                COALESCE(head_content_id, '') AS head_content_id, COALESCE(index_content_id, '') AS index_content_id, \
                COALESCE(worktree_content_id, '') AS worktree_content_id, COALESCE(effective_content_id, '') AS effective_content_id, \
                COALESCE(effective_source, '') AS effective_source, COALESCE(parser_version, '') AS parser_version, \
                COALESCE(extractor_version, '') AS extractor_version, exists_in_head, exists_in_index, exists_in_worktree, \
                COALESCE(last_synced_at, '') AS last_synced_at \
         FROM current_file_state WHERE repo_id IN ({})",
        sql_in_list(repo_ids, esc_pg)
    )
}

fn local_interaction_sessions_sql(repo_ids: &[String]) -> String {
    format!(
        "SELECT session_id, repo_id, COALESCE(branch, '') AS branch, COALESCE(actor_id, '') AS actor_id, \
                COALESCE(actor_name, '') AS actor_name, COALESCE(actor_email, '') AS actor_email, \
                COALESCE(actor_source, '') AS actor_source, COALESCE(agent_type, '') AS agent_type, \
                COALESCE(model, '') AS model, COALESCE(first_prompt, '') AS first_prompt, \
                COALESCE(transcript_path, '') AS transcript_path, COALESCE(worktree_path, '') AS worktree_path, \
                COALESCE(worktree_id, '') AS worktree_id, COALESCE(started_at, '') AS started_at, \
                COALESCE(ended_at, '') AS ended_at, COALESCE(last_event_at, '') AS last_event_at, \
                COALESCE(updated_at, '') AS updated_at \
         FROM interaction_sessions WHERE repo_id IN ({})",
        sql_in_list(repo_ids, esc_pg)
    )
}

fn clickhouse_interaction_sessions_sql(repo_ids: &[String]) -> String {
    local_interaction_sessions_sql(repo_ids)
}

fn local_interaction_turns_sql(repo_ids: &[String]) -> String {
    format!(
        "SELECT turn_id, session_id, repo_id, COALESCE(branch, '') AS branch, COALESCE(actor_id, '') AS actor_id, \
                COALESCE(actor_name, '') AS actor_name, COALESCE(actor_email, '') AS actor_email, \
                COALESCE(actor_source, '') AS actor_source, turn_number, COALESCE(prompt, '') AS prompt, \
                COALESCE(agent_type, '') AS agent_type, COALESCE(model, '') AS model, COALESCE(started_at, '') AS started_at, \
                COALESCE(ended_at, '') AS ended_at, has_token_usage, input_tokens, cache_creation_tokens, \
                cache_read_tokens, output_tokens, api_call_count, COALESCE(summary, '') AS summary, prompt_count, \
                transcript_offset_start, transcript_offset_end, COALESCE(transcript_fragment, '') AS transcript_fragment, \
                COALESCE(files_modified, '[]') AS files_modified, COALESCE(checkpoint_id, '') AS checkpoint_id, \
                COALESCE(updated_at, '') AS updated_at \
         FROM interaction_turns WHERE repo_id IN ({})",
        sql_in_list(repo_ids, esc_pg)
    )
}

fn clickhouse_interaction_turns_sql(repo_ids: &[String]) -> String {
    local_interaction_turns_sql(repo_ids)
}

fn local_interaction_events_sql(repo_ids: &[String]) -> String {
    format!(
        "SELECT event_id, COALESCE(event_time, '') AS event_time, repo_id, session_id, COALESCE(turn_id, '') AS turn_id, \
                COALESCE(branch, '') AS branch, COALESCE(actor_id, '') AS actor_id, COALESCE(actor_name, '') AS actor_name, \
                COALESCE(actor_email, '') AS actor_email, COALESCE(actor_source, '') AS actor_source, \
                COALESCE(event_type, '') AS event_type, COALESCE(source, '') AS source, sequence_number, \
                COALESCE(agent_type, '') AS agent_type, COALESCE(model, '') AS model, COALESCE(tool_use_id, '') AS tool_use_id, \
                COALESCE(tool_kind, '') AS tool_kind, COALESCE(task_description, '') AS task_description, \
                COALESCE(subagent_id, '') AS subagent_id, COALESCE(payload, '{{}}') AS payload \
         FROM interaction_events WHERE repo_id IN ({})",
        sql_in_list(repo_ids, esc_pg)
    )
}

fn clickhouse_interaction_events_sql(repo_ids: &[String]) -> String {
    local_interaction_events_sql(repo_ids)
}

pub(super) async fn clickhouse_rows(cfg: &DevqlConfig, sql: &str) -> Result<Vec<Value>> {
    let data = clickhouse_query_data(cfg, sql).await?;
    Ok(data.as_array().cloned().unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn merge_relational_source_rows_keeps_shared_rows_but_current_file_state_local() {
        let local_current_file_state = vec![json!({
            "repo_id": "repo-local",
            "path": "src/local.rs",
            "last_synced_at": "2026-04-22T09:06:00Z",
        })];

        let (repositories_rows, repo_sync_state_rows, current_file_state_rows) =
            merge_relational_source_rows(
                &[],
                vec![json!({
                    "repo_id": "repo-remote",
                    "repo_root": "/remote",
                    "provider": "github",
                    "organization": "bitloops",
                    "name": "shared",
                    "identity": "github://bitloops/shared",
                    "default_branch": "main",
                    "metadata_json": "{}",
                    "created_at": "2026-04-22T09:00:00Z",
                })],
                vec![json!({
                    "repo_id": "repo-local",
                    "repo_root": "/local",
                    "provider": "github",
                    "organization": "bitloops",
                    "name": "local",
                    "identity": "github://bitloops/local",
                    "default_branch": "main",
                    "metadata_json": "{}",
                    "created_at": "2026-04-22T09:00:00Z",
                })],
                vec![json!({
                    "repo_id": "repo-remote",
                    "repo_root": "/remote",
                    "active_branch": "main",
                    "head_commit_sha": "remote-head",
                    "head_tree_sha": "remote-tree",
                    "parser_version": "1",
                    "extractor_version": "1",
                    "scope_exclusions_fingerprint": "",
                    "last_sync_started_at": "2026-04-22T09:00:00Z",
                    "last_sync_completed_at": "2026-04-22T09:05:00Z",
                    "last_sync_status": "completed",
                    "last_sync_reason": "",
                })],
                vec![json!({
                    "repo_id": "repo-local",
                    "repo_root": "/local",
                    "active_branch": "main",
                    "head_commit_sha": "local-head",
                    "head_tree_sha": "local-tree",
                    "parser_version": "1",
                    "extractor_version": "1",
                    "scope_exclusions_fingerprint": "",
                    "last_sync_started_at": "2026-04-22T09:00:00Z",
                    "last_sync_completed_at": "2026-04-22T09:06:00Z",
                    "last_sync_status": "completed",
                    "last_sync_reason": "",
                })],
                local_current_file_state.clone(),
            );

        assert_eq!(repositories_rows.len(), 2);
        assert!(
            repositories_rows
                .iter()
                .any(|row| row["repo_id"] == "repo-remote")
        );
        assert!(
            repositories_rows
                .iter()
                .any(|row| row["repo_id"] == "repo-local")
        );

        assert_eq!(repo_sync_state_rows.len(), 2);
        assert!(
            repo_sync_state_rows
                .iter()
                .any(|row| row["repo_id"] == "repo-remote")
        );
        assert!(
            repo_sync_state_rows
                .iter()
                .any(|row| row["repo_id"] == "repo-local")
        );

        assert_eq!(current_file_state_rows, local_current_file_state);
    }
}

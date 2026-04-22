use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use serde_json::{Value, json};

use super::super::{
    DevqlConfig, RelationalStorage, clickhouse_query_data, duckdb_query_rows_path, esc_pg,
};
use super::row_access::{ignore_missing_table, row_string, set_row_string, sql_in_list};
use super::specs::{
    CACHE_CURRENT_FILE_STATE_SPEC, CACHE_REPO_SYNC_STATE_SPEC, CACHE_REPOSITORIES_SPEC,
};
use super::types::{AnalyticsRepository, AnalyticsSourceTables};
use crate::config::StoreBackendConfig;

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
    let remote_current_file_state = if backends.relational.has_postgres() {
        relational
            .query_rows_remote(&remote_current_file_state_sql(&repo_ids))
            .await
            .or_else(ignore_missing_table)?
    } else {
        Vec::new()
    };

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
    let current_file_state_rows = merge_rows(
        remote_current_file_state,
        local_current_file_state,
        CACHE_CURRENT_FILE_STATE_SPEC.key_columns,
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

    Ok(AnalyticsSourceTables {
        repositories: repositories_rows,
        repo_sync_state: repo_sync_state_rows,
        current_file_state: current_file_state_rows,
        interaction_sessions,
        interaction_turns,
        interaction_events,
    })
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

fn remote_current_file_state_sql(repo_ids: &[String]) -> String {
    format!(
        "SELECT repo_id, path, COALESCE(analysis_mode, '') AS analysis_mode, COALESCE(file_role, '') AS file_role, \
                COALESCE(text_index_mode, '') AS text_index_mode, COALESCE(language, '') AS language, \
                COALESCE(resolved_language, '') AS resolved_language, COALESCE(dialect, '') AS dialect, \
                COALESCE(primary_context_id, '') AS primary_context_id, \
                COALESCE(secondary_context_ids_json::text, '[]') AS secondary_context_ids_json, \
                COALESCE(frameworks_json::text, '[]') AS frameworks_json, \
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

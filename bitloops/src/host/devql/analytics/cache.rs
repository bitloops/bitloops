use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{Context, Result};
use chrono::Utc;
use serde_json::Value;
use tokio::sync::Mutex as AsyncMutex;

use super::super::{DevqlConfig, RelationalStorage, duckdb_query_rows_path, esc_ch, esc_pg};
use super::derived_tables::derive_interaction_tables;
use super::row_access::{ignore_missing_table, row_string, sql_in_list};
use super::source_tables::{clickhouse_rows, load_source_tables};
use super::specs::{
    CACHE_CURRENT_FILE_STATE_SPEC, CACHE_INTERACTION_EVENTS_SPEC, CACHE_INTERACTION_SESSIONS_SPEC,
    CACHE_INTERACTION_TURNS_SPEC, CACHE_REPO_SYNC_STATE_SPEC, CACHE_REPOSITORIES_SPEC,
    CACHE_SUBAGENT_RUNS_SPEC, CACHE_TOOL_INVOCATIONS_SPEC,
};
use super::types::{
    AnalyticsDerivedTables, AnalyticsRepository, AnalyticsSourceTables, ColumnKind, RepoWatermark,
    TableSpec,
};
use crate::config::StoreBackendConfig;

pub(super) async fn refresh_analytics_cache(
    cfg: &DevqlConfig,
    backends: &StoreBackendConfig,
    repositories: &[AnalyticsRepository],
) -> Result<()> {
    let cache_path = analytics_cache_path(cfg);
    let refresh_lock = analytics_refresh_lock(&cache_path);
    let _guard = refresh_lock.lock().await;

    ensure_cache_schema(&cache_path).await?;

    let repo_ids = repositories
        .iter()
        .map(|repository| repository.repo_id.clone())
        .collect::<Vec<_>>();
    let relational = RelationalStorage::connect(cfg, &backends.relational, "analytics sql")
        .await
        .context("connecting analytics relational storage")?;
    let current_watermarks =
        collect_source_watermarks(cfg, backends, &relational, &repo_ids).await?;
    let cached_watermarks = load_cached_watermarks(&cache_path, &repo_ids).await?;

    let stale_repo_ids = repo_ids
        .iter()
        .filter(|repo_id| cached_watermarks.get(*repo_id) != current_watermarks.get(*repo_id))
        .cloned()
        .collect::<Vec<_>>();

    if stale_repo_ids.is_empty() {
        return Ok(());
    }

    let stale_lookup = stale_repo_ids.iter().cloned().collect::<BTreeSet<_>>();
    let stale_repositories = repositories
        .iter()
        .filter(|repository| stale_lookup.contains(&repository.repo_id))
        .cloned()
        .collect::<Vec<_>>();

    let source_tables = load_source_tables(cfg, backends, &relational, &stale_repositories).await?;
    let derived_tables = derive_interaction_tables(&source_tables.interaction_events);
    write_cache_snapshot(
        &cache_path,
        &stale_repositories,
        &source_tables,
        &derived_tables,
        &current_watermarks,
    )
    .await
}

pub(super) fn analytics_cache_path(cfg: &DevqlConfig) -> PathBuf {
    cfg.daemon_config_root
        .join("stores")
        .join("analytics")
        .join("analytics.duckdb")
}

fn analytics_refresh_lock(cache_path: &Path) -> Arc<AsyncMutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Arc<AsyncMutex<()>>>>> = OnceLock::new();
    let locks = LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = locks.lock().expect("analytics refresh lock poisoned");
    guard
        .entry(cache_path.to_path_buf())
        .or_insert_with(|| Arc::new(AsyncMutex::new(())))
        .clone()
}

async fn ensure_cache_schema(path: &Path) -> Result<()> {
    let cache_path = path.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<()> {
        if let Some(parent) = cache_path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("creating analytics cache directory {}", parent.display())
            })?;
        }
        let conn = duckdb::Connection::open(&cache_path)
            .with_context(|| format!("opening analytics DuckDB cache {}", cache_path.display()))?;
        conn.execute_batch(CACHE_SCHEMA_SQL)
            .context("creating analytics cache schema")?;
        Ok(())
    })
    .await
    .context("joining analytics cache schema task")?
}

const CACHE_SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS analytics_repo_cache_state (
    repo_id VARCHAR PRIMARY KEY,
    relational_watermark VARCHAR NOT NULL DEFAULT '',
    events_watermark VARCHAR NOT NULL DEFAULT '',
    last_refreshed_at VARCHAR NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS cache_repositories (
    repo_id VARCHAR,
    repo_root VARCHAR,
    provider VARCHAR,
    organization VARCHAR,
    name VARCHAR,
    identity VARCHAR,
    default_branch VARCHAR,
    metadata_json VARCHAR,
    created_at VARCHAR,
    PRIMARY KEY (repo_id)
);

CREATE TABLE IF NOT EXISTS cache_repo_sync_state (
    repo_id VARCHAR,
    repo_root VARCHAR,
    active_branch VARCHAR,
    head_commit_sha VARCHAR,
    head_tree_sha VARCHAR,
    parser_version VARCHAR,
    extractor_version VARCHAR,
    scope_exclusions_fingerprint VARCHAR,
    last_sync_started_at VARCHAR,
    last_sync_completed_at VARCHAR,
    last_sync_status VARCHAR,
    last_sync_reason VARCHAR,
    PRIMARY KEY (repo_id)
);

CREATE TABLE IF NOT EXISTS cache_current_file_state (
    repo_id VARCHAR,
    path VARCHAR,
    analysis_mode VARCHAR,
    file_role VARCHAR,
    text_index_mode VARCHAR,
    language VARCHAR,
    resolved_language VARCHAR,
    dialect VARCHAR,
    primary_context_id VARCHAR,
    secondary_context_ids_json VARCHAR,
    frameworks_json VARCHAR,
    runtime_profile VARCHAR,
    classification_reason VARCHAR,
    context_fingerprint VARCHAR,
    extraction_fingerprint VARCHAR,
    head_content_id VARCHAR,
    index_content_id VARCHAR,
    worktree_content_id VARCHAR,
    effective_content_id VARCHAR,
    effective_source VARCHAR,
    parser_version VARCHAR,
    extractor_version VARCHAR,
    exists_in_head BIGINT,
    exists_in_index BIGINT,
    exists_in_worktree BIGINT,
    last_synced_at VARCHAR,
    PRIMARY KEY (repo_id, path)
);

CREATE TABLE IF NOT EXISTS cache_interaction_sessions (
    session_id VARCHAR,
    repo_id VARCHAR,
    branch VARCHAR,
    actor_id VARCHAR,
    actor_name VARCHAR,
    actor_email VARCHAR,
    actor_source VARCHAR,
    agent_type VARCHAR,
    model VARCHAR,
    first_prompt VARCHAR,
    transcript_path VARCHAR,
    worktree_path VARCHAR,
    worktree_id VARCHAR,
    started_at VARCHAR,
    ended_at VARCHAR,
    last_event_at VARCHAR,
    updated_at VARCHAR,
    PRIMARY KEY (repo_id, session_id)
);

CREATE TABLE IF NOT EXISTS cache_interaction_turns (
    turn_id VARCHAR,
    session_id VARCHAR,
    repo_id VARCHAR,
    branch VARCHAR,
    actor_id VARCHAR,
    actor_name VARCHAR,
    actor_email VARCHAR,
    actor_source VARCHAR,
    turn_number BIGINT,
    prompt VARCHAR,
    agent_type VARCHAR,
    model VARCHAR,
    started_at VARCHAR,
    ended_at VARCHAR,
    has_token_usage BIGINT,
    input_tokens BIGINT,
    cache_creation_tokens BIGINT,
    cache_read_tokens BIGINT,
    output_tokens BIGINT,
    api_call_count BIGINT,
    summary VARCHAR,
    prompt_count BIGINT,
    transcript_offset_start BIGINT,
    transcript_offset_end BIGINT,
    transcript_fragment VARCHAR,
    files_modified VARCHAR,
    checkpoint_id VARCHAR,
    updated_at VARCHAR,
    PRIMARY KEY (repo_id, turn_id)
);

CREATE TABLE IF NOT EXISTS cache_interaction_events (
    event_id VARCHAR,
    event_time VARCHAR,
    repo_id VARCHAR,
    session_id VARCHAR,
    turn_id VARCHAR,
    branch VARCHAR,
    actor_id VARCHAR,
    actor_name VARCHAR,
    actor_email VARCHAR,
    actor_source VARCHAR,
    event_type VARCHAR,
    source VARCHAR,
    sequence_number BIGINT,
    agent_type VARCHAR,
    model VARCHAR,
    tool_use_id VARCHAR,
    tool_kind VARCHAR,
    task_description VARCHAR,
    subagent_id VARCHAR,
    payload VARCHAR,
    PRIMARY KEY (repo_id, event_id)
);

CREATE TABLE IF NOT EXISTS cache_interaction_tool_invocations (
    tool_invocation_id VARCHAR,
    repo_id VARCHAR,
    session_id VARCHAR,
    turn_id VARCHAR,
    tool_use_id VARCHAR,
    tool_name VARCHAR,
    source VARCHAR,
    input_summary VARCHAR,
    output_summary VARCHAR,
    command VARCHAR,
    command_binary VARCHAR,
    command_argv VARCHAR,
    transcript_path VARCHAR,
    started_at VARCHAR,
    ended_at VARCHAR,
    started_sequence_number BIGINT,
    ended_sequence_number BIGINT,
    updated_at VARCHAR,
    PRIMARY KEY (repo_id, tool_invocation_id)
);

CREATE TABLE IF NOT EXISTS cache_interaction_subagent_runs (
    subagent_run_id VARCHAR,
    repo_id VARCHAR,
    session_id VARCHAR,
    turn_id VARCHAR,
    tool_use_id VARCHAR,
    subagent_id VARCHAR,
    subagent_type VARCHAR,
    task_description VARCHAR,
    source VARCHAR,
    transcript_path VARCHAR,
    child_session_id VARCHAR,
    started_at VARCHAR,
    ended_at VARCHAR,
    started_sequence_number BIGINT,
    ended_sequence_number BIGINT,
    updated_at VARCHAR,
    PRIMARY KEY (repo_id, subagent_run_id)
);
"#;

async fn load_cached_watermarks(
    cache_path: &Path,
    repo_ids: &[String],
) -> Result<BTreeMap<String, RepoWatermark>> {
    if repo_ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let sql = format!(
        "SELECT repo_id, relational_watermark, events_watermark \
         FROM analytics_repo_cache_state WHERE repo_id IN ({})",
        sql_in_list(repo_ids, esc_pg)
    );
    let rows = duckdb_query_rows_path(cache_path, &sql)
        .await
        .context("querying analytics cache state")?;
    let mut watermarks = BTreeMap::new();
    for row in rows {
        let repo_id = row_string(&row, "repo_id");
        if repo_id.is_empty() {
            continue;
        }
        watermarks.insert(
            repo_id,
            RepoWatermark {
                relational: row_string(&row, "relational_watermark"),
                events: row_string(&row, "events_watermark"),
            },
        );
    }
    Ok(watermarks)
}

async fn collect_source_watermarks(
    cfg: &DevqlConfig,
    backends: &StoreBackendConfig,
    relational: &RelationalStorage,
    repo_ids: &[String],
) -> Result<BTreeMap<String, RepoWatermark>> {
    let mut watermarks = repo_ids
        .iter()
        .cloned()
        .map(|repo_id| (repo_id, RepoWatermark::default()))
        .collect::<BTreeMap<_, _>>();

    merge_relational_watermarks(
        &mut watermarks,
        relational
            .query_rows(&format!(
                "SELECT repo_id, COALESCE(last_sync_completed_at, '') AS watermark \
             FROM repo_sync_state WHERE repo_id IN ({})",
                sql_in_list(repo_ids, esc_pg)
            ))
            .await
            .or_else(ignore_missing_table)?,
        "watermark",
        true,
    );
    merge_relational_watermarks(
        &mut watermarks,
        relational
            .query_rows(&format!(
                "SELECT repo_id, COALESCE(MAX(last_synced_at), '') AS watermark \
             FROM current_file_state WHERE repo_id IN ({}) GROUP BY repo_id",
                sql_in_list(repo_ids, esc_pg)
            ))
            .await
            .or_else(ignore_missing_table)?,
        "watermark",
        true,
    );

    if backends.relational.has_postgres() {
        merge_relational_watermarks(
            &mut watermarks,
            relational
                .query_rows_remote(&format!(
                    "SELECT repo_id, COALESCE(last_sync_completed_at::text, '') AS watermark \
                     FROM repo_sync_state WHERE repo_id IN ({})",
                    sql_in_list(repo_ids, esc_pg)
                ))
                .await
                .or_else(ignore_missing_table)?,
            "watermark",
            true,
        );
        merge_relational_watermarks(
            &mut watermarks,
            relational
                .query_rows_remote(&format!(
                    "SELECT repo_id, COALESCE(MAX(last_synced_at), '') AS watermark \
                     FROM current_file_state WHERE repo_id IN ({}) GROUP BY repo_id",
                    sql_in_list(repo_ids, esc_pg)
                ))
                .await
                .or_else(ignore_missing_table)?,
            "watermark",
            true,
        );
    }

    if backends.events.has_clickhouse() {
        merge_relational_watermarks(
            &mut watermarks,
            clickhouse_rows(
                cfg,
                &format!(
                    "SELECT repo_id, max(updated_at) AS watermark \
                     FROM interaction_sessions WHERE repo_id IN ({}) GROUP BY repo_id",
                    sql_in_list(repo_ids, esc_ch)
                ),
            )
            .await
            .or_else(ignore_missing_table)?,
            "watermark",
            false,
        );
        merge_relational_watermarks(
            &mut watermarks,
            clickhouse_rows(
                cfg,
                &format!(
                    "SELECT repo_id, max(updated_at) AS watermark \
                     FROM interaction_turns WHERE repo_id IN ({}) GROUP BY repo_id",
                    sql_in_list(repo_ids, esc_ch)
                ),
            )
            .await
            .or_else(ignore_missing_table)?,
            "watermark",
            false,
        );
        merge_relational_watermarks(
            &mut watermarks,
            clickhouse_rows(
                cfg,
                &format!(
                    "SELECT repo_id, max(event_time) AS watermark \
                     FROM interaction_events WHERE repo_id IN ({}) GROUP BY repo_id",
                    sql_in_list(repo_ids, esc_ch)
                ),
            )
            .await
            .or_else(ignore_missing_table)?,
            "watermark",
            false,
        );
    } else {
        let duckdb_path = backends
            .events
            .resolve_duckdb_db_path_for_repo(&cfg.repo_root);
        merge_relational_watermarks(
            &mut watermarks,
            duckdb_query_rows_path(
                &duckdb_path,
                &format!(
                    "SELECT repo_id, COALESCE(MAX(updated_at), '') AS watermark \
                     FROM interaction_sessions WHERE repo_id IN ({}) GROUP BY repo_id",
                    sql_in_list(repo_ids, esc_pg)
                ),
            )
            .await
            .or_else(ignore_missing_table)?,
            "watermark",
            false,
        );
        merge_relational_watermarks(
            &mut watermarks,
            duckdb_query_rows_path(
                &duckdb_path,
                &format!(
                    "SELECT repo_id, COALESCE(MAX(updated_at), '') AS watermark \
                     FROM interaction_turns WHERE repo_id IN ({}) GROUP BY repo_id",
                    sql_in_list(repo_ids, esc_pg)
                ),
            )
            .await
            .or_else(ignore_missing_table)?,
            "watermark",
            false,
        );
        merge_relational_watermarks(
            &mut watermarks,
            duckdb_query_rows_path(
                &duckdb_path,
                &format!(
                    "SELECT repo_id, COALESCE(MAX(event_time), '') AS watermark \
                     FROM interaction_events WHERE repo_id IN ({}) GROUP BY repo_id",
                    sql_in_list(repo_ids, esc_pg)
                ),
            )
            .await
            .or_else(ignore_missing_table)?,
            "watermark",
            false,
        );
    }

    Ok(watermarks)
}

fn merge_relational_watermarks(
    target: &mut BTreeMap<String, RepoWatermark>,
    rows: Vec<Value>,
    field: &str,
    relational: bool,
) {
    for row in rows {
        let repo_id = row_string(&row, "repo_id");
        if repo_id.is_empty() {
            continue;
        }
        let value = row_string(&row, field);
        let entry = target.entry(repo_id).or_default();
        if relational {
            if value > entry.relational {
                entry.relational = value;
            }
        } else if value > entry.events {
            entry.events = value;
        }
    }
}
async fn write_cache_snapshot(
    cache_path: &Path,
    repositories: &[AnalyticsRepository],
    source_tables: &AnalyticsSourceTables,
    derived_tables: &AnalyticsDerivedTables,
    watermarks: &BTreeMap<String, RepoWatermark>,
) -> Result<()> {
    let cache_path = cache_path.to_path_buf();
    let repositories = repositories.to_vec();
    let source_tables = source_tables.clone();
    let derived_tables = derived_tables.clone();
    let watermarks = watermarks.clone();

    tokio::task::spawn_blocking(move || -> Result<()> {
        let conn = duckdb::Connection::open(&cache_path)
            .with_context(|| format!("opening analytics cache {}", cache_path.display()))?;
        conn.execute_batch("BEGIN TRANSACTION;")
            .context("starting analytics cache transaction")?;
        let result = (|| -> Result<()> {
            let repo_ids = repositories
                .iter()
                .map(|repository| repository.repo_id.clone())
                .collect::<Vec<_>>();

            delete_cache_rows(&conn, "cache_repositories", &repo_ids)?;
            delete_cache_rows(&conn, "cache_repo_sync_state", &repo_ids)?;
            delete_cache_rows(&conn, "cache_current_file_state", &repo_ids)?;
            delete_cache_rows(&conn, "cache_interaction_sessions", &repo_ids)?;
            delete_cache_rows(&conn, "cache_interaction_turns", &repo_ids)?;
            delete_cache_rows(&conn, "cache_interaction_events", &repo_ids)?;
            delete_cache_rows(&conn, "cache_interaction_tool_invocations", &repo_ids)?;
            delete_cache_rows(&conn, "cache_interaction_subagent_runs", &repo_ids)?;

            insert_rows(&conn, &CACHE_REPOSITORIES_SPEC, &source_tables.repositories)?;
            insert_rows(
                &conn,
                &CACHE_REPO_SYNC_STATE_SPEC,
                &source_tables.repo_sync_state,
            )?;
            insert_rows(
                &conn,
                &CACHE_CURRENT_FILE_STATE_SPEC,
                &source_tables.current_file_state,
            )?;
            insert_rows(
                &conn,
                &CACHE_INTERACTION_SESSIONS_SPEC,
                &source_tables.interaction_sessions,
            )?;
            insert_rows(
                &conn,
                &CACHE_INTERACTION_TURNS_SPEC,
                &source_tables.interaction_turns,
            )?;
            insert_rows(
                &conn,
                &CACHE_INTERACTION_EVENTS_SPEC,
                &source_tables.interaction_events,
            )?;
            insert_rows(
                &conn,
                &CACHE_TOOL_INVOCATIONS_SPEC,
                &derived_tables.interaction_tool_invocations,
            )?;
            insert_rows(
                &conn,
                &CACHE_SUBAGENT_RUNS_SPEC,
                &derived_tables.interaction_subagent_runs,
            )?;

            upsert_cache_state(&conn, &repo_ids, &watermarks)?;
            Ok(())
        })();

        match result {
            Ok(()) => conn
                .execute_batch("COMMIT;")
                .context("committing analytics cache transaction"),
            Err(err) => {
                let _ = conn.execute_batch("ROLLBACK;");
                Err(err)
            }
        }
    })
    .await
    .context("joining analytics cache write task")?
}

fn delete_cache_rows(conn: &duckdb::Connection, table: &str, repo_ids: &[String]) -> Result<()> {
    if repo_ids.is_empty() {
        return Ok(());
    }
    conn.execute_batch(&format!(
        "DELETE FROM {table} WHERE repo_id IN ({})",
        sql_in_list(repo_ids, esc_pg)
    ))
    .with_context(|| format!("deleting stale analytics cache rows from {table}"))?;
    Ok(())
}

fn insert_rows(conn: &duckdb::Connection, table: &TableSpec, rows: &[Value]) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }

    let column_list = table
        .columns
        .iter()
        .map(|column| column.name)
        .collect::<Vec<_>>()
        .join(", ");
    for row in rows {
        let values = table
            .columns
            .iter()
            .map(|column| value_literal(row.get(column.name), column.kind))
            .collect::<Vec<_>>()
            .join(", ");
        conn.execute_batch(&format!(
            "INSERT INTO {} ({column_list}) VALUES ({values})",
            table.name
        ))
        .with_context(|| format!("inserting analytics cache row into {}", table.name))?;
    }
    Ok(())
}

fn upsert_cache_state(
    conn: &duckdb::Connection,
    repo_ids: &[String],
    watermarks: &BTreeMap<String, RepoWatermark>,
) -> Result<()> {
    let refreshed_at = Utc::now().to_rfc3339();
    for repo_id in repo_ids {
        let watermark = watermarks.get(repo_id).cloned().unwrap_or_default();
        conn.execute_batch(&format!(
            "DELETE FROM analytics_repo_cache_state WHERE repo_id = '{}'; \
             INSERT INTO analytics_repo_cache_state (repo_id, relational_watermark, events_watermark, last_refreshed_at) \
             VALUES ('{}', '{}', '{}', '{}')",
            esc_pg(repo_id),
            esc_pg(repo_id),
            esc_pg(&watermark.relational),
            esc_pg(&watermark.events),
            esc_pg(&refreshed_at),
        ))
        .with_context(|| format!("upserting analytics cache state for repo {}", repo_id))?;
    }
    Ok(())
}

fn value_literal(value: Option<&Value>, kind: ColumnKind) -> String {
    let Some(value) = value else {
        return "NULL".to_string();
    };
    match kind {
        ColumnKind::Text => match value {
            Value::Null => "NULL".to_string(),
            Value::String(value) => format!("'{}'", esc_pg(value)),
            other => format!(
                "'{}'",
                esc_pg(&serde_json::to_string(other).unwrap_or_default())
            ),
        },
        ColumnKind::Integer => match value {
            Value::Null => "NULL".to_string(),
            Value::Bool(value) => {
                if *value {
                    "1".to_string()
                } else {
                    "0".to_string()
                }
            }
            Value::Number(number) => number.to_string(),
            Value::String(value) => {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    "NULL".to_string()
                } else if trimmed.parse::<i64>().is_ok() {
                    trimmed.to_string()
                } else {
                    "NULL".to_string()
                }
            }
            _ => "NULL".to_string(),
        },
    }
}

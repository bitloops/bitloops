use anyhow::{Context, Result, anyhow};
use serde_json::{Map, Value};

use super::super::{DevqlConfig, duckdb_value_to_json, esc_pg};
use super::cache::analytics_cache_path;
use super::row_access::sql_in_list;
use super::types::{
    ANALYTICS_FETCH_ROWS, ANALYTICS_MAX_ROWS, ANALYTICS_TIMEOUT, AnalyticsQueryResult,
    AnalyticsSqlColumn,
};

pub(super) async fn run_analytics_query(
    cfg: &DevqlConfig,
    repo_ids: &[String],
    user_sql: &str,
) -> Result<AnalyticsQueryResult> {
    let cache_path = analytics_cache_path(cfg);
    let repo_ids = repo_ids.to_vec();
    let user_sql = user_sql.to_string();

    let connection = tokio::task::spawn_blocking({
        let cache_path = cache_path.clone();
        let repo_ids = repo_ids.clone();
        move || -> Result<duckdb::Connection> {
            let conn = duckdb::Connection::open_in_memory_with_flags(
                duckdb::Config::default()
                    .enable_external_access(false)
                    .context("disabling analytics query external access")?
                    .enable_autoload_extension(false)
                    .context("disabling analytics query extension autoload")?,
            )
            .context("opening in-memory analytics query connection")?;
            prepare_request_views(&conn, &cache_path, &repo_ids)?;
            Ok(conn)
        }
    })
    .await
    .context("joining analytics query setup task")??;

    let interrupt = connection.interrupt_handle();
    let execute = tokio::task::spawn_blocking(move || execute_query(connection, &user_sql));
    tokio::pin!(execute);

    match tokio::time::timeout(ANALYTICS_TIMEOUT, &mut execute).await {
        Ok(result) => result
            .context("joining analytics query task")?
            .context("executing analytics query"),
        Err(_) => {
            interrupt.interrupt();
            let _ = execute.await;
            Err(anyhow!(
                "analytics query timed out after {} seconds",
                ANALYTICS_TIMEOUT.as_secs()
            ))
        }
    }
}

fn prepare_request_views(
    conn: &duckdb::Connection,
    cache_path: &std::path::Path,
    repo_ids: &[String],
) -> Result<()> {
    let repo_filter = sql_in_list(repo_ids, esc_pg);
    let cache_conn = duckdb::Connection::open_with_flags(
        cache_path,
        duckdb::Config::default()
            .access_mode(duckdb::AccessMode::ReadOnly)
            .context("opening analytics cache in read-only mode")?
            .enable_autoload_extension(false)
            .context("disabling analytics cache reader extension autoload")?,
    )
    .with_context(|| format!("opening analytics cache reader {}", cache_path.display()))?;
    conn.execute_batch("CREATE SCHEMA analytics; CREATE SCHEMA analytics_raw;")
        .context("creating request-scoped analytics schemas")?;

    let repositories_view = format!(
        "SELECT repo_id, repo_root, provider, organization, name, identity, default_branch, metadata_json, created_at \
         FROM cache_repositories WHERE repo_id IN ({repo_filter})"
    );
    let repo_sync_state_view = format!(
        "SELECT repo_id, repo_root, active_branch, head_commit_sha, head_tree_sha, parser_version, extractor_version, \
                scope_exclusions_fingerprint, last_sync_started_at, last_sync_completed_at, last_sync_status, last_sync_reason \
         FROM cache_repo_sync_state WHERE repo_id IN ({repo_filter})"
    );
    let current_file_state_view = format!(
        "SELECT c.repo_id, r.repo_root, c.path, c.analysis_mode, c.file_role, c.text_index_mode, c.language, \
                c.resolved_language, c.dialect, c.primary_context_id, c.secondary_context_ids_json, c.frameworks_json, \
                c.runtime_profile, c.classification_reason, c.context_fingerprint, c.extraction_fingerprint, \
                c.head_content_id, c.index_content_id, c.worktree_content_id, c.effective_content_id, c.effective_source, \
                c.parser_version, c.extractor_version, c.exists_in_head, c.exists_in_index, c.exists_in_worktree, c.last_synced_at \
         FROM cache_current_file_state AS c \
         LEFT JOIN cache_repositories AS r ON r.repo_id = c.repo_id \
         WHERE c.repo_id IN ({repo_filter})"
    );
    let sessions_view = format!(
        "SELECT s.session_id, s.repo_id, r.repo_root, s.branch, s.actor_id, s.actor_name, s.actor_email, s.actor_source, \
                s.agent_type, s.model, s.first_prompt, s.transcript_path, s.worktree_path, s.worktree_id, s.started_at, \
                s.ended_at, s.last_event_at, s.updated_at \
         FROM cache_interaction_sessions AS s \
         LEFT JOIN cache_repositories AS r ON r.repo_id = s.repo_id \
         WHERE s.repo_id IN ({repo_filter})"
    );
    let turns_view = format!(
        "SELECT t.turn_id, t.session_id, t.repo_id, r.repo_root, t.branch, t.actor_id, t.actor_name, t.actor_email, t.actor_source, \
                t.turn_number, t.prompt, t.agent_type, t.model, t.started_at, t.ended_at, t.has_token_usage, t.input_tokens, \
                t.cache_creation_tokens, t.cache_read_tokens, t.output_tokens, t.api_call_count, t.summary, t.prompt_count, \
                t.transcript_offset_start, t.transcript_offset_end, t.transcript_fragment, t.files_modified, t.checkpoint_id, t.updated_at \
         FROM cache_interaction_turns AS t \
         LEFT JOIN cache_repositories AS r ON r.repo_id = t.repo_id \
         WHERE t.repo_id IN ({repo_filter})"
    );
    let events_view = format!(
        "SELECT e.event_id, e.event_time, e.repo_id, r.repo_root, e.session_id, e.turn_id, e.branch, e.actor_id, e.actor_name, \
                e.actor_email, e.actor_source, e.event_type, e.source, e.sequence_number, e.agent_type, e.model, e.tool_use_id, \
                e.tool_kind, e.task_description, e.subagent_id, e.payload \
         FROM cache_interaction_events AS e \
         LEFT JOIN cache_repositories AS r ON r.repo_id = e.repo_id \
         WHERE e.repo_id IN ({repo_filter})"
    );
    let tools_view = format!(
        "SELECT t.tool_invocation_id, t.repo_id, r.repo_root, t.session_id, t.turn_id, t.tool_use_id, t.tool_name, t.source, \
                t.input_summary, t.output_summary, t.command, t.command_binary, t.command_argv, t.transcript_path, t.started_at, \
                t.ended_at, t.started_sequence_number, t.ended_sequence_number, t.updated_at \
         FROM cache_interaction_tool_invocations AS t \
         LEFT JOIN cache_repositories AS r ON r.repo_id = t.repo_id \
         WHERE t.repo_id IN ({repo_filter})"
    );
    let subagents_view = format!(
        "SELECT s.subagent_run_id, s.repo_id, r.repo_root, s.session_id, s.turn_id, s.tool_use_id, s.subagent_id, \
                s.subagent_type, s.task_description, s.source, s.transcript_path, s.child_session_id, s.started_at, \
                s.ended_at, s.started_sequence_number, s.ended_sequence_number, s.updated_at \
         FROM cache_interaction_subagent_runs AS s \
         LEFT JOIN cache_repositories AS r ON r.repo_id = s.repo_id \
         WHERE s.repo_id IN ({repo_filter})"
    );
    let shell_commands_view =
        "SELECT tool_invocation_id, repo_id, repo_root, session_id, turn_id, tool_use_id, tool_name, source, \
                command, command_binary, command_argv, transcript_path, started_at, ended_at, updated_at \
         FROM analytics_raw.interaction_tool_invocations \
         WHERE command_binary IS NOT NULL AND trim(command_binary) <> ''"
            .to_string();

    let views = [
        ("repositories", repositories_view.as_str()),
        ("repo_sync_state", repo_sync_state_view.as_str()),
        ("current_file_state", current_file_state_view.as_str()),
        ("interaction_sessions", sessions_view.as_str()),
        ("interaction_turns", turns_view.as_str()),
        ("interaction_events", events_view.as_str()),
        ("interaction_tool_invocations", tools_view.as_str()),
        ("interaction_subagent_runs", subagents_view.as_str()),
    ];

    for (name, definition) in views {
        materialise_request_table(conn, &cache_conn, name, definition)?;
        conn.execute_batch(&format!(
            "CREATE OR REPLACE VIEW {}.{} AS SELECT * FROM {}.{};",
            quote_ident("analytics"),
            quote_ident(name),
            quote_ident("analytics_raw"),
            quote_ident(name)
        ))
        .with_context(|| format!("creating analytics view {name}"))?;
    }
    conn.execute_batch(&format!(
        "CREATE OR REPLACE VIEW analytics.shell_commands AS {shell_commands_view};"
    ))
    .context("creating analytics shell_commands view")?;
    Ok(())
}

fn materialise_request_table(
    conn: &duckdb::Connection,
    cache_conn: &duckdb::Connection,
    name: &str,
    definition: &str,
) -> Result<()> {
    let mut stmt = cache_conn
        .prepare(definition)
        .with_context(|| format!("preparing analytics cache projection {name}"))?;
    let mut rows = stmt
        .query([])
        .with_context(|| format!("querying analytics cache projection {name}"))?;
    let columns = {
        let stmt = rows
            .as_ref()
            .context("analytics cache projection did not expose statement metadata")?;
        stmt.column_names()
            .into_iter()
            .enumerate()
            .map(|(index, column_name)| {
                let column_type = duckdb::types::Type::from(&stmt.column_type(index));
                (column_name, column_type)
            })
            .collect::<Vec<_>>()
    };
    let column_sql = columns
        .iter()
        .map(|(column_name, column_type)| {
            format!(
                "{} {}",
                quote_ident(column_name),
                duckdb_type_sql(column_type)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    conn.execute_batch(&format!(
        "CREATE TABLE {}.{} ({column_sql});",
        quote_ident("analytics_raw"),
        quote_ident(name)
    ))
    .with_context(|| format!("creating analytics_raw table {name}"))?;

    let mut appender = conn
        .appender_to_db(name, "analytics_raw")
        .with_context(|| format!("opening analytics_raw appender for {name}"))?;
    while let Some(row) = rows
        .next()
        .with_context(|| format!("iterating analytics cache projection {name}"))?
    {
        let values = (0..columns.len())
            .map(|index| {
                row.get_ref(index)
                    .with_context(|| {
                        format!("reading analytics cache projection {name} column {index}")
                    })
                    .map(|value| value.to_owned())
            })
            .collect::<Result<Vec<_>>>()?;
        appender
            .append_row(duckdb::appender_params_from_iter(values))
            .with_context(|| format!("appending analytics_raw row into {name}"))?;
    }
    appender
        .flush()
        .with_context(|| format!("flushing analytics_raw rows into {name}"))?;
    Ok(())
}

fn duckdb_type_sql(column_type: &duckdb::types::Type) -> &'static str {
    match column_type {
        duckdb::types::Type::Null => "VARCHAR",
        duckdb::types::Type::Boolean => "BOOLEAN",
        duckdb::types::Type::TinyInt => "TINYINT",
        duckdb::types::Type::SmallInt => "SMALLINT",
        duckdb::types::Type::Int => "INTEGER",
        duckdb::types::Type::BigInt => "BIGINT",
        duckdb::types::Type::HugeInt => "HUGEINT",
        duckdb::types::Type::UTinyInt => "UTINYINT",
        duckdb::types::Type::USmallInt => "USMALLINT",
        duckdb::types::Type::UInt => "UINTEGER",
        duckdb::types::Type::UBigInt => "UBIGINT",
        duckdb::types::Type::Float => "FLOAT",
        duckdb::types::Type::Double => "DOUBLE",
        duckdb::types::Type::Decimal => "DECIMAL",
        duckdb::types::Type::Timestamp => "TIMESTAMP",
        duckdb::types::Type::Text => "VARCHAR",
        duckdb::types::Type::Blob => "BLOB",
        duckdb::types::Type::Date32 => "DATE",
        duckdb::types::Type::Time64 => "TIME",
        duckdb::types::Type::Interval => "INTERVAL",
        duckdb::types::Type::List(_)
        | duckdb::types::Type::Enum
        | duckdb::types::Type::Struct(_)
        | duckdb::types::Type::Map(_, _)
        | duckdb::types::Type::Array(_, _)
        | duckdb::types::Type::Union
        | duckdb::types::Type::Any => "VARCHAR",
    }
}

fn quote_ident(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn execute_query(conn: duckdb::Connection, user_sql: &str) -> Result<AnalyticsQueryResult> {
    let wrapped_sql = format!(
        "SELECT * FROM ({}) AS __bitloops_analytics_query LIMIT {}",
        user_sql, ANALYTICS_FETCH_ROWS
    );
    let mut stmt = conn
        .prepare(&wrapped_sql)
        .context("preparing analytics SQL query")?;
    let mut rows = stmt.query([]).context("executing analytics SQL query")?;
    let columns = {
        let stmt = rows
            .as_ref()
            .context("analytics SQL query did not expose statement metadata")?;
        stmt.column_names()
            .into_iter()
            .enumerate()
            .map(|(index, name)| AnalyticsSqlColumn {
                name,
                logical_type: format!("{:?}", stmt.column_type(index)),
            })
            .collect::<Vec<_>>()
    };
    let mut values = Vec::new();

    while let Some(row) = rows.next().context("iterating analytics SQL rows")? {
        let mut object = Map::new();
        for (index, column) in columns.iter().enumerate() {
            let value = row
                .get_ref(index)
                .with_context(|| format!("reading analytics SQL column `{}`", column.name))?
                .to_owned();
            object.insert(column.name.clone(), duckdb_value_to_json(value));
        }
        values.push(Value::Object(object));
    }

    let truncated = values.len() > ANALYTICS_MAX_ROWS;
    if truncated {
        values.truncate(ANALYTICS_MAX_ROWS);
    }

    Ok(AnalyticsQueryResult {
        columns,
        rows: values,
        truncated,
    })
}

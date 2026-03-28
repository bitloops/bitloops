use super::{DevqlGraphqlContext, GRAPHQL_GIT_SCAN_LIMIT};
use crate::graphql::ResolverScope;
use crate::graphql::types::{Checkpoint, DateTimeScalar, JsonScalar, TelemetryEvent};
use crate::host::devql::{clickhouse_query_data, duckdb_query_rows_path, esc_ch, esc_pg};
use anyhow::{Context, Result, anyhow, bail};
use async_graphql::types::Json;
use chrono::{NaiveDateTime, TimeZone, Utc};
use serde_json::Value;

impl DevqlGraphqlContext {
    pub(crate) async fn list_checkpoints(
        &self,
        scope: &ResolverScope,
        agent: Option<&str>,
        since: Option<&DateTimeScalar>,
    ) -> Result<Vec<Checkpoint>> {
        let backend_config = self
            .backend_config
            .as_ref()
            .context("store backend configuration unavailable")?;
        let repo_id = self.repo_identity.repo_id.as_str();

        let checkpoints = if backend_config.events.has_clickhouse() {
            let cfg = self.config.as_ref().with_context(|| {
                self.config_error
                    .clone()
                    .unwrap_or_else(|| "DevQL configuration unavailable".to_string())
            })?;
            let sql = build_clickhouse_checkpoints_sql(repo_id, scope, agent, since);
            let rows: Vec<Value> = clickhouse_query_data(cfg, &sql)
                .await?
                .as_array()
                .cloned()
                .unwrap_or_default();
            rows.into_iter()
                .map(checkpoint_from_row)
                .collect::<Result<Vec<_>>>()?
        } else {
            let sql = build_duckdb_checkpoints_sql(repo_id, scope, agent, since);
            let duckdb_path = backend_config
                .events
                .resolve_duckdb_db_path_for_repo(&self.repo_root);
            let rows: Vec<Value> = duckdb_query_rows_path(&duckdb_path, &sql).await?;
            rows.into_iter()
                .map(checkpoint_from_row)
                .collect::<Result<Vec<_>>>()?
        };

        if checkpoints.is_empty() {
            return self.list_committed_checkpoints(scope, agent, since).await;
        }

        Ok(checkpoints)
    }

    pub(crate) async fn list_telemetry_events(
        &self,
        scope: &ResolverScope,
        event_type: Option<&str>,
        agent: Option<&str>,
        since: Option<&DateTimeScalar>,
    ) -> Result<Vec<TelemetryEvent>> {
        let backend_config = self
            .backend_config
            .as_ref()
            .context("store backend configuration unavailable")?;
        let repo_id = self.repo_identity.repo_id.as_str();

        if backend_config.events.has_clickhouse() {
            let cfg = self.config.as_ref().with_context(|| {
                self.config_error
                    .clone()
                    .unwrap_or_else(|| "DevQL configuration unavailable".to_string())
            })?;
            let sql = build_clickhouse_telemetry_sql(repo_id, scope, event_type, agent, since);
            let rows: Vec<Value> = clickhouse_query_data(cfg, &sql)
                .await?
                .as_array()
                .cloned()
                .unwrap_or_default();
            return rows
                .into_iter()
                .map(telemetry_event_from_row)
                .collect::<Result<Vec<_>>>();
        }

        let sql = build_duckdb_telemetry_sql(repo_id, scope, event_type, agent, since);
        let duckdb_path = backend_config
            .events
            .resolve_duckdb_db_path_for_repo(&self.repo_root);
        let rows: Vec<Value> = duckdb_query_rows_path(&duckdb_path, &sql).await?;
        rows.into_iter()
            .map(telemetry_event_from_row)
            .collect::<Result<Vec<_>>>()
    }
}

fn build_clickhouse_checkpoints_sql(
    repo_id: &str,
    scope: &ResolverScope,
    agent: Option<&str>,
    since: Option<&DateTimeScalar>,
) -> String {
    let mut conditions = vec![
        format!("repo_id = '{}'", esc_ch(repo_id)),
        "event_type = 'checkpoint_committed'".to_string(),
    ];
    if let Some(agent) = cleaned_filter(agent) {
        conditions.push(format!("agent = '{}'", esc_ch(agent)));
    }
    if let Some(since) = since {
        conditions.push(format!(
            "event_time >= parseDateTime64BestEffortOrZero('{}')",
            esc_ch(since.as_str())
        ));
    }
    if let Some(project_path) = scope.project_path() {
        conditions.push(clickhouse_project_filter(project_path));
    }

    format!(
        "SELECT checkpoint_id, \
max(event_time) AS latest_event_time, \
argMax(session_id, event_time) AS session_id, \
argMax(commit_sha, event_time) AS commit_sha, \
argMax(branch, event_time) AS branch, \
argMax(agent, event_time) AS agent, \
argMax(strategy, event_time) AS strategy, \
argMax(files_touched, event_time) AS files_touched, \
argMax(payload, event_time) AS payload \
FROM checkpoint_events \
WHERE {} \
GROUP BY checkpoint_id \
ORDER BY latest_event_time DESC, checkpoint_id DESC \
LIMIT {}",
        conditions.join(" AND "),
        GRAPHQL_GIT_SCAN_LIMIT
    )
}

fn build_duckdb_checkpoints_sql(
    repo_id: &str,
    scope: &ResolverScope,
    agent: Option<&str>,
    since: Option<&DateTimeScalar>,
) -> String {
    let mut conditions = vec![
        format!("repo_id = '{}'", esc_pg(repo_id)),
        "event_type = 'checkpoint_committed'".to_string(),
    ];
    if let Some(agent) = cleaned_filter(agent) {
        conditions.push(format!("agent = '{}'", esc_pg(agent)));
    }
    if let Some(since) = since {
        conditions.push(format!("event_time >= '{}'", esc_pg(since.as_str())));
    }
    if let Some(project_path) = scope.project_path() {
        conditions.push(duckdb_project_filter(project_path));
    }

    format!(
        "SELECT checkpoint_id, \
max(event_time) AS latest_event_time, \
arg_max(session_id, event_time) AS session_id, \
arg_max(commit_sha, event_time) AS commit_sha, \
arg_max(branch, event_time) AS branch, \
arg_max(agent, event_time) AS agent, \
arg_max(strategy, event_time) AS strategy, \
arg_max(files_touched, event_time) AS files_touched, \
arg_max(payload, event_time) AS payload \
FROM checkpoint_events \
WHERE {} \
GROUP BY checkpoint_id \
ORDER BY latest_event_time DESC, checkpoint_id DESC \
LIMIT {}",
        conditions.join(" AND "),
        GRAPHQL_GIT_SCAN_LIMIT
    )
}

fn build_clickhouse_telemetry_sql(
    repo_id: &str,
    scope: &ResolverScope,
    event_type: Option<&str>,
    agent: Option<&str>,
    since: Option<&DateTimeScalar>,
) -> String {
    let mut conditions = vec![format!("repo_id = '{}'", esc_ch(repo_id))];
    if let Some(event_type) = cleaned_filter(event_type) {
        conditions.push(format!("event_type = '{}'", esc_ch(event_type)));
    }
    if let Some(agent) = cleaned_filter(agent) {
        conditions.push(format!("agent = '{}'", esc_ch(agent)));
    }
    if let Some(since) = since {
        conditions.push(format!(
            "event_time >= parseDateTime64BestEffortOrZero('{}')",
            esc_ch(since.as_str())
        ));
    }
    if let Some(project_path) = scope.project_path() {
        conditions.push(clickhouse_project_filter(project_path));
    }

    format!(
        "SELECT event_id, session_id, event_type, agent, event_time, commit_sha, branch, payload \
FROM checkpoint_events \
WHERE {} \
ORDER BY event_time DESC, event_id DESC \
LIMIT {}",
        conditions.join(" AND "),
        GRAPHQL_GIT_SCAN_LIMIT
    )
}

fn build_duckdb_telemetry_sql(
    repo_id: &str,
    scope: &ResolverScope,
    event_type: Option<&str>,
    agent: Option<&str>,
    since: Option<&DateTimeScalar>,
) -> String {
    let mut conditions = vec![format!("repo_id = '{}'", esc_pg(repo_id))];
    if let Some(event_type) = cleaned_filter(event_type) {
        conditions.push(format!("event_type = '{}'", esc_pg(event_type)));
    }
    if let Some(agent) = cleaned_filter(agent) {
        conditions.push(format!("agent = '{}'", esc_pg(agent)));
    }
    if let Some(since) = since {
        conditions.push(format!("event_time >= '{}'", esc_pg(since.as_str())));
    }
    if let Some(project_path) = scope.project_path() {
        conditions.push(duckdb_project_filter(project_path));
    }

    format!(
        "SELECT event_id, session_id, event_type, agent, event_time, commit_sha, branch, payload \
FROM checkpoint_events \
WHERE {} \
ORDER BY event_time DESC, event_id DESC \
LIMIT {}",
        conditions.join(" AND "),
        GRAPHQL_GIT_SCAN_LIMIT
    )
}

fn clickhouse_project_filter(project_path: &str) -> String {
    format!(
        "arrayExists(path -> path = '{}' OR startsWith(path, '{}/'), files_touched)",
        esc_ch(project_path),
        esc_ch(project_path)
    )
}

fn duckdb_project_filter(project_path: &str) -> String {
    let escaped = esc_pg(&escape_like_literal(project_path));
    format!(
        "(files_touched LIKE '%\"{}\"%' ESCAPE '\\' OR files_touched LIKE '%\"{}/%' ESCAPE '\\')",
        escaped, escaped
    )
}

fn checkpoint_from_row(row: Value) -> Result<Checkpoint> {
    let checkpoint_id = required_string(&row, "checkpoint_id")?;
    let event_time = parse_event_time(&required_string(&row, "latest_event_time")?)?;
    let agent = optional_string(&row, "agent");
    let agents = agent.clone().into_iter().collect::<Vec<_>>();
    Ok(Checkpoint {
        id: checkpoint_id.clone().into(),
        session_id: required_string(&row, "session_id")?,
        commit_sha: optional_string(&row, "commit_sha"),
        branch: optional_string(&row, "branch"),
        agent,
        event_time: event_time.clone(),
        strategy: optional_string(&row, "strategy"),
        files_touched: parse_string_array(row.get("files_touched"))?,
        payload: parse_payload(row.get("payload"))?,
        checkpoints_count: 0,
        session_count: 0,
        token_usage: None,
        agents,
        first_prompt_preview: None,
        created_at: Some(event_time.as_str().to_string()),
        is_task: false,
        tool_use_id: None,
    })
}

fn telemetry_event_from_row(row: Value) -> Result<TelemetryEvent> {
    let event_id = required_string(&row, "event_id")?;
    Ok(TelemetryEvent {
        id: event_id.into(),
        session_id: required_string(&row, "session_id")?,
        event_type: required_string(&row, "event_type")?,
        agent: optional_string(&row, "agent"),
        event_time: parse_event_time(&required_string(&row, "event_time")?)?,
        commit_sha: optional_string(&row, "commit_sha"),
        branch: optional_string(&row, "branch"),
        payload: parse_payload(row.get("payload"))?,
    })
}

fn cleaned_filter(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn required_string(row: &Value, key: &str) -> Result<String> {
    optional_string(row, key).ok_or_else(|| anyhow!("missing `{key}` in events row"))
}

fn optional_string(row: &Value, key: &str) -> Option<String> {
    row.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn parse_string_array(value: Option<&Value>) -> Result<Vec<String>> {
    match value {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(values)) => Ok(values
            .iter()
            .filter_map(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect()),
        Some(Value::String(raw)) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Ok(Vec::new());
            }
            let parsed: Value =
                serde_json::from_str(trimmed).context("parsing events `files_touched` JSON")?;
            parse_string_array(Some(&parsed))
        }
        Some(other) => bail!("unexpected `files_touched` value in events row: {other}"),
    }
}

fn parse_payload(value: Option<&Value>) -> Result<Option<JsonScalar>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(raw)) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            let parsed = serde_json::from_str(trimmed)
                .unwrap_or_else(|_| Value::String(trimmed.to_string()));
            Ok(Some(Json(parsed)))
        }
        Some(other) => Ok(Some(Json(other.clone()))),
    }
}

fn parse_event_time(raw: &str) -> Result<DateTimeScalar> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("event time is empty");
    }

    if let Ok(value) = DateTimeScalar::from_rfc3339(trimmed.to_string()) {
        return Ok(value);
    }

    for pattern in [
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%dT%H:%M:%S",
    ] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(trimmed, pattern) {
            let timestamp = Utc.from_utc_datetime(&naive).to_rfc3339();
            return DateTimeScalar::from_rfc3339(timestamp)
                .context("formatting event timestamp as RFC 3339");
        }
    }

    if let Ok(unix_seconds) = trimmed.parse::<i64>()
        && let Some(timestamp) = Utc.timestamp_opt(unix_seconds, 0).single()
    {
        return DateTimeScalar::from_rfc3339(timestamp.to_rfc3339())
            .context("formatting unix event timestamp as RFC 3339");
    }

    bail!("unsupported event timestamp `{trimmed}`")
}

fn escape_like_literal(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

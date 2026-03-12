/// Connect timeout (seconds) for HTTP when talking to ClickHouse.
const CLICKHOUSE_CONNECT_TIMEOUT_SECS: u64 = 10;
/// Total transfer timeout (seconds) for HTTP when talking to ClickHouse.
const CLICKHOUSE_MAX_TIME_SECS: u64 = 30;

#[derive(Debug, Clone)]
struct ClickHouseEventsStore {
    endpoint: String,
    user: Option<String>,
    password: Option<String>,
}

impl ClickHouseEventsStore {
    fn from_backend(events: &crate::devql_config::EventsBackendConfig) -> Self {
        Self {
            endpoint: events.clickhouse_endpoint(),
            user: events.clickhouse_user.clone(),
            password: events.clickhouse_password.clone(),
        }
    }

    async fn run_sql(&self, sql: &str) -> Result<String> {
        run_clickhouse_sql_http(
            &self.endpoint,
            self.user.as_deref(),
            self.password.as_deref(),
            sql,
        )
        .await
    }

    async fn query_rows(&self, sql: &str) -> Result<Vec<Value>> {
        let mut query = sql.trim().to_string();
        if !query.to_ascii_uppercase().contains("FORMAT JSON") {
            query.push_str(" FORMAT JSON");
        }

        let raw = self.run_sql(&query).await?;
        if raw.trim().is_empty() {
            return Ok(vec![]);
        }

        let parsed: Value = serde_json::from_str(&raw)
            .with_context(|| format!("parsing ClickHouse JSON response: {raw}"))?;
        Ok(parsed
            .get("data")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }
}

impl EventsStore for ClickHouseEventsStore {
    fn provider(&self) -> EventsProvider {
        EventsProvider::ClickHouse
    }

    fn ping<'a>(&'a self) -> StoreFuture<'a, i32> {
        Box::pin(async move {
            let raw = self.run_sql("SELECT 1 FORMAT TabSeparated").await?;
            let value_raw = raw
                .lines()
                .last()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .ok_or_else(|| anyhow!("ClickHouse health query returned an empty response"))?;
            let value = value_raw.parse::<i32>().with_context(|| {
                format!("parsing ClickHouse health query result as integer: {value_raw}")
            })?;
            Ok(value)
        })
    }

    fn init_schema<'a>(&'a self) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            let sql = r#"
CREATE TABLE IF NOT EXISTS checkpoint_events (
    event_id String,
    event_time DateTime64(3, 'UTC'),
    repo_id String,
    checkpoint_id String,
    session_id String,
    commit_sha String,
    branch String,
    event_type String,
    agent String,
    strategy String,
    files_touched Array(String),
    payload String
)
ENGINE = ReplacingMergeTree(event_time)
ORDER BY (repo_id, event_time, event_id)
"#;

            self.run_sql(sql)
                .await
                .context("creating ClickHouse checkpoint_events table")?;
            Ok(())
        })
    }

    fn existing_event_ids<'a>(&'a self, repo_id: String) -> StoreFuture<'a, HashSet<String>> {
        Box::pin(async move {
            let sql = format!(
                "SELECT event_id FROM checkpoint_events WHERE repo_id = '{}' FORMAT JSON",
                esc_ch(&repo_id)
            );
            let rows = self.query_rows(&sql).await?;
            Ok(rows
                .into_iter()
                .filter_map(|row| {
                    row.get("event_id")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .collect())
        })
    }

    fn insert_checkpoint_event<'a>(&'a self, event: CheckpointEventWrite) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            let created_at = event
                .created_at
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty());
            let event_time_expr = if let Some(created_at) = created_at {
                format!(
                    "coalesce(parseDateTime64BestEffortOrNull('{}'), now64(3))",
                    esc_ch(created_at)
                )
            } else if let Some(commit_unix) = event.commit_unix {
                format!("toDateTime64({}, 3, 'UTC')", commit_unix)
            } else {
                "now64(3)".to_string()
            };

            let files_touched = format_ch_array(&event.files_touched);
            let payload = esc_ch(&serde_json::to_string(&event.payload)?);
            let sql = format!(
                "INSERT INTO checkpoint_events (event_id, event_time, repo_id, checkpoint_id, session_id, commit_sha, branch, event_type, agent, strategy, files_touched, payload) \
VALUES ('{}', {}, '{}', '{}', '{}', '{}', '{}', '{}', '{}', '{}', {}, '{}')",
                esc_ch(&event.event_id),
                event_time_expr,
                esc_ch(&event.repo_id),
                esc_ch(&event.checkpoint_id),
                esc_ch(&event.session_id),
                esc_ch(&event.commit_sha),
                esc_ch(&event.branch),
                esc_ch(&event.event_type),
                esc_ch(&event.agent),
                esc_ch(&event.strategy),
                files_touched,
                payload
            );
            self.run_sql(&sql).await.map(|_| ())
        })
    }

    fn query_checkpoints<'a>(&'a self, query: EventsCheckpointQuery) -> StoreFuture<'a, Vec<Value>> {
        Box::pin(async move {
            let mut conditions = vec![
                format!("repo_id = '{}'", esc_ch(&query.repo_id)),
                "event_type = 'checkpoint_committed'".to_string(),
            ];
            if let Some(agent) = query.agent.as_deref() {
                conditions.push(format!("agent = '{}'", esc_ch(agent)));
            }
            if let Some(since) = query.since.as_deref() {
                conditions.push(format!(
                    "event_time >= parseDateTime64BestEffortOrZero('{}')",
                    esc_ch(since)
                ));
            }

            let sql = format!(
                "SELECT checkpoint_id, max(event_time) AS created_at, anyLast(agent) AS agent, anyLast(commit_sha) AS commit_sha, anyLast(branch) AS branch, anyLast(strategy) AS strategy, anyLast(files_touched) AS files_touched FROM checkpoint_events WHERE {} GROUP BY checkpoint_id ORDER BY created_at DESC LIMIT {} FORMAT JSON",
                conditions.join(" AND "),
                query.limit.max(1)
            );
            self.query_rows(&sql).await
        })
    }

    fn query_telemetry<'a>(&'a self, query: EventsTelemetryQuery) -> StoreFuture<'a, Vec<Value>> {
        Box::pin(async move {
            let mut conditions = vec![format!("repo_id = '{}'", esc_ch(&query.repo_id))];
            if let Some(event_type) = query.event_type.as_deref() {
                conditions.push(format!("event_type = '{}'", esc_ch(event_type)));
            }
            if let Some(agent) = query.agent.as_deref() {
                conditions.push(format!("agent = '{}'", esc_ch(agent)));
            }
            if let Some(since) = query.since.as_deref() {
                conditions.push(format!(
                    "event_time >= parseDateTime64BestEffortOrZero('{}')",
                    esc_ch(since)
                ));
            }

            let sql = format!(
                "SELECT event_time, event_type, checkpoint_id, session_id, agent, commit_sha, branch, strategy, files_touched, payload FROM checkpoint_events WHERE {} ORDER BY event_time DESC LIMIT {} FORMAT JSON",
                conditions.join(" AND "),
                query.limit.max(1)
            );
            self.query_rows(&sql).await
        })
    }

    fn query_commit_shas<'a>(&'a self, query: EventsCommitShaQuery) -> StoreFuture<'a, Vec<String>> {
        Box::pin(async move {
            let mut conditions = vec![
                format!("repo_id = '{}'", esc_ch(&query.repo_id)),
                "event_type = 'checkpoint_committed'".to_string(),
                "commit_sha != ''".to_string(),
            ];
            if let Some(agent) = query.agent.as_deref() {
                conditions.push(format!("agent = '{}'", esc_ch(agent)));
            }
            if let Some(since) = query.since.as_deref() {
                conditions.push(format!(
                    "event_time >= parseDateTime64BestEffortOrZero('{}')",
                    esc_ch(since)
                ));
            }

            let sql = format!(
                "SELECT DISTINCT commit_sha FROM checkpoint_events WHERE {} LIMIT {} FORMAT JSON",
                conditions.join(" AND "),
                query.limit.max(1)
            );
            let rows = self.query_rows(&sql).await?;
            Ok(rows
                .into_iter()
                .filter_map(|row| {
                    row.get("commit_sha")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(str::to_string)
                })
                .collect())
        })
    }

    fn query_checkpoint_events<'a>(
        &'a self,
        query: EventsCheckpointHistoryQuery,
    ) -> StoreFuture<'a, Vec<Value>> {
        Box::pin(async move {
            if query.commit_shas.is_empty() {
                return Ok(vec![]);
            }

            let path_has_clause = if query.path_candidates.is_empty() {
                None
            } else {
                Some(
                    query
                        .path_candidates
                        .iter()
                        .map(|candidate| format!("has(files_touched, '{}')", esc_ch(candidate)))
                        .collect::<Vec<_>>()
                        .join(" OR "),
                )
            };

            let mut conditions = vec![
                format!("repo_id = '{}'", esc_ch(&query.repo_id)),
                "event_type = 'checkpoint_committed'".to_string(),
                format!("commit_sha IN ({})", sql_string_list_ch(&query.commit_shas)),
            ];
            if let Some(path_has_clause) = path_has_clause {
                conditions.push(format!("({path_has_clause})"));
            }

            let sql = format!(
                "SELECT event_time, checkpoint_id, session_id, agent, commit_sha, branch, strategy FROM checkpoint_events WHERE {} ORDER BY event_time DESC LIMIT {} FORMAT JSON",
                conditions.join(" AND "),
                query.limit.max(1)
            );
            self.query_rows(&sql).await
        })
    }
}

async fn run_clickhouse_sql_http(
    url: &str,
    user: Option<&str>,
    password: Option<&str>,
    sql: &str,
) -> Result<String> {
    let client = clickhouse_http_client()?;

    let mut request = client.post(url).body(sql.to_string());
    if let Some(username) = user {
        request = request.basic_auth(username, Some(password.unwrap_or("")));
    }

    let response = request.send().await.map_err(|err| {
        if err.is_timeout() {
            anyhow!(
                "ClickHouse request timed out (connect or transfer limit exceeded, {}s/{}s)",
                CLICKHOUSE_CONNECT_TIMEOUT_SECS,
                CLICKHOUSE_MAX_TIME_SECS
            )
        } else {
            anyhow!("sending ClickHouse request: {err}")
        }
    })?;

    let status = response.status();
    let body = response
        .text()
        .await
        .context("reading ClickHouse response body")?;
    if !status.is_success() {
        let detail = body.trim();
        if detail.is_empty() {
            bail!("ClickHouse request failed with status {status}");
        }
        bail!("ClickHouse request failed with status {status}: {detail}");
    }

    Ok(body)
}

fn clickhouse_http_client() -> Result<&'static reqwest::Client> {
    static CLICKHOUSE_HTTP_CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
    let result = CLICKHOUSE_HTTP_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(CLICKHOUSE_CONNECT_TIMEOUT_SECS))
            .timeout(Duration::from_secs(CLICKHOUSE_MAX_TIME_SECS))
            .build()
            .map_err(|err| format!("{err:#}"))
    });

    match result {
        Ok(client) => Ok(client),
        Err(err) => Err(anyhow!("building ClickHouse HTTP client: {err}")),
    }
}


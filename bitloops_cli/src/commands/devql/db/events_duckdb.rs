#[derive(Debug, Clone)]
struct DuckDbEventsStore {
    path: PathBuf,
}

impl DuckDbEventsStore {
    fn from_backend(events: &crate::devql_config::EventsBackendConfig) -> Self {
        Self {
            path: events.duckdb_path_or_default(),
        }
    }

    fn open_connection(&self) -> Result<duckdb::Connection> {
        if let Some(parent) = self.path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("creating DuckDB directory for events store: {}", parent.display())
            })?;
        }

        duckdb::Connection::open(&self.path)
            .with_context(|| format!("opening DuckDB events database at {}", self.path.display()))
    }

    fn query_single_i32(&self, sql: &str) -> Result<i32> {
        let conn = self.open_connection()?;
        let mut stmt = conn
            .prepare(sql)
            .with_context(|| format!("preparing DuckDB query: {sql}"))?;
        let mut rows = stmt
            .query([])
            .with_context(|| format!("executing DuckDB query: {sql}"))?;
        let row = rows
            .next()
            .context("iterating DuckDB query result")?
            .ok_or_else(|| anyhow!("DuckDB query returned no rows"))?;
        let value: i32 = row.get(0).context("reading DuckDB i32 result")?;
        Ok(value)
    }

    fn query_rows_as_strings(&self, sql: &str, column_count: usize) -> Result<Vec<Vec<String>>> {
        let conn = self.open_connection()?;
        let mut stmt = conn
            .prepare(sql)
            .with_context(|| format!("preparing DuckDB query: {sql}"))?;
        let mut rows = stmt
            .query([])
            .with_context(|| format!("executing DuckDB query: {sql}"))?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().context("iterating DuckDB query rows")? {
            let mut values = Vec::with_capacity(column_count);
            for idx in 0..column_count {
                let value: Option<String> = row
                    .get(idx)
                    .with_context(|| format!("reading DuckDB text column {}", idx + 1))?;
                values.push(value.unwrap_or_default());
            }
            out.push(values);
        }
        Ok(out)
    }

    fn execute_batch(&self, sql: &str) -> Result<()> {
        let conn = self.open_connection()?;
        conn.execute_batch(sql).context("executing DuckDB statements")?;
        Ok(())
    }

    async fn query_single_i32_blocking(&self, sql: &str) -> Result<i32> {
        let store = self.clone();
        let statement = sql.to_string();
        tokio::task::spawn_blocking(move || store.query_single_i32(&statement))
            .await
            .context("joining DuckDB scalar query task")?
    }

    async fn query_rows_as_strings_blocking(
        &self,
        sql: &str,
        column_count: usize,
    ) -> Result<Vec<Vec<String>>> {
        let store = self.clone();
        let statement = sql.to_string();
        tokio::task::spawn_blocking(move || store.query_rows_as_strings(&statement, column_count))
            .await
            .context("joining DuckDB query task")?
    }

    async fn execute_batch_blocking(&self, sql: &str) -> Result<()> {
        let store = self.clone();
        let statement = sql.to_string();
        tokio::task::spawn_blocking(move || store.execute_batch(&statement))
            .await
            .context("joining DuckDB execution task")?
    }
}

impl EventsStore for DuckDbEventsStore {
    fn provider(&self) -> EventsProvider {
        EventsProvider::DuckDb
    }

    fn ping<'a>(&'a self) -> StoreFuture<'a, i32> {
        Box::pin(async move { self.query_single_i32_blocking("SELECT 1").await })
    }

    fn init_schema<'a>(&'a self) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            let sql = r#"
CREATE TABLE IF NOT EXISTS checkpoint_events (
    event_id VARCHAR,
    event_time TIMESTAMP,
    repo_id VARCHAR,
    checkpoint_id VARCHAR,
    session_id VARCHAR,
    commit_sha VARCHAR,
    branch VARCHAR,
    event_type VARCHAR,
    agent VARCHAR,
    strategy VARCHAR,
    files_touched_json VARCHAR,
    payload_json VARCHAR
);

CREATE UNIQUE INDEX IF NOT EXISTS checkpoint_events_event_id_uq
ON checkpoint_events(event_id);

CREATE INDEX IF NOT EXISTS checkpoint_events_repo_time_idx
ON checkpoint_events(repo_id, event_time);

CREATE INDEX IF NOT EXISTS checkpoint_events_repo_commit_idx
ON checkpoint_events(repo_id, commit_sha);
"#;

            self.execute_batch_blocking(sql)
                .await
                .context("creating DuckDB checkpoint_events schema")?;
            Ok(())
        })
    }

    fn existing_event_ids<'a>(&'a self, repo_id: String) -> StoreFuture<'a, HashSet<String>> {
        Box::pin(async move {
            let sql = format!(
                "SELECT event_id FROM checkpoint_events WHERE repo_id = '{}'",
                esc_duck(&repo_id)
            );
            let rows = self.query_rows_as_strings_blocking(&sql, 1).await?;
            Ok(rows
                .into_iter()
                .filter_map(|row| row.first().cloned())
                .filter(|value| !value.trim().is_empty())
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
                if let Some(commit_unix) = event.commit_unix {
                    format!(
                        "coalesce(try_cast('{}' AS TIMESTAMP), to_timestamp({}), now())",
                        esc_duck(created_at),
                        commit_unix
                    )
                } else {
                    format!(
                        "coalesce(try_cast('{}' AS TIMESTAMP), now())",
                        esc_duck(created_at)
                    )
                }
            } else if let Some(commit_unix) = event.commit_unix {
                format!("to_timestamp({commit_unix})")
            } else {
                "now()".to_string()
            };

            let files_touched_json = esc_duck(&serde_json::to_string(&event.files_touched)?);
            let payload_json = esc_duck(&serde_json::to_string(&event.payload)?);
            let sql = format!(
                "INSERT INTO checkpoint_events (event_id, event_time, repo_id, checkpoint_id, session_id, commit_sha, branch, event_type, agent, strategy, files_touched_json, payload_json) \
VALUES ('{}', {}, '{}', '{}', '{}', '{}', '{}', '{}', '{}', '{}', '{}', '{}')",
                esc_duck(&event.event_id),
                event_time_expr,
                esc_duck(&event.repo_id),
                esc_duck(&event.checkpoint_id),
                esc_duck(&event.session_id),
                esc_duck(&event.commit_sha),
                esc_duck(&event.branch),
                esc_duck(&event.event_type),
                esc_duck(&event.agent),
                esc_duck(&event.strategy),
                files_touched_json,
                payload_json
            );

            self.execute_batch_blocking(&sql)
                .await
                .context("inserting checkpoint event into DuckDB")?;
            Ok(())
        })
    }

    fn query_checkpoints<'a>(&'a self, query: EventsCheckpointQuery) -> StoreFuture<'a, Vec<Value>> {
        Box::pin(async move {
            let mut conditions = vec![
                format!("repo_id = '{}'", esc_duck(&query.repo_id)),
                "event_type = 'checkpoint_committed'".to_string(),
            ];
            if let Some(agent) = query.agent.as_deref() {
                conditions.push(format!("agent = '{}'", esc_duck(agent)));
            }
            if let Some(since) = query.since.as_deref() {
                conditions.push(format!(
                    "event_time >= coalesce(try_cast('{}' AS TIMESTAMP), timestamp '1970-01-01 00:00:00')",
                    esc_duck(since)
                ));
            }

            let sql = format!(
                "SELECT checkpoint_id, strftime(event_time, '%Y-%m-%dT%H:%M:%S.%fZ') AS created_at, agent, commit_sha, branch, strategy, files_touched_json \
FROM ( \
  SELECT checkpoint_id, event_time, agent, commit_sha, branch, strategy, files_touched_json, event_id, \
         row_number() OVER (PARTITION BY checkpoint_id ORDER BY event_time DESC, event_id DESC) AS rn \
  FROM checkpoint_events \
  WHERE {} \
) latest \
WHERE rn = 1 \
ORDER BY event_time DESC \
LIMIT {}",
                conditions.join(" AND "),
                query.limit.max(1)
            );

            let rows = self.query_rows_as_strings_blocking(&sql, 7).await?;
            Ok(rows
                .into_iter()
                .map(|row| {
                    json!({
                        "checkpoint_id": row.first().cloned().unwrap_or_default(),
                        "created_at": row.get(1).cloned().unwrap_or_default(),
                        "agent": row.get(2).cloned().unwrap_or_default(),
                        "commit_sha": row.get(3).cloned().unwrap_or_default(),
                        "branch": row.get(4).cloned().unwrap_or_default(),
                        "strategy": row.get(5).cloned().unwrap_or_default(),
                        "files_touched": parse_json_string_array(row.get(6).cloned().unwrap_or_default()),
                    })
                })
                .collect())
        })
    }

    fn query_telemetry<'a>(&'a self, query: EventsTelemetryQuery) -> StoreFuture<'a, Vec<Value>> {
        Box::pin(async move {
            let mut conditions = vec![format!("repo_id = '{}'", esc_duck(&query.repo_id))];
            if let Some(event_type) = query.event_type.as_deref() {
                conditions.push(format!("event_type = '{}'", esc_duck(event_type)));
            }
            if let Some(agent) = query.agent.as_deref() {
                conditions.push(format!("agent = '{}'", esc_duck(agent)));
            }
            if let Some(since) = query.since.as_deref() {
                conditions.push(format!(
                    "event_time >= coalesce(try_cast('{}' AS TIMESTAMP), timestamp '1970-01-01 00:00:00')",
                    esc_duck(since)
                ));
            }

            let sql = format!(
                "SELECT strftime(event_time, '%Y-%m-%dT%H:%M:%S.%fZ') AS event_time, event_type, checkpoint_id, session_id, agent, commit_sha, branch, strategy, files_touched_json, payload_json \
FROM checkpoint_events \
WHERE {} \
ORDER BY event_time DESC, event_id DESC \
LIMIT {}",
                conditions.join(" AND "),
                query.limit.max(1)
            );
            let rows = self.query_rows_as_strings_blocking(&sql, 10).await?;
            Ok(rows
                .into_iter()
                .map(|row| {
                    json!({
                        "event_time": row.first().cloned().unwrap_or_default(),
                        "event_type": row.get(1).cloned().unwrap_or_default(),
                        "checkpoint_id": row.get(2).cloned().unwrap_or_default(),
                        "session_id": row.get(3).cloned().unwrap_or_default(),
                        "agent": row.get(4).cloned().unwrap_or_default(),
                        "commit_sha": row.get(5).cloned().unwrap_or_default(),
                        "branch": row.get(6).cloned().unwrap_or_default(),
                        "strategy": row.get(7).cloned().unwrap_or_default(),
                        "files_touched": parse_json_string_array(row.get(8).cloned().unwrap_or_default()),
                        "payload": row.get(9).cloned().unwrap_or_default(),
                    })
                })
                .collect())
        })
    }

    fn query_commit_shas<'a>(&'a self, query: EventsCommitShaQuery) -> StoreFuture<'a, Vec<String>> {
        Box::pin(async move {
            let mut conditions = vec![
                format!("repo_id = '{}'", esc_duck(&query.repo_id)),
                "event_type = 'checkpoint_committed'".to_string(),
                "commit_sha != ''".to_string(),
            ];
            if let Some(agent) = query.agent.as_deref() {
                conditions.push(format!("agent = '{}'", esc_duck(agent)));
            }
            if let Some(since) = query.since.as_deref() {
                conditions.push(format!(
                    "event_time >= coalesce(try_cast('{}' AS TIMESTAMP), timestamp '1970-01-01 00:00:00')",
                    esc_duck(since)
                ));
            }

            let sql = format!(
                "SELECT DISTINCT commit_sha FROM checkpoint_events WHERE {} LIMIT {}",
                conditions.join(" AND "),
                query.limit.max(1)
            );
            let rows = self.query_rows_as_strings_blocking(&sql, 1).await?;
            Ok(rows
                .into_iter()
                .filter_map(|row| row.first().cloned())
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
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

            let mut conditions = vec![
                format!("repo_id = '{}'", esc_duck(&query.repo_id)),
                "event_type = 'checkpoint_committed'".to_string(),
                format!("commit_sha IN ({})", sql_string_list_pg(&query.commit_shas)),
            ];

            if !query.path_candidates.is_empty() {
                let mut terms = Vec::with_capacity(query.path_candidates.len());
                for candidate in &query.path_candidates {
                    let encoded = serde_json::to_string(candidate)
                        .context("serializing DuckDB path candidate")?;
                    terms.push(format!(
                        "files_touched_json LIKE '%{}%'",
                        esc_duck(&encoded)
                    ));
                }
                conditions.push(format!("({})", terms.join(" OR ")));
            }

            let sql = format!(
                "SELECT strftime(event_time, '%Y-%m-%dT%H:%M:%S.%fZ') AS event_time, checkpoint_id, session_id, agent, commit_sha, branch, strategy \
FROM checkpoint_events \
WHERE {} \
ORDER BY event_time DESC, event_id DESC \
LIMIT {}",
                conditions.join(" AND "),
                query.limit.max(1)
            );
            let rows = self.query_rows_as_strings_blocking(&sql, 7).await?;
            Ok(rows
                .into_iter()
                .map(|row| {
                    json!({
                        "event_time": row.first().cloned().unwrap_or_default(),
                        "checkpoint_id": row.get(1).cloned().unwrap_or_default(),
                        "session_id": row.get(2).cloned().unwrap_or_default(),
                        "agent": row.get(3).cloned().unwrap_or_default(),
                        "commit_sha": row.get(4).cloned().unwrap_or_default(),
                        "branch": row.get(5).cloned().unwrap_or_default(),
                        "strategy": row.get(6).cloned().unwrap_or_default(),
                    })
                })
                .collect())
        })
    }
}

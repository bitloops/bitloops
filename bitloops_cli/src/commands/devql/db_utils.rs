use self::store_contracts::{
    CheckpointEventWrite, EventsCheckpointHistoryQuery, EventsCheckpointQuery,
    EventsCommitShaQuery, EventsStore, EventsTelemetryQuery, StoreFuture,
};
use rusqlite::types::ValueRef;

#[derive(Debug)]
struct PostgresRelationalStore {
    client: tokio_postgres::Client,
}

impl PostgresRelationalStore {
    async fn connect(dsn: &str) -> Result<Self> {
        Ok(Self {
            client: connect_postgres_client(dsn).await?,
        })
    }
}

impl store_contracts::RelationalStore for PostgresRelationalStore {
    fn provider(&self) -> RelationalProvider {
        RelationalProvider::Postgres
    }

    fn ping<'a>(&'a self) -> store_contracts::StoreFuture<'a, i32> {
        Box::pin(async move { run_postgres_ping(&self.client).await })
    }

    fn init_schema<'a>(&'a self) -> store_contracts::StoreFuture<'a, ()> {
        Box::pin(async move {
            postgres_exec(&self.client, RELATIONAL_SCHEMA_SQL)
                .await
                .context("creating Postgres DevQL tables")
        })
    }

    fn execute<'a>(&'a self, sql: &'a str) -> store_contracts::StoreFuture<'a, ()> {
        Box::pin(async move { postgres_exec(&self.client, sql).await })
    }

    fn query_rows<'a>(&'a self, sql: &'a str) -> store_contracts::StoreFuture<'a, Vec<Value>> {
        Box::pin(async move { pg_query_rows(&self.client, sql).await })
    }
}

#[derive(Debug, Clone)]
struct SqliteRelationalStore {
    db_path: PathBuf,
}

impl SqliteRelationalStore {
    async fn connect(db_path: PathBuf) -> Result<Self> {
        ensure_sqlite_parent_dir(&db_path)?;
        run_sqlite_ping(db_path.clone()).await?;
        Ok(Self { db_path })
    }
}

impl store_contracts::RelationalStore for SqliteRelationalStore {
    fn provider(&self) -> RelationalProvider {
        RelationalProvider::Sqlite
    }

    fn ping<'a>(&'a self) -> store_contracts::StoreFuture<'a, i32> {
        let db_path = self.db_path.clone();
        Box::pin(async move { run_sqlite_ping(db_path).await })
    }

    fn init_schema<'a>(&'a self) -> store_contracts::StoreFuture<'a, ()> {
        let db_path = self.db_path.clone();
        Box::pin(async move {
            sqlite_exec(db_path, RELATIONAL_SCHEMA_SQL.to_string())
                .await
                .context("creating SQLite DevQL tables")
        })
    }

    fn execute<'a>(&'a self, sql: &'a str) -> store_contracts::StoreFuture<'a, ()> {
        let db_path = self.db_path.clone();
        let statement = sql.to_string();
        Box::pin(async move { sqlite_exec(db_path, statement).await })
    }

    fn query_rows<'a>(&'a self, sql: &'a str) -> store_contracts::StoreFuture<'a, Vec<Value>> {
        let db_path = self.db_path.clone();
        let statement = sql.to_string();
        Box::pin(async move { sqlite_query_rows(db_path, statement).await })
    }
}

async fn postgres_exec(pg_client: &tokio_postgres::Client, sql: &str) -> Result<()> {
    run_postgres_exec(pg_client, sql).await
}

async fn pg_query_rows(pg_client: &tokio_postgres::Client, sql: &str) -> Result<Vec<Value>> {
    let wrapped = format!(
        "SELECT coalesce(json_agg(t), '[]'::json)::text FROM ({}) t",
        sql.trim().trim_end_matches(';')
    );
    let raw = run_postgres_query_scalar_text(pg_client, &wrapped).await?;
    let parsed: Value = serde_json::from_str(raw.trim()).with_context(|| {
        format!(
            "parsing Postgres JSON payload failed: {}",
            truncate_for_error(&raw)
        )
    })?;
    match parsed {
        Value::Array(rows) => Ok(rows),
        Value::Object(_) => Ok(vec![parsed]),
        Value::Null => Ok(vec![]),
        other => bail!("unexpected Postgres JSON payload type: {other}"),
    }
}

async fn run_postgres_exec(pg_client: &tokio_postgres::Client, sql: &str) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(30), pg_client.batch_execute(sql))
        .await
        .context("Postgres statement timeout after 30s")?
        .context("executing Postgres statements")?;
    Ok(())
}

async fn run_postgres_query_scalar_text(
    pg_client: &tokio_postgres::Client,
    sql: &str,
) -> Result<String> {
    let row = tokio::time::timeout(Duration::from_secs(30), pg_client.query_one(sql, &[]))
        .await
        .context("Postgres query timeout after 30s")?
        .context("executing Postgres query")?;
    let value: String = row
        .try_get(0)
        .context("reading Postgres scalar text result")?;
    Ok(value)
}

async fn run_postgres_ping(pg_client: &tokio_postgres::Client) -> Result<i32> {
    let row = tokio::time::timeout(Duration::from_secs(10), pg_client.query_one("SELECT 1", &[]))
        .await
        .context("Postgres health query timeout after 10s")?
        .context("running Postgres health query `SELECT 1`")?;
    let value: i32 = row
        .try_get(0)
        .context("reading Postgres health query result")?;
    Ok(value)
}

async fn run_sqlite_ping(db_path: PathBuf) -> Result<i32> {
    tokio::task::spawn_blocking(move || -> Result<i32> {
        let conn = open_sqlite_connection(&db_path)?;
        let value: i32 = conn
            .query_row("SELECT 1", [], |row| row.get(0))
            .context("running SQLite health query `SELECT 1`")?;
        Ok(value)
    })
    .await
    .context("joining SQLite health check task")?
}

async fn sqlite_exec(db_path: PathBuf, sql: String) -> Result<()> {
    tokio::task::spawn_blocking(move || -> Result<()> {
        let conn = open_sqlite_connection(&db_path)?;
        conn.execute_batch(&sql)
            .with_context(|| format!("executing SQLite statements: {}", truncate_for_error(&sql)))
    })
    .await
    .context("joining SQLite execution task")?
}

async fn sqlite_query_rows(db_path: PathBuf, sql: String) -> Result<Vec<Value>> {
    tokio::task::spawn_blocking(move || -> Result<Vec<Value>> {
        let conn = open_sqlite_connection(&db_path)?;
        let mut stmt = conn
            .prepare(&sql)
            .with_context(|| format!("preparing SQLite query: {}", truncate_for_error(&sql)))?;
        let column_names = stmt
            .column_names()
            .iter()
            .map(|name| (*name).to_string())
            .collect::<Vec<_>>();

        let mut rows = stmt.query([]).context("executing SQLite query")?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().context("reading SQLite row")? {
            let mut object = Map::new();
            for (idx, column_name) in column_names.iter().enumerate() {
                let value = row
                    .get_ref(idx)
                    .with_context(|| format!("reading SQLite column `{column_name}`"))?;
                object.insert(column_name.clone(), sqlite_value_to_json(value));
            }
            out.push(Value::Object(object));
        }

        Ok(out)
    })
    .await
    .context("joining SQLite query task")?
}

fn ensure_sqlite_parent_dir(db_path: &Path) -> Result<()> {
    let parent = db_path
        .parent()
        .filter(|candidate| !candidate.as_os_str().is_empty());
    if let Some(parent) = parent {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating SQLite parent directory {}", parent.display()))?;
    }
    Ok(())
}

fn open_sqlite_connection(db_path: &Path) -> Result<rusqlite::Connection> {
    ensure_sqlite_parent_dir(db_path)?;
    let conn = rusqlite::Connection::open(db_path)
        .with_context(|| format!("opening SQLite database at {}", db_path.display()))?;
    conn.busy_timeout(Duration::from_secs(30))
        .context("setting SQLite busy timeout")?;
    Ok(conn)
}

fn sqlite_value_to_json(value: ValueRef<'_>) -> Value {
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(v) => Value::Number(v.into()),
        ValueRef::Real(v) => serde_json::Number::from_f64(v)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        ValueRef::Text(v) => Value::String(String::from_utf8_lossy(v).to_string()),
        ValueRef::Blob(v) => Value::String(bytes_to_hex(v)),
    }
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn validate_postgres_sslmode_for_notls(dsn: &str, ssl_mode: SslMode) -> Result<()> {
    let dsn_lower = dsn.to_ascii_lowercase();
    if dsn_lower.contains("sslmode=verify-ca") || dsn_lower.contains("sslmode=verify-full") {
        bail!(
            "Postgres DSN requires certificate-based TLS (sslmode=verify-ca/verify-full), \
but this client is configured to use an unencrypted connection (NoTls). \
Use sslmode=disable if plaintext is acceptable, or configure a TLS-enabled Postgres client."
        );
    }

    match ssl_mode {
        SslMode::Disable | SslMode::Prefer => {}
        _ => {
            bail!(
                "Postgres DSN requires TLS (sslmode={:?}), \
but this client is configured to use an unencrypted connection (NoTls). \
Either adjust the DSN to use sslmode=disable if plaintext is acceptable, \
or configure a TLS-enabled Postgres client.",
                ssl_mode
            );
        }
    }
    Ok(())
}

async fn connect_postgres_client(dsn: &str) -> Result<tokio_postgres::Client> {
    let mut pg_cfg: tokio_postgres::Config = dsn.parse().context("parsing Postgres DSN")?;
    validate_postgres_sslmode_for_notls(dsn, pg_cfg.get_ssl_mode())?;

    pg_cfg.connect_timeout(Duration::from_secs(10));

    let (client, connection) = tokio::time::timeout(Duration::from_secs(10), pg_cfg.connect(NoTls))
        .await
        .context("Postgres connect timeout after 10s")?
        .context("connecting to Postgres")?;

    tokio::spawn(async move {
        if let Err(err) = connection.await {
            log::warn!("Postgres connection task ended: {err:#}");
        }
    });

    Ok(client)
}

fn truncate_for_error(input: &str) -> String {
    const MAX: usize = 500;
    let mut out = input.to_string();
    if out.len() > MAX {
        out.truncate(MAX);
        out.push_str("...");
    }
    out
}

fn resolve_events_store_from_backends(
    backends: &DevqlBackendConfig,
) -> Result<Box<dyn EventsStore + Send + Sync>> {
    match backends.events.provider {
        EventsProvider::DuckDb => Ok(Box::new(DuckDbEventsStore::from_backend(&backends.events))),
        EventsProvider::ClickHouse => Ok(Box::new(ClickHouseEventsStore::from_backend(
            &backends.events,
        ))),
    }
}

fn resolve_events_store(cfg: &DevqlConfig) -> Result<Box<dyn EventsStore + Send + Sync>> {
    resolve_events_store_from_backends(&cfg.backends)
}

fn resolve_events_store_for_connection(
    cfg: &DevqlConnectionConfig,
) -> Result<Box<dyn EventsStore + Send + Sync>> {
    resolve_events_store_from_backends(&cfg.backends)
}

async fn events_store_ping(cfg: &DevqlConnectionConfig) -> Result<i32> {
    let store = resolve_events_store_for_connection(cfg)?;
    store.ping().await
}

async fn events_store_init_schema(cfg: &DevqlConfig) -> Result<()> {
    let store = resolve_events_store(cfg)?;
    store.init_schema().await
}

async fn events_store_existing_event_ids(cfg: &DevqlConfig, repo_id: &str) -> Result<HashSet<String>> {
    let store = resolve_events_store(cfg)?;
    store.existing_event_ids(repo_id.to_string()).await
}

async fn events_store_insert_checkpoint_event(
    cfg: &DevqlConfig,
    event: CheckpointEventWrite,
) -> Result<()> {
    let store = resolve_events_store(cfg)?;
    store.insert_checkpoint_event(event).await
}

async fn events_store_query_checkpoints(
    cfg: &DevqlConfig,
    query: EventsCheckpointQuery,
) -> Result<Vec<Value>> {
    let store = resolve_events_store(cfg)?;
    store.query_checkpoints(query).await
}

async fn events_store_query_telemetry(
    cfg: &DevqlConfig,
    query: EventsTelemetryQuery,
) -> Result<Vec<Value>> {
    let store = resolve_events_store(cfg)?;
    store.query_telemetry(query).await
}

async fn events_store_query_commit_shas(
    cfg: &DevqlConfig,
    query: EventsCommitShaQuery,
) -> Result<Vec<String>> {
    let store = resolve_events_store(cfg)?;
    store.query_commit_shas(query).await
}

async fn events_store_query_checkpoint_events(
    cfg: &DevqlConfig,
    query: EventsCheckpointHistoryQuery,
) -> Result<Vec<Value>> {
    let store = resolve_events_store(cfg)?;
    store.query_checkpoint_events(query).await
}

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
}

impl EventsStore for DuckDbEventsStore {
    fn provider(&self) -> EventsProvider {
        EventsProvider::DuckDb
    }

    fn ping<'a>(&'a self) -> StoreFuture<'a, i32> {
        Box::pin(async move { self.query_single_i32("SELECT 1") })
    }

    fn init_schema<'a>(&'a self) -> StoreFuture<'a, ()> {
        Box::pin(async move {
            let conn = self.open_connection()?;
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

            conn.execute_batch(sql)
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
            let rows = self.query_rows_as_strings(&sql, 1)?;
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

            let conn = self.open_connection()?;
            conn.execute_batch(&sql)
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

            let rows = self.query_rows_as_strings(&sql, 7)?;
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
            let rows = self.query_rows_as_strings(&sql, 10)?;
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
            let rows = self.query_rows_as_strings(&sql, 1)?;
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
            let rows = self.query_rows_as_strings(&sql, 7)?;
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

fn esc_pg(value: &str) -> String {
    value.replace('\'', "''")
}

fn esc_ch(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

fn esc_duck(value: &str) -> String {
    value.replace('\'', "''")
}

fn parse_json_string_array(raw: String) -> Value {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Value::Array(vec![]);
    }

    match serde_json::from_str::<Value>(trimmed) {
        Ok(Value::Array(values)) => Value::Array(values),
        _ => Value::Array(vec![]),
    }
}

fn normalize_repo_path(path: &str) -> String {
    let mut normalized = path.trim().replace('\\', "/");
    while normalized.starts_with("./") {
        normalized = normalized[2..].to_string();
    }
    normalized.trim_start_matches('/').to_string()
}

fn build_path_candidates(path: &str) -> Vec<String> {
    let mut out = Vec::new();
    let raw = path.trim();
    if !raw.is_empty() {
        out.push(raw.to_string());
    }

    let normalized = normalize_repo_path(raw);
    if !normalized.is_empty() {
        out.push(normalized.clone());
        out.push(format!("./{normalized}"));
    }

    out.sort();
    out.dedup();
    out
}

fn sql_path_candidates_clause(column: &str, candidates: &[String]) -> String {
    if candidates.is_empty() {
        return "1=0".to_string();
    }

    candidates
        .iter()
        .map(|candidate| format!("{column} = '{}'", esc_pg(candidate)))
        .collect::<Vec<_>>()
        .join(" OR ")
}

fn format_ch_array(values: &[String]) -> String {
    if values.is_empty() {
        return "[]".to_string();
    }

    let parts = values
        .iter()
        .map(|value| format!("'{}'", esc_ch(value)))
        .collect::<Vec<_>>();
    format!("[{}]", parts.join(","))
}

fn glob_to_sql_like(glob: &str) -> String {
    glob.replace("**", "%").replace('*', "%")
}

fn deterministic_uuid(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    let digest = format!("{:x}", hasher.finalize());

    let hex = &digest[..32];
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

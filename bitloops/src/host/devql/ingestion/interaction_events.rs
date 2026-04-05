use super::*;

// ── Events backend abstraction ──────────────────────────────────────────────

#[derive(Debug, Clone)]
enum InteractionEventsStoreInner {
    ClickHouse {
        endpoint: String,
        user: Option<String>,
        password: Option<String>,
    },
    DuckDb {
        path: PathBuf,
    },
}

#[derive(Debug, Clone)]
struct InteractionEventsStore {
    inner: InteractionEventsStoreInner,
}

impl InteractionEventsStore {
    fn from_config(cfg: &DevqlConfig, events_cfg: &EventsBackendConfig) -> Self {
        if events_cfg.has_clickhouse() {
            Self {
                inner: InteractionEventsStoreInner::ClickHouse {
                    endpoint: cfg.clickhouse_endpoint(),
                    user: cfg.clickhouse_user.clone(),
                    password: cfg.clickhouse_password.clone(),
                },
            }
        } else {
            Self {
                inner: InteractionEventsStoreInner::DuckDb {
                    path: events_cfg.resolve_duckdb_db_path_for_repo(&cfg.repo_root),
                },
            }
        }
    }

    async fn insert_batch(&self, rows: &[InteractionEventRow], repo_id: &str) -> Result<()> {
        const CHUNK_SIZE: usize = 1000;
        for chunk in rows.chunks(CHUNK_SIZE) {
            match &self.inner {
                InteractionEventsStoreInner::DuckDb { path } => {
                    self.insert_duckdb_chunk(path, chunk, repo_id).await?;
                }
                InteractionEventsStoreInner::ClickHouse {
                    endpoint,
                    user,
                    password,
                } => {
                    self.insert_clickhouse_chunk(
                        endpoint,
                        user.as_deref(),
                        password.as_deref(),
                        chunk,
                        repo_id,
                    )
                    .await?;
                }
            }
        }
        Ok(())
    }

    async fn insert_duckdb_chunk(
        &self,
        path: &Path,
        chunk: &[InteractionEventRow],
        repo_id: &str,
    ) -> Result<()> {
        let mut values = Vec::with_capacity(chunk.len());
        for row in chunk {
            values.push(format!(
                "('{eid}', '{et}', '{rid}', '{sid}', '{tid}', '{evt}', '{at}', '{m}', '{p}')",
                eid = esc_pg(&row.event_id),
                et = esc_pg(&row.event_time),
                rid = esc_pg(repo_id),
                sid = esc_pg(&row.session_id),
                tid = esc_pg(&row.turn_id),
                evt = esc_pg(&row.event_type),
                at = esc_pg(&row.agent_type),
                m = esc_pg(&row.model),
                p = esc_pg(&row.payload),
            ));
        }
        let sql = format!(
            "INSERT OR IGNORE INTO interaction_events \
             (event_id, event_time, repo_id, session_id, turn_id, event_type, agent_type, model, payload) \
             VALUES {}",
            values.join(", ")
        );
        duckdb_exec_path_allow_create(path, &sql)
            .await
            .context("batch-inserting interaction events into DuckDB")
    }

    async fn insert_clickhouse_chunk(
        &self,
        endpoint: &str,
        user: Option<&str>,
        password: Option<&str>,
        chunk: &[InteractionEventRow],
        repo_id: &str,
    ) -> Result<()> {
        let mut values = Vec::with_capacity(chunk.len());
        for row in chunk {
            values.push(format!(
                "('{}', coalesce(parseDateTime64BestEffortOrNull('{}'), now64(3)), '{}', '{}', '{}', '{}', '{}', '{}', '{}')",
                esc_ch(&row.event_id),
                esc_ch(&row.event_time),
                esc_ch(repo_id),
                esc_ch(&row.session_id),
                esc_ch(&row.turn_id),
                esc_ch(&row.event_type),
                esc_ch(&row.agent_type),
                esc_ch(&row.model),
                esc_ch(&row.payload),
            ));
        }
        let sql = format!(
            "INSERT INTO interaction_events \
             (event_id, event_time, repo_id, session_id, turn_id, event_type, agent_type, model, payload) \
             VALUES {}",
            values.join(", ")
        );
        run_clickhouse_sql_http(endpoint, user, password, &sql)
            .await
            .map(|_| ())
            .context("batch-inserting interaction events into ClickHouse")
    }
}

// ── Public ingestion entry point ────────────────────────────────────────────

/// Read interaction events from the checkpoint SQLite and insert them into
/// the configured events backend (DuckDB or ClickHouse).
///
/// Idempotency: DuckDB uses `INSERT OR IGNORE`; ClickHouse uses
/// `ReplacingMergeTree` deduplication. Returns the number of source rows
/// attempted (not the actual insert count — duplicates are silently skipped).
pub(super) async fn ingest_interaction_events(
    checkpoint_sqlite: &crate::storage::SqliteConnectionPool,
    cfg: &DevqlConfig,
    events_cfg: &EventsBackendConfig,
    repo_id: &str,
) -> Result<usize> {
    let rows = read_interaction_events_from_sqlite(checkpoint_sqlite, repo_id)?;
    if rows.is_empty() {
        return Ok(0);
    }

    let attempted = rows.len();
    let store = InteractionEventsStore::from_config(cfg, events_cfg);
    store
        .insert_batch(&rows, repo_id)
        .await
        .context("ingesting interaction events")?;

    Ok(attempted)
}

// ── SQLite reader ───────────────────────────────────────────────────────────

#[derive(Debug)]
struct InteractionEventRow {
    event_id: String,
    session_id: String,
    turn_id: String,
    event_type: String,
    event_time: String,
    agent_type: String,
    model: String,
    payload: String,
}

fn read_interaction_events_from_sqlite(
    sqlite: &crate::storage::SqliteConnectionPool,
    repo_id: &str,
) -> Result<Vec<InteractionEventRow>> {
    let repo_id = repo_id.to_string();
    sqlite.with_connection(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT event_id, session_id, COALESCE(turn_id, ''), event_type, event_time, \
                 COALESCE(agent_type, ''), COALESCE(model, ''), COALESCE(payload, '{}') \
                 FROM interaction_events \
                 WHERE repo_id = ?1",
            )
            .context("preparing interaction_events query")?;

        let rows = stmt
            .query_map(rusqlite::params![repo_id], |row| {
                Ok(InteractionEventRow {
                    event_id: row.get(0)?,
                    session_id: row.get(1)?,
                    turn_id: row.get(2)?,
                    event_type: row.get(3)?,
                    event_time: row.get(4)?,
                    agent_type: row.get(5)?,
                    model: row.get(6)?,
                    payload: row.get(7)?,
                })
            })
            .context("querying interaction_events")?
            .collect::<Result<Vec<_>, _>>()
            .context("collecting interaction_events rows")?;

        Ok(rows)
    })
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_checkpoint_sqlite(dir: &Path) -> crate::storage::SqliteConnectionPool {
        let sqlite_path = dir.join("checkpoints.sqlite");
        let pool =
            crate::storage::SqliteConnectionPool::connect(sqlite_path).expect("connect sqlite");
        pool.initialise_checkpoint_schema()
            .expect("initialise checkpoint schema");
        pool
    }

    fn create_duckdb_with_schema(dir: &Path) -> PathBuf {
        let duckdb_path = dir.join("events.duckdb");
        let conn = duckdb::Connection::open(&duckdb_path).expect("open duckdb");
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS interaction_events (
                event_id VARCHAR PRIMARY KEY,
                event_time VARCHAR,
                repo_id VARCHAR,
                session_id VARCHAR,
                turn_id VARCHAR,
                event_type VARCHAR,
                agent_type VARCHAR,
                model VARCHAR,
                payload VARCHAR
            );",
        )
        .expect("create interaction_events table in duckdb");
        duckdb_path
    }

    struct TestEvent<'a> {
        event_id: &'a str,
        session_id: &'a str,
        turn_id: &'a str,
        repo_id: &'a str,
        event_type: &'a str,
        event_time: &'a str,
        agent_type: &'a str,
        model: &'a str,
        payload: &'a str,
    }

    fn insert_sqlite_event(pool: &crate::storage::SqliteConnectionPool, evt: &TestEvent<'_>) {
        pool.with_connection(|conn| {
            conn.execute(
                "INSERT INTO interaction_events \
                 (event_id, session_id, turn_id, repo_id, event_type, event_time, agent_type, model, payload) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                rusqlite::params![
                    evt.event_id,
                    evt.session_id,
                    evt.turn_id,
                    evt.repo_id,
                    evt.event_type,
                    evt.event_time,
                    evt.agent_type,
                    evt.model,
                    evt.payload
                ],
            )
            .context("inserting test interaction event")?;
            Ok(())
        })
        .expect("insert sqlite event");
    }

    fn count_duckdb_rows(duckdb_path: &Path, repo_id: &str) -> usize {
        let conn = duckdb::Connection::open(duckdb_path).expect("open duckdb");
        let count: i64 = conn
            .query_row(
                &format!(
                    "SELECT COUNT(*) FROM interaction_events WHERE repo_id = '{}'",
                    repo_id.replace('\'', "''")
                ),
                [],
                |row| row.get(0),
            )
            .expect("count duckdb rows");
        count as usize
    }

    fn test_duckdb_store(dir: &Path) -> InteractionEventsStore {
        let duckdb_path = create_duckdb_with_schema(dir);
        InteractionEventsStore {
            inner: InteractionEventsStoreInner::DuckDb { path: duckdb_path },
        }
    }

    #[tokio::test]
    async fn ingest_duckdb_copies_rows() {
        let temp = TempDir::new().expect("temp dir");
        let sqlite = create_checkpoint_sqlite(temp.path());
        let store = test_duckdb_store(temp.path());

        insert_sqlite_event(
            &sqlite,
            &TestEvent {
                event_id: "evt-1",
                session_id: "sess-1",
                turn_id: "turn-1",
                repo_id: "repo-test",
                event_type: "session_start",
                event_time: "2025-01-01T00:00:00Z",
                agent_type: "claude",
                model: "opus",
                payload: r#"{"key":"value"}"#,
            },
        );
        insert_sqlite_event(
            &sqlite,
            &TestEvent {
                event_id: "evt-2",
                session_id: "sess-1",
                turn_id: "turn-1",
                repo_id: "repo-test",
                event_type: "turn_start",
                event_time: "2025-01-01T00:01:00Z",
                agent_type: "claude",
                model: "opus",
                payload: "{}",
            },
        );

        let rows = read_interaction_events_from_sqlite(&sqlite, "repo-test").unwrap();
        store
            .insert_batch(&rows, "repo-test")
            .await
            .expect("insert batch");

        let duckdb_path = match &store.inner {
            InteractionEventsStoreInner::DuckDb { path } => path.clone(),
            _ => unreachable!(),
        };
        assert_eq!(count_duckdb_rows(&duckdb_path, "repo-test"), 2);
    }

    #[tokio::test]
    async fn ingest_duckdb_is_idempotent() {
        let temp = TempDir::new().expect("temp dir");
        let sqlite = create_checkpoint_sqlite(temp.path());
        let store = test_duckdb_store(temp.path());

        insert_sqlite_event(
            &sqlite,
            &TestEvent {
                event_id: "evt-1",
                session_id: "sess-1",
                turn_id: "turn-1",
                repo_id: "repo-test",
                event_type: "session_start",
                event_time: "2025-01-01T00:00:00Z",
                agent_type: "claude",
                model: "opus",
                payload: "{}",
            },
        );

        let rows = read_interaction_events_from_sqlite(&sqlite, "repo-test").unwrap();
        store
            .insert_batch(&rows, "repo-test")
            .await
            .expect("first insert");
        store
            .insert_batch(&rows, "repo-test")
            .await
            .expect("second insert");

        let duckdb_path = match &store.inner {
            InteractionEventsStoreInner::DuckDb { path } => path.clone(),
            _ => unreachable!(),
        };
        assert_eq!(count_duckdb_rows(&duckdb_path, "repo-test"), 1);
    }

    #[tokio::test]
    async fn ingest_returns_zero_when_no_rows() {
        let temp = TempDir::new().expect("temp dir");
        let sqlite = create_checkpoint_sqlite(temp.path());

        let rows = read_interaction_events_from_sqlite(&sqlite, "repo-test").unwrap();
        assert!(rows.is_empty());
    }
}

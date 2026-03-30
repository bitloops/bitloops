use super::*;

pub(crate) async fn init_clickhouse_schema(cfg: &DevqlConfig) -> Result<()> {
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

    clickhouse_exec(cfg, sql)
        .await
        .context("creating ClickHouse checkpoint_events table")?;
    Ok(())
}

pub(crate) async fn init_duckdb_schema(
    repo_root: &Path,
    events_cfg: &EventsBackendConfig,
) -> Result<()> {
    let sql = r#"
CREATE TABLE IF NOT EXISTS checkpoint_events (
    event_id VARCHAR PRIMARY KEY,
    event_time VARCHAR,
    repo_id VARCHAR,
    checkpoint_id VARCHAR,
    session_id VARCHAR,
    commit_sha VARCHAR,
    branch VARCHAR,
    event_type VARCHAR,
    agent VARCHAR,
    strategy VARCHAR,
    files_touched VARCHAR,
    payload VARCHAR
);

CREATE INDEX IF NOT EXISTS checkpoint_events_repo_time_idx
ON checkpoint_events (repo_id, event_time);

CREATE INDEX IF NOT EXISTS checkpoint_events_repo_branch_time_idx
ON checkpoint_events (repo_id, branch, event_time);

CREATE INDEX IF NOT EXISTS checkpoint_events_repo_commit_idx
ON checkpoint_events (repo_id, commit_sha);

CREATE INDEX IF NOT EXISTS checkpoint_events_repo_branch_commit_idx
ON checkpoint_events (repo_id, branch, commit_sha);

CREATE INDEX IF NOT EXISTS checkpoint_events_repo_branch_event_time_idx
ON checkpoint_events (repo_id, branch, event_type, event_time);
"#;

    let duckdb_path = events_cfg.resolve_duckdb_db_path_for_repo(repo_root);
    duckdb_exec_path_allow_create(&duckdb_path, sql)
        .await
        .context("creating DuckDB checkpoint_events table")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::init_duckdb_schema;
    use crate::config::EventsBackendConfig;
    use tempfile::TempDir;

    #[tokio::test]
    async fn init_duckdb_schema_creates_branch_aware_event_indexes() {
        let repo = TempDir::new().expect("temp dir");
        let events_cfg = EventsBackendConfig {
            duckdb_path: Some(".bitloops/stores/events.duckdb".into()),
            clickhouse_url: None,
            clickhouse_user: None,
            clickhouse_password: None,
            clickhouse_database: None,
        };

        init_duckdb_schema(repo.path(), &events_cfg)
            .await
            .expect("initialise duckdb schema");

        let duckdb_path = events_cfg.resolve_duckdb_db_path_for_repo(repo.path());
        let conn = duckdb::Connection::open(duckdb_path).expect("open duckdb");
        let mut stmt = conn
            .prepare(
                "SELECT index_name
                 FROM duckdb_indexes()
                 WHERE table_name = 'checkpoint_events'
                 ORDER BY index_name",
            )
            .expect("prepare duckdb index query");
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .expect("query duckdb indexes")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect duckdb indexes");

        assert!(rows.contains(&"checkpoint_events_repo_time_idx".to_string()));
        assert!(rows.contains(&"checkpoint_events_repo_branch_time_idx".to_string()));
        assert!(rows.contains(&"checkpoint_events_repo_commit_idx".to_string()));
        assert!(rows.contains(&"checkpoint_events_repo_branch_commit_idx".to_string()));
        assert!(rows.contains(&"checkpoint_events_repo_branch_event_time_idx".to_string()));
    }
}

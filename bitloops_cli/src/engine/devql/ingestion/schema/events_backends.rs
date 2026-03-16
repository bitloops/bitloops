async fn init_clickhouse_schema(cfg: &DevqlConfig) -> Result<()> {
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

async fn init_duckdb_schema(events_cfg: &EventsBackendConfig) -> Result<()> {
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

CREATE INDEX IF NOT EXISTS checkpoint_events_repo_commit_idx
ON checkpoint_events (repo_id, commit_sha);
"#;

    duckdb_exec_path(&events_cfg.duckdb_path_or_default(), sql)
        .await
        .context("creating DuckDB checkpoint_events table")?;
    Ok(())
}

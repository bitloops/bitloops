use super::*;

pub(crate) async fn init_clickhouse_schema(cfg: &DevqlConfig) -> Result<()> {
    let checkpoint_events_sql = r#"
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

    clickhouse_exec(cfg, checkpoint_events_sql)
        .await
        .context("creating ClickHouse checkpoint_events table")?;

    let interaction_sessions_sql = r#"
CREATE TABLE IF NOT EXISTS interaction_sessions (
    session_id String,
    repo_id String,
    branch String,
    actor_id String,
    actor_name String,
    actor_email String,
    actor_source String,
    agent_type String,
    model String,
    first_prompt String,
    transcript_path String,
    worktree_path String,
    worktree_id String,
    started_at String,
    ended_at String,
    last_event_at String,
    updated_at DateTime64(3, 'UTC')
)
ENGINE = ReplacingMergeTree(updated_at)
ORDER BY (repo_id, session_id)
"#;
    clickhouse_exec(cfg, interaction_sessions_sql)
        .await
        .context("creating ClickHouse interaction_sessions table")?;

    let interaction_turns_sql = r#"
CREATE TABLE IF NOT EXISTS interaction_turns (
    turn_id String,
    session_id String,
    repo_id String,
    branch String,
    actor_id String,
    actor_name String,
    actor_email String,
    actor_source String,
    turn_number UInt32,
    prompt String,
    agent_type String,
    model String,
    started_at String,
    ended_at String,
    has_token_usage UInt8,
    input_tokens UInt64,
    cache_creation_tokens UInt64,
    cache_read_tokens UInt64,
    output_tokens UInt64,
    api_call_count UInt64,
    summary String,
    prompt_count UInt32,
    transcript_offset_start Nullable(Int64),
    transcript_offset_end Nullable(Int64),
    transcript_fragment String,
    files_modified Array(String),
    checkpoint_id String,
    updated_at DateTime64(3, 'UTC')
)
ENGINE = ReplacingMergeTree(updated_at)
ORDER BY (repo_id, session_id, turn_id)
"#;
    clickhouse_exec(cfg, interaction_turns_sql)
        .await
        .context("creating ClickHouse interaction_turns table")?;

    let interaction_events_sql = r#"
CREATE TABLE IF NOT EXISTS interaction_events (
    event_id String,
    event_time DateTime64(3, 'UTC'),
    repo_id String,
    session_id String,
    turn_id String,
    branch String,
    actor_id String,
    actor_name String,
    actor_email String,
    actor_source String,
    event_type String,
    agent_type String,
    model String,
    tool_use_id String,
    tool_kind String,
    task_description String,
    subagent_id String,
    payload String
)
ENGINE = ReplacingMergeTree(event_time)
ORDER BY (repo_id, event_time, event_id)
"#;

    clickhouse_exec(cfg, interaction_events_sql)
        .await
        .context("creating ClickHouse interaction_events table")?;
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

    let interaction_sql = r#"
CREATE TABLE IF NOT EXISTS interaction_sessions (
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

CREATE INDEX IF NOT EXISTS interaction_sessions_repo_idx
ON interaction_sessions (repo_id, last_event_at, started_at);

CREATE TABLE IF NOT EXISTS interaction_turns (
    turn_id VARCHAR,
    session_id VARCHAR,
    repo_id VARCHAR,
    branch VARCHAR,
    actor_id VARCHAR,
    actor_name VARCHAR,
    actor_email VARCHAR,
    actor_source VARCHAR,
    turn_number INTEGER,
    prompt VARCHAR,
    agent_type VARCHAR,
    model VARCHAR,
    started_at VARCHAR,
    ended_at VARCHAR,
    has_token_usage INTEGER,
    input_tokens BIGINT,
    cache_creation_tokens BIGINT,
    cache_read_tokens BIGINT,
    output_tokens BIGINT,
    api_call_count BIGINT,
    summary VARCHAR,
    prompt_count INTEGER,
    transcript_offset_start BIGINT,
    transcript_offset_end BIGINT,
    transcript_fragment VARCHAR,
    files_modified VARCHAR,
    checkpoint_id VARCHAR,
    updated_at VARCHAR,
    PRIMARY KEY (repo_id, turn_id)
);

CREATE INDEX IF NOT EXISTS interaction_turns_session_idx
ON interaction_turns (repo_id, session_id, turn_number, started_at);

CREATE INDEX IF NOT EXISTS interaction_turns_pending_idx
ON interaction_turns (repo_id, checkpoint_id, session_id, turn_number);

CREATE TABLE IF NOT EXISTS interaction_events (
    event_id VARCHAR PRIMARY KEY,
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
    agent_type VARCHAR,
    model VARCHAR,
    tool_use_id VARCHAR,
    tool_kind VARCHAR,
    task_description VARCHAR,
    subagent_id VARCHAR,
    payload VARCHAR
);

CREATE INDEX IF NOT EXISTS interaction_events_repo_time_idx
ON interaction_events (repo_id, event_time);

CREATE INDEX IF NOT EXISTS interaction_events_session_idx
ON interaction_events (repo_id, session_id, event_time);

CREATE INDEX IF NOT EXISTS interaction_events_type_idx
ON interaction_events (repo_id, event_type, event_time);
"#;

    duckdb_exec_path_allow_create(&duckdb_path, interaction_sql)
        .await
        .context("creating DuckDB interaction_events table")?;
    duckdb_exec_path_allow_create(&duckdb_path, knowledge_schema_sql_duckdb())
        .await
        .context("creating DuckDB knowledge_document_versions table")?;
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

    #[tokio::test]
    async fn init_duckdb_schema_creates_interaction_event_indexes() {
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
                 WHERE table_name IN ('interaction_sessions', 'interaction_turns', 'interaction_events')
                 ORDER BY index_name",
            )
            .expect("prepare duckdb index query");
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .expect("query duckdb indexes")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect duckdb indexes");

        assert!(rows.contains(&"interaction_sessions_repo_idx".to_string()));
        assert!(rows.contains(&"interaction_turns_session_idx".to_string()));
        assert!(rows.contains(&"interaction_turns_pending_idx".to_string()));
        assert!(rows.contains(&"interaction_events_repo_time_idx".to_string()));
        assert!(rows.contains(&"interaction_events_session_idx".to_string()));
        assert!(rows.contains(&"interaction_events_type_idx".to_string()));
    }

    #[tokio::test]
    async fn init_duckdb_schema_creates_knowledge_document_versions_table() {
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
        let table_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM information_schema.tables WHERE table_name = 'knowledge_document_versions'",
                [],
                |row| row.get(0),
            )
            .expect("count knowledge_document_versions table rows");

        assert_eq!(table_exists, 1);
    }
}

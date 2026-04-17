use anyhow::{Context, Result};

const INTERACTION_SESSIONS_TABLE_SQL: &str = r#"
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
)
"#;

const INTERACTION_TURNS_TABLE_SQL: &str = r#"
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
)
"#;

const INTERACTION_EVENTS_TABLE_SQL: &str = r#"
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
)
"#;

const INTERACTION_INDEX_SQL: &[&str] = &[
    "CREATE INDEX IF NOT EXISTS interaction_sessions_repo_idx ON interaction_sessions (repo_id, last_event_at, started_at)",
    "CREATE INDEX IF NOT EXISTS interaction_turns_session_idx ON interaction_turns (repo_id, session_id, turn_number, started_at)",
    "CREATE INDEX IF NOT EXISTS interaction_turns_pending_idx ON interaction_turns (repo_id, checkpoint_id, session_id, turn_number)",
    "CREATE INDEX IF NOT EXISTS interaction_events_repo_time_idx ON interaction_events (repo_id, event_time)",
    "CREATE INDEX IF NOT EXISTS interaction_events_session_idx ON interaction_events (repo_id, session_id, event_time)",
    "CREATE INDEX IF NOT EXISTS interaction_events_type_idx ON interaction_events (repo_id, event_type, event_time)",
];

pub(super) fn ensure_current_schema(conn: &duckdb::Connection) -> Result<()> {
    conn.execute_batch(INTERACTION_SESSIONS_TABLE_SQL)
        .context("creating DuckDB interaction_sessions table")?;
    conn.execute_batch(INTERACTION_TURNS_TABLE_SQL)
        .context("creating DuckDB interaction_turns table")?;
    conn.execute_batch(INTERACTION_EVENTS_TABLE_SQL)
        .context("creating DuckDB interaction_events table")?;
    ensure_promoted_columns(conn)?;
    ensure_turn_columns(conn)?;
    ensure_repo_scoped_primary_key(conn, "interaction_sessions", &["repo_id", "session_id"])?;
    ensure_repo_scoped_primary_key(conn, "interaction_turns", &["repo_id", "turn_id"])?;
    ensure_interaction_indexes(conn)?;
    Ok(())
}

fn ensure_promoted_columns(conn: &duckdb::Connection) -> Result<()> {
    let missing = [
        (
            "interaction_sessions",
            "branch",
            "ALTER TABLE interaction_sessions ADD COLUMN branch VARCHAR DEFAULT ''",
        ),
        (
            "interaction_sessions",
            "actor_id",
            "ALTER TABLE interaction_sessions ADD COLUMN actor_id VARCHAR DEFAULT ''",
        ),
        (
            "interaction_sessions",
            "actor_name",
            "ALTER TABLE interaction_sessions ADD COLUMN actor_name VARCHAR DEFAULT ''",
        ),
        (
            "interaction_sessions",
            "actor_email",
            "ALTER TABLE interaction_sessions ADD COLUMN actor_email VARCHAR DEFAULT ''",
        ),
        (
            "interaction_sessions",
            "actor_source",
            "ALTER TABLE interaction_sessions ADD COLUMN actor_source VARCHAR DEFAULT ''",
        ),
        (
            "interaction_turns",
            "branch",
            "ALTER TABLE interaction_turns ADD COLUMN branch VARCHAR DEFAULT ''",
        ),
        (
            "interaction_turns",
            "actor_id",
            "ALTER TABLE interaction_turns ADD COLUMN actor_id VARCHAR DEFAULT ''",
        ),
        (
            "interaction_turns",
            "actor_name",
            "ALTER TABLE interaction_turns ADD COLUMN actor_name VARCHAR DEFAULT ''",
        ),
        (
            "interaction_turns",
            "actor_email",
            "ALTER TABLE interaction_turns ADD COLUMN actor_email VARCHAR DEFAULT ''",
        ),
        (
            "interaction_turns",
            "actor_source",
            "ALTER TABLE interaction_turns ADD COLUMN actor_source VARCHAR DEFAULT ''",
        ),
        (
            "interaction_events",
            "branch",
            "ALTER TABLE interaction_events ADD COLUMN branch VARCHAR DEFAULT ''",
        ),
        (
            "interaction_events",
            "actor_id",
            "ALTER TABLE interaction_events ADD COLUMN actor_id VARCHAR DEFAULT ''",
        ),
        (
            "interaction_events",
            "actor_name",
            "ALTER TABLE interaction_events ADD COLUMN actor_name VARCHAR DEFAULT ''",
        ),
        (
            "interaction_events",
            "actor_email",
            "ALTER TABLE interaction_events ADD COLUMN actor_email VARCHAR DEFAULT ''",
        ),
        (
            "interaction_events",
            "actor_source",
            "ALTER TABLE interaction_events ADD COLUMN actor_source VARCHAR DEFAULT ''",
        ),
        (
            "interaction_events",
            "tool_use_id",
            "ALTER TABLE interaction_events ADD COLUMN tool_use_id VARCHAR DEFAULT ''",
        ),
        (
            "interaction_events",
            "tool_kind",
            "ALTER TABLE interaction_events ADD COLUMN tool_kind VARCHAR DEFAULT ''",
        ),
        (
            "interaction_events",
            "task_description",
            "ALTER TABLE interaction_events ADD COLUMN task_description VARCHAR DEFAULT ''",
        ),
        (
            "interaction_events",
            "subagent_id",
            "ALTER TABLE interaction_events ADD COLUMN subagent_id VARCHAR DEFAULT ''",
        ),
    ];
    for (table, column, alter_sql) in missing {
        let exists: i64 = conn
            .query_row(
                &format!(
                    "SELECT COUNT(*) FROM information_schema.columns \
                     WHERE table_name = '{table}' AND column_name = '{column}'"
                ),
                [],
                |row| row.get(0),
            )
            .with_context(|| format!("checking DuckDB interaction column {table}.{column}"))?;
        if exists == 0 {
            conn.execute_batch(alter_sql)
                .with_context(|| format!("adding DuckDB interaction column {table}.{column}"))?;
        }
    }
    Ok(())
}

fn ensure_turn_columns(conn: &duckdb::Connection) -> Result<()> {
    let missing = [
        (
            "summary",
            "ALTER TABLE interaction_turns ADD COLUMN summary VARCHAR DEFAULT ''",
        ),
        (
            "prompt_count",
            "ALTER TABLE interaction_turns ADD COLUMN prompt_count INTEGER DEFAULT 0",
        ),
        (
            "transcript_offset_start",
            "ALTER TABLE interaction_turns ADD COLUMN transcript_offset_start BIGINT",
        ),
        (
            "transcript_offset_end",
            "ALTER TABLE interaction_turns ADD COLUMN transcript_offset_end BIGINT",
        ),
        (
            "transcript_fragment",
            "ALTER TABLE interaction_turns ADD COLUMN transcript_fragment VARCHAR DEFAULT ''",
        ),
    ];
    for (column, alter_sql) in missing {
        let exists: i64 = conn
            .query_row(
                &format!(
                    "SELECT COUNT(*) FROM information_schema.columns \
                     WHERE table_name = 'interaction_turns' AND column_name = '{column}'"
                ),
                [],
                |row| row.get(0),
            )
            .with_context(|| format!("checking DuckDB interaction_turns.{column} column"))?;
        if exists == 0 {
            conn.execute_batch(alter_sql)
                .with_context(|| format!("adding DuckDB interaction_turns.{column} column"))?;
        }
    }
    Ok(())
}

fn ensure_repo_scoped_primary_key(
    conn: &duckdb::Connection,
    table: &str,
    expected_columns: &[&str],
) -> Result<()> {
    let actual_columns = duckdb_table_pk_columns(conn, table)?;
    let expected_columns = expected_columns
        .iter()
        .map(|column| column.to_string())
        .collect::<Vec<_>>();
    if actual_columns == expected_columns {
        return Ok(());
    }

    match table {
        "interaction_sessions" => rebuild_interaction_sessions_table(conn),
        "interaction_turns" => rebuild_interaction_turns_table(conn),
        _ => Ok(()),
    }
}

fn rebuild_interaction_sessions_table(conn: &duckdb::Connection) -> Result<()> {
    conn.execute_batch(
        r#"
BEGIN TRANSACTION;
CREATE TABLE interaction_sessions__repo_scoped_migration (
    session_id VARCHAR,
    repo_id VARCHAR,
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
INSERT INTO interaction_sessions__repo_scoped_migration (
    session_id, repo_id, agent_type, model, first_prompt,
    transcript_path, worktree_path, worktree_id, started_at,
    ended_at, last_event_at, updated_at
)
SELECT
    session_id, repo_id, agent_type, model, first_prompt,
    transcript_path, worktree_path, worktree_id, started_at,
    ended_at, last_event_at, updated_at
FROM interaction_sessions;
DROP TABLE interaction_sessions;
ALTER TABLE interaction_sessions__repo_scoped_migration RENAME TO interaction_sessions;
COMMIT;
"#,
    )
    .context("rebuilding DuckDB interaction_sessions with repo-scoped primary key")?;
    Ok(())
}

fn rebuild_interaction_turns_table(conn: &duckdb::Connection) -> Result<()> {
    conn.execute_batch(
        r#"
BEGIN TRANSACTION;
CREATE TABLE interaction_turns__repo_scoped_migration (
    turn_id VARCHAR,
    session_id VARCHAR,
    repo_id VARCHAR,
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
INSERT INTO interaction_turns__repo_scoped_migration (
    turn_id, session_id, repo_id, turn_number, prompt,
    agent_type, model, started_at, ended_at, has_token_usage,
    input_tokens, cache_creation_tokens, cache_read_tokens,
    output_tokens, api_call_count, summary, prompt_count,
    transcript_offset_start, transcript_offset_end, transcript_fragment,
    files_modified, checkpoint_id, updated_at
)
SELECT
    turn_id, session_id, repo_id, turn_number, prompt,
    agent_type, model, started_at, ended_at, has_token_usage,
    input_tokens, cache_creation_tokens, cache_read_tokens,
    output_tokens, api_call_count, summary, prompt_count,
    transcript_offset_start, transcript_offset_end, transcript_fragment,
    files_modified, checkpoint_id, updated_at
FROM interaction_turns;
DROP TABLE interaction_turns;
ALTER TABLE interaction_turns__repo_scoped_migration RENAME TO interaction_turns;
COMMIT;
"#,
    )
    .context("rebuilding DuckDB interaction_turns with repo-scoped primary key")?;
    Ok(())
}

fn ensure_interaction_indexes(conn: &duckdb::Connection) -> Result<()> {
    for sql in INTERACTION_INDEX_SQL {
        conn.execute_batch(sql)
            .with_context(|| format!("ensuring DuckDB interaction index: {sql}"))?;
    }
    Ok(())
}

pub(super) fn duckdb_table_pk_columns(
    conn: &duckdb::Connection,
    table: &str,
) -> Result<Vec<String>> {
    let sql = format!(
        "SELECT kcu.column_name
         FROM information_schema.table_constraints tc
         JOIN information_schema.key_column_usage kcu
           ON tc.constraint_name = kcu.constraint_name
          AND tc.constraint_schema = kcu.constraint_schema
         WHERE tc.table_name = '{table}'
           AND tc.constraint_type = 'PRIMARY KEY'
         ORDER BY kcu.ordinal_position"
    );
    let mut stmt = conn
        .prepare(&sql)
        .with_context(|| format!("preparing DuckDB primary key metadata query for `{table}`"))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .with_context(|| format!("querying DuckDB primary key metadata for `{table}`"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(anyhow::Error::from)
}

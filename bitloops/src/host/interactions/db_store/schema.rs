use std::collections::HashSet;

use anyhow::{Context, Result};

pub(super) const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS interaction_sessions (
    session_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    branch TEXT NOT NULL DEFAULT '',
    actor_id TEXT NOT NULL DEFAULT '',
    actor_name TEXT NOT NULL DEFAULT '',
    actor_email TEXT NOT NULL DEFAULT '',
    actor_source TEXT NOT NULL DEFAULT '',
    agent_type TEXT NOT NULL DEFAULT '',
    model TEXT NOT NULL DEFAULT '',
    first_prompt TEXT NOT NULL DEFAULT '',
    transcript_path TEXT NOT NULL DEFAULT '',
    worktree_path TEXT NOT NULL DEFAULT '',
    worktree_id TEXT NOT NULL DEFAULT '',
    started_at TEXT NOT NULL DEFAULT '',
    ended_at TEXT,
    last_event_at TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (repo_id, session_id)
);

CREATE INDEX IF NOT EXISTS interaction_sessions_repo_idx
ON interaction_sessions (repo_id, last_event_at, started_at);

CREATE TABLE IF NOT EXISTS interaction_turns (
    turn_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    branch TEXT NOT NULL DEFAULT '',
    actor_id TEXT NOT NULL DEFAULT '',
    actor_name TEXT NOT NULL DEFAULT '',
    actor_email TEXT NOT NULL DEFAULT '',
    actor_source TEXT NOT NULL DEFAULT '',
    turn_number INTEGER NOT NULL DEFAULT 0,
    prompt TEXT NOT NULL DEFAULT '',
    agent_type TEXT NOT NULL DEFAULT '',
    model TEXT NOT NULL DEFAULT '',
    started_at TEXT NOT NULL DEFAULT '',
    ended_at TEXT,
    has_token_usage INTEGER NOT NULL DEFAULT 0,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
    cache_read_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    api_call_count INTEGER NOT NULL DEFAULT 0,
    summary TEXT NOT NULL DEFAULT '',
    prompt_count INTEGER NOT NULL DEFAULT 0,
    transcript_offset_start INTEGER,
    transcript_offset_end INTEGER,
    transcript_fragment TEXT NOT NULL DEFAULT '',
    files_modified TEXT NOT NULL DEFAULT '[]',
    checkpoint_id TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (repo_id, turn_id)
);

CREATE INDEX IF NOT EXISTS interaction_turns_session_idx
ON interaction_turns (repo_id, session_id, turn_number, started_at);

CREATE INDEX IF NOT EXISTS interaction_turns_pending_idx
ON interaction_turns (repo_id, checkpoint_id, session_id, turn_number);

CREATE TABLE IF NOT EXISTS interaction_events (
    event_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    turn_id TEXT,
    repo_id TEXT NOT NULL,
    branch TEXT NOT NULL DEFAULT '',
    actor_id TEXT NOT NULL DEFAULT '',
    actor_name TEXT NOT NULL DEFAULT '',
    actor_email TEXT NOT NULL DEFAULT '',
    actor_source TEXT NOT NULL DEFAULT '',
    event_type TEXT NOT NULL,
    event_time TEXT NOT NULL,
    agent_type TEXT NOT NULL DEFAULT '',
    model TEXT NOT NULL DEFAULT '',
    tool_use_id TEXT NOT NULL DEFAULT '',
    tool_kind TEXT NOT NULL DEFAULT '',
    task_description TEXT NOT NULL DEFAULT '',
    subagent_id TEXT NOT NULL DEFAULT '',
    payload TEXT NOT NULL DEFAULT '{}',
    PRIMARY KEY (repo_id, event_id)
);

CREATE INDEX IF NOT EXISTS interaction_events_repo_time_idx
ON interaction_events (repo_id, event_time, event_id);

CREATE INDEX IF NOT EXISTS interaction_events_session_idx
ON interaction_events (repo_id, session_id, event_time, event_id);

CREATE INDEX IF NOT EXISTS interaction_events_tool_use_idx
ON interaction_events (repo_id, tool_use_id, event_time, event_id);

CREATE TABLE IF NOT EXISTS interaction_tool_uses (
    tool_use_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    session_id TEXT NOT NULL DEFAULT '',
    turn_id TEXT NOT NULL DEFAULT '',
    tool_kind TEXT NOT NULL DEFAULT '',
    task_description TEXT NOT NULL DEFAULT '',
    subagent_id TEXT NOT NULL DEFAULT '',
    transcript_path TEXT NOT NULL DEFAULT '',
    started_at TEXT,
    ended_at TEXT,
    updated_at TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (repo_id, tool_use_id)
);

CREATE INDEX IF NOT EXISTS interaction_tool_uses_session_idx
ON interaction_tool_uses (repo_id, session_id, turn_id, updated_at);

CREATE TABLE IF NOT EXISTS interaction_session_search_documents (
    session_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    started_at TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL DEFAULT '',
    prompt_text TEXT NOT NULL DEFAULT '',
    summary_text TEXT NOT NULL DEFAULT '',
    transcript_text TEXT NOT NULL DEFAULT '',
    tool_text TEXT NOT NULL DEFAULT '',
    paths_text TEXT NOT NULL DEFAULT '',
    combined_text TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (repo_id, session_id)
);

CREATE INDEX IF NOT EXISTS interaction_session_search_documents_time_idx
ON interaction_session_search_documents (repo_id, started_at, updated_at);

CREATE TABLE IF NOT EXISTS interaction_turn_search_documents (
    turn_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    session_id TEXT NOT NULL DEFAULT '',
    started_at TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL DEFAULT '',
    prompt_text TEXT NOT NULL DEFAULT '',
    summary_text TEXT NOT NULL DEFAULT '',
    transcript_text TEXT NOT NULL DEFAULT '',
    tool_text TEXT NOT NULL DEFAULT '',
    paths_text TEXT NOT NULL DEFAULT '',
    combined_text TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (repo_id, turn_id)
);

CREATE INDEX IF NOT EXISTS interaction_turn_search_documents_session_idx
ON interaction_turn_search_documents (repo_id, session_id, started_at, updated_at);

CREATE TABLE IF NOT EXISTS interaction_session_search_terms (
    repo_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    term TEXT NOT NULL,
    field TEXT NOT NULL,
    occurrences INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (repo_id, session_id, term, field)
);

CREATE INDEX IF NOT EXISTS interaction_session_search_terms_lookup_idx
ON interaction_session_search_terms (repo_id, term, field, session_id);

CREATE TABLE IF NOT EXISTS interaction_turn_search_terms (
    repo_id TEXT NOT NULL,
    turn_id TEXT NOT NULL,
    term TEXT NOT NULL,
    field TEXT NOT NULL,
    occurrences INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (repo_id, turn_id, term, field)
);

CREATE INDEX IF NOT EXISTS interaction_turn_search_terms_lookup_idx
ON interaction_turn_search_terms (repo_id, term, field, turn_id);

CREATE TABLE IF NOT EXISTS interaction_spool_queue (
    mutation_id INTEGER PRIMARY KEY AUTOINCREMENT,
    repo_id TEXT NOT NULL,
    mutation_type TEXT NOT NULL,
    payload TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    last_error TEXT NOT NULL DEFAULT '',
    created_at TEXT DEFAULT (datetime('now')),
    updated_at TEXT DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS interaction_spool_queue_repo_idx
ON interaction_spool_queue (repo_id, mutation_id);
"#;

pub(super) fn ensure_additive_columns(conn: &rusqlite::Connection) -> Result<()> {
    ensure_table_columns(
        conn,
        "interaction_sessions",
        &[
            (
                "repo_id",
                "ALTER TABLE interaction_sessions ADD COLUMN repo_id TEXT NOT NULL DEFAULT ''",
            ),
            (
                "branch",
                "ALTER TABLE interaction_sessions ADD COLUMN branch TEXT NOT NULL DEFAULT ''",
            ),
            (
                "actor_id",
                "ALTER TABLE interaction_sessions ADD COLUMN actor_id TEXT NOT NULL DEFAULT ''",
            ),
            (
                "actor_name",
                "ALTER TABLE interaction_sessions ADD COLUMN actor_name TEXT NOT NULL DEFAULT ''",
            ),
            (
                "actor_email",
                "ALTER TABLE interaction_sessions ADD COLUMN actor_email TEXT NOT NULL DEFAULT ''",
            ),
            (
                "actor_source",
                "ALTER TABLE interaction_sessions ADD COLUMN actor_source TEXT NOT NULL DEFAULT ''",
            ),
            (
                "agent_type",
                "ALTER TABLE interaction_sessions ADD COLUMN agent_type TEXT NOT NULL DEFAULT ''",
            ),
            (
                "model",
                "ALTER TABLE interaction_sessions ADD COLUMN model TEXT NOT NULL DEFAULT ''",
            ),
            (
                "first_prompt",
                "ALTER TABLE interaction_sessions ADD COLUMN first_prompt TEXT NOT NULL DEFAULT ''",
            ),
            (
                "transcript_path",
                "ALTER TABLE interaction_sessions ADD COLUMN transcript_path TEXT NOT NULL DEFAULT ''",
            ),
            (
                "worktree_path",
                "ALTER TABLE interaction_sessions ADD COLUMN worktree_path TEXT NOT NULL DEFAULT ''",
            ),
            (
                "worktree_id",
                "ALTER TABLE interaction_sessions ADD COLUMN worktree_id TEXT NOT NULL DEFAULT ''",
            ),
            (
                "started_at",
                "ALTER TABLE interaction_sessions ADD COLUMN started_at TEXT NOT NULL DEFAULT ''",
            ),
            (
                "ended_at",
                "ALTER TABLE interaction_sessions ADD COLUMN ended_at TEXT",
            ),
            (
                "last_event_at",
                "ALTER TABLE interaction_sessions ADD COLUMN last_event_at TEXT NOT NULL DEFAULT ''",
            ),
            (
                "updated_at",
                "ALTER TABLE interaction_sessions ADD COLUMN updated_at TEXT NOT NULL DEFAULT ''",
            ),
        ],
    )?;
    ensure_table_columns(
        conn,
        "interaction_turns",
        &[
            (
                "session_id",
                "ALTER TABLE interaction_turns ADD COLUMN session_id TEXT NOT NULL DEFAULT ''",
            ),
            (
                "repo_id",
                "ALTER TABLE interaction_turns ADD COLUMN repo_id TEXT NOT NULL DEFAULT ''",
            ),
            (
                "branch",
                "ALTER TABLE interaction_turns ADD COLUMN branch TEXT NOT NULL DEFAULT ''",
            ),
            (
                "actor_id",
                "ALTER TABLE interaction_turns ADD COLUMN actor_id TEXT NOT NULL DEFAULT ''",
            ),
            (
                "actor_name",
                "ALTER TABLE interaction_turns ADD COLUMN actor_name TEXT NOT NULL DEFAULT ''",
            ),
            (
                "actor_email",
                "ALTER TABLE interaction_turns ADD COLUMN actor_email TEXT NOT NULL DEFAULT ''",
            ),
            (
                "actor_source",
                "ALTER TABLE interaction_turns ADD COLUMN actor_source TEXT NOT NULL DEFAULT ''",
            ),
            (
                "turn_number",
                "ALTER TABLE interaction_turns ADD COLUMN turn_number INTEGER NOT NULL DEFAULT 0",
            ),
            (
                "prompt",
                "ALTER TABLE interaction_turns ADD COLUMN prompt TEXT NOT NULL DEFAULT ''",
            ),
            (
                "agent_type",
                "ALTER TABLE interaction_turns ADD COLUMN agent_type TEXT NOT NULL DEFAULT ''",
            ),
            (
                "model",
                "ALTER TABLE interaction_turns ADD COLUMN model TEXT NOT NULL DEFAULT ''",
            ),
            (
                "started_at",
                "ALTER TABLE interaction_turns ADD COLUMN started_at TEXT NOT NULL DEFAULT ''",
            ),
            (
                "ended_at",
                "ALTER TABLE interaction_turns ADD COLUMN ended_at TEXT",
            ),
            (
                "has_token_usage",
                "ALTER TABLE interaction_turns ADD COLUMN has_token_usage INTEGER NOT NULL DEFAULT 0",
            ),
            (
                "input_tokens",
                "ALTER TABLE interaction_turns ADD COLUMN input_tokens INTEGER NOT NULL DEFAULT 0",
            ),
            (
                "cache_creation_tokens",
                "ALTER TABLE interaction_turns ADD COLUMN cache_creation_tokens INTEGER NOT NULL DEFAULT 0",
            ),
            (
                "cache_read_tokens",
                "ALTER TABLE interaction_turns ADD COLUMN cache_read_tokens INTEGER NOT NULL DEFAULT 0",
            ),
            (
                "output_tokens",
                "ALTER TABLE interaction_turns ADD COLUMN output_tokens INTEGER NOT NULL DEFAULT 0",
            ),
            (
                "api_call_count",
                "ALTER TABLE interaction_turns ADD COLUMN api_call_count INTEGER NOT NULL DEFAULT 0",
            ),
            (
                "summary",
                "ALTER TABLE interaction_turns ADD COLUMN summary TEXT NOT NULL DEFAULT ''",
            ),
            (
                "prompt_count",
                "ALTER TABLE interaction_turns ADD COLUMN prompt_count INTEGER NOT NULL DEFAULT 0",
            ),
            (
                "transcript_offset_start",
                "ALTER TABLE interaction_turns ADD COLUMN transcript_offset_start INTEGER",
            ),
            (
                "transcript_offset_end",
                "ALTER TABLE interaction_turns ADD COLUMN transcript_offset_end INTEGER",
            ),
            (
                "transcript_fragment",
                "ALTER TABLE interaction_turns ADD COLUMN transcript_fragment TEXT NOT NULL DEFAULT ''",
            ),
            (
                "files_modified",
                "ALTER TABLE interaction_turns ADD COLUMN files_modified TEXT NOT NULL DEFAULT '[]'",
            ),
            (
                "checkpoint_id",
                "ALTER TABLE interaction_turns ADD COLUMN checkpoint_id TEXT NOT NULL DEFAULT ''",
            ),
            (
                "updated_at",
                "ALTER TABLE interaction_turns ADD COLUMN updated_at TEXT NOT NULL DEFAULT ''",
            ),
        ],
    )?;
    ensure_table_columns(
        conn,
        "interaction_events",
        &[
            (
                "session_id",
                "ALTER TABLE interaction_events ADD COLUMN session_id TEXT NOT NULL DEFAULT ''",
            ),
            (
                "turn_id",
                "ALTER TABLE interaction_events ADD COLUMN turn_id TEXT",
            ),
            (
                "repo_id",
                "ALTER TABLE interaction_events ADD COLUMN repo_id TEXT NOT NULL DEFAULT ''",
            ),
            (
                "branch",
                "ALTER TABLE interaction_events ADD COLUMN branch TEXT NOT NULL DEFAULT ''",
            ),
            (
                "actor_id",
                "ALTER TABLE interaction_events ADD COLUMN actor_id TEXT NOT NULL DEFAULT ''",
            ),
            (
                "actor_name",
                "ALTER TABLE interaction_events ADD COLUMN actor_name TEXT NOT NULL DEFAULT ''",
            ),
            (
                "actor_email",
                "ALTER TABLE interaction_events ADD COLUMN actor_email TEXT NOT NULL DEFAULT ''",
            ),
            (
                "actor_source",
                "ALTER TABLE interaction_events ADD COLUMN actor_source TEXT NOT NULL DEFAULT ''",
            ),
            (
                "event_type",
                "ALTER TABLE interaction_events ADD COLUMN event_type TEXT NOT NULL DEFAULT ''",
            ),
            (
                "event_time",
                "ALTER TABLE interaction_events ADD COLUMN event_time TEXT NOT NULL DEFAULT ''",
            ),
            (
                "agent_type",
                "ALTER TABLE interaction_events ADD COLUMN agent_type TEXT NOT NULL DEFAULT ''",
            ),
            (
                "model",
                "ALTER TABLE interaction_events ADD COLUMN model TEXT NOT NULL DEFAULT ''",
            ),
            (
                "tool_use_id",
                "ALTER TABLE interaction_events ADD COLUMN tool_use_id TEXT NOT NULL DEFAULT ''",
            ),
            (
                "tool_kind",
                "ALTER TABLE interaction_events ADD COLUMN tool_kind TEXT NOT NULL DEFAULT ''",
            ),
            (
                "task_description",
                "ALTER TABLE interaction_events ADD COLUMN task_description TEXT NOT NULL DEFAULT ''",
            ),
            (
                "subagent_id",
                "ALTER TABLE interaction_events ADD COLUMN subagent_id TEXT NOT NULL DEFAULT ''",
            ),
            (
                "payload",
                "ALTER TABLE interaction_events ADD COLUMN payload TEXT NOT NULL DEFAULT '{}'",
            ),
        ],
    )?;
    conn.execute_batch(
        r#"
CREATE INDEX IF NOT EXISTS interaction_events_tool_use_idx
ON interaction_events (repo_id, tool_use_id, event_time, event_id);
CREATE TABLE IF NOT EXISTS interaction_tool_uses (
    tool_use_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    session_id TEXT NOT NULL DEFAULT '',
    turn_id TEXT NOT NULL DEFAULT '',
    tool_kind TEXT NOT NULL DEFAULT '',
    task_description TEXT NOT NULL DEFAULT '',
    subagent_id TEXT NOT NULL DEFAULT '',
    transcript_path TEXT NOT NULL DEFAULT '',
    started_at TEXT,
    ended_at TEXT,
    updated_at TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (repo_id, tool_use_id)
);
CREATE INDEX IF NOT EXISTS interaction_tool_uses_session_idx
ON interaction_tool_uses (repo_id, session_id, turn_id, updated_at);
CREATE TABLE IF NOT EXISTS interaction_session_search_documents (
    session_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    started_at TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL DEFAULT '',
    prompt_text TEXT NOT NULL DEFAULT '',
    summary_text TEXT NOT NULL DEFAULT '',
    transcript_text TEXT NOT NULL DEFAULT '',
    tool_text TEXT NOT NULL DEFAULT '',
    paths_text TEXT NOT NULL DEFAULT '',
    combined_text TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (repo_id, session_id)
);
CREATE INDEX IF NOT EXISTS interaction_session_search_documents_time_idx
ON interaction_session_search_documents (repo_id, started_at, updated_at);
CREATE TABLE IF NOT EXISTS interaction_turn_search_documents (
    turn_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    session_id TEXT NOT NULL DEFAULT '',
    started_at TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL DEFAULT '',
    prompt_text TEXT NOT NULL DEFAULT '',
    summary_text TEXT NOT NULL DEFAULT '',
    transcript_text TEXT NOT NULL DEFAULT '',
    tool_text TEXT NOT NULL DEFAULT '',
    paths_text TEXT NOT NULL DEFAULT '',
    combined_text TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (repo_id, turn_id)
);
CREATE INDEX IF NOT EXISTS interaction_turn_search_documents_session_idx
ON interaction_turn_search_documents (repo_id, session_id, started_at, updated_at);
CREATE TABLE IF NOT EXISTS interaction_session_search_terms (
    repo_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    term TEXT NOT NULL,
    field TEXT NOT NULL,
    occurrences INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (repo_id, session_id, term, field)
);
CREATE INDEX IF NOT EXISTS interaction_session_search_terms_lookup_idx
ON interaction_session_search_terms (repo_id, term, field, session_id);
CREATE TABLE IF NOT EXISTS interaction_turn_search_terms (
    repo_id TEXT NOT NULL,
    turn_id TEXT NOT NULL,
    term TEXT NOT NULL,
    field TEXT NOT NULL,
    occurrences INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (repo_id, turn_id, term, field)
);
CREATE INDEX IF NOT EXISTS interaction_turn_search_terms_lookup_idx
ON interaction_turn_search_terms (repo_id, term, field, turn_id);
"#,
    )
    .context("creating interaction search projection tables")
}

fn ensure_table_columns(
    conn: &rusqlite::Connection,
    table: &str,
    missing: &[(&str, &str)],
) -> Result<()> {
    let existing = sqlite_table_columns(conn, table)?;
    for (column, sql) in missing {
        if existing.contains(*column) {
            continue;
        }
        conn.execute_batch(sql)
            .with_context(|| format!("adding {table}.{column} column"))?;
    }
    Ok(())
}

fn sqlite_table_columns(conn: &rusqlite::Connection, table: &str) -> Result<HashSet<String>> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .with_context(|| format!("preparing PRAGMA table_info for `{table}`"))?;
    let mut rows = stmt
        .query([])
        .with_context(|| format!("querying PRAGMA table_info for `{table}`"))?;
    let mut columns = HashSet::new();
    while let Some(row) = rows.next().context("reading PRAGMA row")? {
        let name: String = row
            .get(1)
            .with_context(|| format!("reading column name from `{table}`"))?;
        columns.insert(name);
    }
    Ok(columns)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_additive_columns_migrates_legacy_spool_tables() {
        let conn = rusqlite::Connection::open_in_memory().expect("sqlite");
        conn.execute_batch(
            r#"
CREATE TABLE interaction_sessions (
    session_id TEXT PRIMARY KEY
);
CREATE TABLE interaction_turns (
    turn_id TEXT PRIMARY KEY
);
CREATE TABLE interaction_events (
    event_id TEXT PRIMARY KEY
);
"#,
        )
        .expect("create legacy tables");

        ensure_additive_columns(&conn).expect("apply additive columns");

        let session_columns = sqlite_table_columns(&conn, "interaction_sessions").unwrap();
        assert!(session_columns.contains("last_event_at"));
        assert!(session_columns.contains("updated_at"));

        let turn_columns = sqlite_table_columns(&conn, "interaction_turns").unwrap();
        assert!(turn_columns.contains("summary"));
        assert!(turn_columns.contains("transcript_fragment"));
        assert!(turn_columns.contains("updated_at"));

        let event_columns = sqlite_table_columns(&conn, "interaction_events").unwrap();
        assert!(event_columns.contains("model"));
        assert!(event_columns.contains("payload"));
    }
}

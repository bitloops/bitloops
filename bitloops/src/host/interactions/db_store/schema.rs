use std::collections::HashSet;

use anyhow::{Context, Result};

pub(super) const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS interaction_sessions (
    session_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    agent_type TEXT NOT NULL DEFAULT '',
    model TEXT NOT NULL DEFAULT '',
    first_prompt TEXT NOT NULL DEFAULT '',
    transcript_path TEXT NOT NULL DEFAULT '',
    worktree_path TEXT NOT NULL DEFAULT '',
    worktree_id TEXT NOT NULL DEFAULT '',
    started_at TEXT NOT NULL DEFAULT '',
    ended_at TEXT,
    last_event_at TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL DEFAULT ''
);

CREATE INDEX IF NOT EXISTS interaction_sessions_repo_idx
ON interaction_sessions (repo_id, last_event_at, started_at);

CREATE TABLE IF NOT EXISTS interaction_turns (
    turn_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
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
    files_modified TEXT NOT NULL DEFAULT '[]',
    checkpoint_id TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL DEFAULT ''
);

CREATE INDEX IF NOT EXISTS interaction_turns_session_idx
ON interaction_turns (session_id, turn_number, started_at);

CREATE INDEX IF NOT EXISTS interaction_turns_pending_idx
ON interaction_turns (repo_id, checkpoint_id, session_id, turn_number);

CREATE TABLE IF NOT EXISTS interaction_events (
    event_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    turn_id TEXT,
    repo_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    event_time TEXT NOT NULL,
    agent_type TEXT NOT NULL DEFAULT '',
    model TEXT NOT NULL DEFAULT '',
    payload TEXT NOT NULL DEFAULT '{}'
);

CREATE INDEX IF NOT EXISTS interaction_events_repo_time_idx
ON interaction_events (repo_id, event_time, event_id);

CREATE INDEX IF NOT EXISTS interaction_events_session_idx
ON interaction_events (session_id, event_time, event_id);

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
    let existing = sqlite_table_columns(conn, "interaction_turns")?;
    let missing = [
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
    ];
    for (column, sql) in missing {
        if existing.contains(column) {
            continue;
        }
        conn.execute_batch(sql)
            .with_context(|| format!("adding interaction_turns.{column} column"))?;
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

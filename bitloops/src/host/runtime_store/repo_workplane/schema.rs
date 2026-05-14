//! Schema definition and migration helpers for the repo workplane tables.

use anyhow::{Context, Result};

use crate::storage::SqliteConnectionPool;

pub(crate) const REPO_WORKPLANE_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS capability_workplane_cursor_generations (
    repo_id TEXT NOT NULL,
    generation_seq INTEGER NOT NULL,
    source_task_id TEXT,
    sync_mode TEXT NOT NULL,
    active_branch TEXT,
    head_commit_sha TEXT,
    requires_full_reconcile INTEGER NOT NULL DEFAULT 0,
    created_at_unix INTEGER NOT NULL,
    PRIMARY KEY (repo_id, generation_seq)
);

CREATE INDEX IF NOT EXISTS idx_capability_workplane_cursor_generations_repo_created
ON capability_workplane_cursor_generations (repo_id, created_at_unix DESC);

CREATE TABLE IF NOT EXISTS capability_workplane_cursor_file_changes (
    repo_id TEXT NOT NULL,
    generation_seq INTEGER NOT NULL,
    path TEXT NOT NULL,
    change_kind TEXT NOT NULL,
    language TEXT,
    content_id TEXT
);

CREATE INDEX IF NOT EXISTS idx_capability_workplane_cursor_file_changes_repo_generation
ON capability_workplane_cursor_file_changes (repo_id, generation_seq, path);

CREATE TABLE IF NOT EXISTS capability_workplane_cursor_artefact_changes (
    repo_id TEXT NOT NULL,
    generation_seq INTEGER NOT NULL,
    symbol_id TEXT NOT NULL,
    change_kind TEXT NOT NULL,
    artefact_id TEXT NOT NULL,
    path TEXT NOT NULL,
    canonical_kind TEXT,
    name TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_capability_workplane_cursor_artefact_changes_repo_generation
ON capability_workplane_cursor_artefact_changes (repo_id, generation_seq, symbol_id);

CREATE TABLE IF NOT EXISTS capability_workplane_cursor_mailboxes (
    repo_id TEXT NOT NULL,
    capability_id TEXT NOT NULL,
    mailbox_name TEXT NOT NULL,
    last_applied_generation_seq INTEGER,
    last_error TEXT,
    updated_at_unix INTEGER NOT NULL,
    PRIMARY KEY (repo_id, capability_id, mailbox_name)
);

CREATE INDEX IF NOT EXISTS idx_capability_workplane_cursor_mailboxes_repo_capability
ON capability_workplane_cursor_mailboxes (repo_id, capability_id, mailbox_name);

CREATE TABLE IF NOT EXISTS capability_workplane_mailbox_intents (
    repo_id TEXT NOT NULL,
    capability_id TEXT NOT NULL,
    mailbox_name TEXT NOT NULL,
    active INTEGER NOT NULL DEFAULT 1,
    source TEXT,
    updated_at_unix INTEGER NOT NULL,
    PRIMARY KEY (repo_id, capability_id, mailbox_name)
);

CREATE INDEX IF NOT EXISTS idx_capability_workplane_mailbox_intents_repo_capability
ON capability_workplane_mailbox_intents (repo_id, capability_id, mailbox_name, active);

CREATE TABLE IF NOT EXISTS capability_workplane_cursor_runs (
    run_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    repo_root TEXT NOT NULL,
    capability_id TEXT NOT NULL,
    mailbox_name TEXT NOT NULL,
    init_session_id TEXT,
    from_generation_seq INTEGER NOT NULL,
    to_generation_seq INTEGER NOT NULL,
    reconcile_mode TEXT NOT NULL,
    status TEXT NOT NULL,
    attempts INTEGER NOT NULL,
    submitted_at_unix INTEGER NOT NULL,
    started_at_unix INTEGER,
    updated_at_unix INTEGER NOT NULL,
    completed_at_unix INTEGER,
    error TEXT
);

CREATE INDEX IF NOT EXISTS idx_capability_workplane_cursor_runs_repo_mailbox_status
ON capability_workplane_cursor_runs (repo_id, capability_id, mailbox_name, status, submitted_at_unix);

CREATE TABLE IF NOT EXISTS capability_workplane_jobs (
    job_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    repo_root TEXT NOT NULL,
    config_root TEXT NOT NULL,
    capability_id TEXT NOT NULL,
    mailbox_name TEXT NOT NULL,
    init_session_id TEXT,
    dedupe_key TEXT,
    payload TEXT NOT NULL,
    status TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    available_at_unix INTEGER NOT NULL,
    submitted_at_unix INTEGER NOT NULL,
    started_at_unix INTEGER,
    updated_at_unix INTEGER NOT NULL,
    completed_at_unix INTEGER,
    lease_owner TEXT,
    lease_expires_at_unix INTEGER,
    last_error TEXT
);

CREATE INDEX IF NOT EXISTS idx_capability_workplane_jobs_repo_mailbox_status
ON capability_workplane_jobs (repo_id, capability_id, mailbox_name, status, available_at_unix, submitted_at_unix);

CREATE INDEX IF NOT EXISTS idx_capability_workplane_jobs_status_mailbox_available_submitted
ON capability_workplane_jobs (status, mailbox_name, available_at_unix, submitted_at_unix);

CREATE INDEX IF NOT EXISTS idx_capability_workplane_jobs_dedupe
ON capability_workplane_jobs (repo_id, capability_id, mailbox_name, dedupe_key);

CREATE TABLE IF NOT EXISTS semantic_summary_mailbox_items (
    item_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    repo_root TEXT NOT NULL,
    config_root TEXT NOT NULL,
    init_session_id TEXT,
    item_kind TEXT NOT NULL,
    artefact_id TEXT,
    payload_json TEXT,
    dedupe_key TEXT,
    status TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    available_at_unix INTEGER NOT NULL,
    submitted_at_unix INTEGER NOT NULL,
    leased_at_unix INTEGER,
    lease_expires_at_unix INTEGER,
    lease_token TEXT,
    updated_at_unix INTEGER NOT NULL,
    last_error TEXT
);

CREATE INDEX IF NOT EXISTS idx_semantic_summary_mailbox_items_repo_status
ON semantic_summary_mailbox_items (repo_id, status, available_at_unix, submitted_at_unix);

CREATE INDEX IF NOT EXISTS idx_semantic_summary_mailbox_items_status_available
ON semantic_summary_mailbox_items (status, available_at_unix, submitted_at_unix);

CREATE INDEX IF NOT EXISTS idx_semantic_summary_mailbox_items_dedupe
ON semantic_summary_mailbox_items (repo_id, dedupe_key);

CREATE TABLE IF NOT EXISTS semantic_embedding_mailbox_items (
    item_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    repo_root TEXT NOT NULL,
    config_root TEXT NOT NULL,
    init_session_id TEXT,
    representation_kind TEXT NOT NULL,
    item_kind TEXT NOT NULL,
    artefact_id TEXT,
    payload_json TEXT,
    dedupe_key TEXT,
    status TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    available_at_unix INTEGER NOT NULL,
    submitted_at_unix INTEGER NOT NULL,
    leased_at_unix INTEGER,
    lease_expires_at_unix INTEGER,
    lease_token TEXT,
    updated_at_unix INTEGER NOT NULL,
    last_error TEXT
);

CREATE INDEX IF NOT EXISTS idx_semantic_embedding_mailbox_items_repo_status
ON semantic_embedding_mailbox_items (repo_id, representation_kind, status, available_at_unix, submitted_at_unix);

CREATE INDEX IF NOT EXISTS idx_semantic_embedding_mailbox_items_status_available
ON semantic_embedding_mailbox_items (status, representation_kind, available_at_unix, submitted_at_unix);

CREATE INDEX IF NOT EXISTS idx_semantic_embedding_mailbox_items_dedupe
ON semantic_embedding_mailbox_items (repo_id, representation_kind, dedupe_key);
"#;

pub(crate) fn ensure_repo_workplane_schema_upgrades(sqlite: &SqliteConnectionPool) -> Result<()> {
    sqlite.with_write_connection(|conn| {
        ensure_table_has_column(
            conn,
            "capability_workplane_cursor_runs",
            "init_session_id",
            "ALTER TABLE capability_workplane_cursor_runs ADD COLUMN init_session_id TEXT",
        )?;
        ensure_table_has_column(
            conn,
            "capability_workplane_jobs",
            "init_session_id",
            "ALTER TABLE capability_workplane_jobs ADD COLUMN init_session_id TEXT",
        )?;
        Ok(())
    })
}

fn ensure_table_has_column(
    conn: &rusqlite::Connection,
    table: &str,
    column: &str,
    alter_sql: &str,
) -> Result<()> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .with_context(|| format!("preparing PRAGMA table_info for `{table}`"))?;
    let mut rows = stmt
        .query([])
        .with_context(|| format!("querying PRAGMA table_info for `{table}`"))?;
    while let Some(row) = rows.next().context("reading PRAGMA row")? {
        let name: String = row
            .get(1)
            .with_context(|| format!("reading column name from `{table}`"))?;
        if name == column {
            return Ok(());
        }
    }
    conn.execute_batch(alter_sql)
        .with_context(|| format!("adding `{column}` column to `{table}`"))?;
    Ok(())
}

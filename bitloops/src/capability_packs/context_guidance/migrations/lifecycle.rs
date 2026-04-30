use std::collections::BTreeSet;

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, params};

use crate::host::capability_host::{
    CapabilityMigration, CapabilityMigrationContext, MigrationRunner,
};

use super::super::descriptor::CONTEXT_GUIDANCE_CAPABILITY_ID;

fn run_context_guidance_lifecycle_schema(ctx: &mut dyn CapabilityMigrationContext) -> Result<()> {
    ctx.apply_devql_sqlite_migration(&mut migrate_context_guidance_lifecycle_schema)
}

pub static CONTEXT_GUIDANCE_LIFECYCLE_MIGRATION: CapabilityMigration = CapabilityMigration {
    capability_id: CONTEXT_GUIDANCE_CAPABILITY_ID,
    version: "0.0.21",
    description: "Context guidance lifecycle, supersession, and target compaction tables",
    run: MigrationRunner::Core(run_context_guidance_lifecycle_schema),
};

pub fn context_guidance_lifecycle_sqlite_schema_sql() -> &'static str {
    CONTEXT_GUIDANCE_LIFECYCLE_OBJECTS_SQL
}

fn migrate_context_guidance_lifecycle_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch("BEGIN IMMEDIATE")
        .context("starting context guidance lifecycle migration")?;
    let result = migrate_context_guidance_lifecycle_schema_tx(conn);
    match result {
        Ok(()) => conn
            .execute_batch("COMMIT")
            .context("committing context guidance lifecycle migration"),
        Err(err) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(err)
        }
    }
}

fn migrate_context_guidance_lifecycle_schema_tx(conn: &Connection) -> Result<()> {
    let main_exists = sqlite_table_exists(conn, "context_guidance_facts")?;
    let backup_exists = sqlite_table_exists(conn, "context_guidance_facts_old_v021")?;

    match (main_exists, backup_exists) {
        (true, true) => {
            bail!("context guidance lifecycle migration found both active and backup facts tables")
        }
        (true, false) => conn
            .execute_batch(
                "ALTER TABLE context_guidance_facts RENAME TO context_guidance_facts_old_v021;",
            )
            .context("renaming context guidance facts for lifecycle migration")?,
        (false, true) => {}
        (false, false) => {
            conn.execute_batch(CREATE_CONTEXT_GUIDANCE_FACTS_TABLE_SQL)
                .context("creating context guidance facts table")?;
            conn.execute_batch(context_guidance_lifecycle_sqlite_schema_sql())
                .context("creating context guidance lifecycle tables")?;
            return Ok(());
        }
    }

    let source_columns = sqlite_table_columns(conn, "context_guidance_facts_old_v021")?;
    conn.execute_batch(CREATE_CONTEXT_GUIDANCE_FACTS_TABLE_SQL)
        .context("creating migrated context guidance facts table")?;
    conn.execute_batch(&copy_context_guidance_facts_sql(&source_columns))
        .context("copying migrated context guidance facts")?;
    conn.execute_batch("DROP TABLE context_guidance_facts_old_v021;")
        .context("dropping migrated context guidance facts backup")?;
    conn.execute_batch(context_guidance_lifecycle_sqlite_schema_sql())
        .context("creating context guidance lifecycle tables")?;
    Ok(())
}

fn sqlite_table_exists(conn: &Connection, table_name: &str) -> Result<bool> {
    let count = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            params![table_name],
            |row| row.get::<_, i64>(0),
        )
        .with_context(|| format!("checking SQLite table {table_name}"))?;
    Ok(count == 1)
}

fn sqlite_table_columns(conn: &Connection, table_name: &str) -> Result<BTreeSet<String>> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table_name})"))
        .with_context(|| format!("preparing SQLite table info for {table_name}"))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .with_context(|| format!("querying SQLite table info for {table_name}"))?;
    let mut columns = BTreeSet::new();
    for row in rows {
        columns.insert(row.with_context(|| format!("reading SQLite column for {table_name}"))?);
    }
    Ok(columns)
}

fn copy_context_guidance_facts_sql(source_columns: &BTreeSet<String>) -> String {
    let source_expressions = [
        source_column(source_columns, "guidance_id", "''"),
        source_column(source_columns, "run_id", "''"),
        source_column(source_columns, "repo_id", "''"),
        source_column(source_columns, "active", "1"),
        source_column(source_columns, "category", "'DECISION'"),
        source_column(source_columns, "kind", "''"),
        source_column(source_columns, "guidance", "''"),
        source_column(source_columns, "evidence_excerpt", "''"),
        source_column(source_columns, "confidence", "'MEDIUM'"),
        source_column(source_columns, "lifecycle_status", "'active'"),
        source_column(source_columns, "fact_fingerprint", "''"),
        source_column(source_columns, "value_score", "0.0"),
        source_column(source_columns, "superseded_by_guidance_id", "NULL"),
        source_column(source_columns, "lifecycle_reason", "''"),
        source_column(source_columns, "generated_at", "datetime('now')"),
        source_column(source_columns, "updated_at", "datetime('now')"),
    ];

    format!(
        r#"INSERT INTO context_guidance_facts (
    guidance_id,
    run_id,
    repo_id,
    active,
    category,
    kind,
    guidance,
    evidence_excerpt,
    confidence,
    lifecycle_status,
    fact_fingerprint,
    value_score,
    superseded_by_guidance_id,
    lifecycle_reason,
    generated_at,
    updated_at
)
SELECT
    {}
FROM context_guidance_facts_old_v021;"#,
        source_expressions.join(",\n    ")
    )
}

fn source_column(source_columns: &BTreeSet<String>, column: &str, fallback: &str) -> String {
    if source_columns.contains(column) {
        column.to_string()
    } else {
        fallback.to_string()
    }
}

const CREATE_CONTEXT_GUIDANCE_FACTS_TABLE_SQL: &str = r#"
CREATE TABLE context_guidance_facts (
    guidance_id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL,
    repo_id TEXT NOT NULL,
    active INTEGER NOT NULL DEFAULT 1,
    category TEXT NOT NULL,
    kind TEXT NOT NULL,
    guidance TEXT NOT NULL,
    evidence_excerpt TEXT NOT NULL,
    confidence TEXT NOT NULL,
    lifecycle_status TEXT NOT NULL DEFAULT 'active',
    fact_fingerprint TEXT NOT NULL DEFAULT '',
    value_score REAL NOT NULL DEFAULT 0.0,
    superseded_by_guidance_id TEXT,
    lifecycle_reason TEXT NOT NULL DEFAULT '',
    generated_at TEXT DEFAULT (datetime('now')),
    updated_at TEXT DEFAULT (datetime('now'))
);
"#;

const CONTEXT_GUIDANCE_LIFECYCLE_OBJECTS_SQL: &str = r#"
CREATE INDEX IF NOT EXISTS context_guidance_facts_repo_category_idx
ON context_guidance_facts (repo_id, active, category, kind);

CREATE INDEX IF NOT EXISTS context_guidance_facts_run_idx
ON context_guidance_facts (run_id);

CREATE INDEX IF NOT EXISTS context_guidance_facts_lifecycle_idx
ON context_guidance_facts (repo_id, active, lifecycle_status, value_score);

CREATE INDEX IF NOT EXISTS context_guidance_facts_fingerprint_idx
ON context_guidance_facts (repo_id, fact_fingerprint, lifecycle_status);

CREATE TABLE IF NOT EXISTS context_guidance_compaction_runs (
    compaction_run_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    target_type TEXT NOT NULL,
    target_value TEXT NOT NULL,
    source_fact_count INTEGER NOT NULL DEFAULT 0,
    retained_fact_count INTEGER NOT NULL DEFAULT 0,
    compacted_fact_count INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'completed',
    summary_json TEXT NOT NULL DEFAULT '{}',
    source_model TEXT DEFAULT '',
    source_profile TEXT DEFAULT '',
    generated_at TEXT DEFAULT (datetime('now')),
    updated_at TEXT DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS context_guidance_compaction_runs_target_idx
ON context_guidance_compaction_runs (repo_id, target_type, target_value, generated_at);

CREATE TABLE IF NOT EXISTS context_guidance_compaction_members (
    compaction_member_id TEXT PRIMARY KEY,
    compaction_run_id TEXT NOT NULL,
    guidance_id TEXT NOT NULL,
    action TEXT NOT NULL,
    reason TEXT NOT NULL DEFAULT '',
    created_at TEXT DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS context_guidance_compaction_members_run_idx
ON context_guidance_compaction_members (compaction_run_id);

CREATE INDEX IF NOT EXISTS context_guidance_compaction_members_guidance_idx
ON context_guidance_compaction_members (guidance_id);

CREATE TABLE IF NOT EXISTS context_guidance_target_summaries (
    target_summary_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    target_type TEXT NOT NULL,
    target_value TEXT NOT NULL,
    summary_json TEXT NOT NULL DEFAULT '{}',
    active_guidance_count INTEGER NOT NULL DEFAULT 0,
    latest_compaction_run_id TEXT,
    generated_at TEXT DEFAULT (datetime('now')),
    updated_at TEXT DEFAULT (datetime('now'))
);

CREATE UNIQUE INDEX IF NOT EXISTS context_guidance_target_summaries_target_idx
ON context_guidance_target_summaries (repo_id, target_type, target_value);
"#;

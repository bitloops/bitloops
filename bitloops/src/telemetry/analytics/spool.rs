use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
#[cfg(test)]
use rusqlite::OptionalExtension;
use rusqlite::{Connection, TransactionBehavior, params, params_from_iter};

use super::EventPayload;

pub(super) const MAX_SPOOL_EVENTS: i64 = 1_000;
pub(super) const DEFAULT_BATCH_SIZE: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EnqueueOutcome {
    Queued,
    Full,
}

#[derive(Debug)]
pub(super) struct SpoolEvent {
    pub(super) id: String,
    pub(super) payload_json: String,
}

pub(super) fn default_spool_path() -> Result<PathBuf> {
    Ok(crate::utils::platform_dirs::bitloops_state_dir()?.join("telemetry_spool.sqlite3"))
}

pub(super) fn enqueue_payload(payload: &EventPayload, now: i64) -> Result<EnqueueOutcome> {
    let payload_json = serde_json::to_string(payload).context("serialising telemetry payload")?;
    enqueue_payload_json_at_path(
        &default_spool_path()?,
        &uuid::Uuid::new_v4().to_string(),
        &payload_json,
        now,
        MAX_SPOOL_EVENTS,
    )
}

pub(super) fn enqueue_payload_json_at_path(
    path: &Path,
    id: &str,
    payload_json: &str,
    now: i64,
    max_events: i64,
) -> Result<EnqueueOutcome> {
    let mut conn = open_spool(path)?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .context("starting telemetry spool transaction")?;
    let count: i64 = tx
        .query_row("SELECT COUNT(*) FROM analytics_spool", [], |row| row.get(0))
        .context("counting telemetry spool rows")?;
    if count >= max_events {
        tx.commit()
            .context("committing full telemetry spool transaction")?;
        return Ok(EnqueueOutcome::Full);
    }

    tx.execute(
        "INSERT INTO analytics_spool (
            id, created_at, next_attempt_at, attempt_count, payload_json, last_error
        ) VALUES (?1, ?2, ?2, 0, ?3, NULL)",
        params![id, now, payload_json],
    )
    .context("inserting telemetry spool row")?;
    tx.commit()
        .context("committing telemetry spool transaction")?;

    Ok(EnqueueOutcome::Queued)
}

pub(super) fn load_due_batch(path: &Path, now: i64, limit: usize) -> Result<Vec<SpoolEvent>> {
    let conn = open_spool(path)?;
    let mut stmt = conn
        .prepare(
            "SELECT id, payload_json
             FROM analytics_spool
             WHERE next_attempt_at <= ?1
             ORDER BY created_at ASC, id ASC
             LIMIT ?2",
        )
        .context("preparing telemetry spool batch query")?;
    let rows = stmt
        .query_map(
            params![now, i64::try_from(limit).unwrap_or(i64::MAX)],
            |row| {
                Ok(SpoolEvent {
                    id: row.get(0)?,
                    payload_json: row.get(1)?,
                })
            },
        )
        .context("querying telemetry spool batch")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("reading telemetry spool batch rows")?;
    Ok(rows)
}

pub(super) fn delete_events(path: &Path, ids: &[String]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let conn = open_spool(path)?;
    let placeholders = placeholders(ids.len());
    conn.execute(
        &format!("DELETE FROM analytics_spool WHERE id IN ({placeholders})"),
        params_from_iter(ids.iter()),
    )
    .context("deleting telemetry spool rows")?;
    Ok(())
}

pub(super) fn mark_send_failure(path: &Path, ids: &[String], now: i64, error: &str) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let conn = open_spool(path)?;
    let placeholders = placeholders(ids.len());
    let next_attempt_case = retry_backoff_case_sql(now);
    let sql = format!(
        "UPDATE analytics_spool
         SET attempt_count = attempt_count + 1,
             next_attempt_at = {next_attempt_case},
             last_error = ?
         WHERE id IN ({placeholders})",
    );
    let params = std::iter::once(error).chain(ids.iter().map(String::as_str));
    conn.execute(&sql, params_from_iter(params))
        .context("marking telemetry spool rows as failed")?;
    Ok(())
}

#[cfg(test)]
pub(super) fn count_events(path: &Path) -> Result<i64> {
    let conn = open_spool(path)?;
    conn.query_row("SELECT COUNT(*) FROM analytics_spool", [], |row| row.get(0))
        .context("counting telemetry spool rows")
}

#[cfg(test)]
pub(super) fn next_attempt_at(path: &Path, id: &str) -> Result<Option<i64>> {
    let conn = open_spool(path)?;
    conn.query_row(
        "SELECT next_attempt_at FROM analytics_spool WHERE id = ?1",
        params![id],
        |row| row.get(0),
    )
    .optional()
    .context("loading telemetry spool next attempt")
}

fn open_spool(path: &Path) -> Result<Connection> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating telemetry spool directory {}", parent.display()))?;
    }
    let conn = Connection::open(path)
        .with_context(|| format!("opening telemetry spool {}", path.display()))?;
    configure_connection(&conn)?;
    initialise_schema(&conn)?;
    Ok(conn)
}

fn configure_connection(conn: &Connection) -> Result<()> {
    conn.busy_timeout(Duration::from_secs(2))
        .context("setting telemetry spool busy timeout")?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;",
    )
    .context("configuring telemetry spool pragmas")?;
    Ok(())
}

fn initialise_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS analytics_spool (
            id TEXT PRIMARY KEY,
            created_at INTEGER NOT NULL,
            next_attempt_at INTEGER NOT NULL,
            attempt_count INTEGER NOT NULL DEFAULT 0,
            payload_json TEXT NOT NULL,
            last_error TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_analytics_spool_due
            ON analytics_spool(next_attempt_at, created_at, id);",
    )
    .context("initialising telemetry spool schema")?;
    Ok(())
}

fn placeholders(count: usize) -> String {
    std::iter::repeat_n("?", count)
        .collect::<Vec<_>>()
        .join(",")
}

fn retry_backoff_case_sql(now: i64) -> String {
    format!(
        "CASE
            WHEN attempt_count <= 0 THEN {now} + 5
            WHEN attempt_count = 1 THEN {now} + 10
            WHEN attempt_count = 2 THEN {now} + 20
            WHEN attempt_count = 3 THEN {now} + 40
            WHEN attempt_count = 4 THEN {now} + 80
            ELSE {now} + 300
         END"
    )
}

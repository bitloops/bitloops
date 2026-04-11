use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params};
use uuid::Uuid;

use crate::host::capability_host::{ChangedArtefact, SyncArtefactDiff, SyncFileDiff};

use super::super::types::{CapabilityEventRunRecord, CapabilityEventRunStatus};

const REMOVED_ARTEFACT_PLACEHOLDER_NAME: &str = "<removed>";

#[derive(Debug, Clone)]
pub(super) struct StoredRunRecord {
    pub(super) record: CapabilityEventRunRecord,
    pub(super) repo_root: PathBuf,
}

#[derive(Debug, Clone)]
pub(super) struct GenerationRow {
    pub(super) generation_seq: u64,
    pub(super) active_branch: Option<String>,
    pub(super) head_commit_sha: Option<String>,
    pub(super) requires_full_reconcile: bool,
}

#[derive(Debug, Clone)]
pub(super) struct FileChangeRow {
    pub(super) generation_seq: u64,
    pub(super) path: String,
    pub(super) change_kind: String,
    pub(super) language: Option<String>,
    pub(super) content_id: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct ArtefactChangeRow {
    pub(super) generation_seq: u64,
    pub(super) symbol_id: String,
    pub(super) change_kind: String,
    pub(super) artefact_id: String,
    pub(super) path: String,
    pub(super) canonical_kind: Option<String>,
    pub(super) name: String,
}

#[derive(Debug, Clone)]
pub(super) struct ConsumerCursorRow {
    pub(super) last_applied_generation_seq: Option<u64>,
}

pub(super) fn insert_file_changes(
    conn: &rusqlite::Connection,
    repo_id: &str,
    generation_seq: u64,
    diff: &SyncFileDiff,
) -> Result<()> {
    for file in &diff.added {
        insert_file_change_row(
            conn,
            repo_id,
            generation_seq,
            &file.path,
            "added",
            Some(&file.language),
            Some(&file.content_id),
        )?;
    }
    for file in &diff.changed {
        insert_file_change_row(
            conn,
            repo_id,
            generation_seq,
            &file.path,
            "changed",
            Some(&file.language),
            Some(&file.content_id),
        )?;
    }
    for file in &diff.removed {
        insert_file_change_row(
            conn,
            repo_id,
            generation_seq,
            &file.path,
            "removed",
            None,
            None,
        )?;
    }
    Ok(())
}

fn insert_file_change_row(
    conn: &rusqlite::Connection,
    repo_id: &str,
    generation_seq: u64,
    path: &str,
    change_kind: &str,
    language: Option<&str>,
    content_id: Option<&str>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO pack_reconcile_file_changes (repo_id, generation_seq, path, change_kind, language, content_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            repo_id,
            sql_i64(generation_seq)?,
            path,
            change_kind,
            language,
            content_id
        ],
    )
    .with_context(|| format!("inserting file change `{path}` for generation {generation_seq}"))?;
    Ok(())
}

pub(super) fn insert_artefact_changes(
    conn: &rusqlite::Connection,
    repo_id: &str,
    generation_seq: u64,
    diff: &SyncArtefactDiff,
) -> Result<()> {
    for artefact in &diff.added {
        insert_artefact_change_row(conn, repo_id, generation_seq, artefact, "added")?;
    }
    for artefact in &diff.changed {
        insert_artefact_change_row(conn, repo_id, generation_seq, artefact, "changed")?;
    }
    for artefact in &diff.removed {
        conn.execute(
            "INSERT INTO pack_reconcile_artefact_changes (repo_id, generation_seq, symbol_id, change_kind, artefact_id, path, canonical_kind, name) VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7)",
            params![
                repo_id,
                sql_i64(generation_seq)?,
                artefact.symbol_id,
                "removed",
                artefact.artefact_id,
                artefact.path,
                REMOVED_ARTEFACT_PLACEHOLDER_NAME,
            ],
        )
        .with_context(|| {
            format!(
                "inserting removed artefact `{}` for generation {}",
                artefact.symbol_id, generation_seq
            )
        })?;
    }
    Ok(())
}

fn insert_artefact_change_row(
    conn: &rusqlite::Connection,
    repo_id: &str,
    generation_seq: u64,
    artefact: &ChangedArtefact,
    change_kind: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO pack_reconcile_artefact_changes (repo_id, generation_seq, symbol_id, change_kind, artefact_id, path, canonical_kind, name) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            repo_id,
            sql_i64(generation_seq)?,
            artefact.symbol_id,
            change_kind,
            artefact.artefact_id,
            artefact.path,
            artefact.canonical_kind,
            artefact.name,
        ],
    )
    .with_context(|| {
        format!(
            "inserting artefact change `{}` for generation {}",
            artefact.symbol_id, generation_seq
        )
    })?;
    Ok(())
}

pub(super) fn next_generation_seq(conn: &rusqlite::Connection, repo_id: &str) -> Result<u64> {
    conn.query_row(
        "SELECT COALESCE(MAX(generation_seq), 0) + 1 FROM pack_reconcile_generations WHERE repo_id = ?1",
        params![repo_id],
        |row| row.get::<_, i64>(0),
    )
    .map(|value| u64::try_from(value).unwrap_or_default())
    .map_err(anyhow::Error::from)
}

pub(super) fn upsert_consumer_row(
    conn: &rusqlite::Connection,
    repo_id: &str,
    capability_id: &str,
    consumer_id: &str,
    now: u64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO pack_reconcile_consumers (repo_id, consumer_id, capability_id, last_applied_generation_seq, last_error, updated_at_unix) VALUES (?1, ?2, ?3, NULL, NULL, ?4) \
         ON CONFLICT (repo_id, consumer_id) DO UPDATE SET capability_id = excluded.capability_id, updated_at_unix = excluded.updated_at_unix",
        params![repo_id, consumer_id, capability_id, sql_i64(now)?],
    )
    .with_context(|| {
        format!(
            "upserting current-state consumer `{consumer_id}` for repo `{repo_id}`"
        )
    })?;
    Ok(())
}

pub(super) fn ensure_consumer_run(
    conn: &rusqlite::Connection,
    repo_id: &str,
    repo_root: &Path,
    capability_id: &str,
    consumer_id: &str,
    now: u64,
) -> Result<Option<StoredRunRecord>> {
    let latest_generation = latest_generation_seq(conn, repo_id)?;
    let Some(latest_generation) = latest_generation else {
        return Ok(None);
    };

    let last_applied_generation = load_consumer_cursor(conn, repo_id, consumer_id)?
        .and_then(|cursor| cursor.last_applied_generation_seq)
        .unwrap_or(0);
    if latest_generation <= last_applied_generation {
        return Ok(None);
    }

    if let Some(run) = load_active_run_for_lane(conn, repo_id, consumer_id)? {
        if run.record.status == CapabilityEventRunStatus::Queued {
            let run_id = run.record.run_id.clone();
            conn.execute(
                "UPDATE pack_reconcile_runs SET from_generation_seq = ?1, to_generation_seq = ?2, updated_at_unix = ?3 WHERE run_id = ?4",
                params![
                    sql_i64(last_applied_generation)?,
                    sql_i64(latest_generation)?,
                    sql_i64(now)?,
                    &run_id,
                ],
            )
            .with_context(|| {
                format!(
                    "refreshing queued current-state consumer run `{}`",
                    run_id
                )
            })?;
            let mut refreshed = run.record.clone();
            refreshed.from_generation_seq = last_applied_generation;
            refreshed.to_generation_seq = latest_generation;
            refreshed.updated_at_unix = now;
            return Ok(Some(StoredRunRecord {
                record: refreshed,
                repo_root: run.repo_root,
            }));
        }
        return Ok(None);
    }

    let run_id = format!("current-state-consumer-run-{}", Uuid::new_v4());
    let record = CapabilityEventRunRecord {
        run_id: run_id.clone(),
        repo_id: repo_id.to_string(),
        capability_id: capability_id.to_string(),
        consumer_id: consumer_id.to_string(),
        handler_id: consumer_id.to_string(),
        from_generation_seq: last_applied_generation,
        to_generation_seq: latest_generation,
        reconcile_mode: "merged_delta".to_string(),
        event_kind: "current_state_consumer".to_string(),
        lane_key: build_lane_key(repo_id, consumer_id),
        event_payload_json: String::new(),
        status: CapabilityEventRunStatus::Queued,
        attempts: 0,
        submitted_at_unix: now,
        started_at_unix: None,
        updated_at_unix: now,
        completed_at_unix: None,
        error: None,
    };
    conn.execute(
        "INSERT INTO pack_reconcile_runs (run_id, repo_id, repo_root, consumer_id, capability_id, from_generation_seq, to_generation_seq, reconcile_mode, status, attempts, submitted_at_unix, started_at_unix, updated_at_unix, completed_at_unix, error) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL, ?12, NULL, NULL)",
        params![
            &record.run_id,
            &record.repo_id,
            repo_root.to_string_lossy().to_string(),
            &record.consumer_id,
            &record.capability_id,
            sql_i64(record.from_generation_seq)?,
            sql_i64(record.to_generation_seq)?,
            &record.reconcile_mode,
            record.status.to_string(),
            record.attempts,
            sql_i64(record.submitted_at_unix)?,
            sql_i64(record.updated_at_unix)?,
        ],
    )
    .with_context(|| format!("creating current-state consumer run `{run_id}`"))?;

    Ok(Some(StoredRunRecord {
        record,
        repo_root: repo_root.to_path_buf(),
    }))
}

pub(super) fn latest_generation_seq(
    conn: &rusqlite::Connection,
    repo_id: &str,
) -> Result<Option<u64>> {
    conn.query_row(
        "SELECT MAX(generation_seq) FROM pack_reconcile_generations WHERE repo_id = ?1",
        params![repo_id],
        |row| row.get::<_, Option<i64>>(0),
    )
    .map(|value| value.and_then(|v| u64::try_from(v).ok()))
    .map_err(anyhow::Error::from)
}

pub(super) fn load_consumer_cursor(
    conn: &rusqlite::Connection,
    repo_id: &str,
    consumer_id: &str,
) -> Result<Option<ConsumerCursorRow>> {
    conn.query_row(
        "SELECT last_applied_generation_seq FROM pack_reconcile_consumers WHERE repo_id = ?1 AND consumer_id = ?2",
        params![repo_id, consumer_id],
        |row| {
            Ok(ConsumerCursorRow {
                last_applied_generation_seq: row
                    .get::<_, Option<i64>>(0)?
                    .and_then(|value| u64::try_from(value).ok()),
            })
        },
    )
    .optional()
    .map_err(anyhow::Error::from)
}

fn load_active_run_for_lane(
    conn: &rusqlite::Connection,
    repo_id: &str,
    consumer_id: &str,
) -> Result<Option<StoredRunRecord>> {
    load_runs(
        conn,
        "SELECT run_id, repo_id, repo_root, consumer_id, capability_id, from_generation_seq, to_generation_seq, reconcile_mode, status, attempts, submitted_at_unix, started_at_unix, updated_at_unix, completed_at_unix, error FROM pack_reconcile_runs WHERE repo_id = ?1 AND consumer_id = ?2 AND status IN (?3, ?4) ORDER BY CASE status WHEN 'running' THEN 0 ELSE 1 END, submitted_at_unix ASC LIMIT 1",
        params![
            repo_id,
            consumer_id,
            CapabilityEventRunStatus::Running.to_string(),
            CapabilityEventRunStatus::Queued.to_string(),
        ],
    )
    .map(|mut runs| runs.pop())
}

pub(super) fn load_generations(
    conn: &rusqlite::Connection,
    repo_id: &str,
    from_generation_seq: u64,
    to_generation_seq: u64,
) -> Result<Vec<GenerationRow>> {
    let mut stmt = conn.prepare(
        "SELECT generation_seq, active_branch, head_commit_sha, requires_full_reconcile FROM pack_reconcile_generations WHERE repo_id = ?1 AND generation_seq >= ?2 AND generation_seq <= ?3 ORDER BY generation_seq ASC",
    )?;
    let rows = stmt
        .query_map(
            params![
                repo_id,
                sql_i64(from_generation_seq)?,
                sql_i64(to_generation_seq)?,
            ],
            |row| {
                Ok(GenerationRow {
                    generation_seq: u64::try_from(row.get::<_, i64>(0)?).unwrap_or_default(),
                    active_branch: row.get(1)?,
                    head_commit_sha: row.get(2)?,
                    requires_full_reconcile: row.get::<_, i64>(3)? != 0,
                })
            },
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub(super) fn load_file_changes(
    conn: &rusqlite::Connection,
    repo_id: &str,
    from_generation_seq: u64,
    to_generation_seq: u64,
) -> Result<Vec<FileChangeRow>> {
    let mut stmt = conn.prepare(
        "SELECT generation_seq, path, change_kind, language, content_id FROM pack_reconcile_file_changes WHERE repo_id = ?1 AND generation_seq >= ?2 AND generation_seq <= ?3 ORDER BY generation_seq ASC, rowid ASC",
    )?;
    let rows = stmt
        .query_map(
            params![
                repo_id,
                sql_i64(from_generation_seq)?,
                sql_i64(to_generation_seq)?,
            ],
            |row| {
                Ok(FileChangeRow {
                    generation_seq: u64::try_from(row.get::<_, i64>(0)?).unwrap_or_default(),
                    path: row.get(1)?,
                    change_kind: row.get(2)?,
                    language: row.get(3)?,
                    content_id: row.get(4)?,
                })
            },
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub(super) fn load_artefact_changes(
    conn: &rusqlite::Connection,
    repo_id: &str,
    from_generation_seq: u64,
    to_generation_seq: u64,
) -> Result<Vec<ArtefactChangeRow>> {
    let mut stmt = conn.prepare(
        "SELECT generation_seq, symbol_id, change_kind, artefact_id, path, canonical_kind, name FROM pack_reconcile_artefact_changes WHERE repo_id = ?1 AND generation_seq >= ?2 AND generation_seq <= ?3 ORDER BY generation_seq ASC, rowid ASC",
    )?;
    let rows = stmt
        .query_map(
            params![
                repo_id,
                sql_i64(from_generation_seq)?,
                sql_i64(to_generation_seq)?,
            ],
            |row| {
                Ok(ArtefactChangeRow {
                    generation_seq: u64::try_from(row.get::<_, i64>(0)?).unwrap_or_default(),
                    symbol_id: row.get(1)?,
                    change_kind: row.get(2)?,
                    artefact_id: row.get(3)?,
                    path: row.get(4)?,
                    canonical_kind: row.get(5)?,
                    name: row.get(6)?,
                })
            },
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[allow(dead_code)]
pub(super) fn load_run_by_id(
    conn: &rusqlite::Connection,
    run_id: &str,
) -> Result<Option<StoredRunRecord>> {
    load_runs(
        conn,
        "SELECT run_id, repo_id, repo_root, consumer_id, capability_id, from_generation_seq, to_generation_seq, reconcile_mode, status, attempts, submitted_at_unix, started_at_unix, updated_at_unix, completed_at_unix, error FROM pack_reconcile_runs WHERE run_id = ?1 LIMIT 1",
        params![run_id],
    )
    .map(|mut runs| runs.pop())
}

pub(super) fn load_runs<P>(
    conn: &rusqlite::Connection,
    sql: &str,
    params: P,
) -> Result<Vec<StoredRunRecord>>
where
    P: rusqlite::Params,
{
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map(params, |row| {
            Ok(StoredRunRecord {
                record: CapabilityEventRunRecord {
                    run_id: row.get(0)?,
                    repo_id: row.get(1)?,
                    capability_id: row.get(4)?,
                    consumer_id: row.get(3)?,
                    handler_id: row.get(3)?,
                    from_generation_seq: u64::try_from(row.get::<_, i64>(5)?).unwrap_or_default(),
                    to_generation_seq: u64::try_from(row.get::<_, i64>(6)?).unwrap_or_default(),
                    reconcile_mode: row.get(7)?,
                    event_kind: "current_state_consumer".to_string(),
                    lane_key: build_lane_key(&row.get::<_, String>(1)?, &row.get::<_, String>(3)?),
                    event_payload_json: String::new(),
                    status: parse_run_status(&row.get::<_, String>(8)?),
                    attempts: row.get(9)?,
                    submitted_at_unix: u64::try_from(row.get::<_, i64>(10)?).unwrap_or_default(),
                    started_at_unix: row
                        .get::<_, Option<i64>>(11)?
                        .and_then(|value| u64::try_from(value).ok()),
                    updated_at_unix: u64::try_from(row.get::<_, i64>(12)?).unwrap_or_default(),
                    completed_at_unix: row
                        .get::<_, Option<i64>>(13)?
                        .and_then(|value| u64::try_from(value).ok()),
                    error: row.get(14)?,
                },
                repo_root: row.get::<_, String>(2)?.into(),
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn build_lane_key(repo_id: &str, consumer_id: &str) -> String {
    format!("{repo_id}:{consumer_id}")
}

fn parse_run_status(value: &str) -> CapabilityEventRunStatus {
    match value {
        "running" => CapabilityEventRunStatus::Running,
        "completed" => CapabilityEventRunStatus::Completed,
        "failed" => CapabilityEventRunStatus::Failed,
        "cancelled" => CapabilityEventRunStatus::Cancelled,
        _ => CapabilityEventRunStatus::Queued,
    }
}

pub(super) fn prune_terminal_runs(conn: &rusqlite::Connection) -> Result<()> {
    conn.execute(
        "DELETE FROM pack_reconcile_runs WHERE run_id IN (
            SELECT run_id FROM pack_reconcile_runs
            WHERE status IN (?1, ?2, ?3)
            ORDER BY COALESCE(completed_at_unix, updated_at_unix) DESC
            LIMIT -1 OFFSET 100
        )",
        params![
            CapabilityEventRunStatus::Completed.to_string(),
            CapabilityEventRunStatus::Failed.to_string(),
            CapabilityEventRunStatus::Cancelled.to_string(),
        ],
    )
    .context("pruning historical current-state consumer runs")?;
    Ok(())
}

pub(super) fn sql_i64(value: u64) -> Result<i64> {
    i64::try_from(value).context("converting unsigned runtime value to SQLite integer")
}

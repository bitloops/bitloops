use std::path::PathBuf;

use anyhow::Result;
use rusqlite::params;

use crate::host::runtime_store::{WorkplaneJobRecord, WorkplaneJobStatus};

pub(crate) fn load_workplane_jobs_by_status(
    conn: &rusqlite::Connection,
    status: WorkplaneJobStatus,
) -> Result<Vec<WorkplaneJobRecord>> {
    let mut stmt = conn.prepare(
        "SELECT job_id, repo_id, repo_root, config_root, capability_id, mailbox_name,
                init_session_id, dedupe_key, payload, status, attempts, available_at_unix, submitted_at_unix,
                started_at_unix, updated_at_unix, completed_at_unix, lease_owner,
                lease_expires_at_unix, last_error
         FROM capability_workplane_jobs
         WHERE status = ?1
         ORDER BY CASE mailbox_name
                      WHEN 'semantic_clones.embedding.code' THEN 0
                      WHEN 'semantic_clones.embedding.summary' THEN 0
                      WHEN 'semantic_clones.summary_refresh' THEN 1
                      WHEN 'semantic_clones.clone_rebuild' THEN 2
                  ELSE 3
                  END ASC,
                  available_at_unix ASC,
                  submitted_at_unix ASC",
    )?;
    let rows = stmt.query_map(params![status.as_str()], map_workplane_job_row)?;
    let mut values = Vec::new();
    for row in rows {
        values.push(row?);
    }
    Ok(values)
}

pub(crate) fn map_workplane_job_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<WorkplaneJobRecord> {
    let payload_raw = row.get::<_, String>(8)?;
    Ok(WorkplaneJobRecord {
        job_id: row.get(0)?,
        repo_id: row.get(1)?,
        repo_root: PathBuf::from(row.get::<_, String>(2)?),
        config_root: PathBuf::from(row.get::<_, String>(3)?),
        capability_id: row.get(4)?,
        mailbox_name: row.get(5)?,
        init_session_id: row.get(6)?,
        dedupe_key: row.get(7)?,
        payload: serde_json::from_str(&payload_raw).unwrap_or(serde_json::Value::Null),
        status: WorkplaneJobStatus::parse(&row.get::<_, String>(9)?),
        attempts: row.get(10)?,
        available_at_unix: u64::try_from(row.get::<_, i64>(11)?).unwrap_or_default(),
        submitted_at_unix: u64::try_from(row.get::<_, i64>(12)?).unwrap_or_default(),
        started_at_unix: row
            .get::<_, Option<i64>>(13)?
            .and_then(|value| u64::try_from(value).ok()),
        updated_at_unix: u64::try_from(row.get::<_, i64>(14)?).unwrap_or_default(),
        completed_at_unix: row
            .get::<_, Option<i64>>(15)?
            .and_then(|value| u64::try_from(value).ok()),
        lease_owner: row.get(16)?,
        lease_expires_at_unix: row
            .get::<_, Option<i64>>(17)?
            .and_then(|value| u64::try_from(value).ok()),
        last_error: row.get(18)?,
    })
}

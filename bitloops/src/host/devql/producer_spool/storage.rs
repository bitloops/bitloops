use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::host::runtime_store::open_runtime_sqlite_for_config_root;
use crate::storage::SqliteConnectionPool;

use super::{ProducerSpoolJobPayload, ProducerSpoolJobRecord, ProducerSpoolJobStatus};

pub(super) fn open_repo_runtime_sqlite_for_config_root(
    config_root: &Path,
) -> Result<SqliteConnectionPool> {
    open_runtime_sqlite_for_config_root(config_root)
}

pub(super) fn map_producer_spool_job_record_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ProducerSpoolJobRecord> {
    let payload_raw = row.get::<_, String>(9)?;
    let payload =
        serde_json::from_str(&payload_raw).unwrap_or(ProducerSpoolJobPayload::PrePushSync {
            remote: String::new(),
            stdin_lines: Vec::new(),
        });
    Ok(ProducerSpoolJobRecord {
        job_id: row.get(0)?,
        repo_id: row.get(1)?,
        repo_root: PathBuf::from(row.get::<_, String>(2)?),
        config_root: PathBuf::from(row.get::<_, String>(3)?),
        repo_name: row.get(4)?,
        repo_provider: row.get(5)?,
        repo_organisation: row.get(6)?,
        repo_identity: row.get(7)?,
        dedupe_key: row.get(8)?,
        payload,
        status: ProducerSpoolJobStatus::parse(&row.get::<_, String>(10)?),
        attempts: row.get(11)?,
        available_at_unix: parse_u64(row.get::<_, i64>(12)?),
        submitted_at_unix: parse_u64(row.get::<_, i64>(13)?),
        updated_at_unix: parse_u64(row.get::<_, i64>(14)?),
        last_error: row.get(15)?,
    })
}

pub(super) fn sql_i64(value: u64) -> Result<i64> {
    i64::try_from(value).context("converting DevQL producer spool integer to sqlite i64")
}

pub(super) fn unix_timestamp_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn parse_u64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or_default()
}

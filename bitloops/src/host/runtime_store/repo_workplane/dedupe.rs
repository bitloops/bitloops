//! Dedupe lookups and row mappers for workplane jobs and semantic mailbox items.

use anyhow::Result;
use rusqlite::{OptionalExtension, params};
use serde_json::Value;
use std::path::PathBuf;

use super::types::{
    SemanticEmbeddingMailboxItemRecord, SemanticMailboxItemKind, SemanticMailboxItemStatus,
    SemanticSummaryMailboxItemRecord, WorkplaneJobRecord, WorkplaneJobStatus,
};
use super::util::parse_u64;

pub(crate) fn load_deduped_job(
    conn: &rusqlite::Connection,
    repo_id: &str,
    capability_id: &str,
    mailbox_name: &str,
    init_session_id: Option<&str>,
    dedupe_key: Option<&str>,
) -> Result<Option<WorkplaneJobRecord>> {
    let Some(dedupe_key) = dedupe_key else {
        return Ok(None);
    };
    if let Some(init_session_id) = init_session_id {
        return conn
            .query_row(
                "SELECT job_id, repo_id, repo_root, config_root, capability_id, mailbox_name,
                        init_session_id, dedupe_key, payload, status, attempts, available_at_unix, submitted_at_unix,
                        started_at_unix, updated_at_unix, completed_at_unix, lease_owner,
                        lease_expires_at_unix, last_error
                 FROM capability_workplane_jobs
                 WHERE repo_id = ?1
                   AND capability_id = ?2
                   AND mailbox_name = ?3
                   AND init_session_id = ?4
                   AND dedupe_key = ?5
                   AND status IN (?6, ?7)
                 ORDER BY CASE status WHEN 'running' THEN 0 ELSE 1 END, submitted_at_unix ASC
                 LIMIT 1",
                params![
                    repo_id,
                    capability_id,
                    mailbox_name,
                    init_session_id,
                    dedupe_key,
                    WorkplaneJobStatus::Running.as_str(),
                    WorkplaneJobStatus::Pending.as_str(),
                ],
                map_workplane_job_record_row,
            )
            .optional()
            .map_err(anyhow::Error::from);
    }

    conn.query_row(
        "SELECT job_id, repo_id, repo_root, config_root, capability_id, mailbox_name,
                init_session_id, dedupe_key, payload, status, attempts, available_at_unix, submitted_at_unix,
                started_at_unix, updated_at_unix, completed_at_unix, lease_owner,
                lease_expires_at_unix, last_error
         FROM capability_workplane_jobs
         WHERE repo_id = ?1
           AND capability_id = ?2
           AND mailbox_name = ?3
           AND init_session_id IS NULL
           AND dedupe_key = ?4
           AND status IN (?5, ?6)
         ORDER BY CASE status WHEN 'running' THEN 0 ELSE 1 END, submitted_at_unix ASC
         LIMIT 1",
        params![
            repo_id,
            capability_id,
            mailbox_name,
            dedupe_key,
            WorkplaneJobStatus::Running.as_str(),
            WorkplaneJobStatus::Pending.as_str(),
        ],
        map_workplane_job_record_row,
    )
    .optional()
    .map_err(anyhow::Error::from)
}

pub(crate) fn load_deduped_summary_mailbox_item(
    conn: &rusqlite::Connection,
    repo_id: &str,
    dedupe_key: Option<&str>,
) -> Result<Option<SemanticSummaryMailboxItemRecord>> {
    let Some(dedupe_key) = dedupe_key else {
        return Ok(None);
    };
    conn.query_row(
        "SELECT item_id, repo_id, repo_root, config_root, init_session_id, item_kind,
                artefact_id, payload_json, dedupe_key, status, attempts, available_at_unix,
                submitted_at_unix, leased_at_unix, lease_expires_at_unix, lease_token,
                updated_at_unix, last_error
         FROM semantic_summary_mailbox_items
         WHERE repo_id = ?1
           AND dedupe_key = ?2
           AND status IN (?3, ?4, ?5)
         ORDER BY CASE status
                      WHEN 'leased' THEN 0
                      WHEN 'pending' THEN 1
                      ELSE 2
                  END,
                  submitted_at_unix ASC
         LIMIT 1",
        params![
            repo_id,
            dedupe_key,
            SemanticMailboxItemStatus::Leased.as_str(),
            SemanticMailboxItemStatus::Pending.as_str(),
            SemanticMailboxItemStatus::Failed.as_str(),
        ],
        map_summary_mailbox_item_record_row,
    )
    .optional()
    .map_err(anyhow::Error::from)
}

pub(crate) fn load_deduped_embedding_mailbox_item(
    conn: &rusqlite::Connection,
    repo_id: &str,
    representation_kind: &str,
    dedupe_key: Option<&str>,
) -> Result<Option<SemanticEmbeddingMailboxItemRecord>> {
    let Some(dedupe_key) = dedupe_key else {
        return Ok(None);
    };
    conn.query_row(
        "SELECT item_id, repo_id, repo_root, config_root, init_session_id, representation_kind,
                item_kind, artefact_id, payload_json, dedupe_key, status, attempts,
                available_at_unix, submitted_at_unix, leased_at_unix, lease_expires_at_unix,
                lease_token, updated_at_unix, last_error
         FROM semantic_embedding_mailbox_items
         WHERE repo_id = ?1
           AND representation_kind = ?2
           AND dedupe_key = ?3
           AND status IN (?4, ?5, ?6)
         ORDER BY CASE status
                      WHEN 'leased' THEN 0
                      WHEN 'pending' THEN 1
                      ELSE 2
                  END,
                  submitted_at_unix ASC
         LIMIT 1",
        params![
            repo_id,
            representation_kind,
            dedupe_key,
            SemanticMailboxItemStatus::Leased.as_str(),
            SemanticMailboxItemStatus::Pending.as_str(),
            SemanticMailboxItemStatus::Failed.as_str(),
        ],
        map_embedding_mailbox_item_record_row,
    )
    .optional()
    .map_err(anyhow::Error::from)
}

fn map_workplane_job_record_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkplaneJobRecord> {
    let payload_raw = row.get::<_, String>(8)?;
    let payload = serde_json::from_str(&payload_raw).unwrap_or(Value::Null);
    Ok(WorkplaneJobRecord {
        job_id: row.get(0)?,
        repo_id: row.get(1)?,
        repo_root: PathBuf::from(row.get::<_, String>(2)?),
        config_root: PathBuf::from(row.get::<_, String>(3)?),
        capability_id: row.get(4)?,
        mailbox_name: row.get(5)?,
        init_session_id: row.get(6)?,
        dedupe_key: row.get(7)?,
        payload,
        status: WorkplaneJobStatus::parse(&row.get::<_, String>(9)?),
        attempts: row.get(10)?,
        available_at_unix: parse_u64(row.get::<_, i64>(11)?),
        submitted_at_unix: parse_u64(row.get::<_, i64>(12)?),
        started_at_unix: row.get::<_, Option<i64>>(13)?.map(parse_u64),
        updated_at_unix: parse_u64(row.get::<_, i64>(14)?),
        completed_at_unix: row.get::<_, Option<i64>>(15)?.map(parse_u64),
        lease_owner: row.get(16)?,
        lease_expires_at_unix: row.get::<_, Option<i64>>(17)?.map(parse_u64),
        last_error: row.get(18)?,
    })
}

fn map_summary_mailbox_item_record_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<SemanticSummaryMailboxItemRecord> {
    let payload_json = row
        .get::<_, Option<String>>(7)?
        .and_then(|raw| serde_json::from_str(&raw).ok());
    Ok(SemanticSummaryMailboxItemRecord {
        item_id: row.get(0)?,
        repo_id: row.get(1)?,
        repo_root: PathBuf::from(row.get::<_, String>(2)?),
        config_root: PathBuf::from(row.get::<_, String>(3)?),
        init_session_id: row.get(4)?,
        item_kind: SemanticMailboxItemKind::parse(&row.get::<_, String>(5)?),
        artefact_id: row.get(6)?,
        payload_json,
        dedupe_key: row.get(8)?,
        status: SemanticMailboxItemStatus::parse(&row.get::<_, String>(9)?),
        attempts: row.get(10)?,
        available_at_unix: parse_u64(row.get::<_, i64>(11)?),
        submitted_at_unix: parse_u64(row.get::<_, i64>(12)?),
        leased_at_unix: row.get::<_, Option<i64>>(13)?.map(parse_u64),
        lease_expires_at_unix: row.get::<_, Option<i64>>(14)?.map(parse_u64),
        lease_token: row.get(15)?,
        updated_at_unix: parse_u64(row.get::<_, i64>(16)?),
        last_error: row.get(17)?,
    })
}

fn map_embedding_mailbox_item_record_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<SemanticEmbeddingMailboxItemRecord> {
    let payload_json = row
        .get::<_, Option<String>>(8)?
        .and_then(|raw| serde_json::from_str(&raw).ok());
    Ok(SemanticEmbeddingMailboxItemRecord {
        item_id: row.get(0)?,
        repo_id: row.get(1)?,
        repo_root: PathBuf::from(row.get::<_, String>(2)?),
        config_root: PathBuf::from(row.get::<_, String>(3)?),
        init_session_id: row.get(4)?,
        representation_kind: row.get(5)?,
        item_kind: SemanticMailboxItemKind::parse(&row.get::<_, String>(6)?),
        artefact_id: row.get(7)?,
        payload_json,
        dedupe_key: row.get(9)?,
        status: SemanticMailboxItemStatus::parse(&row.get::<_, String>(10)?),
        attempts: row.get(11)?,
        available_at_unix: parse_u64(row.get::<_, i64>(12)?),
        submitted_at_unix: parse_u64(row.get::<_, i64>(13)?),
        leased_at_unix: row.get::<_, Option<i64>>(14)?.map(parse_u64),
        lease_expires_at_unix: row.get::<_, Option<i64>>(15)?.map(parse_u64),
        lease_token: row.get(16)?,
        updated_at_unix: parse_u64(row.get::<_, i64>(17)?),
        last_error: row.get(18)?,
    })
}

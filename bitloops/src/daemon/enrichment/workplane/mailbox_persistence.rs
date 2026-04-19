use std::path::PathBuf;

use anyhow::Result;
use rusqlite::params;

use crate::daemon::types::unix_timestamp_now;
use crate::host::runtime_store::{
    DaemonSqliteRuntimeStore, SemanticEmbeddingMailboxItemRecord, SemanticMailboxItemKind,
    SemanticMailboxItemStatus, SemanticSummaryMailboxItemRecord,
};

use super::job_completion::{
    WORKPLANE_TRANSIENT_EMBEDDING_RETRY_LIMIT, WorkplaneJobCompletionDisposition,
    transient_embedding_retry_backoff_secs,
};
use super::mailbox_claim::{ClaimedEmbeddingMailboxBatch, ClaimedSummaryMailboxBatch};
use super::sql::{parse_u64, sql_i64, sql_string_list};

pub(crate) fn fail_summary_mailbox_batch(
    workplane_store: &DaemonSqliteRuntimeStore,
    batch: &ClaimedSummaryMailboxBatch,
    error: &str,
) -> Result<()> {
    update_summary_mailbox_batch_failure(
        workplane_store,
        batch,
        SemanticMailboxItemStatus::Failed,
        None,
        error,
    )
}

pub(crate) fn requeue_summary_mailbox_batch(
    workplane_store: &DaemonSqliteRuntimeStore,
    batch: &ClaimedSummaryMailboxBatch,
    retry_in_secs: u64,
    error: &str,
) -> Result<()> {
    update_summary_mailbox_batch_failure(
        workplane_store,
        batch,
        SemanticMailboxItemStatus::Pending,
        Some(retry_in_secs),
        error,
    )
}

pub(crate) fn persist_embedding_mailbox_batch_failure(
    workplane_store: &DaemonSqliteRuntimeStore,
    batch: &ClaimedEmbeddingMailboxBatch,
    error: &str,
) -> Result<WorkplaneJobCompletionDisposition> {
    let now = unix_timestamp_now();
    let attempts = batch
        .items
        .iter()
        .map(|item| item.attempts)
        .max()
        .unwrap_or(0);
    let disposition = if attempts < WORKPLANE_TRANSIENT_EMBEDDING_RETRY_LIMIT
        && error.contains("timed out after")
    {
        let retry_in_secs = transient_embedding_retry_backoff_secs(attempts);
        WorkplaneJobCompletionDisposition::RetryScheduled {
            available_at_unix: now.saturating_add(retry_in_secs),
            retry_in_secs,
        }
    } else {
        WorkplaneJobCompletionDisposition::Failed
    };
    match disposition {
        WorkplaneJobCompletionDisposition::RetryScheduled { retry_in_secs, .. } => {
            update_embedding_mailbox_batch_failure(
                workplane_store,
                batch,
                SemanticMailboxItemStatus::Pending,
                Some(retry_in_secs),
                error,
            )?;
        }
        WorkplaneJobCompletionDisposition::Failed => {
            update_embedding_mailbox_batch_failure(
                workplane_store,
                batch,
                SemanticMailboxItemStatus::Failed,
                None,
                error,
            )?;
        }
        WorkplaneJobCompletionDisposition::Completed => {}
    }
    Ok(disposition)
}

pub(crate) fn requeue_embedding_mailbox_batch(
    workplane_store: &DaemonSqliteRuntimeStore,
    batch: &ClaimedEmbeddingMailboxBatch,
    retry_in_secs: u64,
    error: &str,
) -> Result<()> {
    update_embedding_mailbox_batch_failure(
        workplane_store,
        batch,
        SemanticMailboxItemStatus::Pending,
        Some(retry_in_secs),
        error,
    )
}

fn update_summary_mailbox_batch_failure(
    workplane_store: &DaemonSqliteRuntimeStore,
    batch: &ClaimedSummaryMailboxBatch,
    status: SemanticMailboxItemStatus,
    retry_in_secs: Option<u64>,
    error: &str,
) -> Result<()> {
    let now = unix_timestamp_now();
    workplane_store.with_connection(|conn| {
        conn.execute(
            &format!(
                "UPDATE semantic_summary_mailbox_items
                 SET status = ?1,
                     available_at_unix = ?2,
                     leased_at_unix = NULL,
                     lease_expires_at_unix = NULL,
                     lease_token = NULL,
                     updated_at_unix = ?3,
                     last_error = ?4
                 WHERE lease_token = ?5
                   AND item_id IN ({})",
                sql_string_list(
                    &batch
                        .items
                        .iter()
                        .map(|item| item.item_id.clone())
                        .collect::<Vec<_>>(),
                )
            ),
            params![
                status.as_str(),
                sql_i64(now.saturating_add(retry_in_secs.unwrap_or(0)))?,
                sql_i64(now)?,
                error,
                &batch.lease_token,
            ],
        )?;
        Ok(())
    })
}

fn update_embedding_mailbox_batch_failure(
    workplane_store: &DaemonSqliteRuntimeStore,
    batch: &ClaimedEmbeddingMailboxBatch,
    status: SemanticMailboxItemStatus,
    retry_in_secs: Option<u64>,
    error: &str,
) -> Result<()> {
    let now = unix_timestamp_now();
    workplane_store.with_connection(|conn| {
        conn.execute(
            &format!(
                "UPDATE semantic_embedding_mailbox_items
                 SET status = ?1,
                     available_at_unix = ?2,
                     leased_at_unix = NULL,
                     lease_expires_at_unix = NULL,
                     lease_token = NULL,
                     updated_at_unix = ?3,
                     last_error = ?4
                 WHERE lease_token = ?5
                   AND item_id IN ({})",
                sql_string_list(
                    &batch
                        .items
                        .iter()
                        .map(|item| item.item_id.clone())
                        .collect::<Vec<_>>(),
                )
            ),
            params![
                status.as_str(),
                sql_i64(now.saturating_add(retry_in_secs.unwrap_or(0)))?,
                sql_i64(now)?,
                error,
                &batch.lease_token,
            ],
        )?;
        Ok(())
    })
}

pub(crate) fn load_summary_mailbox_items_by_ids(
    conn: &rusqlite::Connection,
    item_ids: &[String],
) -> Result<Vec<SemanticSummaryMailboxItemRecord>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT item_id, repo_id, repo_root, config_root, init_session_id, item_kind,
                    artefact_id, payload_json, dedupe_key, status, attempts, available_at_unix,
                    submitted_at_unix, leased_at_unix, lease_expires_at_unix, lease_token,
                    updated_at_unix, last_error
             FROM semantic_summary_mailbox_items
             WHERE item_id IN ({})
             ORDER BY submitted_at_unix ASC, item_id ASC",
        sql_string_list(item_ids)
    ))?;
    let rows = stmt.query_map([], map_summary_mailbox_item_row)?;
    let mut values = Vec::new();
    for row in rows {
        values.push(row?);
    }
    Ok(values)
}

pub(crate) fn load_summary_mailbox_items_by_status(
    conn: &rusqlite::Connection,
    status: SemanticMailboxItemStatus,
) -> Result<Vec<SemanticSummaryMailboxItemRecord>> {
    let mut stmt = conn.prepare(
        "SELECT item_id, repo_id, repo_root, config_root, init_session_id, item_kind,
                artefact_id, payload_json, dedupe_key, status, attempts, available_at_unix,
                submitted_at_unix, leased_at_unix, lease_expires_at_unix, lease_token,
                updated_at_unix, last_error
         FROM semantic_summary_mailbox_items
         WHERE status = ?1
         ORDER BY available_at_unix ASC, submitted_at_unix ASC, item_id ASC",
    )?;
    let rows = stmt.query_map(params![status.as_str()], map_summary_mailbox_item_row)?;
    let mut values = Vec::new();
    for row in rows {
        values.push(row?);
    }
    Ok(values)
}

pub(crate) fn load_embedding_mailbox_items_by_ids(
    conn: &rusqlite::Connection,
    item_ids: &[String],
) -> Result<Vec<SemanticEmbeddingMailboxItemRecord>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT item_id, repo_id, repo_root, config_root, init_session_id,
                    representation_kind, item_kind, artefact_id, payload_json, dedupe_key,
                    status, attempts, available_at_unix, submitted_at_unix, leased_at_unix,
                    lease_expires_at_unix, lease_token, updated_at_unix, last_error
             FROM semantic_embedding_mailbox_items
             WHERE item_id IN ({})
             ORDER BY submitted_at_unix ASC, item_id ASC",
        sql_string_list(item_ids)
    ))?;
    let rows = stmt.query_map([], map_embedding_mailbox_item_row)?;
    let mut values = Vec::new();
    for row in rows {
        values.push(row?);
    }
    Ok(values)
}

pub(crate) fn load_embedding_mailbox_items_by_status(
    conn: &rusqlite::Connection,
    status: SemanticMailboxItemStatus,
) -> Result<Vec<SemanticEmbeddingMailboxItemRecord>> {
    let mut stmt = conn.prepare(
        "SELECT item_id, repo_id, repo_root, config_root, init_session_id,
                representation_kind, item_kind, artefact_id, payload_json, dedupe_key,
                status, attempts, available_at_unix, submitted_at_unix, leased_at_unix,
                lease_expires_at_unix, lease_token, updated_at_unix, last_error
         FROM semantic_embedding_mailbox_items
         WHERE status = ?1
         ORDER BY available_at_unix ASC, submitted_at_unix ASC, item_id ASC",
    )?;
    let rows = stmt.query_map(params![status.as_str()], map_embedding_mailbox_item_row)?;
    let mut values = Vec::new();
    for row in rows {
        values.push(row?);
    }
    Ok(values)
}

fn map_summary_mailbox_item_row(
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

fn map_embedding_mailbox_item_row(
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

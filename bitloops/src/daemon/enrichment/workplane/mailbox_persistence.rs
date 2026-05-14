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
    )?;
    log::warn!(
        "summary mailbox batch requeued: repo_id={} leased_count={} attempts={} retry_in_secs={} failure_stage={} failure_kind={}",
        batch.repo_id,
        batch.items.len(),
        batch
            .items
            .iter()
            .map(|item| item.attempts)
            .max()
            .unwrap_or(0),
        retry_in_secs,
        classify_summary_retry_failure_stage(error),
        classify_summary_retry_failure_kind(error),
    );
    Ok(())
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
    workplane_store.with_write_connection(|conn| {
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

fn classify_summary_retry_failure_stage(error: &str) -> &'static str {
    let lower = error.to_ascii_lowercase();
    if lower.contains("executing semantic summary sql") {
        "summary_sql"
    } else if lower.contains("inserting runtime summary mailbox item") {
        "runtime_summary_mailbox_insert"
    } else if lower.contains("refreshing pending runtime embedding mailbox item")
        || lower.contains("inserting runtime embedding mailbox item")
    {
        "runtime_embedding_mailbox_upsert"
    } else if lower.contains("deleting acknowledged semantic summary mailbox items") {
        "runtime_summary_mailbox_delete"
    } else if lower.contains("committing semantic summary batch transaction") {
        "transaction_commit"
    } else {
        "unknown"
    }
}

fn classify_summary_retry_failure_kind(error: &str) -> &'static str {
    let lower = error.to_ascii_lowercase();
    if lower.contains("database is locked")
        || lower.contains("busy timeout")
        || lower.contains("error code 5")
    {
        "sqlite_lock_or_busy_timeout"
    } else {
        "other"
    }
}

fn update_embedding_mailbox_batch_failure(
    workplane_store: &DaemonSqliteRuntimeStore,
    batch: &ClaimedEmbeddingMailboxBatch,
    status: SemanticMailboxItemStatus,
    retry_in_secs: Option<u64>,
    error: &str,
) -> Result<()> {
    let now = unix_timestamp_now();
    workplane_store.with_write_connection(|conn| {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::runtime_store::SemanticSummaryMailboxItemRecord;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[test]
    fn summary_retry_classification_distinguishes_stage_and_lock_contention() {
        let delete_locked = "committing semantic summary batch for repo `repo-1`: deleting acknowledged semantic summary mailbox items: database is locked";
        assert_eq!(
            classify_summary_retry_failure_stage(delete_locked),
            "runtime_summary_mailbox_delete"
        );
        assert_eq!(
            classify_summary_retry_failure_kind(delete_locked),
            "sqlite_lock_or_busy_timeout"
        );

        let commit_failure = "committing semantic summary batch for repo `repo-1`: committing semantic summary batch transaction: constraint failed";
        assert_eq!(
            classify_summary_retry_failure_stage(commit_failure),
            "transaction_commit"
        );
        assert_eq!(classify_summary_retry_failure_kind(commit_failure), "other");

        let sql_failure = "committing semantic summary batch for repo `repo-1`: executing semantic summary SQL: syntax error";
        assert_eq!(
            classify_summary_retry_failure_stage(sql_failure),
            "summary_sql"
        );
    }

    #[test]
    fn requeue_summary_mailbox_batch_restores_pending_state_and_clears_lease_fields() {
        let temp = TempDir::new().expect("temp dir");
        let store = DaemonSqliteRuntimeStore::open_at(temp.path().join("runtime.sqlite"))
            .expect("open runtime store");
        let lease_token = "semantic-summary-lease-test";
        let item_id = "summary-item-1";
        let repo_root = PathBuf::from("/tmp/repo");
        let config_root = PathBuf::from("/tmp/config");
        store
            .with_connection(|conn| {
                conn.execute(
                    "INSERT INTO semantic_summary_mailbox_items (
                         item_id, repo_id, repo_root, config_root, init_session_id, item_kind,
                         artefact_id, payload_json, dedupe_key, status, attempts, available_at_unix,
                         submitted_at_unix, leased_at_unix, lease_expires_at_unix, lease_token,
                         updated_at_unix, last_error
                     ) VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6, NULL, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, NULL)",
                    params![
                        item_id,
                        "repo-1",
                        repo_root.to_string_lossy().to_string(),
                        config_root.to_string_lossy().to_string(),
                        SemanticMailboxItemKind::Artefact.as_str(),
                        "artefact-1",
                        "semantic_clones.summary_refresh:artefact-1",
                        SemanticMailboxItemStatus::Leased.as_str(),
                        3_u32,
                        sql_i64(10)?,
                        sql_i64(10)?,
                        sql_i64(10)?,
                        sql_i64(20)?,
                        lease_token,
                        sql_i64(10)?,
                    ],
                )?;
                Ok::<_, anyhow::Error>(())
            })
            .expect("insert leased summary mailbox row");

        let batch = ClaimedSummaryMailboxBatch {
            repo_id: "repo-1".to_string(),
            repo_root: repo_root.clone(),
            config_root: config_root.clone(),
            lease_token: lease_token.to_string(),
            items: vec![SemanticSummaryMailboxItemRecord {
                item_id: item_id.to_string(),
                repo_id: "repo-1".to_string(),
                repo_root,
                config_root,
                init_session_id: None,
                item_kind: SemanticMailboxItemKind::Artefact,
                artefact_id: Some("artefact-1".to_string()),
                payload_json: None,
                dedupe_key: Some("semantic_clones.summary_refresh:artefact-1".to_string()),
                status: SemanticMailboxItemStatus::Leased,
                attempts: 3,
                available_at_unix: 10,
                submitted_at_unix: 10,
                leased_at_unix: Some(10),
                lease_expires_at_unix: Some(20),
                lease_token: Some(lease_token.to_string()),
                updated_at_unix: 10,
                last_error: None,
            }],
        };

        requeue_summary_mailbox_batch(
            &store,
            &batch,
            5,
            "committing semantic summary batch for repo `repo-1`: deleting acknowledged semantic summary mailbox items: database is locked",
        )
        .expect("requeue summary mailbox batch");

        let pending = store
            .with_connection(|conn| {
                load_summary_mailbox_items_by_status(conn, SemanticMailboxItemStatus::Pending)
            })
            .expect("load pending summary mailbox items");
        assert_eq!(pending.len(), 1);
        let item = &pending[0];
        assert_eq!(item.item_id, item_id);
        assert_eq!(item.status, SemanticMailboxItemStatus::Pending);
        assert_eq!(item.attempts, 3);
        assert!(item.leased_at_unix.is_none());
        assert!(item.lease_expires_at_unix.is_none());
        assert!(item.lease_token.is_none());
        assert_eq!(
            item.last_error.as_deref(),
            Some(
                "committing semantic summary batch for repo `repo-1`: deleting acknowledged semantic summary mailbox items: database is locked"
            )
        );
        assert_eq!(
            item.available_at_unix.saturating_sub(item.updated_at_unix),
            5
        );
    }
}

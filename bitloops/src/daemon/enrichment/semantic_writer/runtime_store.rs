use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags, OptionalExtension, Transaction};
use std::path::Path;
use std::time::Duration;

use crate::host::devql::esc_pg;
use crate::host::runtime_store::{
    CapabilityWorkplaneJobInsert, SemanticEmbeddingMailboxItemInsert,
    SemanticSummaryMailboxItemInsert,
};

use super::SemanticBatchRepoContext;

pub(super) fn open_semantic_writer_connection(
    runtime_db_path: &Path,
    relational_db_path: &Path,
) -> Result<Connection> {
    if !runtime_db_path.is_file() {
        anyhow::bail!(
            "runtime SQLite database not found at {}",
            runtime_db_path.display()
        );
    }
    if !relational_db_path.is_file() {
        anyhow::bail!(
            "relational SQLite database not found at {}",
            relational_db_path.display()
        );
    }
    crate::sqlite_vec_auto_extension::register_sqlite_vec_auto_extension()
        .context("registering sqlite-vec auto-extension for semantic writer connection")?;
    let conn = Connection::open_with_flags(relational_db_path, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .with_context(|| {
        format!(
            "opening SQLite database at {}",
            relational_db_path.display()
        )
    })?;
    conn.busy_timeout(Duration::from_secs(30))
        .context("setting semantic writer busy timeout")?;
    conn.execute_batch(
        "PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;",
    )
    .context("configuring semantic writer connection")?;
    conn.execute(
        "ATTACH DATABASE ?1 AS runtime_store",
        [runtime_db_path.to_string_lossy().to_string()],
    )
    .context("attaching runtime SQLite database to semantic writer connection")?;
    Ok(conn)
}

pub(super) fn upsert_runtime_embedding_mailbox_item(
    tx: &Transaction<'_>,
    repo: &SemanticBatchRepoContext,
    item: &SemanticEmbeddingMailboxItemInsert,
) -> Result<()> {
    let existing = tx
        .query_row(
            "SELECT item_id, status
             FROM runtime_store.semantic_embedding_mailbox_items
             WHERE repo_id = ?1
               AND representation_kind = ?2
               AND dedupe_key = ?3
               AND status IN ('pending', 'leased', 'failed')
             ORDER BY CASE status
                          WHEN 'leased' THEN 0
                          WHEN 'pending' THEN 1
                          ELSE 2
                      END,
                      submitted_at_unix ASC
             LIMIT 1",
            rusqlite::params![
                repo.repo_id,
                item.representation_kind,
                item.dedupe_key.as_deref()
            ],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    let now = unix_timestamp_now();
    if let Some((item_id, status)) = existing {
        if status == "pending" {
            tx.execute(
                "UPDATE runtime_store.semantic_embedding_mailbox_items
                 SET init_session_id = COALESCE(init_session_id, ?1),
                     payload_json = ?2,
                     available_at_unix = ?3,
                     updated_at_unix = ?4,
                     last_error = NULL
                 WHERE item_id = ?5",
                rusqlite::params![
                    item.init_session_id.as_deref(),
                    item.payload_json.as_ref().map(serde_json::Value::to_string),
                    sql_i64(now)?,
                    sql_i64(now)?,
                    item_id,
                ],
            )
            .context("refreshing pending runtime embedding mailbox item")?;
        }
        return Ok(());
    }

    insert_runtime_embedding_mailbox_item(tx, repo, item)
}

pub(super) fn insert_runtime_embedding_mailbox_item(
    tx: &Transaction<'_>,
    repo: &SemanticBatchRepoContext,
    item: &SemanticEmbeddingMailboxItemInsert,
) -> Result<()> {
    let now = unix_timestamp_now();
    let item_id = format!("semantic-embedding-mailbox-item-{}", uuid::Uuid::new_v4());
    tx.execute(
        "INSERT INTO runtime_store.semantic_embedding_mailbox_items (
            item_id, repo_id, repo_root, config_root, init_session_id, representation_kind,
            item_kind, artefact_id, payload_json, dedupe_key, status, attempts,
            available_at_unix, submitted_at_unix, leased_at_unix, lease_expires_at_unix,
            lease_token, updated_at_unix, last_error
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6,
            ?7, ?8, ?9, ?10, 'pending', 0,
            ?11, ?12, NULL, NULL,
            NULL, ?13, NULL
         )",
        rusqlite::params![
            item_id,
            repo.repo_id,
            repo.repo_root.to_string_lossy().to_string(),
            repo.config_root.to_string_lossy().to_string(),
            item.init_session_id.as_deref(),
            item.representation_kind.as_str(),
            item.item_kind.as_str(),
            item.artefact_id.as_deref(),
            item.payload_json.as_ref().map(serde_json::Value::to_string),
            item.dedupe_key.as_deref(),
            sql_i64(now)?,
            sql_i64(now)?,
            sql_i64(now)?,
        ],
    )
    .context("inserting runtime embedding mailbox item")?;
    Ok(())
}

pub(super) fn insert_runtime_summary_mailbox_item(
    tx: &Transaction<'_>,
    repo: &SemanticBatchRepoContext,
    item: &SemanticSummaryMailboxItemInsert,
) -> Result<()> {
    let now = unix_timestamp_now();
    let item_id = format!("semantic-summary-mailbox-item-{}", uuid::Uuid::new_v4());
    tx.execute(
        "INSERT INTO runtime_store.semantic_summary_mailbox_items (
            item_id, repo_id, repo_root, config_root, init_session_id, item_kind,
            artefact_id, payload_json, dedupe_key, status, attempts, available_at_unix,
            submitted_at_unix, leased_at_unix, lease_expires_at_unix, lease_token,
            updated_at_unix, last_error
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6,
            ?7, ?8, ?9, 'pending', 0, ?10,
            ?11, NULL, NULL, NULL,
            ?12, NULL
         )",
        rusqlite::params![
            item_id,
            repo.repo_id,
            repo.repo_root.to_string_lossy().to_string(),
            repo.config_root.to_string_lossy().to_string(),
            item.init_session_id.as_deref(),
            item.item_kind.as_str(),
            item.artefact_id.as_deref(),
            item.payload_json.as_ref().map(serde_json::Value::to_string),
            item.dedupe_key.as_deref(),
            sql_i64(now)?,
            sql_i64(now)?,
            sql_i64(now)?,
        ],
    )
    .context("inserting runtime summary mailbox item")?;
    Ok(())
}

pub(super) fn delete_runtime_summary_mailbox_items(
    tx: &Transaction<'_>,
    lease_token: &str,
    item_ids: &[String],
) -> Result<()> {
    if item_ids.is_empty() {
        return Ok(());
    }
    tx.execute_batch(&format!(
        "DELETE FROM runtime_store.semantic_summary_mailbox_items
         WHERE lease_token = '{lease_token}'
           AND item_id IN ({item_ids})",
        lease_token = esc_pg(lease_token),
        item_ids = sql_string_list(item_ids),
    ))
    .context("deleting acknowledged semantic summary mailbox items")?;
    Ok(())
}

pub(super) fn delete_runtime_embedding_mailbox_items(
    tx: &Transaction<'_>,
    lease_token: &str,
    item_ids: &[String],
) -> Result<()> {
    if item_ids.is_empty() {
        return Ok(());
    }
    tx.execute_batch(&format!(
        "DELETE FROM runtime_store.semantic_embedding_mailbox_items
         WHERE lease_token = '{lease_token}'
           AND item_id IN ({item_ids})",
        lease_token = esc_pg(lease_token),
        item_ids = sql_string_list(item_ids),
    ))
    .context("deleting acknowledged semantic embedding mailbox items")?;
    Ok(())
}

pub(super) fn upsert_runtime_clone_rebuild_signal(
    tx: &Transaction<'_>,
    repo: &SemanticBatchRepoContext,
    signal: &CapabilityWorkplaneJobInsert,
) -> Result<()> {
    let existing = tx
        .query_row(
            "SELECT job_id, status
             FROM runtime_store.capability_workplane_jobs
             WHERE repo_id = ?1
               AND capability_id = ?2
               AND mailbox_name = ?3
               AND dedupe_key = ?4
               AND status IN ('pending', 'running')
             ORDER BY CASE status WHEN 'running' THEN 0 ELSE 1 END, submitted_at_unix ASC
             LIMIT 1",
            rusqlite::params![
                repo.repo_id,
                crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CAPABILITY_ID,
                signal.mailbox_name.as_str(),
                signal.dedupe_key.as_deref(),
            ],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    let now = unix_timestamp_now();
    if let Some((job_id, status)) = existing {
        if status == "pending" {
            tx.execute(
                "UPDATE runtime_store.capability_workplane_jobs
                 SET payload = ?1, updated_at_unix = ?2, available_at_unix = ?3, last_error = NULL
                 WHERE job_id = ?4",
                rusqlite::params![
                    signal.payload.to_string(),
                    sql_i64(now)?,
                    sql_i64(now)?,
                    job_id,
                ],
            )
            .context("refreshing pending clone rebuild signal")?;
        }
        return Ok(());
    }

    let job_id = format!("workplane-job-{}", uuid::Uuid::new_v4());
    tx.execute(
        "INSERT INTO runtime_store.capability_workplane_jobs (
            job_id, repo_id, repo_root, config_root, capability_id, mailbox_name,
            init_session_id, dedupe_key, payload, status, attempts, available_at_unix,
            submitted_at_unix, started_at_unix, updated_at_unix, completed_at_unix,
            lease_owner, lease_expires_at_unix, last_error
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6,
            NULL, ?7, ?8, 'pending', 0, ?9,
            ?10, NULL, ?11, NULL,
            NULL, NULL, NULL
         )",
        rusqlite::params![
            job_id,
            repo.repo_id,
            repo.repo_root.to_string_lossy().to_string(),
            repo.config_root.to_string_lossy().to_string(),
            crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CAPABILITY_ID,
            signal.mailbox_name.as_str(),
            signal.dedupe_key.as_deref(),
            signal.payload.to_string(),
            sql_i64(now)?,
            sql_i64(now)?,
            sql_i64(now)?,
        ],
    )
    .context("inserting clone rebuild signal")?;
    Ok(())
}

fn sql_string_list(values: &[String]) -> String {
    values
        .iter()
        .map(|value| format!("'{}'", esc_pg(value)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn sql_i64(value: u64) -> Result<i64> {
    i64::try_from(value).context("converting semantic writer integer to SQLite i64")
}

fn unix_timestamp_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

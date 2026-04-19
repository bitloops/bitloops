//! Enqueue and dedupe of semantic summary and embedding mailbox items.

use anyhow::{Context, Result};
use rusqlite::params;
use serde_json::Value;
use uuid::Uuid;

use super::dedupe::{load_deduped_embedding_mailbox_item, load_deduped_summary_mailbox_item};
use super::types::{
    CapabilityWorkplaneEnqueueResult, SemanticEmbeddingMailboxItemInsert,
    SemanticMailboxItemStatus, SemanticSummaryMailboxItemInsert,
};
use super::util::{sql_i64, unix_timestamp_now};
use crate::host::runtime_store::types::RepoSqliteRuntimeStore;

impl RepoSqliteRuntimeStore {
    pub fn enqueue_semantic_summary_mailbox_items(
        &self,
        items: Vec<SemanticSummaryMailboxItemInsert>,
    ) -> Result<CapabilityWorkplaneEnqueueResult> {
        if items.is_empty() {
            return Ok(CapabilityWorkplaneEnqueueResult::default());
        }

        let sqlite = self.connect_repo_sqlite()?;
        sqlite.with_connection(|conn| {
            conn.execute_batch("BEGIN IMMEDIATE TRANSACTION;")
                .context("starting semantic summary mailbox enqueue transaction")?;
            let result = (|| {
                let now = unix_timestamp_now();
                let mut inserted_jobs = 0u64;
                let mut updated_jobs = 0u64;
                for item in items {
                    if let Some(existing) = load_deduped_summary_mailbox_item(
                        conn,
                        &self.repo_id,
                        item.dedupe_key.as_deref(),
                    )? {
                        if existing.status == SemanticMailboxItemStatus::Pending {
                            conn.execute(
                                "UPDATE semantic_summary_mailbox_items
                                 SET init_session_id = COALESCE(init_session_id, ?1),
                                     payload_json = ?2,
                                     updated_at_unix = ?3,
                                     available_at_unix = ?4,
                                     last_error = NULL
                                 WHERE item_id = ?5",
                                params![
                                    item.init_session_id.as_deref(),
                                    item.payload_json.as_ref().map(Value::to_string),
                                    sql_i64(now)?,
                                    sql_i64(now)?,
                                    existing.item_id,
                                ],
                            )
                            .with_context(|| {
                                format!(
                                    "refreshing pending semantic summary mailbox item `{}`",
                                    existing.item_id
                                )
                            })?;
                        }
                        updated_jobs += 1;
                        continue;
                    }

                    let item_id = format!("semantic-summary-mailbox-item-{}", Uuid::new_v4());
                    conn.execute(
                        "INSERT INTO semantic_summary_mailbox_items (
                            item_id, repo_id, repo_root, config_root, init_session_id, item_kind,
                            artefact_id, payload_json, dedupe_key, status, attempts,
                            available_at_unix, submitted_at_unix, leased_at_unix,
                            lease_expires_at_unix, lease_token, updated_at_unix, last_error
                         ) VALUES (
                            ?1, ?2, ?3, ?4, ?5, ?6,
                            ?7, ?8, ?9, ?10, 0,
                            ?11, ?12, NULL,
                            NULL, NULL, ?13, NULL
                         )",
                        params![
                            &item_id,
                            &self.repo_id,
                            self.repo_root.to_string_lossy().to_string(),
                            self.config_root.to_string_lossy().to_string(),
                            item.init_session_id.as_deref(),
                            item.item_kind.as_str(),
                            item.artefact_id.as_deref(),
                            item.payload_json.as_ref().map(Value::to_string),
                            item.dedupe_key.as_deref(),
                            SemanticMailboxItemStatus::Pending.as_str(),
                            sql_i64(now)?,
                            sql_i64(now)?,
                            sql_i64(now)?,
                        ],
                    )
                    .with_context(|| {
                        format!("inserting semantic summary mailbox item `{item_id}`")
                    })?;
                    inserted_jobs += 1;
                }
                Ok(CapabilityWorkplaneEnqueueResult {
                    inserted_jobs,
                    updated_jobs,
                })
            })();

            match result {
                Ok(result) => {
                    conn.execute_batch("COMMIT;")
                        .context("committing semantic summary mailbox enqueue transaction")?;
                    Ok(result)
                }
                Err(err) => {
                    let _ = conn.execute_batch("ROLLBACK;");
                    Err(err)
                }
            }
        })
    }

    pub fn enqueue_semantic_embedding_mailbox_items(
        &self,
        items: Vec<SemanticEmbeddingMailboxItemInsert>,
    ) -> Result<CapabilityWorkplaneEnqueueResult> {
        if items.is_empty() {
            return Ok(CapabilityWorkplaneEnqueueResult::default());
        }

        let sqlite = self.connect_repo_sqlite()?;
        sqlite.with_connection(|conn| {
            conn.execute_batch("BEGIN IMMEDIATE TRANSACTION;")
                .context("starting semantic embedding mailbox enqueue transaction")?;
            let result = (|| {
                let now = unix_timestamp_now();
                let mut inserted_jobs = 0u64;
                let mut updated_jobs = 0u64;
                for item in items {
                    if let Some(existing) = load_deduped_embedding_mailbox_item(
                        conn,
                        &self.repo_id,
                        &item.representation_kind,
                        item.dedupe_key.as_deref(),
                    )? {
                        if existing.status == SemanticMailboxItemStatus::Pending {
                            conn.execute(
                                "UPDATE semantic_embedding_mailbox_items
                                 SET init_session_id = COALESCE(init_session_id, ?1),
                                     payload_json = ?2,
                                     updated_at_unix = ?3,
                                     available_at_unix = ?4,
                                     last_error = NULL
                                 WHERE item_id = ?5",
                                params![
                                    item.init_session_id.as_deref(),
                                    item.payload_json.as_ref().map(Value::to_string),
                                    sql_i64(now)?,
                                    sql_i64(now)?,
                                    existing.item_id,
                                ],
                            )
                            .with_context(|| {
                                format!(
                                    "refreshing pending semantic embedding mailbox item `{}`",
                                    existing.item_id
                                )
                            })?;
                        }
                        updated_jobs += 1;
                        continue;
                    }

                    let item_id = format!("semantic-embedding-mailbox-item-{}", Uuid::new_v4());
                    conn.execute(
                        "INSERT INTO semantic_embedding_mailbox_items (
                            item_id, repo_id, repo_root, config_root, init_session_id,
                            representation_kind, item_kind, artefact_id, payload_json,
                            dedupe_key, status, attempts, available_at_unix,
                            submitted_at_unix, leased_at_unix, lease_expires_at_unix,
                            lease_token, updated_at_unix, last_error
                         ) VALUES (
                            ?1, ?2, ?3, ?4, ?5,
                            ?6, ?7, ?8, ?9,
                            ?10, ?11, 0, ?12,
                            ?13, NULL, NULL,
                            NULL, ?14, NULL
                         )",
                        params![
                            &item_id,
                            &self.repo_id,
                            self.repo_root.to_string_lossy().to_string(),
                            self.config_root.to_string_lossy().to_string(),
                            item.init_session_id.as_deref(),
                            &item.representation_kind,
                            item.item_kind.as_str(),
                            item.artefact_id.as_deref(),
                            item.payload_json.as_ref().map(Value::to_string),
                            item.dedupe_key.as_deref(),
                            SemanticMailboxItemStatus::Pending.as_str(),
                            sql_i64(now)?,
                            sql_i64(now)?,
                            sql_i64(now)?,
                        ],
                    )
                    .with_context(|| {
                        format!("inserting semantic embedding mailbox item `{item_id}`")
                    })?;
                    inserted_jobs += 1;
                }
                Ok(CapabilityWorkplaneEnqueueResult {
                    inserted_jobs,
                    updated_jobs,
                })
            })();

            match result {
                Ok(result) => {
                    conn.execute_batch("COMMIT;")
                        .context("committing semantic embedding mailbox enqueue transaction")?;
                    Ok(result)
                }
                Err(err) => {
                    let _ = conn.execute_batch("ROLLBACK;");
                    Err(err)
                }
            }
        })
    }
}

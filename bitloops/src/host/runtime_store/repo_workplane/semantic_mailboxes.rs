//! Enqueue and dedupe of semantic summary and embedding mailbox items.

use anyhow::{Context, Result};
use rusqlite::params;
use serde_json::Value;
use std::collections::HashMap;
use uuid::Uuid;

use super::dedupe::{load_deduped_embedding_mailbox_items, load_deduped_summary_mailbox_item};
use super::types::{
    CapabilityWorkplaneEnqueueResult, SemanticEmbeddingMailboxItemInsert,
    SemanticEmbeddingMailboxItemRecord, SemanticMailboxItemStatus,
    SemanticSummaryMailboxItemInsert,
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
                let repo_root = self.repo_root.to_string_lossy().to_string();
                let config_root = self.config_root.to_string_lossy().to_string();
                let mut dedupe_keys_by_representation = HashMap::<String, Vec<String>>::new();
                for item in &items {
                    if let Some(dedupe_key) = &item.dedupe_key {
                        dedupe_keys_by_representation
                            .entry(item.representation_kind.clone())
                            .or_default()
                            .push(dedupe_key.clone());
                    }
                }
                let mut deduped_items =
                    HashMap::<(String, String), SemanticEmbeddingMailboxItemRecord>::new();
                for (representation_kind, dedupe_keys) in dedupe_keys_by_representation {
                    for (dedupe_key, item) in load_deduped_embedding_mailbox_items(
                        conn,
                        &self.repo_id,
                        &representation_kind,
                        &dedupe_keys,
                    )? {
                        deduped_items.insert((representation_kind.clone(), dedupe_key), item);
                    }
                }
                let mut update_stmt = conn
                    .prepare(
                        "UPDATE semantic_embedding_mailbox_items
                         SET init_session_id = COALESCE(init_session_id, ?1),
                             payload_json = ?2,
                             updated_at_unix = ?3,
                             available_at_unix = ?4,
                             last_error = NULL
                         WHERE item_id = ?5",
                    )
                    .context("preparing semantic embedding mailbox item update")?;
                let mut insert_stmt = conn
                    .prepare(
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
                    )
                    .context("preparing semantic embedding mailbox item insert")?;
                for item in items {
                    let dedupe_lookup_key = item
                        .dedupe_key
                        .as_ref()
                        .map(|dedupe_key| (item.representation_kind.clone(), dedupe_key.clone()));
                    if let Some(existing) = dedupe_lookup_key
                        .as_ref()
                        .and_then(|lookup_key| deduped_items.get(lookup_key).cloned())
                    {
                        if existing.status == SemanticMailboxItemStatus::Pending {
                            update_stmt
                                .execute(params![
                                    item.init_session_id.as_deref(),
                                    item.payload_json.as_ref().map(Value::to_string),
                                    sql_i64(now)?,
                                    sql_i64(now)?,
                                    existing.item_id,
                                ])
                                .with_context(|| {
                                    format!(
                                        "refreshing pending semantic embedding mailbox item `{}`",
                                        existing.item_id
                                    )
                                })?;
                            if let Some(lookup_key) = dedupe_lookup_key.clone() {
                                deduped_items.insert(
                                    lookup_key,
                                    SemanticEmbeddingMailboxItemRecord {
                                        payload_json: item.payload_json.clone(),
                                        updated_at_unix: now,
                                        available_at_unix: now,
                                        last_error: None,
                                        ..existing
                                    },
                                );
                            }
                        }
                        updated_jobs += 1;
                        continue;
                    }

                    let item_id = format!("semantic-embedding-mailbox-item-{}", Uuid::new_v4());
                    insert_stmt
                        .execute(params![
                            &item_id,
                            &self.repo_id,
                            &repo_root,
                            &config_root,
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
                        ])
                        .with_context(|| {
                            format!("inserting semantic embedding mailbox item `{item_id}`")
                        })?;
                    if let Some(lookup_key) = dedupe_lookup_key {
                        deduped_items.insert(
                            lookup_key,
                            SemanticEmbeddingMailboxItemRecord {
                                item_id,
                                repo_id: self.repo_id.clone(),
                                repo_root: self.repo_root.clone(),
                                config_root: self.config_root.clone(),
                                init_session_id: item.init_session_id,
                                representation_kind: item.representation_kind,
                                item_kind: item.item_kind,
                                artefact_id: item.artefact_id,
                                payload_json: item.payload_json,
                                dedupe_key: item.dedupe_key,
                                status: SemanticMailboxItemStatus::Pending,
                                attempts: 0,
                                available_at_unix: now,
                                submitted_at_unix: now,
                                leased_at_unix: None,
                                lease_expires_at_unix: None,
                                lease_token: None,
                                updated_at_unix: now,
                                last_error: None,
                            },
                        );
                    }
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

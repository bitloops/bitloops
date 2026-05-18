use anyhow::{Context, Result};
use rusqlite::params;
use uuid::Uuid;

use crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind;
use crate::capability_packs::semantic_clones::types::{
    SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX, SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
};
use crate::capability_packs::semantic_clones::workplane::{
    payload_artefact_id, payload_is_repo_backfill, payload_repo_backfill_artefact_ids,
};
use crate::daemon::types::unix_timestamp_now;
use crate::host::runtime_store::{
    DaemonSqliteRuntimeStore, SemanticMailboxItemKind, SemanticMailboxItemStatus,
    WorkplaneJobStatus,
};

use super::super::EnrichmentControlState;
use super::sql::{parse_u64, sql_i64, sql_string_list};

pub(crate) fn default_state() -> EnrichmentControlState {
    EnrichmentControlState {
        version: 1,
        last_action: Some("initialized".to_string()),
        ..EnrichmentControlState::default()
    }
}

pub(crate) fn migrate_legacy_semantic_workplane_rows(
    workplane_store: &DaemonSqliteRuntimeStore,
) -> Result<u64> {
    workplane_store.with_write_connection(|conn| {
        conn.execute_batch("BEGIN IMMEDIATE TRANSACTION;")
            .context("starting legacy semantic workplane migration transaction")?;
        let result = (|| {
            let now = unix_timestamp_now();
            let mut migrated = 0u64;
            let mut stmt = conn.prepare(
                "SELECT job_id, repo_id, repo_root, config_root, mailbox_name, init_session_id,
                        dedupe_key, payload, status, attempts, available_at_unix,
                        submitted_at_unix, started_at_unix, lease_expires_at_unix,
                        updated_at_unix, last_error
                 FROM capability_workplane_jobs
                 WHERE mailbox_name IN (?1, ?2, ?3)
                   AND status IN (?4, ?5)
                 ORDER BY submitted_at_unix ASC",
            )?;
            let rows = stmt.query_map(
                params![
                    SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
                    SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
                    SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
                    WorkplaneJobStatus::Pending.as_str(),
                    WorkplaneJobStatus::Running.as_str(),
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, String>(8)?,
                        row.get::<_, u32>(9)?,
                        row.get::<_, i64>(10)?,
                        row.get::<_, i64>(11)?,
                        row.get::<_, Option<i64>>(12)?,
                        row.get::<_, Option<i64>>(13)?,
                        row.get::<_, i64>(14)?,
                        row.get::<_, Option<String>>(15)?,
                    ))
                },
            )?;
            let mut migrated_job_ids = Vec::new();
            for row in rows {
                let (
                    job_id,
                    repo_id,
                    repo_root,
                    config_root,
                    mailbox_name,
                    init_session_id,
                    dedupe_key,
                    payload_raw,
                    status,
                    attempts,
                    available_at_unix,
                    submitted_at_unix,
                    started_at_unix,
                    lease_expires_at_unix,
                    updated_at_unix,
                    last_error,
                ) = row?;
                let payload =
                    serde_json::from_str::<serde_json::Value>(&payload_raw).unwrap_or_default();
                let item_kind = if payload_is_repo_backfill(&payload) {
                    SemanticMailboxItemKind::RepoBackfill
                } else {
                    SemanticMailboxItemKind::Artefact
                };
                let artefact_id = payload_artefact_id(&payload);
                let payload_json = payload_repo_backfill_artefact_ids(&payload)
                    .map(serde_json::to_value)
                    .transpose()
                    .context("serialising migrated semantic payload")?;
                let item_status = match WorkplaneJobStatus::parse(&status) {
                    WorkplaneJobStatus::Running => SemanticMailboxItemStatus::Leased,
                    _ => SemanticMailboxItemStatus::Pending,
                };
                let lease_token = if item_status == SemanticMailboxItemStatus::Leased {
                    Some(format!("migrated-semantic-lease-{}", Uuid::new_v4()))
                } else {
                    None
                };
                let leased_at_unix = started_at_unix.map(parse_u64);
                let lease_expires_at_unix = if item_status == SemanticMailboxItemStatus::Leased {
                    Some(lease_expires_at_unix.map(parse_u64).unwrap_or(now))
                } else {
                    None
                };

                if mailbox_name == SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX {
                    conn.execute(
                        "INSERT INTO semantic_summary_mailbox_items (
                            item_id, repo_id, repo_root, config_root, init_session_id, item_kind,
                            artefact_id, payload_json, dedupe_key, status, attempts,
                            available_at_unix, submitted_at_unix, leased_at_unix,
                            lease_expires_at_unix, lease_token, updated_at_unix, last_error
                         ) VALUES (
                            ?1, ?2, ?3, ?4, ?5, ?6,
                            ?7, ?8, ?9, ?10, ?11,
                            ?12, ?13, ?14,
                            ?15, ?16, ?17, ?18
                         )",
                        params![
                            format!("semantic-summary-mailbox-item-{}", Uuid::new_v4()),
                            repo_id,
                            repo_root,
                            config_root,
                            init_session_id,
                            item_kind.as_str(),
                            artefact_id.as_deref(),
                            payload_json.as_ref().map(serde_json::Value::to_string),
                            dedupe_key.as_deref(),
                            item_status.as_str(),
                            attempts,
                            available_at_unix,
                            submitted_at_unix,
                            leased_at_unix.map(sql_i64).transpose()?,
                            lease_expires_at_unix.map(sql_i64).transpose()?,
                            lease_token.as_deref(),
                            updated_at_unix,
                            last_error.as_deref(),
                        ],
                    )
                    .context("migrating legacy summary workplane row into semantic inbox")?;
                } else {
                    let representation_kind =
                        if mailbox_name == SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX {
                            EmbeddingRepresentationKind::Summary.to_string()
                        } else {
                            EmbeddingRepresentationKind::Code.to_string()
                        };
                    conn.execute(
                        "INSERT INTO semantic_embedding_mailbox_items (
                            item_id, repo_id, repo_root, config_root, init_session_id,
                            representation_kind, item_kind, artefact_id, payload_json, dedupe_key,
                            status, attempts, available_at_unix, submitted_at_unix, leased_at_unix,
                            lease_expires_at_unix, lease_token, updated_at_unix, last_error
                         ) VALUES (
                            ?1, ?2, ?3, ?4, ?5,
                            ?6, ?7, ?8, ?9, ?10,
                            ?11, ?12, ?13, ?14, ?15,
                            ?16, ?17, ?18, ?19
                         )",
                        params![
                            format!("semantic-embedding-mailbox-item-{}", Uuid::new_v4()),
                            repo_id,
                            repo_root,
                            config_root,
                            init_session_id,
                            representation_kind,
                            item_kind.as_str(),
                            artefact_id.as_deref(),
                            payload_json.as_ref().map(serde_json::Value::to_string),
                            dedupe_key.as_deref(),
                            item_status.as_str(),
                            attempts,
                            available_at_unix,
                            submitted_at_unix,
                            leased_at_unix.map(sql_i64).transpose()?,
                            lease_expires_at_unix.map(sql_i64).transpose()?,
                            lease_token.as_deref(),
                            updated_at_unix,
                            last_error.as_deref(),
                        ],
                    )
                    .context("migrating legacy embedding workplane row into semantic inbox")?;
                }
                migrated_job_ids.push(job_id);
                migrated += 1;
            }

            if !migrated_job_ids.is_empty() {
                conn.execute(
                    &format!(
                        "DELETE FROM capability_workplane_jobs WHERE job_id IN ({})",
                        sql_string_list(&migrated_job_ids)
                    ),
                    [],
                )
                .context("deleting migrated legacy semantic workplane rows")?;
            }
            Ok(migrated)
        })();

        match result {
            Ok(result) => {
                conn.execute_batch("COMMIT;")
                    .context("committing legacy semantic workplane migration transaction")?;
                Ok(result)
            }
            Err(err) => {
                let _ = conn.execute_batch("ROLLBACK;");
                Err(err)
            }
        }
    })
}

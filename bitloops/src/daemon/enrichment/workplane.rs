use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::capability_packs::semantic_clones::types::{
    SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX, SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_CODE_EMBEDDINGS_SLOT, SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_SUMMARY_EMBEDDINGS_SLOT, SEMANTIC_CLONES_SUMMARY_GENERATION_SLOT,
    SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
};
use crate::capability_packs::semantic_clones::workplane::{
    SemanticClonesMailboxPayload, payload_artefact_id, payload_is_repo_backfill,
    payload_repo_backfill_artefact_ids, payload_work_item_count, repo_backfill_dedupe_key,
};
use crate::capability_packs::semantic_clones::{
    SEMANTIC_CLONES_CAPABILITY_ID, embeddings::EmbeddingRepresentationKind,
};
use crate::host::capability_host::{
    CapabilityMailboxBacklogPolicy, CapabilityMailboxReadinessPolicy, CapabilityMailboxRegistration,
};
use crate::host::devql::{RepoIdentity, esc_pg};
use crate::host::inference::InferenceGateway;
use crate::host::runtime_store::{
    DaemonSqliteRuntimeStore, RepoSqliteRuntimeStore, SemanticEmbeddingMailboxItemInsert,
    SemanticEmbeddingMailboxItemRecord, SemanticMailboxItemKind, SemanticMailboxItemStatus,
    SemanticSummaryMailboxItemInsert, SemanticSummaryMailboxItemRecord, WorkplaneJobRecord,
    WorkplaneJobStatus,
};

use super::super::types::{
    BlockedMailboxStatus, EnrichmentQueueMode,
    EnrichmentQueueState as ProjectedEnrichmentQueueState, EnrichmentWorkerPoolKind,
    EnrichmentWorkerPoolStatus, FailedEmbeddingJobSummary, unix_timestamp_now,
};
use super::worker_count::{EnrichmentWorkerBudgets, EnrichmentWorkerPool};
use super::{
    EnrichmentControlState, EnrichmentJobTarget, JobExecutionOutcome,
    WORKPLANE_TERMINAL_RETENTION_SECS, WORKPLANE_TERMINAL_ROW_LIMIT, WorkplaneMailboxReadiness,
};

const WORKPLANE_JOB_CLAIM_CANDIDATE_LIMIT: usize = 32;
const WORKPLANE_TRANSIENT_EMBEDDING_RETRY_LIMIT: u32 = 3;
pub(super) const SEMANTIC_MAILBOX_BATCH_SIZE: usize = 50;
const SEMANTIC_MAILBOX_LEASE_SECS: u64 = 300;

pub(super) fn default_state() -> EnrichmentControlState {
    EnrichmentControlState {
        version: 1,
        last_action: Some("initialized".to_string()),
        ..EnrichmentControlState::default()
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct WorkplaneBucketCounts {
    jobs: u64,
    work_items: u64,
}

#[derive(Debug, Clone)]
pub(super) struct ClaimedSummaryMailboxBatch {
    pub repo_id: String,
    pub repo_root: PathBuf,
    pub config_root: PathBuf,
    pub lease_token: String,
    pub items: Vec<SemanticSummaryMailboxItemRecord>,
}

#[derive(Debug, Clone)]
pub(super) struct ClaimedEmbeddingMailboxBatch {
    pub repo_id: String,
    pub repo_root: PathBuf,
    pub config_root: PathBuf,
    pub representation_kind: EmbeddingRepresentationKind,
    pub lease_token: String,
    pub items: Vec<SemanticEmbeddingMailboxItemRecord>,
}

pub(super) fn enqueue_workplane_summary_jobs(
    target: &EnrichmentJobTarget,
    artefact_ids: Vec<String>,
) -> Result<()> {
    let store = RepoSqliteRuntimeStore::open_for_roots(&target.config_root, &target.repo_root)?;
    let result = if artefact_ids.is_empty() {
        store.enqueue_semantic_summary_mailbox_items(vec![
            SemanticSummaryMailboxItemInsert::new(
                target.init_session_id.clone(),
                SemanticMailboxItemKind::RepoBackfill,
                None,
                None,
                Some(repo_backfill_dedupe_key(
                    SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
                )),
            ),
        ])?
    } else {
        store.enqueue_semantic_summary_mailbox_items(
            artefact_ids
                .into_iter()
                .map(|artefact_id| {
                    SemanticSummaryMailboxItemInsert::new(
                        target.init_session_id.clone(),
                        SemanticMailboxItemKind::Artefact,
                        Some(artefact_id.clone()),
                        None,
                        Some(format!(
                            "{}:{artefact_id}",
                            SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX
                        )),
                    )
                })
                .collect(),
        )?
    };
    let _ = result;
    Ok(())
}

pub(super) fn enqueue_workplane_embedding_jobs(
    target: &EnrichmentJobTarget,
    artefact_ids: Vec<String>,
    representation_kind: EmbeddingRepresentationKind,
) -> Result<()> {
    let mailbox_name = match representation_kind {
        EmbeddingRepresentationKind::Code => SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
        EmbeddingRepresentationKind::Summary => SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
    };
    let store = RepoSqliteRuntimeStore::open_for_roots(&target.config_root, &target.repo_root)?;
    let result = if artefact_ids.is_empty() {
        store.enqueue_semantic_embedding_mailbox_items(vec![
            SemanticEmbeddingMailboxItemInsert::new(
                target.init_session_id.clone(),
                representation_kind.to_string(),
                SemanticMailboxItemKind::RepoBackfill,
                None,
                None,
                Some(repo_backfill_dedupe_key(mailbox_name)),
            ),
        ])?
    } else {
        store.enqueue_semantic_embedding_mailbox_items(
            artefact_ids
                .into_iter()
                .map(|artefact_id| {
                    SemanticEmbeddingMailboxItemInsert::new(
                        target.init_session_id.clone(),
                        representation_kind.to_string(),
                        SemanticMailboxItemKind::Artefact,
                        Some(artefact_id.clone()),
                        None,
                        Some(format!("{mailbox_name}:{artefact_id}")),
                    )
                })
                .collect(),
        )?
    };
    let _ = result;
    Ok(())
}

pub(super) fn enqueue_workplane_embedding_repo_backfill_job(
    target: &EnrichmentJobTarget,
    artefact_ids: Vec<String>,
    representation_kind: EmbeddingRepresentationKind,
) -> Result<()> {
    if artefact_ids.is_empty() {
        return Ok(());
    }
    let store = RepoSqliteRuntimeStore::open_for_roots(&target.config_root, &target.repo_root)?;
    let mailbox_name = match representation_kind {
        EmbeddingRepresentationKind::Code => SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
        EmbeddingRepresentationKind::Summary => SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
    };
    let dedupe_key = repo_backfill_dedupe_key(mailbox_name);
    let _ = store.enqueue_semantic_embedding_mailbox_items(vec![
        SemanticEmbeddingMailboxItemInsert::new(
            target.init_session_id.clone(),
            representation_kind.to_string(),
            SemanticMailboxItemKind::RepoBackfill,
            None,
            Some(
                serde_json::to_value(artefact_ids)
                    .expect("embedding repo backfill payload should serialize"),
            ),
            Some(dedupe_key),
        ),
    ])?;
    Ok(())
}

pub(super) fn enqueue_workplane_clone_rebuild(target: &EnrichmentJobTarget) -> Result<()> {
    let store = RepoSqliteRuntimeStore::open_for_roots(&target.config_root, &target.repo_root)?;
    let _ = store.enqueue_capability_workplane_jobs(
        SEMANTIC_CLONES_CAPABILITY_ID,
        vec![
            crate::host::runtime_store::CapabilityWorkplaneJobInsert::new(
                SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
                target.init_session_id.clone(),
                Some(SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX.to_string()),
                serde_json::to_value(SemanticClonesMailboxPayload::RepoBackfill {
                    work_item_count: Some(1),
                    artefact_ids: None,
                })
                .expect("clone rebuild payload should serialize"),
            ),
        ],
    )?;
    Ok(())
}

pub(super) fn compact_and_prune_workplane_jobs(
    workplane_store: &DaemonSqliteRuntimeStore,
) -> Result<()> {
    workplane_store.with_connection(|conn| {
        prune_terminal_workplane_jobs(conn)?;
        Ok(())
    })
}

fn prune_terminal_workplane_jobs(conn: &rusqlite::Connection) -> Result<()> {
    let cutoff = unix_timestamp_now().saturating_sub(WORKPLANE_TERMINAL_RETENTION_SECS);
    let mut stmt = conn.prepare(
        "SELECT repo_id, capability_id, mailbox_name, COUNT(*)
         FROM capability_workplane_jobs
         WHERE status IN (?1, ?2)
         GROUP BY repo_id, capability_id, mailbox_name
         HAVING COUNT(*) > ?3",
    )?;
    let rows = stmt.query_map(
        params![
            WorkplaneJobStatus::Completed.as_str(),
            WorkplaneJobStatus::Failed.as_str(),
            sql_i64(WORKPLANE_TERMINAL_ROW_LIMIT)?,
        ],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        },
    )?;
    for row in rows {
        let (repo_id, capability_id, mailbox_name) = row?;
        conn.execute(
            "DELETE FROM capability_workplane_jobs
             WHERE repo_id = ?1
               AND capability_id = ?2
               AND mailbox_name = ?3
               AND status IN (?4, ?5)
               AND COALESCE(completed_at_unix, updated_at_unix) <= ?6",
            params![
                repo_id,
                capability_id,
                mailbox_name,
                WorkplaneJobStatus::Completed.as_str(),
                WorkplaneJobStatus::Failed.as_str(),
                sql_i64(cutoff)?,
            ],
        )?;
    }
    Ok(())
}

pub(super) fn migrate_legacy_semantic_workplane_rows(
    workplane_store: &DaemonSqliteRuntimeStore,
) -> Result<u64> {
    workplane_store.with_connection(|conn| {
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

pub(super) fn recover_expired_semantic_inbox_leases(
    workplane_store: &DaemonSqliteRuntimeStore,
) -> Result<u64> {
    let now = unix_timestamp_now();
    workplane_store.with_connection(|conn| {
        let summary = conn.execute(
            "UPDATE semantic_summary_mailbox_items
             SET status = ?1,
                 leased_at_unix = NULL,
                 lease_expires_at_unix = NULL,
                 lease_token = NULL,
                 updated_at_unix = ?2
             WHERE status = ?3
               AND lease_expires_at_unix IS NOT NULL
               AND lease_expires_at_unix <= ?4",
            params![
                SemanticMailboxItemStatus::Pending.as_str(),
                sql_i64(now)?,
                SemanticMailboxItemStatus::Leased.as_str(),
                sql_i64(now)?,
            ],
        )?;
        let embedding = conn.execute(
            "UPDATE semantic_embedding_mailbox_items
             SET status = ?1,
                 leased_at_unix = NULL,
                 lease_expires_at_unix = NULL,
                 lease_token = NULL,
                 updated_at_unix = ?2
             WHERE status = ?3
               AND lease_expires_at_unix IS NOT NULL
               AND lease_expires_at_unix <= ?4",
            params![
                SemanticMailboxItemStatus::Pending.as_str(),
                sql_i64(now)?,
                SemanticMailboxItemStatus::Leased.as_str(),
                sql_i64(now)?,
            ],
        )?;
        Ok(u64::try_from(summary + embedding).unwrap_or_default())
    })
}

pub(super) fn requeue_leased_semantic_inbox_items(
    workplane_store: &DaemonSqliteRuntimeStore,
) -> Result<u64> {
    let now = unix_timestamp_now();
    workplane_store.with_connection(|conn| {
        let summary = conn.execute(
            "UPDATE semantic_summary_mailbox_items
             SET status = ?1,
                 leased_at_unix = NULL,
                 lease_expires_at_unix = NULL,
                 lease_token = NULL,
                 updated_at_unix = ?2,
                 last_error = NULL
             WHERE status = ?3",
            params![
                SemanticMailboxItemStatus::Pending.as_str(),
                sql_i64(now)?,
                SemanticMailboxItemStatus::Leased.as_str(),
            ],
        )?;
        let embedding = conn.execute(
            "UPDATE semantic_embedding_mailbox_items
             SET status = ?1,
                 leased_at_unix = NULL,
                 lease_expires_at_unix = NULL,
                 lease_token = NULL,
                 updated_at_unix = ?2,
                 last_error = NULL
             WHERE status = ?3",
            params![
                SemanticMailboxItemStatus::Pending.as_str(),
                sql_i64(now)?,
                SemanticMailboxItemStatus::Leased.as_str(),
            ],
        )?;
        Ok(u64::try_from(summary + embedding).unwrap_or_default())
    })
}

pub(super) fn prune_failed_semantic_inbox_items(
    workplane_store: &DaemonSqliteRuntimeStore,
) -> Result<()> {
    let cutoff = unix_timestamp_now().saturating_sub(WORKPLANE_TERMINAL_RETENTION_SECS);
    workplane_store.with_connection(|conn| {
        conn.execute(
            "DELETE FROM semantic_summary_mailbox_items
             WHERE status = ?1
               AND updated_at_unix <= ?2",
            params![SemanticMailboxItemStatus::Failed.as_str(), sql_i64(cutoff)?,],
        )?;
        conn.execute(
            "DELETE FROM semantic_embedding_mailbox_items
             WHERE status = ?1
               AND updated_at_unix <= ?2",
            params![SemanticMailboxItemStatus::Failed.as_str(), sql_i64(cutoff)?,],
        )?;
        Ok(())
    })
}

pub(super) fn retry_failed_semantic_inbox_items(
    workplane_store: &DaemonSqliteRuntimeStore,
) -> Result<u64> {
    let now = unix_timestamp_now();
    workplane_store.with_connection(|conn| {
        let summary = conn.execute(
            "UPDATE semantic_summary_mailbox_items
             SET status = ?1,
                 leased_at_unix = NULL,
                 lease_expires_at_unix = NULL,
                 lease_token = NULL,
                 updated_at_unix = ?2,
                 last_error = NULL
             WHERE status = ?3",
            params![
                SemanticMailboxItemStatus::Pending.as_str(),
                sql_i64(now)?,
                SemanticMailboxItemStatus::Failed.as_str(),
            ],
        )?;
        let embedding = conn.execute(
            "UPDATE semantic_embedding_mailbox_items
             SET status = ?1,
                 leased_at_unix = NULL,
                 lease_expires_at_unix = NULL,
                 lease_token = NULL,
                 updated_at_unix = ?2,
                 last_error = NULL
             WHERE status = ?3",
            params![
                SemanticMailboxItemStatus::Pending.as_str(),
                sql_i64(now)?,
                SemanticMailboxItemStatus::Failed.as_str(),
            ],
        )?;
        Ok(u64::try_from(summary + embedding).unwrap_or_default())
    })
}

pub(super) fn claim_summary_mailbox_batch(
    workplane_store: &DaemonSqliteRuntimeStore,
    runtime_store: &DaemonSqliteRuntimeStore,
    control_state: &EnrichmentControlState,
) -> Result<Option<ClaimedSummaryMailboxBatch>> {
    if control_state.paused_semantic {
        return Ok(None);
    }
    workplane_store.with_connection(|conn| {
        conn.execute_batch("BEGIN IMMEDIATE TRANSACTION;")
            .context("starting semantic summary mailbox claim transaction")?;
        let result = (|| {
            let now = unix_timestamp_now();
            let candidates = load_summary_mailbox_repo_candidates(conn, now)?;
            let mut readiness_cache = BTreeMap::new();
            for (repo_id, repo_root, config_root) in candidates {
                let job = mailbox_readiness_job(
                    &repo_id,
                    &repo_root,
                    &config_root,
                    SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
                );
                if mailbox_claim_readiness(runtime_store, &mut readiness_cache, &job)?.blocked {
                    continue;
                }
                if let Some(batch) = lease_summary_mailbox_batch_for_repo(conn, &repo_id, now)? {
                    return Ok(Some(batch));
                }
            }
            Ok(None)
        })();
        match result {
            Ok(batch) => {
                conn.execute_batch("COMMIT;")
                    .context("committing semantic summary mailbox claim transaction")?;
                Ok(batch)
            }
            Err(err) => {
                let _ = conn.execute_batch("ROLLBACK;");
                Err(err)
            }
        }
    })
}

pub(super) fn claim_embedding_mailbox_batch(
    workplane_store: &DaemonSqliteRuntimeStore,
    runtime_store: &DaemonSqliteRuntimeStore,
    control_state: &EnrichmentControlState,
) -> Result<Option<ClaimedEmbeddingMailboxBatch>> {
    if control_state.paused_embeddings {
        return Ok(None);
    }
    workplane_store.with_connection(|conn| {
        conn.execute_batch("BEGIN IMMEDIATE TRANSACTION;")
            .context("starting semantic embedding mailbox claim transaction")?;
        let result = (|| {
            let now = unix_timestamp_now();
            let candidates = load_embedding_mailbox_repo_candidates(conn, now)?;
            let mut readiness_cache = BTreeMap::new();
            for (repo_id, repo_root, config_root, representation_kind) in candidates {
                let mailbox_name = match representation_kind {
                    EmbeddingRepresentationKind::Summary => {
                        SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX
                    }
                    EmbeddingRepresentationKind::Code => SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
                };
                let job = mailbox_readiness_job(&repo_id, &repo_root, &config_root, mailbox_name);
                if mailbox_claim_readiness(runtime_store, &mut readiness_cache, &job)?.blocked {
                    continue;
                }
                if let Some(batch) = lease_embedding_mailbox_batch_for_repo(
                    conn,
                    &repo_id,
                    representation_kind,
                    now,
                )? {
                    return Ok(Some(batch));
                }
            }
            Ok(None)
        })();
        match result {
            Ok(batch) => {
                conn.execute_batch("COMMIT;")
                    .context("committing semantic embedding mailbox claim transaction")?;
                Ok(batch)
            }
            Err(err) => {
                let _ = conn.execute_batch("ROLLBACK;");
                Err(err)
            }
        }
    })
}

pub(super) fn fail_summary_mailbox_batch(
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

pub(super) fn requeue_summary_mailbox_batch(
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

pub(super) fn persist_embedding_mailbox_batch_failure(
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

pub(super) fn requeue_embedding_mailbox_batch(
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

fn load_summary_mailbox_repo_candidates(
    conn: &rusqlite::Connection,
    now: u64,
) -> Result<Vec<(String, PathBuf, PathBuf)>> {
    let limit = i64::try_from(WORKPLANE_JOB_CLAIM_CANDIDATE_LIMIT)
        .context("converting summary mailbox claim candidate limit")?;
    let mut stmt = conn.prepare(
        "SELECT repo_id, repo_root, config_root, MIN(submitted_at_unix) AS oldest_submitted
         FROM semantic_summary_mailbox_items
         WHERE status = ?1
           AND available_at_unix <= ?2
         GROUP BY repo_id, repo_root, config_root
         ORDER BY oldest_submitted ASC
         LIMIT ?3",
    )?;
    let rows = stmt.query_map(
        params![
            SemanticMailboxItemStatus::Pending.as_str(),
            sql_i64(now)?,
            limit,
        ],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                PathBuf::from(row.get::<_, String>(1)?),
                PathBuf::from(row.get::<_, String>(2)?),
            ))
        },
    )?;
    let mut values = Vec::new();
    for row in rows {
        values.push(row?);
    }
    Ok(values)
}

fn load_embedding_mailbox_repo_candidates(
    conn: &rusqlite::Connection,
    now: u64,
) -> Result<Vec<(String, PathBuf, PathBuf, EmbeddingRepresentationKind)>> {
    let limit = i64::try_from(WORKPLANE_JOB_CLAIM_CANDIDATE_LIMIT)
        .context("converting embedding mailbox claim candidate limit")?;
    let mut stmt = conn.prepare(
        "SELECT repo_id, repo_root, config_root, representation_kind, MIN(submitted_at_unix) AS oldest_submitted
         FROM semantic_embedding_mailbox_items
         WHERE status = ?1
           AND available_at_unix <= ?2
         GROUP BY repo_id, repo_root, config_root, representation_kind
         ORDER BY oldest_submitted ASC
         LIMIT ?3",
    )?;
    let rows = stmt.query_map(
        params![
            SemanticMailboxItemStatus::Pending.as_str(),
            sql_i64(now)?,
            limit,
        ],
        |row| {
            let representation_kind = match row.get::<_, String>(3)?.as_str() {
                "summary" => EmbeddingRepresentationKind::Summary,
                _ => EmbeddingRepresentationKind::Code,
            };
            Ok((
                row.get::<_, String>(0)?,
                PathBuf::from(row.get::<_, String>(1)?),
                PathBuf::from(row.get::<_, String>(2)?),
                representation_kind,
            ))
        },
    )?;
    let mut values = Vec::new();
    for row in rows {
        values.push(row?);
    }
    Ok(values)
}

fn mailbox_readiness_job(
    repo_id: &str,
    repo_root: &Path,
    config_root: &Path,
    mailbox_name: &str,
) -> WorkplaneJobRecord {
    WorkplaneJobRecord {
        job_id: format!("mailbox-readiness-{mailbox_name}"),
        repo_id: repo_id.to_string(),
        repo_root: repo_root.to_path_buf(),
        config_root: config_root.to_path_buf(),
        capability_id: SEMANTIC_CLONES_CAPABILITY_ID.to_string(),
        mailbox_name: mailbox_name.to_string(),
        init_session_id: None,
        dedupe_key: None,
        payload: serde_json::Value::Null,
        status: WorkplaneJobStatus::Pending,
        attempts: 0,
        available_at_unix: 0,
        submitted_at_unix: 0,
        started_at_unix: None,
        updated_at_unix: 0,
        completed_at_unix: None,
        lease_owner: None,
        lease_expires_at_unix: None,
        last_error: None,
    }
}

fn lease_summary_mailbox_batch_for_repo(
    conn: &rusqlite::Connection,
    repo_id: &str,
    now: u64,
) -> Result<Option<ClaimedSummaryMailboxBatch>> {
    let Some((repo_root, config_root, oldest_kind)) = conn
        .query_row(
            "SELECT repo_root, config_root, item_kind
             FROM semantic_summary_mailbox_items
             WHERE repo_id = ?1
               AND status = ?2
               AND available_at_unix <= ?3
             ORDER BY submitted_at_unix ASC, item_id ASC
             LIMIT 1",
            params![
                repo_id,
                SemanticMailboxItemStatus::Pending.as_str(),
                sql_i64(now)?,
            ],
            |row| {
                Ok((
                    PathBuf::from(row.get::<_, String>(0)?),
                    PathBuf::from(row.get::<_, String>(1)?),
                    SemanticMailboxItemKind::parse(&row.get::<_, String>(2)?),
                ))
            },
        )
        .optional()?
    else {
        return Ok(None);
    };

    let lease_token = format!("semantic-summary-lease-{}", Uuid::new_v4());
    let selected_ids = load_selected_summary_item_ids(conn, repo_id, oldest_kind, now)?;
    if selected_ids.is_empty() {
        return Ok(None);
    }

    conn.execute(
        &format!(
            "UPDATE semantic_summary_mailbox_items
             SET status = '{leased}',
                 attempts = attempts + 1,
                 leased_at_unix = {leased_at},
                 lease_expires_at_unix = {lease_expires_at},
                 lease_token = '{lease_token}',
                 updated_at_unix = {updated_at}
             WHERE status = '{pending}'
               AND item_id IN ({item_ids})",
            leased = SemanticMailboxItemStatus::Leased.as_str(),
            leased_at = sql_i64(now)?,
            lease_expires_at = sql_i64(now.saturating_add(SEMANTIC_MAILBOX_LEASE_SECS))?,
            lease_token = esc_pg(&lease_token),
            updated_at = sql_i64(now)?,
            pending = SemanticMailboxItemStatus::Pending.as_str(),
            item_ids = sql_string_list(&selected_ids),
        ),
        [],
    )
    .context("leasing semantic summary mailbox items")?;
    let items = load_summary_mailbox_items_by_ids(conn, &selected_ids)?;
    Ok(Some(ClaimedSummaryMailboxBatch {
        repo_id: repo_id.to_string(),
        repo_root,
        config_root,
        lease_token,
        items,
    }))
}

fn lease_embedding_mailbox_batch_for_repo(
    conn: &rusqlite::Connection,
    repo_id: &str,
    representation_kind: EmbeddingRepresentationKind,
    now: u64,
) -> Result<Option<ClaimedEmbeddingMailboxBatch>> {
    let Some((repo_root, config_root, oldest_kind)) = conn
        .query_row(
            "SELECT repo_root, config_root, item_kind
             FROM semantic_embedding_mailbox_items
             WHERE repo_id = ?1
               AND representation_kind = ?2
               AND status = ?3
               AND available_at_unix <= ?4
             ORDER BY submitted_at_unix ASC, item_id ASC
             LIMIT 1",
            params![
                repo_id,
                representation_kind.to_string(),
                SemanticMailboxItemStatus::Pending.as_str(),
                sql_i64(now)?,
            ],
            |row| {
                Ok((
                    PathBuf::from(row.get::<_, String>(0)?),
                    PathBuf::from(row.get::<_, String>(1)?),
                    SemanticMailboxItemKind::parse(&row.get::<_, String>(2)?),
                ))
            },
        )
        .optional()?
    else {
        return Ok(None);
    };

    let lease_token = format!("semantic-embedding-lease-{}", Uuid::new_v4());
    let selected_ids =
        load_selected_embedding_item_ids(conn, repo_id, representation_kind, oldest_kind, now)?;
    if selected_ids.is_empty() {
        return Ok(None);
    }

    conn.execute(
        &format!(
            "UPDATE semantic_embedding_mailbox_items
             SET status = '{leased}',
                 attempts = attempts + 1,
                 leased_at_unix = {leased_at},
                 lease_expires_at_unix = {lease_expires_at},
                 lease_token = '{lease_token}',
                 updated_at_unix = {updated_at}
             WHERE status = '{pending}'
               AND item_id IN ({item_ids})",
            leased = SemanticMailboxItemStatus::Leased.as_str(),
            leased_at = sql_i64(now)?,
            lease_expires_at = sql_i64(now.saturating_add(SEMANTIC_MAILBOX_LEASE_SECS))?,
            lease_token = esc_pg(&lease_token),
            updated_at = sql_i64(now)?,
            pending = SemanticMailboxItemStatus::Pending.as_str(),
            item_ids = sql_string_list(&selected_ids),
        ),
        [],
    )
    .context("leasing semantic embedding mailbox items")?;
    let items = load_embedding_mailbox_items_by_ids(conn, &selected_ids)?;
    Ok(Some(ClaimedEmbeddingMailboxBatch {
        repo_id: repo_id.to_string(),
        repo_root,
        config_root,
        representation_kind,
        lease_token,
        items,
    }))
}

fn load_selected_summary_item_ids(
    conn: &rusqlite::Connection,
    repo_id: &str,
    item_kind: SemanticMailboxItemKind,
    now: u64,
) -> Result<Vec<String>> {
    let limit = if item_kind == SemanticMailboxItemKind::RepoBackfill {
        1
    } else {
        SEMANTIC_MAILBOX_BATCH_SIZE
    };
    let mut stmt = conn.prepare(
        "SELECT item_id
         FROM semantic_summary_mailbox_items
         WHERE repo_id = ?1
           AND item_kind = ?2
           AND status = ?3
           AND available_at_unix <= ?4
         ORDER BY submitted_at_unix ASC, item_id ASC
         LIMIT ?5",
    )?;
    let rows = stmt.query_map(
        params![
            repo_id,
            item_kind.as_str(),
            SemanticMailboxItemStatus::Pending.as_str(),
            sql_i64(now)?,
            i64::try_from(limit)?,
        ],
        |row| row.get::<_, String>(0),
    )?;
    let mut values = Vec::new();
    for row in rows {
        values.push(row?);
    }
    Ok(values)
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

fn load_selected_embedding_item_ids(
    conn: &rusqlite::Connection,
    repo_id: &str,
    representation_kind: EmbeddingRepresentationKind,
    item_kind: SemanticMailboxItemKind,
    now: u64,
) -> Result<Vec<String>> {
    let limit = if item_kind == SemanticMailboxItemKind::RepoBackfill {
        1
    } else {
        SEMANTIC_MAILBOX_BATCH_SIZE
    };
    let mut stmt = conn.prepare(
        "SELECT item_id
         FROM semantic_embedding_mailbox_items
         WHERE repo_id = ?1
           AND representation_kind = ?2
           AND item_kind = ?3
           AND status = ?4
           AND available_at_unix <= ?5
         ORDER BY submitted_at_unix ASC, item_id ASC
         LIMIT ?6",
    )?;
    let rows = stmt.query_map(
        params![
            repo_id,
            representation_kind.to_string(),
            item_kind.as_str(),
            SemanticMailboxItemStatus::Pending.as_str(),
            sql_i64(now)?,
            i64::try_from(limit)?,
        ],
        |row| row.get::<_, String>(0),
    )?;
    let mut values = Vec::new();
    for row in rows {
        values.push(row?);
    }
    Ok(values)
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

fn load_summary_mailbox_items_by_ids(
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

fn load_summary_mailbox_items_by_status(
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

fn load_embedding_mailbox_items_by_ids(
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

fn load_embedding_mailbox_items_by_status(
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

pub(super) fn claim_next_workplane_job(
    workplane_store: &DaemonSqliteRuntimeStore,
    runtime_store: &DaemonSqliteRuntimeStore,
    control_state: &EnrichmentControlState,
    pool: EnrichmentWorkerPool,
) -> Result<Option<WorkplaneJobRecord>> {
    workplane_store.with_connection(|conn| {
        conn.execute_batch("BEGIN IMMEDIATE TRANSACTION;")
            .context("starting capability workplane job claim transaction")?;
        let result = (|| {
            let now = unix_timestamp_now();
            let jobs = load_workplane_claim_candidates(conn, pool, now)?;
            let mut readiness_cache = BTreeMap::new();
            for mut job in jobs {
                if job_is_paused_for_mailbox(control_state, &job.mailbox_name) {
                    continue;
                }
                if mailbox_claim_readiness(runtime_store, &mut readiness_cache, &job)?.blocked {
                    continue;
                }
                let updated = conn
                    .execute(
                        "UPDATE capability_workplane_jobs
                     SET status = ?1,
                         attempts = ?2,
                         started_at_unix = COALESCE(started_at_unix, ?3),
                         updated_at_unix = ?4
                     WHERE job_id = ?5
                       AND status = ?6",
                        params![
                            WorkplaneJobStatus::Running.as_str(),
                            job.attempts + 1,
                            sql_i64(now)?,
                            sql_i64(now)?,
                            &job.job_id,
                            WorkplaneJobStatus::Pending.as_str(),
                        ],
                    )
                    .with_context(|| {
                        format!("claiming capability workplane job `{}`", job.job_id)
                    })?;
                if updated == 0 {
                    continue;
                }
                job.status = WorkplaneJobStatus::Running;
                job.attempts += 1;
                job.started_at_unix = Some(job.started_at_unix.unwrap_or(now));
                job.updated_at_unix = now;
                return Ok(Some(job));
            }
            Ok(None)
        })();

        match result {
            Ok(job) => {
                conn.execute_batch("COMMIT;")
                    .context("committing capability workplane job claim transaction")?;
                Ok(job)
            }
            Err(err) => {
                let _ = conn.execute_batch("ROLLBACK;");
                Err(err)
            }
        }
    })
}

fn load_workplane_claim_candidates(
    conn: &rusqlite::Connection,
    pool: EnrichmentWorkerPool,
    now: u64,
) -> Result<Vec<WorkplaneJobRecord>> {
    let limit = i64::try_from(WORKPLANE_JOB_CLAIM_CANDIDATE_LIMIT)
        .context("converting workplane claim candidate limit")?;
    let now = sql_i64(now)?;
    let mut values = Vec::new();
    match pool {
        EnrichmentWorkerPool::SummaryRefresh => {
            let mut stmt = conn.prepare(
                "SELECT job_id, repo_id, repo_root, config_root, capability_id, mailbox_name,
                        init_session_id, dedupe_key, payload, status, attempts, available_at_unix, submitted_at_unix,
                        started_at_unix, updated_at_unix, completed_at_unix, lease_owner,
                        lease_expires_at_unix, last_error
                 FROM capability_workplane_jobs
                 WHERE status = ?1
                   AND mailbox_name = ?2
                   AND available_at_unix <= ?3
                 ORDER BY CASE mailbox_name
                              WHEN 'semantic_clones.embedding.code' THEN 0
                              WHEN 'semantic_clones.embedding.summary' THEN 0
                              WHEN 'semantic_clones.summary_refresh' THEN 1
                              WHEN 'semantic_clones.clone_rebuild' THEN 2
                          ELSE 3
                          END ASC,
                          available_at_unix ASC,
                          submitted_at_unix ASC
                 LIMIT ?4",
            )?;
            let rows = stmt.query_map(
                params![
                    WorkplaneJobStatus::Pending.as_str(),
                    SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
                    now,
                    limit,
                ],
                map_workplane_job_row,
            )?;
            for row in rows {
                values.push(row?);
            }
        }
        EnrichmentWorkerPool::Embeddings => {
            let mut stmt = conn.prepare(
                "SELECT job_id, repo_id, repo_root, config_root, capability_id, mailbox_name,
                        init_session_id, dedupe_key, payload, status, attempts, available_at_unix, submitted_at_unix,
                        started_at_unix, updated_at_unix, completed_at_unix, lease_owner,
                        lease_expires_at_unix, last_error
                 FROM capability_workplane_jobs
                 WHERE status = ?1
                   AND mailbox_name IN (?2, ?3)
                   AND available_at_unix <= ?4
                 ORDER BY CASE mailbox_name
                              WHEN 'semantic_clones.embedding.code' THEN 0
                              WHEN 'semantic_clones.embedding.summary' THEN 0
                              WHEN 'semantic_clones.summary_refresh' THEN 1
                              WHEN 'semantic_clones.clone_rebuild' THEN 2
                          ELSE 3
                          END ASC,
                          available_at_unix ASC,
                          submitted_at_unix ASC
                 LIMIT ?5",
            )?;
            let rows = stmt.query_map(
                params![
                    WorkplaneJobStatus::Pending.as_str(),
                    SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
                    SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
                    now,
                    limit,
                ],
                map_workplane_job_row,
            )?;
            for row in rows {
                values.push(row?);
            }
        }
        EnrichmentWorkerPool::CloneRebuild => {
            let mut stmt = conn.prepare(
                "SELECT job_id, repo_id, repo_root, config_root, capability_id, mailbox_name,
                        init_session_id, dedupe_key, payload, status, attempts, available_at_unix, submitted_at_unix,
                        started_at_unix, updated_at_unix, completed_at_unix, lease_owner,
                        lease_expires_at_unix, last_error
                 FROM capability_workplane_jobs
                 WHERE status = ?1
                   AND mailbox_name = ?2
                   AND available_at_unix <= ?3
                 ORDER BY CASE mailbox_name
                              WHEN 'semantic_clones.embedding.code' THEN 0
                              WHEN 'semantic_clones.embedding.summary' THEN 0
                              WHEN 'semantic_clones.summary_refresh' THEN 1
                              WHEN 'semantic_clones.clone_rebuild' THEN 2
                          ELSE 3
                          END ASC,
                          available_at_unix ASC,
                          submitted_at_unix ASC
                 LIMIT ?4",
            )?;
            let rows = stmt.query_map(
                params![
                    WorkplaneJobStatus::Pending.as_str(),
                    SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
                    now,
                    limit,
                ],
                map_workplane_job_row,
            )?;
            for row in rows {
                values.push(row?);
            }
        }
    }
    Ok(values)
}

pub(super) fn persist_workplane_job_completion(
    workplane_store: &DaemonSqliteRuntimeStore,
    job: &WorkplaneJobRecord,
    outcome: &JobExecutionOutcome,
) -> Result<WorkplaneJobCompletionDisposition> {
    let now = unix_timestamp_now();
    let disposition = classify_workplane_job_completion(job, outcome, now);
    workplane_store.with_connection(|conn| {
        match disposition {
            WorkplaneJobCompletionDisposition::Completed
            | WorkplaneJobCompletionDisposition::Failed => {
                conn.execute(
                    "UPDATE capability_workplane_jobs
                     SET status = ?1,
                         updated_at_unix = ?2,
                         completed_at_unix = ?3,
                         last_error = ?4,
                         lease_owner = NULL,
                         lease_expires_at_unix = NULL
                     WHERE job_id = ?5",
                    params![
                        if matches!(disposition, WorkplaneJobCompletionDisposition::Failed) {
                            WorkplaneJobStatus::Failed.as_str()
                        } else {
                            WorkplaneJobStatus::Completed.as_str()
                        },
                        sql_i64(now)?,
                        sql_i64(now)?,
                        outcome.error.as_deref(),
                        &job.job_id,
                    ],
                )
                .with_context(|| {
                    format!(
                        "persisting completion for capability workplane job `{}`",
                        job.job_id
                    )
                })?;
            }
            WorkplaneJobCompletionDisposition::RetryScheduled {
                available_at_unix, ..
            } => {
                conn.execute(
                    "UPDATE capability_workplane_jobs
                     SET status = ?1,
                         available_at_unix = ?2,
                         started_at_unix = NULL,
                         updated_at_unix = ?3,
                         completed_at_unix = NULL,
                         last_error = ?4,
                         lease_owner = NULL,
                         lease_expires_at_unix = NULL
                     WHERE job_id = ?5",
                    params![
                        WorkplaneJobStatus::Pending.as_str(),
                        sql_i64(available_at_unix)?,
                        sql_i64(now)?,
                        outcome.error.as_deref(),
                        &job.job_id,
                    ],
                )
                .with_context(|| {
                    format!(
                        "scheduling retry for capability workplane job `{}`",
                        job.job_id
                    )
                })?;
            }
        }
        Ok(())
    })?;
    log::info!(
        "{}",
        format_workplane_job_completion_log_with_disposition(job, now, outcome, disposition)
    );
    if let Some(error) = outcome.error.as_ref()
        && matches!(disposition, WorkplaneJobCompletionDisposition::Failed)
    {
        log_workplane_job_failure(job, error);
    }
    Ok(disposition)
}

#[cfg(test)]
pub(super) fn format_workplane_job_completion_log(
    job: &WorkplaneJobRecord,
    completed_at_unix: u64,
    outcome: &JobExecutionOutcome,
) -> String {
    let disposition = if outcome.error.is_some() {
        WorkplaneJobCompletionDisposition::Failed
    } else {
        WorkplaneJobCompletionDisposition::Completed
    };
    format_workplane_job_completion_log_with_disposition(
        job,
        completed_at_unix,
        outcome,
        disposition,
    )
}

fn format_workplane_job_completion_log_with_disposition(
    job: &WorkplaneJobRecord,
    completed_at_unix: u64,
    _outcome: &JobExecutionOutcome,
    disposition: WorkplaneJobCompletionDisposition,
) -> String {
    let started_at_unix = job.started_at_unix.unwrap_or(completed_at_unix);
    let queue_wait_secs = started_at_unix.saturating_sub(job.submitted_at_unix);
    let run_secs = completed_at_unix.saturating_sub(started_at_unix);
    let mut line = format!(
        "capability workplane job completed: id={} repo={} mailbox_name={} payload_work_item_count={} queue_wait_secs={} run_secs={} attempts={} outcome={}",
        job.job_id,
        job.repo_id,
        job.mailbox_name,
        payload_work_item_count(&job.payload, &job.mailbox_name),
        queue_wait_secs,
        run_secs,
        job.attempts,
        disposition.outcome_label(),
    );
    if let WorkplaneJobCompletionDisposition::RetryScheduled { retry_in_secs, .. } = disposition {
        line.push_str(&format!(" retry_in_secs={retry_in_secs}"));
    }
    line
}

pub(super) fn project_workplane_status(
    workplane_store: &DaemonSqliteRuntimeStore,
    control_state: &EnrichmentControlState,
    budgets: EnrichmentWorkerBudgets,
) -> Result<ProjectedEnrichmentQueueState> {
    let (pending_summary, pending_embeddings, pending_rebuilds) =
        count_workplane_job_buckets(workplane_store, WorkplaneJobStatus::Pending)?;
    let (running_summary, running_embeddings, running_rebuilds) =
        count_workplane_job_buckets(workplane_store, WorkplaneJobStatus::Running)?;
    let (failed_summary, failed_embeddings, failed_rebuilds) =
        count_workplane_job_buckets(workplane_store, WorkplaneJobStatus::Failed)?;
    let (completed_summary, completed_embeddings, completed_rebuilds) =
        count_workplane_job_buckets(workplane_store, WorkplaneJobStatus::Completed)?;
    let worker_pools = vec![
        EnrichmentWorkerPoolStatus {
            kind: EnrichmentWorkerPoolKind::SummaryRefresh,
            worker_budget: budgets.summary_refresh as u64,
            active_workers: running_summary.jobs,
            pending_jobs: pending_summary.jobs,
            running_jobs: running_summary.jobs,
            failed_jobs: failed_summary.jobs,
            completed_recent_jobs: completed_summary.jobs,
        },
        EnrichmentWorkerPoolStatus {
            kind: EnrichmentWorkerPoolKind::Embeddings,
            worker_budget: budgets.embeddings as u64,
            active_workers: running_embeddings.jobs,
            pending_jobs: pending_embeddings.jobs,
            running_jobs: running_embeddings.jobs,
            failed_jobs: failed_embeddings.jobs,
            completed_recent_jobs: completed_embeddings.jobs,
        },
        EnrichmentWorkerPoolStatus {
            kind: EnrichmentWorkerPoolKind::CloneRebuild,
            worker_budget: budgets.clone_rebuild as u64,
            active_workers: running_rebuilds.jobs,
            pending_jobs: pending_rebuilds.jobs,
            running_jobs: running_rebuilds.jobs,
            failed_jobs: failed_rebuilds.jobs,
            completed_recent_jobs: completed_rebuilds.jobs,
        },
    ];

    Ok(ProjectedEnrichmentQueueState {
        version: 1,
        mode: if control_state.paused_embeddings || control_state.paused_semantic {
            EnrichmentQueueMode::Paused
        } else {
            EnrichmentQueueMode::Running
        },
        worker_pools,
        pending_jobs: pending_summary.jobs + pending_embeddings.jobs + pending_rebuilds.jobs,
        pending_work_items: pending_summary.work_items
            + pending_embeddings.work_items
            + pending_rebuilds.work_items,
        pending_semantic_jobs: pending_summary.jobs,
        pending_semantic_work_items: pending_summary.work_items,
        pending_embedding_jobs: pending_embeddings.jobs,
        pending_embedding_work_items: pending_embeddings.work_items,
        pending_clone_edges_rebuild_jobs: pending_rebuilds.jobs,
        pending_clone_edges_rebuild_work_items: pending_rebuilds.work_items,
        completed_recent_jobs: completed_summary.jobs
            + completed_embeddings.jobs
            + completed_rebuilds.jobs,
        running_jobs: running_summary.jobs + running_embeddings.jobs + running_rebuilds.jobs,
        running_work_items: running_summary.work_items
            + running_embeddings.work_items
            + running_rebuilds.work_items,
        running_semantic_jobs: running_summary.jobs,
        running_semantic_work_items: running_summary.work_items,
        running_embedding_jobs: running_embeddings.jobs,
        running_embedding_work_items: running_embeddings.work_items,
        running_clone_edges_rebuild_jobs: running_rebuilds.jobs,
        running_clone_edges_rebuild_work_items: running_rebuilds.work_items,
        failed_jobs: failed_summary.jobs + failed_embeddings.jobs + failed_rebuilds.jobs,
        failed_work_items: failed_summary.work_items
            + failed_embeddings.work_items
            + failed_rebuilds.work_items,
        failed_semantic_jobs: failed_summary.jobs,
        failed_semantic_work_items: failed_summary.work_items,
        failed_embedding_jobs: failed_embeddings.jobs,
        failed_embedding_work_items: failed_embeddings.work_items,
        failed_clone_edges_rebuild_jobs: failed_rebuilds.jobs,
        failed_clone_edges_rebuild_work_items: failed_rebuilds.work_items,
        retried_failed_jobs: control_state.retried_failed_jobs,
        last_action: control_state.last_action.clone(),
        last_updated_unix: control_state.updated_at_unix,
        paused_reason: control_state.paused_reason.clone(),
    })
}

pub(super) fn iter_workplane_job_config_roots(
    workplane_store: &DaemonSqliteRuntimeStore,
) -> Result<Vec<PathBuf>> {
    workplane_store.with_connection(|conn| {
        let mut stmt = conn.prepare(
            "SELECT DISTINCT config_root
             FROM capability_workplane_jobs
             WHERE status IN (?1, ?2)
             UNION
             SELECT DISTINCT config_root
             FROM semantic_summary_mailbox_items
             WHERE status IN (?3, ?4)
             UNION
             SELECT DISTINCT config_root
             FROM semantic_embedding_mailbox_items
             WHERE status IN (?5, ?6)",
        )?;
        let rows = stmt.query_map(
            params![
                WorkplaneJobStatus::Pending.as_str(),
                WorkplaneJobStatus::Running.as_str(),
                SemanticMailboxItemStatus::Pending.as_str(),
                SemanticMailboxItemStatus::Leased.as_str(),
                SemanticMailboxItemStatus::Pending.as_str(),
                SemanticMailboxItemStatus::Leased.as_str(),
            ],
            |row| row.get::<_, String>(0),
        )?;
        let mut values = Vec::new();
        for row in rows {
            values.push(PathBuf::from(row?));
        }
        Ok(values)
    })
}

pub(super) fn retry_failed_workplane_jobs(
    workplane_store: &DaemonSqliteRuntimeStore,
) -> Result<u64> {
    workplane_store.with_connection(|conn| {
        conn.execute(
            "UPDATE capability_workplane_jobs
                 SET status = ?1,
                     started_at_unix = NULL,
                     updated_at_unix = ?2,
                     completed_at_unix = NULL,
                     last_error = NULL
                 WHERE status = ?3",
            params![
                WorkplaneJobStatus::Pending.as_str(),
                sql_i64(unix_timestamp_now())?,
                WorkplaneJobStatus::Failed.as_str(),
            ],
        )
        .map(|count| u64::try_from(count).unwrap_or_default())
        .map_err(anyhow::Error::from)
    })
}

pub(super) fn last_failed_embedding_job_from_workplane(
    workplane_store: &DaemonSqliteRuntimeStore,
) -> Result<Option<FailedEmbeddingJobSummary>> {
    workplane_store.with_connection(|conn| {
        let legacy = conn
            .query_row(
            "SELECT job_id, repo_id, mailbox_name, payload, attempts, last_error, updated_at_unix
             FROM capability_workplane_jobs
             WHERE status = ?1
               AND mailbox_name IN (?2, ?3)
             ORDER BY updated_at_unix DESC
             LIMIT 1",
            params![
                WorkplaneJobStatus::Failed.as_str(),
                SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
                SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
            ],
                |row| {
                    let mailbox_name = row.get::<_, String>(2)?;
                    let payload_raw = row.get::<_, String>(3)?;
                    let payload =
                        serde_json::from_str::<serde_json::Value>(&payload_raw).unwrap_or_default();
                    Ok(FailedEmbeddingJobSummary {
                        job_id: row.get(0)?,
                        repo_id: row.get(1)?,
                        branch: "unknown".to_string(),
                        representation_kind: if mailbox_name
                            == SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX
                        {
                            "code".to_string()
                        } else {
                            "summary".to_string()
                        },
                        artefact_count: payload_work_item_count(&payload, &mailbox_name),
                        attempts: row.get(4)?,
                        error: row.get(5)?,
                        updated_at_unix: u64::try_from(row.get::<_, i64>(6)?)
                            .unwrap_or_default(),
                    })
                },
            )
            .optional()?;
        let inbox = conn
            .query_row(
                "SELECT item_id, repo_id, representation_kind, item_kind, payload_json,
                        attempts, last_error, updated_at_unix
                 FROM semantic_embedding_mailbox_items
                 WHERE status = ?1
                 ORDER BY updated_at_unix DESC
                 LIMIT 1",
                params![SemanticMailboxItemStatus::Failed.as_str()],
                |row| {
                    let payload_json = row
                        .get::<_, Option<String>>(4)?
                        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok());
                    let item_kind = SemanticMailboxItemKind::parse(&row.get::<_, String>(3)?);
                    Ok(FailedEmbeddingJobSummary {
                        job_id: row.get(0)?,
                        repo_id: row.get(1)?,
                        branch: "unknown".to_string(),
                        representation_kind: row.get(2)?,
                        artefact_count: semantic_mailbox_item_work_item_count(
                            item_kind,
                            payload_json.as_ref(),
                        ),
                        attempts: row.get(5)?,
                        error: row.get(6)?,
                        updated_at_unix: parse_u64(row.get::<_, i64>(7)?),
                    })
                },
            )
            .optional()?;

        Ok(match (legacy, inbox) {
            (Some(legacy), Some(inbox)) => {
                if inbox.updated_at_unix >= legacy.updated_at_unix {
                    Some(inbox)
                } else {
                    Some(legacy)
                }
            }
            (Some(legacy), None) => Some(legacy),
            (None, Some(inbox)) => Some(inbox),
            (None, None) => None,
        })
    })
}

fn count_workplane_job_buckets(
    workplane_store: &DaemonSqliteRuntimeStore,
    status: WorkplaneJobStatus,
) -> Result<(
    WorkplaneBucketCounts,
    WorkplaneBucketCounts,
    WorkplaneBucketCounts,
)> {
    workplane_store.with_connection(|conn| {
        let summary = count_summary_bucket_counts(conn, status)?;
        let embeddings = count_embedding_bucket_counts(conn, status)?;
        let rebuilds = count_clone_rebuild_bucket_counts(conn, status)?;
        Ok((summary, embeddings, rebuilds))
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WorkplaneJobCompletionDisposition {
    Completed,
    Failed,
    RetryScheduled {
        available_at_unix: u64,
        retry_in_secs: u64,
    },
}

impl WorkplaneJobCompletionDisposition {
    const fn outcome_label(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::RetryScheduled { .. } => "retry_scheduled",
        }
    }
}

fn classify_workplane_job_completion(
    job: &WorkplaneJobRecord,
    outcome: &JobExecutionOutcome,
    now: u64,
) -> WorkplaneJobCompletionDisposition {
    let Some(error) = outcome.error.as_deref() else {
        return WorkplaneJobCompletionDisposition::Completed;
    };
    if should_retry_transient_embedding_failure(job, error) {
        let retry_in_secs = transient_embedding_retry_backoff_secs(job.attempts);
        return WorkplaneJobCompletionDisposition::RetryScheduled {
            available_at_unix: now.saturating_add(retry_in_secs),
            retry_in_secs,
        };
    }
    WorkplaneJobCompletionDisposition::Failed
}

fn should_retry_transient_embedding_failure(job: &WorkplaneJobRecord, error: &str) -> bool {
    is_embedding_mailbox(job.mailbox_name.as_str())
        && job.attempts < WORKPLANE_TRANSIENT_EMBEDDING_RETRY_LIMIT
        && error.contains("timed out after")
}

fn transient_embedding_retry_backoff_secs(attempts: u32) -> u64 {
    match attempts {
        0 | 1 => 5,
        2 => 15,
        _ => 30,
    }
}

fn is_embedding_mailbox(mailbox_name: &str) -> bool {
    matches!(
        mailbox_name,
        SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX | SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX
    )
}

pub(super) fn current_workplane_mailbox_blocked_statuses(
    workplane_store: &DaemonSqliteRuntimeStore,
    runtime_store: &DaemonSqliteRuntimeStore,
) -> Result<Vec<BlockedMailboxStatus>> {
    current_workplane_mailbox_blocked_statuses_for_repo_internal(
        workplane_store,
        runtime_store,
        None,
    )
}

pub(crate) fn current_workplane_mailbox_blocked_statuses_for_repo(
    workplane_store: &DaemonSqliteRuntimeStore,
    runtime_store: &DaemonSqliteRuntimeStore,
    repo_id: &str,
) -> Result<Vec<BlockedMailboxStatus>> {
    current_workplane_mailbox_blocked_statuses_for_repo_internal(
        workplane_store,
        runtime_store,
        Some(repo_id),
    )
}

fn current_workplane_mailbox_blocked_statuses_for_repo_internal(
    workplane_store: &DaemonSqliteRuntimeStore,
    runtime_store: &DaemonSqliteRuntimeStore,
    repo_id: Option<&str>,
) -> Result<Vec<BlockedMailboxStatus>> {
    let jobs = workplane_store.with_connection(load_pending_mailbox_readiness_jobs)?;
    let mut readiness_cache = BTreeMap::new();
    let mut blocked_by_mailbox = BTreeMap::<String, String>::new();
    for job in jobs {
        if repo_id.is_some_and(|repo_id| job.repo_id != repo_id) {
            continue;
        }
        let readiness = mailbox_claim_readiness(runtime_store, &mut readiness_cache, &job)?;
        if readiness.blocked
            && let Some(reason) = readiness.reason
        {
            blocked_by_mailbox
                .entry(job.mailbox_name.clone())
                .or_insert(reason);
        }
    }
    Ok(blocked_by_mailbox
        .into_iter()
        .map(|(mailbox_name, reason)| BlockedMailboxStatus {
            mailbox_name,
            reason,
        })
        .collect())
}

fn count_summary_bucket_counts(
    conn: &rusqlite::Connection,
    status: WorkplaneJobStatus,
) -> Result<WorkplaneBucketCounts> {
    let mut counts = WorkplaneBucketCounts::default();
    for job in load_workplane_jobs_by_status(conn, status)? {
        if job.mailbox_name == SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX {
            counts.jobs += 1;
            counts.work_items += payload_work_item_count(&job.payload, &job.mailbox_name);
        }
    }
    let Some(mailbox_status) = semantic_mailbox_status_for_workplane_status(status) else {
        return Ok(counts);
    };
    let items = load_summary_mailbox_items_by_status(conn, mailbox_status)?;
    counts.jobs += summary_mailbox_job_count(mailbox_status, &items);
    counts.work_items += items
        .iter()
        .map(|item| {
            semantic_mailbox_item_work_item_count(item.item_kind, item.payload_json.as_ref())
        })
        .sum::<u64>();
    Ok(counts)
}

fn count_embedding_bucket_counts(
    conn: &rusqlite::Connection,
    status: WorkplaneJobStatus,
) -> Result<WorkplaneBucketCounts> {
    let mut counts = WorkplaneBucketCounts::default();
    for job in load_workplane_jobs_by_status(conn, status)? {
        if matches!(
            job.mailbox_name.as_str(),
            SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX | SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX
        ) {
            counts.jobs += 1;
            counts.work_items += payload_work_item_count(&job.payload, &job.mailbox_name);
        }
    }
    let Some(mailbox_status) = semantic_mailbox_status_for_workplane_status(status) else {
        return Ok(counts);
    };
    let items = load_embedding_mailbox_items_by_status(conn, mailbox_status)?;
    counts.jobs += embedding_mailbox_job_count(mailbox_status, &items);
    counts.work_items += items
        .iter()
        .map(|item| {
            semantic_mailbox_item_work_item_count(item.item_kind, item.payload_json.as_ref())
        })
        .sum::<u64>();
    Ok(counts)
}

fn count_clone_rebuild_bucket_counts(
    conn: &rusqlite::Connection,
    status: WorkplaneJobStatus,
) -> Result<WorkplaneBucketCounts> {
    let mut counts = WorkplaneBucketCounts::default();
    for job in load_workplane_jobs_by_status(conn, status)? {
        if job.mailbox_name == SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX {
            counts.jobs += 1;
            counts.work_items += payload_work_item_count(&job.payload, &job.mailbox_name);
        }
    }
    Ok(counts)
}

fn semantic_mailbox_status_for_workplane_status(
    status: WorkplaneJobStatus,
) -> Option<SemanticMailboxItemStatus> {
    match status {
        WorkplaneJobStatus::Pending => Some(SemanticMailboxItemStatus::Pending),
        WorkplaneJobStatus::Running => Some(SemanticMailboxItemStatus::Leased),
        WorkplaneJobStatus::Failed => Some(SemanticMailboxItemStatus::Failed),
        WorkplaneJobStatus::Completed => None,
    }
}

fn summary_mailbox_job_count(
    status: SemanticMailboxItemStatus,
    items: &[SemanticSummaryMailboxItemRecord],
) -> u64 {
    if status != SemanticMailboxItemStatus::Leased {
        return u64::try_from(items.len()).unwrap_or_default();
    }
    items
        .iter()
        .map(|item| {
            item.lease_token
                .clone()
                .unwrap_or_else(|| item.item_id.clone())
        })
        .collect::<BTreeSet<_>>()
        .len() as u64
}

fn embedding_mailbox_job_count(
    status: SemanticMailboxItemStatus,
    items: &[SemanticEmbeddingMailboxItemRecord],
) -> u64 {
    if status != SemanticMailboxItemStatus::Leased {
        return u64::try_from(items.len()).unwrap_or_default();
    }
    items
        .iter()
        .map(|item| {
            item.lease_token
                .clone()
                .unwrap_or_else(|| item.item_id.clone())
        })
        .collect::<BTreeSet<_>>()
        .len() as u64
}

fn semantic_mailbox_item_work_item_count(
    item_kind: SemanticMailboxItemKind,
    payload_json: Option<&serde_json::Value>,
) -> u64 {
    match item_kind {
        SemanticMailboxItemKind::Artefact => 1,
        SemanticMailboxItemKind::RepoBackfill => payload_json
            .and_then(serde_json::Value::as_array)
            .map(|artefact_ids| artefact_ids.len() as u64)
            .unwrap_or(1),
    }
}

fn load_pending_mailbox_readiness_jobs(
    conn: &rusqlite::Connection,
) -> Result<Vec<WorkplaneJobRecord>> {
    let mut jobs = load_workplane_jobs_by_status(conn, WorkplaneJobStatus::Pending)?;
    let summary_items =
        load_summary_mailbox_items_by_status(conn, SemanticMailboxItemStatus::Pending)?;
    jobs.extend(
        summary_items
            .into_iter()
            .map(summary_mailbox_item_as_readiness_job),
    );
    let embedding_items =
        load_embedding_mailbox_items_by_status(conn, SemanticMailboxItemStatus::Pending)?;
    jobs.extend(
        embedding_items
            .into_iter()
            .map(embedding_mailbox_item_as_readiness_job),
    );
    Ok(jobs)
}

fn summary_mailbox_item_as_readiness_job(
    item: SemanticSummaryMailboxItemRecord,
) -> WorkplaneJobRecord {
    WorkplaneJobRecord {
        job_id: item.item_id,
        repo_id: item.repo_id,
        repo_root: item.repo_root,
        config_root: item.config_root,
        capability_id: SEMANTIC_CLONES_CAPABILITY_ID.to_string(),
        mailbox_name: SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX.to_string(),
        init_session_id: item.init_session_id,
        dedupe_key: item.dedupe_key,
        payload: serde_json::Value::Null,
        status: WorkplaneJobStatus::Pending,
        attempts: item.attempts,
        available_at_unix: item.available_at_unix,
        submitted_at_unix: item.submitted_at_unix,
        started_at_unix: None,
        updated_at_unix: item.updated_at_unix,
        completed_at_unix: None,
        lease_owner: None,
        lease_expires_at_unix: None,
        last_error: item.last_error,
    }
}

fn embedding_mailbox_item_as_readiness_job(
    item: SemanticEmbeddingMailboxItemRecord,
) -> WorkplaneJobRecord {
    let mailbox_name =
        if item.representation_kind == EmbeddingRepresentationKind::Summary.to_string() {
            SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX
        } else {
            SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX
        };
    WorkplaneJobRecord {
        job_id: item.item_id,
        repo_id: item.repo_id,
        repo_root: item.repo_root,
        config_root: item.config_root,
        capability_id: SEMANTIC_CLONES_CAPABILITY_ID.to_string(),
        mailbox_name: mailbox_name.to_string(),
        init_session_id: item.init_session_id,
        dedupe_key: item.dedupe_key,
        payload: serde_json::Value::Null,
        status: WorkplaneJobStatus::Pending,
        attempts: item.attempts,
        available_at_unix: item.available_at_unix,
        submitted_at_unix: item.submitted_at_unix,
        started_at_unix: None,
        updated_at_unix: item.updated_at_unix,
        completed_at_unix: None,
        lease_owner: None,
        lease_expires_at_unix: None,
        last_error: item.last_error,
    }
}

fn mailbox_claim_readiness(
    runtime_store: &DaemonSqliteRuntimeStore,
    cache: &mut BTreeMap<(PathBuf, String, String), WorkplaneMailboxReadiness>,
    job: &WorkplaneJobRecord,
) -> Result<WorkplaneMailboxReadiness> {
    let key = (
        job.repo_root.clone(),
        job.capability_id.clone(),
        job.mailbox_name.clone(),
    );
    if let Some(readiness) = cache.get(&key) {
        return Ok(readiness.clone());
    }
    let Some(registration) = workplane_mailbox_registration(&job.capability_id, &job.mailbox_name)
    else {
        let readiness = WorkplaneMailboxReadiness {
            blocked: true,
            reason: Some(format!(
                "mailbox `{}` is not registered for capability `{}`",
                job.mailbox_name, job.capability_id
            )),
        };
        cache.insert(key, readiness.clone());
        return Ok(readiness);
    };

    let readiness = match registration.readiness_policy {
        CapabilityMailboxReadinessPolicy::None => WorkplaneMailboxReadiness::default(),
        CapabilityMailboxReadinessPolicy::TextGenerationSlot(slot_name) => {
            resolve_mailbox_provider_readiness(runtime_store, job, slot_name, true)?
        }
        CapabilityMailboxReadinessPolicy::EmbeddingsSlot(slot_name) => {
            resolve_mailbox_provider_readiness(runtime_store, job, slot_name, false)?
        }
    };
    cache.insert(key, readiness.clone());
    Ok(readiness)
}

fn resolve_mailbox_provider_readiness(
    runtime_store: &DaemonSqliteRuntimeStore,
    job: &WorkplaneJobRecord,
    slot_name: &str,
    text_generation: bool,
) -> Result<WorkplaneMailboxReadiness> {
    if !text_generation {
        let gate_status = crate::daemon::embeddings_bootstrap::gate_status_for_config_path(
            runtime_store,
            &job.config_root
                .join(crate::config::BITLOOPS_CONFIG_RELATIVE_PATH),
        )?;
        if gate_status.blocked {
            return Ok(WorkplaneMailboxReadiness {
                blocked: true,
                reason: gate_status.reason,
            });
        }
    }

    let repo = crate::host::devql::resolve_repo_identity(&job.repo_root)
        .unwrap_or_else(|_| fallback_repo_identity(&job.repo_root, &job.repo_id));
    let capability_host = crate::host::devql::build_capability_host(&job.repo_root, repo)?;
    let inference = capability_host.inference_for_capability(&job.capability_id);
    let Some(_slot) = inference.describe(slot_name) else {
        return Ok(WorkplaneMailboxReadiness {
            blocked: true,
            reason: Some(format!(
                "{} slot `{slot_name}` is not configured yet",
                if text_generation {
                    "text-generation"
                } else {
                    "embedding"
                }
            )),
        });
    };

    let resolution = if text_generation {
        inference.text_generation(slot_name).map(|_| ())
    } else {
        inference.embeddings(slot_name).map(|_| ())
    };
    match resolution {
        Ok(()) => Ok(WorkplaneMailboxReadiness::default()),
        Err(err) => Ok(WorkplaneMailboxReadiness {
            blocked: true,
            reason: Some(format!("{err:#}")),
        }),
    }
}

pub(super) fn load_workplane_jobs_by_status(
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

fn map_workplane_job_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkplaneJobRecord> {
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

fn workplane_mailbox_registration(
    capability_id: &str,
    mailbox_name: &str,
) -> Option<CapabilityMailboxRegistration> {
    if capability_id != SEMANTIC_CLONES_CAPABILITY_ID {
        return None;
    }
    match mailbox_name {
        SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX => Some(
            CapabilityMailboxRegistration::new(
                SEMANTIC_CLONES_CAPABILITY_ID,
                SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
                crate::host::capability_host::CapabilityMailboxPolicy::Job,
                crate::host::capability_host::CapabilityMailboxHandler::Ingester(
                    crate::capability_packs::semantic_clones::SEMANTIC_CLONES_SEMANTIC_FEATURES_REFRESH_INGESTER_ID,
                ),
            )
            .readiness_policy(CapabilityMailboxReadinessPolicy::TextGenerationSlot(
                SEMANTIC_CLONES_SUMMARY_GENERATION_SLOT,
            ))
            .backlog_policy(CapabilityMailboxBacklogPolicy::ArtefactCompaction),
        ),
        SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX => Some(
            CapabilityMailboxRegistration::new(
                SEMANTIC_CLONES_CAPABILITY_ID,
                SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
                crate::host::capability_host::CapabilityMailboxPolicy::Job,
                crate::host::capability_host::CapabilityMailboxHandler::Ingester(
                    crate::capability_packs::semantic_clones::SEMANTIC_CLONES_SYMBOL_EMBEDDINGS_REFRESH_INGESTER_ID,
                ),
            )
            .readiness_policy(CapabilityMailboxReadinessPolicy::EmbeddingsSlot(
                SEMANTIC_CLONES_CODE_EMBEDDINGS_SLOT,
            ))
            .backlog_policy(CapabilityMailboxBacklogPolicy::ArtefactCompaction),
        ),
        SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX => Some(
            CapabilityMailboxRegistration::new(
                SEMANTIC_CLONES_CAPABILITY_ID,
                SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
                crate::host::capability_host::CapabilityMailboxPolicy::Job,
                crate::host::capability_host::CapabilityMailboxHandler::Ingester(
                    crate::capability_packs::semantic_clones::SEMANTIC_CLONES_SYMBOL_EMBEDDINGS_REFRESH_INGESTER_ID,
                ),
            )
            .readiness_policy(CapabilityMailboxReadinessPolicy::EmbeddingsSlot(
                SEMANTIC_CLONES_SUMMARY_EMBEDDINGS_SLOT,
            ))
            .backlog_policy(CapabilityMailboxBacklogPolicy::ArtefactCompaction),
        ),
        SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX => Some(
            CapabilityMailboxRegistration::new(
                SEMANTIC_CLONES_CAPABILITY_ID,
                SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
                crate::host::capability_host::CapabilityMailboxPolicy::Job,
                crate::host::capability_host::CapabilityMailboxHandler::Ingester(
                    crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CLONE_EDGES_REBUILD_INGESTER_ID,
                ),
            )
            .backlog_policy(CapabilityMailboxBacklogPolicy::RepoCoalesced),
        ),
        _ => None,
    }
}

fn job_is_paused_for_mailbox(state: &EnrichmentControlState, mailbox_name: &str) -> bool {
    match mailbox_name {
        SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX => state.paused_semantic,
        SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX
        | SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX
        | SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX => state.paused_embeddings,
        _ => false,
    }
}

fn log_workplane_job_failure(job: &WorkplaneJobRecord, error: &str) {
    log::error!(
        "daemon enrichment job failed: id={} repo={} mailbox={} attempts={} error={}",
        job.job_id,
        job.repo_id,
        job.mailbox_name,
        job.attempts,
        error,
    );
}

pub(super) fn sql_i64(value: u64) -> Result<i64> {
    i64::try_from(value).context("converting enrichment runtime value to SQLite integer")
}

fn parse_u64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or_default()
}

fn sql_string_list(values: &[String]) -> String {
    values
        .iter()
        .map(|value| format!("'{}'", esc_pg(value)))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn fallback_repo_identity(repo_root: &Path, repo_id: &str) -> RepoIdentity {
    let name = repo_root
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("repository")
        .to_string();
    RepoIdentity {
        provider: "git".to_string(),
        organization: "local".to_string(),
        name: name.clone(),
        identity: format!("git/local/{name}"),
        repo_id: repo_id.to_string(),
    }
}

use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::capability_packs::semantic_clones::types::{
    SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX, SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_CODE_EMBEDDINGS_SLOT, SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_SUMMARY_EMBEDDINGS_SLOT, SEMANTIC_CLONES_SUMMARY_GENERATION_SLOT,
    SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
};
use crate::capability_packs::semantic_clones::workplane::{
    SemanticClonesMailboxPayload, payload_work_item_count, repo_backfill_dedupe_key,
};
use crate::capability_packs::semantic_clones::{
    SEMANTIC_CLONES_CAPABILITY_ID, embeddings::EmbeddingRepresentationKind,
};
use crate::host::capability_host::{
    CapabilityMailboxBacklogPolicy, CapabilityMailboxReadinessPolicy, CapabilityMailboxRegistration,
};
use crate::host::devql::RepoIdentity;
use crate::host::inference::InferenceGateway;
use crate::host::runtime_store::{
    DaemonSqliteRuntimeStore, RepoSqliteRuntimeStore, WorkplaneJobRecord, WorkplaneJobStatus,
};

use super::super::types::{
    BlockedMailboxStatus, EnrichmentQueueMode,
    EnrichmentQueueState as ProjectedEnrichmentQueueState, FailedEmbeddingJobSummary,
    unix_timestamp_now,
};
use super::{
    EnrichmentJobTarget, EnrichmentQueueState, JobExecutionOutcome,
    WORKPLANE_PENDING_COMPACTION_MIN_AGE_SECS, WORKPLANE_PENDING_COMPACTION_MIN_COUNT,
    WORKPLANE_TERMINAL_RETENTION_SECS, WORKPLANE_TERMINAL_ROW_LIMIT, WorkplaneMailboxReadiness,
};

pub(super) fn default_state() -> EnrichmentQueueState {
    EnrichmentQueueState {
        version: 1,
        last_action: Some("initialized".to_string()),
        ..EnrichmentQueueState::default()
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct WorkplaneBucketCounts {
    jobs: u64,
    work_items: u64,
}

pub(super) fn enqueue_workplane_summary_jobs(
    target: &EnrichmentJobTarget,
    artefact_ids: Vec<String>,
) -> Result<()> {
    let store = RepoSqliteRuntimeStore::open_for_roots(&target.config_root, &target.repo_root)?;
    let jobs = if artefact_ids.is_empty() {
        vec![
            crate::host::runtime_store::CapabilityWorkplaneJobInsert::new(
                SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
                Some(repo_backfill_dedupe_key(
                    SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
                )),
                serde_json::to_value(SemanticClonesMailboxPayload::RepoBackfill)
                    .expect("summary repo backfill payload should serialize"),
            ),
        ]
    } else {
        artefact_ids
            .into_iter()
            .map(|artefact_id| {
                crate::host::runtime_store::CapabilityWorkplaneJobInsert::new(
                    SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
                    Some(format!(
                        "{}:{artefact_id}",
                        SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX
                    )),
                    serde_json::to_value(SemanticClonesMailboxPayload::Artefact { artefact_id })
                        .expect("summary artefact payload should serialize"),
                )
            })
            .collect()
    };
    let _ = store.enqueue_capability_workplane_jobs(SEMANTIC_CLONES_CAPABILITY_ID, jobs)?;
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
    let jobs = if artefact_ids.is_empty() {
        vec![
            crate::host::runtime_store::CapabilityWorkplaneJobInsert::new(
                mailbox_name,
                Some(repo_backfill_dedupe_key(mailbox_name)),
                serde_json::to_value(SemanticClonesMailboxPayload::RepoBackfill)
                    .expect("embedding repo backfill payload should serialize"),
            ),
        ]
    } else {
        artefact_ids
            .into_iter()
            .map(|artefact_id| {
                crate::host::runtime_store::CapabilityWorkplaneJobInsert::new(
                    mailbox_name,
                    Some(format!("{mailbox_name}:{artefact_id}")),
                    serde_json::to_value(SemanticClonesMailboxPayload::Artefact { artefact_id })
                        .expect("embedding artefact payload should serialize"),
                )
            })
            .collect()
    };
    let _ = store.enqueue_capability_workplane_jobs(SEMANTIC_CLONES_CAPABILITY_ID, jobs)?;
    Ok(())
}

pub(super) fn enqueue_workplane_clone_rebuild(target: &EnrichmentJobTarget) -> Result<()> {
    let store = RepoSqliteRuntimeStore::open_for_roots(&target.config_root, &target.repo_root)?;
    let _ = store.enqueue_capability_workplane_jobs(
        SEMANTIC_CLONES_CAPABILITY_ID,
        vec![
            crate::host::runtime_store::CapabilityWorkplaneJobInsert::new(
                SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
                Some(SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX.to_string()),
                serde_json::to_value(SemanticClonesMailboxPayload::RepoBackfill)
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
        compact_pending_artefact_backlogs(conn)?;
        prune_inactive_summary_refresh_jobs(conn)?;
        prune_terminal_workplane_jobs(conn)?;
        Ok(())
    })
}

fn compact_pending_artefact_backlogs(conn: &rusqlite::Connection) -> Result<()> {
    let now = unix_timestamp_now();
    for registration in [
        workplane_mailbox_registration(
            SEMANTIC_CLONES_CAPABILITY_ID,
            SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
        ),
        workplane_mailbox_registration(
            SEMANTIC_CLONES_CAPABILITY_ID,
            SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
        ),
        workplane_mailbox_registration(
            SEMANTIC_CLONES_CAPABILITY_ID,
            SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
        ),
    ]
    .into_iter()
    .flatten()
    {
        if registration.backlog_policy != CapabilityMailboxBacklogPolicy::ArtefactCompaction {
            continue;
        }
        let backfill_dedupe_key = repo_backfill_dedupe_key(registration.mailbox_name);
        let mut stmt = conn.prepare(
            "SELECT repo_id, repo_root, config_root, MIN(submitted_at_unix), COUNT(*)
             FROM capability_workplane_jobs
             WHERE capability_id = ?1
               AND mailbox_name = ?2
               AND status = ?3
               AND (dedupe_key IS NULL OR dedupe_key != ?4)
             GROUP BY repo_id, repo_root, config_root",
        )?;
        let rows = stmt.query_map(
            params![
                registration.capability_id,
                registration.mailbox_name,
                WorkplaneJobStatus::Pending.as_str(),
                backfill_dedupe_key.as_str(),
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            },
        )?;
        for row in rows {
            let (repo_id, repo_root, config_root, oldest_submitted, pending_count) = row?;
            let oldest_submitted = u64::try_from(oldest_submitted).unwrap_or_default();
            let pending_count = u64::try_from(pending_count).unwrap_or_default();
            if pending_count < WORKPLANE_PENDING_COMPACTION_MIN_COUNT {
                continue;
            }
            if now.saturating_sub(oldest_submitted) < WORKPLANE_PENDING_COMPACTION_MIN_AGE_SECS {
                continue;
            }
            if !ensure_pending_repo_backfill_job(
                conn,
                &repo_id,
                &repo_root,
                &config_root,
                &registration,
                &backfill_dedupe_key,
                now,
            )? {
                continue;
            }
            conn.execute(
                "DELETE FROM capability_workplane_jobs
                 WHERE repo_id = ?1
                   AND capability_id = ?2
                   AND mailbox_name = ?3
                   AND status = ?4
                   AND (dedupe_key IS NULL OR dedupe_key != ?5)",
                params![
                    repo_id,
                    registration.capability_id,
                    registration.mailbox_name,
                    WorkplaneJobStatus::Pending.as_str(),
                    backfill_dedupe_key,
                ],
            )?;
        }
    }
    Ok(())
}

fn ensure_pending_repo_backfill_job(
    conn: &rusqlite::Connection,
    repo_id: &str,
    repo_root: &str,
    config_root: &str,
    registration: &CapabilityMailboxRegistration,
    dedupe_key: &str,
    now: u64,
) -> Result<bool> {
    let existing = conn
        .query_row(
            "SELECT job_id, status
             FROM capability_workplane_jobs
             WHERE repo_id = ?1
               AND capability_id = ?2
               AND mailbox_name = ?3
               AND dedupe_key = ?4
               AND status IN (?5, ?6)
             ORDER BY CASE status WHEN 'running' THEN 0 ELSE 1 END, submitted_at_unix ASC
             LIMIT 1",
            params![
                repo_id,
                registration.capability_id,
                registration.mailbox_name,
                dedupe_key,
                WorkplaneJobStatus::Running.as_str(),
                WorkplaneJobStatus::Pending.as_str(),
            ],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    let payload = serde_json::to_string(&SemanticClonesMailboxPayload::RepoBackfill)
        .expect("repo backfill payload should serialize");
    match existing {
        Some((_, status)) if WorkplaneJobStatus::parse(&status) == WorkplaneJobStatus::Running => {
            Ok(false)
        }
        Some((job_id, _)) => {
            conn.execute(
                "UPDATE capability_workplane_jobs
                 SET payload = ?1, updated_at_unix = ?2, available_at_unix = ?3, last_error = NULL
                 WHERE job_id = ?4",
                params![payload, sql_i64(now)?, sql_i64(now)?, job_id],
            )?;
            Ok(true)
        }
        None => {
            let job_id = format!("workplane-job-{}", uuid::Uuid::new_v4());
            conn.execute(
                "INSERT INTO capability_workplane_jobs (
                    job_id, repo_id, repo_root, config_root, capability_id, mailbox_name,
                    dedupe_key, payload, status, attempts, available_at_unix, submitted_at_unix,
                    started_at_unix, updated_at_unix, completed_at_unix, lease_owner,
                    lease_expires_at_unix, last_error
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6,
                    ?7, ?8, ?9, 0, ?10, ?11,
                    NULL, ?12, NULL, NULL,
                    NULL, NULL
                 )",
                params![
                    job_id,
                    repo_id,
                    repo_root,
                    config_root,
                    registration.capability_id,
                    registration.mailbox_name,
                    dedupe_key,
                    payload,
                    WorkplaneJobStatus::Pending.as_str(),
                    sql_i64(now)?,
                    sql_i64(now)?,
                    sql_i64(now)?,
                ],
            )?;
            Ok(true)
        }
    }
}

fn prune_inactive_summary_refresh_jobs(conn: &rusqlite::Connection) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT repo_id, repo_root
         FROM capability_workplane_jobs
         WHERE capability_id = ?1
           AND mailbox_name = ?2
           AND status = ?3",
    )?;
    let rows = stmt.query_map(
        params![
            SEMANTIC_CLONES_CAPABILITY_ID,
            SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
            WorkplaneJobStatus::Pending.as_str(),
        ],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    )?;

    for row in rows {
        let (repo_id, repo_root) = row?;
        let repo_root = PathBuf::from(repo_root);
        let summary_refresh_active = match summary_refresh_active_for_repo(&repo_root) {
            Ok(active) => active,
            Err(err) => {
                log::warn!(
                    "failed to resolve semantic summary mailbox intent for {}: {err:#}",
                    repo_root.display()
                );
                continue;
            }
        };
        if summary_refresh_active {
            continue;
        }
        conn.execute(
            "DELETE FROM capability_workplane_jobs
             WHERE repo_id = ?1
               AND capability_id = ?2
               AND mailbox_name = ?3
               AND status = ?4",
            params![
                repo_id,
                SEMANTIC_CLONES_CAPABILITY_ID,
                SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
                WorkplaneJobStatus::Pending.as_str(),
            ],
        )?;
    }

    Ok(())
}

fn summary_refresh_active_for_repo(repo_root: &Path) -> Result<bool> {
    let config = crate::config::resolve_semantic_clones_config_for_repo(repo_root);
    let intent =
        crate::capability_packs::semantic_clones::workplane::load_effective_mailbox_intent_for_repo(
            repo_root,
            &config,
        )?;
    Ok(intent.summary_refresh_active)
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

pub(super) fn claim_next_workplane_job(
    workplane_store: &DaemonSqliteRuntimeStore,
    runtime_store: &DaemonSqliteRuntimeStore,
    control_state: &EnrichmentQueueState,
) -> Result<Option<WorkplaneJobRecord>> {
    workplane_store.with_connection(|conn| {
        conn.execute_batch("BEGIN IMMEDIATE TRANSACTION;")
            .context("starting capability workplane job claim transaction")?;
        let result = (|| {
            let now = unix_timestamp_now();
            let jobs = load_workplane_jobs_by_status(conn, WorkplaneJobStatus::Pending)?;
            let mut readiness_cache = BTreeMap::new();
            for mut job in jobs {
                if job_is_paused_for_mailbox(control_state, &job.mailbox_name) {
                    continue;
                }
                if mailbox_claim_readiness(runtime_store, &mut readiness_cache, &job)?.blocked {
                    continue;
                }
                if job.available_at_unix > now {
                    continue;
                }
                conn.execute(
                    "UPDATE capability_workplane_jobs
                     SET status = ?1,
                         attempts = ?2,
                         started_at_unix = COALESCE(started_at_unix, ?3),
                         updated_at_unix = ?4
                     WHERE job_id = ?5",
                    params![
                        WorkplaneJobStatus::Running.as_str(),
                        job.attempts + 1,
                        sql_i64(now)?,
                        sql_i64(now)?,
                        &job.job_id,
                    ],
                )
                .with_context(|| format!("claiming capability workplane job `{}`", job.job_id))?;
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

pub(super) fn persist_workplane_job_completion(
    workplane_store: &DaemonSqliteRuntimeStore,
    job: &WorkplaneJobRecord,
    outcome: &JobExecutionOutcome,
) -> Result<()> {
    let now = unix_timestamp_now();
    workplane_store.with_connection(|conn| {
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
                if outcome.error.is_some() {
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
        Ok(())
    })?;
    if let Some(error) = outcome.error.as_ref() {
        log_workplane_job_failure(job, error);
    }
    Ok(())
}

pub(super) fn project_workplane_status(
    workplane_store: &DaemonSqliteRuntimeStore,
    control_state: &EnrichmentQueueState,
) -> Result<ProjectedEnrichmentQueueState> {
    let (pending_summary, pending_embeddings, pending_rebuilds) =
        count_workplane_job_buckets(workplane_store, WorkplaneJobStatus::Pending)?;
    let (running_summary, running_embeddings, running_rebuilds) =
        count_workplane_job_buckets(workplane_store, WorkplaneJobStatus::Running)?;
    let (failed_summary, failed_embeddings, failed_rebuilds) =
        count_workplane_job_buckets(workplane_store, WorkplaneJobStatus::Failed)?;

    Ok(ProjectedEnrichmentQueueState {
        version: 1,
        mode: if control_state.paused_embeddings || control_state.paused_semantic {
            EnrichmentQueueMode::Paused
        } else {
            EnrichmentQueueMode::Running
        },
        pending_jobs: pending_summary + pending_embeddings + pending_rebuilds,
        pending_semantic_jobs: pending_summary,
        pending_embedding_jobs: pending_embeddings,
        pending_clone_edges_rebuild_jobs: pending_rebuilds,
        running_jobs: running_summary + running_embeddings + running_rebuilds,
        running_semantic_jobs: running_summary,
        running_embedding_jobs: running_embeddings,
        running_clone_edges_rebuild_jobs: running_rebuilds,
        failed_jobs: failed_summary + failed_embeddings + failed_rebuilds,
        failed_semantic_jobs: failed_summary,
        failed_embedding_jobs: failed_embeddings,
        failed_clone_edges_rebuild_jobs: failed_rebuilds,
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
             WHERE status IN (?1, ?2)",
        )?;
        let rows = stmt.query_map(
            params![
                WorkplaneJobStatus::Pending.as_str(),
                WorkplaneJobStatus::Running.as_str()
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
        conn.query_row(
            "SELECT job_id, repo_id, mailbox_name, attempts, last_error, updated_at_unix
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
                Ok(FailedEmbeddingJobSummary {
                    job_id: row.get(0)?,
                    repo_id: row.get(1)?,
                    branch: "unknown".to_string(),
                    representation_kind: if mailbox_name == SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX {
                        "code".to_string()
                    } else {
                        "summary".to_string()
                    },
                    artefact_count: 1,
                    attempts: row.get(3)?,
                    error: row.get(4)?,
                    updated_at_unix: u64::try_from(row.get::<_, i64>(5)?).unwrap_or_default(),
                })
            },
        )
        .optional()
        .map_err(anyhow::Error::from)
    })
}

fn count_workplane_job_buckets(
    workplane_store: &DaemonSqliteRuntimeStore,
    status: WorkplaneJobStatus,
) -> Result<(u64, u64, u64)> {
    workplane_store.with_connection(|conn| {
        let mut stmt = conn.prepare(
            "SELECT mailbox_name, COUNT(*)
             FROM capability_workplane_jobs
             WHERE status = ?1
             GROUP BY mailbox_name",
        )?;
        let rows = stmt.query_map(params![status.as_str()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        let mut summary = 0u64;
        let mut embeddings = 0u64;
        let mut rebuilds = 0u64;
        for row in rows {
            let (mailbox_name, count) = row?;
            let count = u64::try_from(count).unwrap_or_default();
            match mailbox_name.as_str() {
                SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX => summary += count,
                SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX
                | SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX => embeddings += count,
                SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX => rebuilds += count,
                _ => {}
            }
        }
        Ok((summary, embeddings, rebuilds))
    })
}

pub(super) fn current_workplane_mailbox_blocked_statuses(
    workplane_store: &DaemonSqliteRuntimeStore,
    runtime_store: &DaemonSqliteRuntimeStore,
) -> Result<Vec<BlockedMailboxStatus>> {
    let jobs = workplane_store
        .with_connection(|conn| load_workplane_jobs_by_status(conn, WorkplaneJobStatus::Pending))?;
    let mut readiness_cache = BTreeMap::new();
    let mut blocked_by_mailbox = BTreeMap::<String, String>::new();
    for job in jobs {
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
    let repo = crate::host::devql::resolve_repo_identity(&job.repo_root)
        .unwrap_or_else(|_| fallback_repo_identity(&job.repo_root, &job.repo_id));
    let capability_host = crate::host::devql::build_capability_host(&job.repo_root, repo)?;
    let inference = capability_host.inference_for_capability(&job.capability_id);
    let Some(slot) = inference.describe(slot_name) else {
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

    if !text_generation
        && slot.driver.as_deref() == Some(crate::host::inference::BITLOOPS_EMBEDDINGS_IPC_DRIVER)
        && slot.runtime.as_deref()
            == Some(crate::host::inference::BITLOOPS_LOCAL_EMBEDDINGS_RUNTIME_ID)
    {
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
                dedupe_key, payload, status, attempts, available_at_unix, submitted_at_unix,
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
    let payload_raw = row.get::<_, String>(7)?;
    Ok(WorkplaneJobRecord {
        job_id: row.get(0)?,
        repo_id: row.get(1)?,
        repo_root: PathBuf::from(row.get::<_, String>(2)?),
        config_root: PathBuf::from(row.get::<_, String>(3)?),
        capability_id: row.get(4)?,
        mailbox_name: row.get(5)?,
        dedupe_key: row.get(6)?,
        payload: serde_json::from_str(&payload_raw).unwrap_or(serde_json::Value::Null),
        status: WorkplaneJobStatus::parse(&row.get::<_, String>(8)?),
        attempts: row.get(9)?,
        available_at_unix: u64::try_from(row.get::<_, i64>(10)?).unwrap_or_default(),
        submitted_at_unix: u64::try_from(row.get::<_, i64>(11)?).unwrap_or_default(),
        started_at_unix: row
            .get::<_, Option<i64>>(12)?
            .and_then(|value| u64::try_from(value).ok()),
        updated_at_unix: u64::try_from(row.get::<_, i64>(13)?).unwrap_or_default(),
        completed_at_unix: row
            .get::<_, Option<i64>>(14)?
            .and_then(|value| u64::try_from(value).ok()),
        lease_owner: row.get(15)?,
        lease_expires_at_unix: row
            .get::<_, Option<i64>>(16)?
            .and_then(|value| u64::try_from(value).ok()),
        last_error: row.get(17)?,
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

fn job_is_paused_for_mailbox(state: &EnrichmentQueueState, mailbox_name: &str) -> bool {
    match mailbox_name {
        SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX => state.paused_semantic,
        SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX
        | SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX
        | SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX => state.paused_embeddings,
        _ => false,
    }
}

fn log_workplane_job_failure(job: &WorkplaneJobRecord, error: &str) {
    log::warn!(
        "capability workplane job failed: id={} repo={} mailbox={} attempts={} error={}",
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

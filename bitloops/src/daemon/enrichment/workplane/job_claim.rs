use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use rusqlite::params;

use crate::capability_packs::semantic_clones::types::{
    SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX, SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX, SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
};
use crate::daemon::types::unix_timestamp_now;
use crate::host::capability_host::{
    CapabilityMailboxHandler, CapabilityMailboxPolicy, CapabilityMailboxReadinessPolicy,
};
use crate::host::runtime_store::{
    DaemonSqliteRuntimeStore, WorkplaneJobRecord, WorkplaneJobStatus,
};

use super::super::worker_count::EnrichmentWorkerPool;
use super::super::{EnrichmentControlState, WorkplaneMailboxReadiness};
use super::jobs::map_workplane_job_row;
use super::mailbox_claim::WORKPLANE_JOB_CLAIM_CANDIDATE_LIMIT;
use super::readiness::{
    mailbox_claim_readiness, mailbox_claim_readiness_for_registration,
    workplane_mailbox_registration_for_job,
};
use super::sql::sql_i64;

pub(crate) fn claim_next_workplane_job(
    workplane_store: &DaemonSqliteRuntimeStore,
    runtime_store: &DaemonSqliteRuntimeStore,
    control_state: &EnrichmentControlState,
    pool: EnrichmentWorkerPool,
) -> Result<Option<WorkplaneJobRecord>> {
    workplane_store.with_write_connection(|conn| {
        conn.execute_batch("BEGIN IMMEDIATE TRANSACTION;")
            .context("starting capability workplane job claim transaction")?;
        let result = (|| {
            let now = unix_timestamp_now();
            let mut readiness_cache = BTreeMap::new();
            let jobs = load_workplane_claim_candidates(
                conn,
                runtime_store,
                pool,
                now,
                &mut readiness_cache,
            )?;
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
    runtime_store: &DaemonSqliteRuntimeStore,
    pool: EnrichmentWorkerPool,
    now: u64,
    readiness_cache: &mut BTreeMap<(PathBuf, String, String), WorkplaneMailboxReadiness>,
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
                   AND available_at_unix <= ?2
                 ORDER BY CASE mailbox_name
                              WHEN 'semantic_clones.embedding.code' THEN 0
                              WHEN 'semantic_clones.embedding.summary' THEN 0
                              WHEN 'semantic_clones.summary_refresh' THEN 1
                              WHEN 'semantic_clones.clone_rebuild' THEN 2
                          ELSE 3
                          END ASC,
                          available_at_unix ASC,
                          submitted_at_unix ASC
                 LIMIT ?3",
            )?;
            let rows = stmt.query_map(
                params![WorkplaneJobStatus::Pending.as_str(), now, limit,],
                map_workplane_job_row,
            )?;
            for row in rows {
                let job = row?;
                if job.mailbox_name == SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX
                    || is_generic_inference_job(runtime_store, readiness_cache, &job)?
                {
                    values.push(job);
                }
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

fn is_generic_inference_job(
    runtime_store: &DaemonSqliteRuntimeStore,
    readiness_cache: &mut BTreeMap<(PathBuf, String, String), WorkplaneMailboxReadiness>,
    job: &WorkplaneJobRecord,
) -> Result<bool> {
    if job.capability_id == crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CAPABILITY_ID
        && job.mailbox_name == SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX
    {
        return Ok(false);
    }
    let Some(registration) = workplane_mailbox_registration_for_job(job)? else {
        return Ok(false);
    };
    if registration.policy != CapabilityMailboxPolicy::Job {
        return Ok(false);
    }
    if !matches!(registration.handler, CapabilityMailboxHandler::Ingester(_)) {
        return Ok(false);
    }
    if !is_generic_inference_readiness_policy(registration.readiness_policy) {
        return Ok(false);
    }
    Ok(!mailbox_claim_readiness_for_registration(
        runtime_store,
        readiness_cache,
        job,
        &registration,
    )?
    .blocked)
}

const fn is_generic_inference_readiness_policy(policy: CapabilityMailboxReadinessPolicy) -> bool {
    matches!(
        policy,
        CapabilityMailboxReadinessPolicy::TextGenerationSlot(_)
            | CapabilityMailboxReadinessPolicy::OptionalTextGenerationSlot(_)
            | CapabilityMailboxReadinessPolicy::StructuredGenerationSlot(_)
            | CapabilityMailboxReadinessPolicy::OptionalStructuredGenerationSlot(_)
    )
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

#[cfg(test)]
mod tests {
    use super::is_generic_inference_readiness_policy;
    use crate::host::capability_host::CapabilityMailboxReadinessPolicy;

    #[test]
    fn generic_inference_readiness_policy_accepts_text_and_structured_generation() {
        assert!(is_generic_inference_readiness_policy(
            CapabilityMailboxReadinessPolicy::TextGenerationSlot("slot")
        ));
        assert!(is_generic_inference_readiness_policy(
            CapabilityMailboxReadinessPolicy::OptionalTextGenerationSlot("slot")
        ));
        assert!(is_generic_inference_readiness_policy(
            CapabilityMailboxReadinessPolicy::StructuredGenerationSlot("slot")
        ));
        assert!(is_generic_inference_readiness_policy(
            CapabilityMailboxReadinessPolicy::OptionalStructuredGenerationSlot("slot")
        ));
    }

    #[test]
    fn generic_inference_readiness_policy_rejects_none_and_embeddings() {
        assert!(!is_generic_inference_readiness_policy(
            CapabilityMailboxReadinessPolicy::None
        ));
        assert!(!is_generic_inference_readiness_policy(
            CapabilityMailboxReadinessPolicy::EmbeddingsSlot("slot")
        ));
    }
}

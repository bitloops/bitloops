use std::collections::BTreeSet;
use std::path::PathBuf;

use anyhow::Result;
use rusqlite::{OptionalExtension, params};

use crate::capability_packs::semantic_clones::types::{
    SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX, SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX, SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
};
use crate::capability_packs::semantic_clones::workplane::payload_work_item_count;
use crate::daemon::types::{
    EnrichmentQueueMode, EnrichmentQueueState as ProjectedEnrichmentQueueState,
    EnrichmentWorkerPoolKind, EnrichmentWorkerPoolStatus, FailedEmbeddingJobSummary,
};
use crate::host::runtime_store::{
    DaemonSqliteRuntimeStore, SemanticEmbeddingMailboxItemRecord, SemanticMailboxItemKind,
    SemanticMailboxItemStatus, SemanticSummaryMailboxItemRecord, WorkplaneJobStatus,
};

use super::super::EnrichmentControlState;
use super::super::worker_count::EnrichmentWorkerBudgets;
use super::jobs::load_workplane_jobs_by_status;
use super::mailbox_persistence::{
    load_embedding_mailbox_items_by_status, load_summary_mailbox_items_by_status,
};
use super::sql::parse_u64;

#[derive(Debug, Clone, Copy, Default)]
struct WorkplaneBucketCounts {
    jobs: u64,
    work_items: u64,
}

pub(crate) fn project_workplane_status(
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

pub(crate) fn iter_workplane_job_config_roots(
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

pub(crate) fn last_failed_embedding_job_from_workplane(
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

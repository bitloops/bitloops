use anyhow::{Context, Result};
use rusqlite::params;

use crate::capability_packs::semantic_clones::types::{
    SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX, SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
};
use crate::capability_packs::semantic_clones::workplane::payload_work_item_count;
use crate::daemon::types::unix_timestamp_now;
use crate::host::runtime_store::{
    DaemonSqliteRuntimeStore, WorkplaneJobRecord, WorkplaneJobStatus,
};

use super::super::JobExecutionOutcome;
use super::sql::sql_i64;

pub(crate) const WORKPLANE_TRANSIENT_EMBEDDING_RETRY_LIMIT: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkplaneJobCompletionDisposition {
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

pub(crate) fn persist_workplane_job_completion(
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
pub(crate) fn format_workplane_job_completion_log(
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

pub(crate) fn transient_embedding_retry_backoff_secs(attempts: u32) -> u64 {
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

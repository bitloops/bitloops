use anyhow::{Context, Result, anyhow};
use rusqlite::params;

use crate::daemon::capability_events::queue::{prune_terminal_runs, sql_i64};
use crate::daemon::types::{CapabilityEventRunStatus, unix_timestamp_now};

use super::types::{CapabilityEventCoordinator, RunCompletion};

impl CapabilityEventCoordinator {
    pub(crate) fn apply_completion(&self, completion: RunCompletion) -> Result<()> {
        let completion_event = match &completion {
            RunCompletion::NoopCompleted { run }
            | RunCompletion::Completed { run, .. }
            | RunCompletion::RetryableFailure { run, .. }
            | RunCompletion::Failed { run, .. } => {
                run.init_session_id.clone().map(|init_session_id| {
                    crate::daemon::RuntimeEventRecord {
                        domain: "current_state_consumer".to_string(),
                        repo_id: run.repo_id.clone(),
                        init_session_id: Some(init_session_id),
                        updated_at_unix: unix_timestamp_now(),
                        task_id: None,
                        run_id: Some(run.run_id.clone()),
                        mailbox_name: Some(run.consumer_id.clone()),
                    }
                })
            }
        };
        let _guard = self
            .lock
            .lock()
            .map_err(|_| anyhow!("current-state consumer lock poisoned"))?;
        self.runtime_store.with_write_connection(|conn| {
            conn.execute_batch("BEGIN IMMEDIATE TRANSACTION;")
                .context("starting current-state consumer completion transaction")?;
            let result = (|| {
                let now = unix_timestamp_now();
                match completion {
                    RunCompletion::NoopCompleted { run } => {
                        conn.execute(
                            "UPDATE capability_workplane_cursor_mailboxes SET last_error = NULL, updated_at_unix = ?1 WHERE repo_id = ?2 AND capability_id = ?3 AND mailbox_name = ?4",
                            params![sql_i64(now)?, run.repo_id, run.capability_id, run.consumer_id],
                        )
                        .with_context(|| {
                            format!(
                                "clearing current-state consumer error for noop run `{}`",
                                run.run_id
                            )
                        })?;
                        conn.execute(
                            "UPDATE capability_workplane_cursor_runs SET from_generation_seq = ?1, to_generation_seq = ?2, reconcile_mode = ?3, status = ?4, updated_at_unix = ?5, completed_at_unix = ?6, error = NULL WHERE run_id = ?7",
                            params![
                                sql_i64(run.from_generation_seq)?,
                                sql_i64(run.to_generation_seq)?,
                                run.reconcile_mode,
                                CapabilityEventRunStatus::Completed.to_string(),
                                sql_i64(now)?,
                                sql_i64(now)?,
                                run.run_id,
                            ],
                        )
                        .with_context(|| {
                            format!(
                                "marking noop current-state consumer run `{}` complete",
                                run.run_id
                            )
                        })?;
                    }
                    RunCompletion::Completed {
                        run,
                        applied_to_generation_seq,
                    } => {
                        conn.execute(
                            "UPDATE capability_workplane_cursor_mailboxes SET last_applied_generation_seq = ?1, last_error = NULL, updated_at_unix = ?2 WHERE repo_id = ?3 AND capability_id = ?4 AND mailbox_name = ?5",
                            params![
                                sql_i64(applied_to_generation_seq)?,
                                sql_i64(now)?,
                                run.repo_id,
                                run.capability_id,
                                run.consumer_id,
                            ],
                        )
                        .with_context(|| {
                            format!(
                                "advancing current-state consumer cursor for run `{}`",
                                run.run_id
                            )
                        })?;
                        conn.execute(
                            "UPDATE capability_workplane_cursor_runs SET from_generation_seq = ?1, to_generation_seq = ?2, reconcile_mode = ?3, status = ?4, updated_at_unix = ?5, completed_at_unix = ?6, error = NULL WHERE run_id = ?7",
                            params![
                                sql_i64(run.from_generation_seq)?,
                                sql_i64(run.to_generation_seq)?,
                                run.reconcile_mode,
                                CapabilityEventRunStatus::Completed.to_string(),
                                sql_i64(now)?,
                                sql_i64(now)?,
                                run.run_id,
                            ],
                        )
                        .with_context(|| {
                            format!("marking current-state consumer run `{}` complete", run.run_id)
                        })?;
                    }
                    RunCompletion::RetryableFailure { run, error } => {
                        conn.execute(
                            "UPDATE capability_workplane_cursor_mailboxes SET last_error = ?1, updated_at_unix = ?2 WHERE repo_id = ?3 AND capability_id = ?4 AND mailbox_name = ?5",
                            params![error, sql_i64(now)?, run.repo_id, run.capability_id, run.consumer_id],
                        )
                        .with_context(|| {
                            format!(
                                "updating retryable current-state consumer error for run `{}`",
                                run.run_id
                            )
                        })?;
                        conn.execute(
                            "UPDATE capability_workplane_cursor_runs SET status = ?1, started_at_unix = NULL, updated_at_unix = ?2, completed_at_unix = NULL, error = ?3 WHERE run_id = ?4",
                            params![
                                CapabilityEventRunStatus::Queued.to_string(),
                                sql_i64(now)?,
                                error,
                                run.run_id,
                            ],
                        )
                        .with_context(|| {
                            format!(
                                "re-queueing current-state consumer run `{}` after retryable failure",
                                run.run_id
                            )
                        })?;
                    }
                    RunCompletion::Failed { run, error } => {
                        conn.execute(
                            "UPDATE capability_workplane_cursor_mailboxes SET last_error = ?1, updated_at_unix = ?2 WHERE repo_id = ?3 AND capability_id = ?4 AND mailbox_name = ?5",
                            params![error, sql_i64(now)?, run.repo_id, run.capability_id, run.consumer_id],
                        )
                        .with_context(|| {
                            format!("persisting terminal current-state consumer error for `{}`", run.run_id)
                        })?;
                        conn.execute(
                            "UPDATE capability_workplane_cursor_runs SET status = ?1, updated_at_unix = ?2, completed_at_unix = ?3, error = ?4 WHERE run_id = ?5",
                            params![
                                CapabilityEventRunStatus::Failed.to_string(),
                                sql_i64(now)?,
                                sql_i64(now)?,
                                error,
                                run.run_id,
                            ],
                        )
                        .with_context(|| {
                            format!("marking current-state consumer run `{}` failed", run.run_id)
                        })?;
                    }
                }
                prune_terminal_runs(conn)?;
                Ok(())
            })();

            match result {
                Ok(()) => {
                    conn.execute_batch("COMMIT;")
                        .context("committing current-state consumer completion transaction")?;
                    Ok(())
                }
                Err(err) => {
                    let _ = conn.execute_batch("ROLLBACK;");
                    Err(err)
                }
            }
        })?;
        if let Some(event) = completion_event {
            crate::daemon::shared_init_runtime_coordinator().publish_runtime_event(event);
        }
        Ok(())
    }
}

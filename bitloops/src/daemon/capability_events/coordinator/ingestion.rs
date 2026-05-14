use anyhow::{Context, Result, anyhow};
use rusqlite::params;

use crate::daemon::capability_events::queue::{
    ConsumerRunRequest, ensure_consumer_run, insert_artefact_changes, insert_file_changes,
    next_generation_seq, prune_terminal_runs, sql_i64, upsert_consumer_row,
};
use crate::daemon::types::unix_timestamp_now;
use crate::host::capability_host::{CapabilityMailboxInitPolicy, DevqlCapabilityHost};
use crate::host::devql::{DevqlConfig, SyncSummary};

use super::types::{CapabilityEventCoordinator, CapabilityEventEnqueueResult, SyncGenerationInput};

impl CapabilityEventCoordinator {
    pub(crate) fn record_sync_generation(
        &self,
        host: &DevqlCapabilityHost,
        cfg: &DevqlConfig,
        summary: &SyncSummary,
        input: SyncGenerationInput<'_>,
    ) -> Result<CapabilityEventEnqueueResult> {
        if !summary.success || summary.mode == "validate" {
            return Ok(CapabilityEventEnqueueResult { runs: Vec::new() });
        }

        let registrations = host
            .workplane_mailboxes()
            .iter()
            .copied()
            .filter(|registration| {
                registration.policy == crate::host::capability_host::CapabilityMailboxPolicy::Cursor
            })
            .collect::<Vec<_>>();
        let now = unix_timestamp_now();
        let repo_id = cfg.repo.repo_id.clone();
        let repo_root = cfg.repo_root.clone();
        let source_task_id = input.source_task_id.map(str::to_string);
        let requires_full_reconcile = matches!(summary.mode.as_str(), "full" | "repair");

        let _guard = self
            .lock
            .lock()
            .map_err(|_| anyhow!("current-state consumer lock poisoned"))?;
        let runs = self.runtime_store.with_write_connection(|conn| {
            conn.execute_batch("BEGIN IMMEDIATE TRANSACTION;")
                .context("starting current-state generation transaction")?;
            let result = (|| {
                let generation_seq = next_generation_seq(conn, &repo_id)?;
                conn.execute(
                    "INSERT INTO capability_workplane_cursor_generations (repo_id, generation_seq, source_task_id, sync_mode, active_branch, head_commit_sha, requires_full_reconcile, created_at_unix) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        &repo_id,
                        sql_i64(generation_seq)?,
                        source_task_id.as_deref(),
                        &summary.mode,
                        summary.active_branch.as_deref(),
                        summary.head_commit_sha.as_deref(),
                        if requires_full_reconcile { 1_i64 } else { 0_i64 },
                        sql_i64(now)?,
                    ],
                )
                .context("inserting current-state generation")?;
                insert_file_changes(conn, &repo_id, generation_seq, &input.file_diff)?;
                insert_artefact_changes(conn, &repo_id, generation_seq, &input.artefact_diff)?;

                let mut scheduled_runs = Vec::new();
                for registration in &registrations {
                    let crate::host::capability_host::CapabilityMailboxHandler::CurrentStateConsumer(
                        handler_id,
                    ) = registration.handler
                    else {
                        continue;
                    };
                    upsert_consumer_row(
                        conn,
                        &repo_id,
                        registration.capability_id,
                        registration.mailbox_name,
                        now,
                    )?;
                    let run_init_session_id = if registration.init_policy
                        == CapabilityMailboxInitPolicy::BlocksInitCompletion
                    {
                        input.init_session_id
                    } else {
                        None
                    };
                    if let Some(run) = ensure_consumer_run(
                        conn,
                        ConsumerRunRequest {
                            repo_id: &repo_id,
                            repo_root: &repo_root,
                            capability_id: registration.capability_id,
                            mailbox_name: registration.mailbox_name,
                            handler_id,
                            init_session_id: run_init_session_id,
                            now,
                        },
                    )? {
                        scheduled_runs.push(run.record);
                    }
                }

                prune_terminal_runs(conn)?;
                Ok(scheduled_runs)
            })();

            match result {
                Ok(runs) => {
                    conn.execute_batch("COMMIT;")
                        .context("committing current-state generation transaction")?;
                    Ok(runs)
                }
                Err(err) => {
                    let _ = conn.execute_batch("ROLLBACK;");
                    Err(err)
                }
            }
        })?;

        if !runs.is_empty() {
            self.notify.notify_waiters();
        }

        for run in &runs {
            if let Some(init_session_id) = run.init_session_id.clone() {
                crate::daemon::shared_init_runtime_coordinator().publish_runtime_event(
                    crate::daemon::RuntimeEventRecord {
                        domain: "current_state_consumer".to_string(),
                        repo_id: run.repo_id.clone(),
                        init_session_id: Some(init_session_id),
                        updated_at_unix: run.updated_at_unix,
                        task_id: None,
                        run_id: Some(run.run_id.clone()),
                        mailbox_name: Some(run.consumer_id.clone()),
                    },
                );
            }
        }

        Ok(CapabilityEventEnqueueResult { runs })
    }
}

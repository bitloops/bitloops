use anyhow::Result;
use std::time::Instant;
use tokio::time::{Duration, sleep};

use crate::daemon::types::unix_timestamp_now;
use crate::host::relational_store::DefaultRelationalStore;
use crate::host::runtime_store::WorkplaneJobRecord;

use super::coordinator::EnrichmentCoordinator;
use super::execution;
use super::runtime_events::publish_job_runtime_event;
use super::semantic_writer::{commit_embedding_batch, commit_summary_batch};
use super::worker_count::EnrichmentWorkerPool;
use super::workplane::{
    WorkplaneJobCompletionDisposition, claim_embedding_mailbox_batch, claim_next_workplane_job,
    claim_summary_mailbox_batch, fail_summary_mailbox_batch,
    persist_embedding_mailbox_batch_failure, persist_workplane_job_completion,
    requeue_embedding_mailbox_batch, requeue_summary_mailbox_batch,
};

impl EnrichmentCoordinator {
    pub(crate) async fn run_loop(self: std::sync::Arc<Self>, pool: EnrichmentWorkerPool) {
        loop {
            match self.process_next_job(pool).await {
                Ok(true) => continue,
                Ok(false) => {}
                Err(err) => {
                    log::warn!(
                        "daemon enrichment worker error for pool {}: {err:#}",
                        pool.as_str()
                    );
                }
            }
            tokio::select! {
                _ = self.notify.notified() => {},
                _ = sleep(Duration::from_secs(2)) => {},
            }
        }
    }

    async fn process_next_job(&self, pool: EnrichmentWorkerPool) -> Result<bool> {
        match pool {
            EnrichmentWorkerPool::SummaryRefresh => self.process_next_summary_batch().await,
            EnrichmentWorkerPool::Embeddings => self.process_next_embedding_batch().await,
            EnrichmentWorkerPool::CloneRebuild => self.process_next_clone_rebuild_job().await,
        }
    }

    async fn process_next_summary_batch(&self) -> Result<bool> {
        let batch = {
            let _guard = self.lock.lock().await;
            let mut state = self.load_state()?;
            let Some(batch) =
                claim_summary_mailbox_batch(&self.workplane_store, &self.runtime_store, &state)?
            else {
                let Some(job) = claim_next_workplane_job(
                    &self.workplane_store,
                    &self.runtime_store,
                    &state,
                    EnrichmentWorkerPool::SummaryRefresh,
                )?
                else {
                    return Ok(false);
                };
                state.last_action = Some(format!("running:{}", job.mailbox_name));
                self.save_state(&mut state)?;
                drop(state);
                drop(_guard);
                return self.execute_claimed_workplane_job(job).await;
            };
            state.last_action = Some("running:summary_refresh".to_string());
            self.save_state(&mut state)?;
            batch
        };
        let init_runtime = crate::daemon::shared_init_runtime_coordinator();

        let queue_wait_ms = batch
            .items
            .iter()
            .map(|item| unix_timestamp_now().saturating_sub(item.submitted_at_unix) * 1_000)
            .max()
            .unwrap_or(0);
        let inference_started = Instant::now();
        let prepared = match execution::prepare_summary_mailbox_batch(
            &batch,
            |artefact_id, init_session_ids| {
                init_runtime.record_summary_in_memory_artefact(
                    &batch.repo_id,
                    &batch.lease_token,
                    artefact_id,
                    init_session_ids,
                );
            },
        )
        .await
        {
            Ok(prepared) => prepared,
            Err(err) => {
                init_runtime.clear_summary_in_memory_batch(&batch.lease_token);
                fail_summary_mailbox_batch(&self.workplane_store, &batch, &format!("{err:#}"))?;
                let _guard = self.lock.lock().await;
                let mut state = self.load_state()?;
                state.last_action = Some("failed".to_string());
                self.save_state(&mut state)?;
                log::warn!(
                    "semantic mailbox batch failed: pipeline=summary_refresh repo_id={} leased_count={} outcome=failed error={err:#}",
                    batch.repo_id,
                    batch.items.len(),
                );
                return Ok(true);
            }
        };
        let inference_ms = inference_started.elapsed().as_millis() as u64;
        let flush_started = Instant::now();
        let relational_store =
            DefaultRelationalStore::open_local_for_roots(&batch.config_root, &batch.repo_root)?;
        let expanded_count = prepared.expanded_count;
        let attempts = prepared.attempts;
        let flush_result = commit_summary_batch(
            self.workplane_store.db_path(),
            relational_store.sqlite_path(),
            prepared.commit,
        )
        .await;
        let flush_ms = flush_started.elapsed().as_millis() as u64;
        init_runtime.clear_summary_in_memory_batch(&batch.lease_token);
        let flush_succeeded = flush_result.is_ok();

        {
            let _guard = self.lock.lock().await;
            let mut state = self.load_state()?;
            state.last_action = Some(if flush_succeeded {
                "completed".to_string()
            } else {
                "retry_scheduled".to_string()
            });
            self.save_state(&mut state)?;
        }

        match flush_result {
            Err(err) => {
                let err_text = format!("{err:#}");
                let timings = err.timings();
                requeue_summary_mailbox_batch(&self.workplane_store, &batch, 5, &err_text)?;
                log::warn!(
                    "semantic mailbox batch completed: pipeline=summary_refresh repo_id={} leased_count={} expanded_count={} queue_wait_ms={} inference_ms={} flush_ms={} total_ms={} attempts={} outcome=retry_scheduled retry_in_ms=5000 failure_substage={} runtime_store_writes_succeeded_in_tx={} transaction_start_ms={} summary_sql_ms={} runtime_embedding_mailbox_upsert_ms={} replacement_summary_backfill_insert_ms={} summary_mailbox_delete_ms={} transaction_commit_ms={}",
                    batch.repo_id,
                    batch.items.len(),
                    expanded_count,
                    queue_wait_ms,
                    inference_ms,
                    flush_ms,
                    inference_ms.saturating_add(flush_ms),
                    attempts,
                    err.phase().as_str(),
                    err.runtime_store_writes_succeeded_in_tx(),
                    timings.transaction_start_ms,
                    timings.summary_sql_ms,
                    timings.runtime_embedding_mailbox_upsert_ms,
                    timings.replacement_summary_backfill_insert_ms,
                    timings.summary_mailbox_delete_ms,
                    timings.transaction_commit_ms,
                );
                return Ok(true);
            }
            Ok(report) => {
                log::info!(
                    "semantic mailbox batch completed: pipeline=summary_refresh repo_id={} leased_count={} expanded_count={} queue_wait_ms={} inference_ms={} flush_ms={} total_ms={} attempts={} outcome=completed transaction_start_ms={} summary_sql_ms={} runtime_embedding_mailbox_upsert_ms={} replacement_summary_backfill_insert_ms={} summary_mailbox_delete_ms={} transaction_commit_ms={}",
                    batch.repo_id,
                    batch.items.len(),
                    expanded_count,
                    queue_wait_ms,
                    inference_ms,
                    flush_ms,
                    inference_ms.saturating_add(flush_ms),
                    attempts,
                    report.timings.transaction_start_ms,
                    report.timings.summary_sql_ms,
                    report.timings.runtime_embedding_mailbox_upsert_ms,
                    report.timings.replacement_summary_backfill_insert_ms,
                    report.timings.summary_mailbox_delete_ms,
                    report.timings.transaction_commit_ms,
                );
            }
        }

        Ok(true)
    }

    async fn process_next_embedding_batch(&self) -> Result<bool> {
        let batch = {
            let _guard = self.lock.lock().await;
            let mut state = self.load_state()?;
            let Some(batch) =
                claim_embedding_mailbox_batch(&self.workplane_store, &self.runtime_store, &state)?
            else {
                return Ok(false);
            };
            state.last_action = Some("running:embeddings".to_string());
            self.save_state(&mut state)?;
            batch
        };

        let queue_wait_ms = batch
            .items
            .iter()
            .map(|item| unix_timestamp_now().saturating_sub(item.submitted_at_unix) * 1_000)
            .max()
            .unwrap_or(0);
        let prepare_started = Instant::now();
        let prepared = match execution::prepare_embedding_mailbox_batch(&batch).await {
            Ok(prepared) => prepared,
            Err(err) => {
                let disposition = persist_embedding_mailbox_batch_failure(
                    &self.workplane_store,
                    &batch,
                    &format!("{err:#}"),
                )?;
                let _guard = self.lock.lock().await;
                let mut state = self.load_state()?;
                state.last_action = Some(match disposition {
                    WorkplaneJobCompletionDisposition::RetryScheduled { .. } => {
                        "retry_scheduled".to_string()
                    }
                    _ => "failed".to_string(),
                });
                self.save_state(&mut state)?;
                return Ok(true);
            }
        };
        let prepare_ms = prepare_started.elapsed().as_millis() as u64;
        let prepare_timings = prepared.timings;
        let flush_started = Instant::now();
        let relational_store =
            DefaultRelationalStore::open_local_for_roots(&batch.config_root, &batch.repo_root)?;
        let expanded_count = prepared.expanded_count;
        let attempts = prepared.attempts;
        let flush_result = commit_embedding_batch(
            self.workplane_store.db_path(),
            relational_store.sqlite_path(),
            prepared.commit,
        )
        .await;
        let flush_ms = flush_started.elapsed().as_millis() as u64;

        {
            let _guard = self.lock.lock().await;
            let mut state = self.load_state()?;
            state.last_action = Some(if flush_result.is_ok() {
                "completed".to_string()
            } else {
                "retry_scheduled".to_string()
            });
            self.save_state(&mut state)?;
        }

        if let Err(err) = flush_result {
            requeue_embedding_mailbox_batch(&self.workplane_store, &batch, 5, &format!("{err:#}"))?;
            log::warn!(
                "semantic mailbox batch completed: pipeline=embedding repo_id={} representation_kind={} leased_count={} expanded_count={} queue_wait_ms={} prepare_ms={} prepare_config_ms={} prepare_input_ms={} prepare_summary_ms={} prepare_freshness_ms={} prepare_embedding_ms={} prepare_sql_ms={} prepare_setup_ms={} prepare_total_ms={} flush_ms={} total_ms={} attempts={} outcome=retry_scheduled retry_in_ms=5000",
                batch.repo_id,
                batch.representation_kind,
                batch.items.len(),
                expanded_count,
                queue_wait_ms,
                prepare_ms,
                prepare_timings.config_ms,
                prepare_timings.input_ms,
                prepare_timings.summary_ms,
                prepare_timings.freshness_ms,
                prepare_timings.embedding_ms,
                prepare_timings.sql_ms,
                prepare_timings.setup_ms,
                prepare_timings.total_ms,
                flush_ms,
                prepare_ms.saturating_add(flush_ms),
                attempts,
            );
            return Ok(true);
        }

        log::info!(
            "semantic mailbox batch completed: pipeline=embedding repo_id={} representation_kind={} leased_count={} expanded_count={} queue_wait_ms={} prepare_ms={} prepare_config_ms={} prepare_input_ms={} prepare_summary_ms={} prepare_freshness_ms={} prepare_embedding_ms={} prepare_sql_ms={} prepare_setup_ms={} prepare_total_ms={} flush_ms={} total_ms={} attempts={} outcome=completed",
            batch.repo_id,
            batch.representation_kind,
            batch.items.len(),
            expanded_count,
            queue_wait_ms,
            prepare_ms,
            prepare_timings.config_ms,
            prepare_timings.input_ms,
            prepare_timings.summary_ms,
            prepare_timings.freshness_ms,
            prepare_timings.embedding_ms,
            prepare_timings.sql_ms,
            prepare_timings.setup_ms,
            prepare_timings.total_ms,
            flush_ms,
            prepare_ms.saturating_add(flush_ms),
            attempts,
        );

        Ok(true)
    }

    async fn process_next_clone_rebuild_job(&self) -> Result<bool> {
        let job = {
            let _guard = self.lock.lock().await;
            let mut state = self.load_state()?;
            let Some(job) = claim_next_workplane_job(
                &self.workplane_store,
                &self.runtime_store,
                &state,
                EnrichmentWorkerPool::CloneRebuild,
            )?
            else {
                return Ok(false);
            };
            state.last_action = Some("running:clone_rebuild".to_string());
            self.save_state(&mut state)?;
            job
        };

        self.execute_claimed_workplane_job(job).await
    }

    async fn execute_claimed_workplane_job(&self, job: WorkplaneJobRecord) -> Result<bool> {
        publish_job_runtime_event(&job);
        let outcome = execution::execute_workplane_job(&job).await;
        {
            let _guard = self.lock.lock().await;
            let mut state = self.load_state()?;
            let disposition =
                persist_workplane_job_completion(&self.workplane_store, &job, &outcome)?;
            state.last_action = Some(match disposition {
                WorkplaneJobCompletionDisposition::Completed => "completed".to_string(),
                WorkplaneJobCompletionDisposition::Failed => "failed".to_string(),
                WorkplaneJobCompletionDisposition::RetryScheduled { .. } => {
                    "retry_scheduled".to_string()
                }
            });
            self.save_state(&mut state)?;
        }
        publish_job_runtime_event(&job);

        for follow_up in outcome.follow_ups {
            self.enqueue_follow_up(follow_up).await?;
        }

        Ok(true)
    }
}

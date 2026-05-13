use std::collections::BTreeMap;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use anyhow::{Context, Result, anyhow};
use futures_util::FutureExt;
use rusqlite::params;
use tokio::time::sleep;

use crate::daemon::capability_events::plan::{
    build_execution_plan, find_current_state_consumer, validate_consumer_result,
};
use crate::daemon::capability_events::queue::{StoredRunRecord, load_runs, sql_i64};
use crate::daemon::memory::{PageReleaseResult, ProcessMemorySnapshot};
use crate::daemon::types::{
    CapabilityEventRunRecord, CapabilityEventRunStatus, unix_timestamp_now,
};
use crate::host::capability_host::{DevqlCapabilityHost, ReconcileMode};
use crate::host::devql::resolve_repo_identity;

use super::types::{
    CapabilityEventCoordinator, IDLE_RECLAIM_MAX_ATTEMPTS, IDLE_RECLAIM_RETRY_INTERVAL,
    MAX_RUN_ATTEMPTS, RunCompletion, WORKER_POLL_INTERVAL, WorkerStartedGuard,
};

impl CapabilityEventCoordinator {
    pub(super) async fn maybe_reclaim_after_repo_idle(
        &self,
        completion: &RunCompletion,
    ) -> Result<()> {
        if !should_attempt_idle_reclaim(completion) {
            return Ok(());
        }
        let RunCompletion::Completed { run, .. } = completion else {
            return Ok(());
        };
        if self
            .snapshot(Some(&run.repo_id))?
            .current_repo_run
            .is_some()
        {
            return Ok(());
        }

        let before = self.memory_maintenance.capture_process_memory();
        let mut after = before.clone();
        let mut release_result = PageReleaseResult {
            strategy: "unsupported",
            released: false,
        };
        let mut attempt_count = 0;

        while attempt_count < IDLE_RECLAIM_MAX_ATTEMPTS {
            attempt_count += 1;
            release_result = self.memory_maintenance.release_unused_pages();
            sleep(IDLE_RECLAIM_RETRY_INTERVAL).await;
            after = self.memory_maintenance.capture_process_memory();

            if memory_drop_detected(before.as_ref(), after.as_ref()) {
                break;
            }

            if before.is_none()
                || self
                    .snapshot(Some(&run.repo_id))?
                    .current_repo_run
                    .is_some()
            {
                break;
            }
        }
        let fields = build_idle_reclaim_log_fields(before, after, release_result, attempt_count);

        log::info!(
            "current-state consumer memory reclaim: repo_id={} capability_id={} consumer_id={} reclaim_trigger=post_full_reconcile_idle strategy={} attempt_count={} resident_before_bytes={} resident_after_bytes={} phys_before_bytes={} phys_after_bytes={} released={}",
            run.repo_id,
            run.capability_id,
            run.consumer_id,
            fields.strategy,
            fields.attempt_count,
            optional_u64_log_value(fields.resident_before_bytes),
            optional_u64_log_value(fields.resident_after_bytes),
            optional_u64_log_value(fields.phys_before_bytes),
            optional_u64_log_value(fields.phys_after_bytes),
            fields.released,
        );
        Ok(())
    }

    pub(crate) fn activate_worker(self: &Arc<Self>) {
        if self.worker_started.swap(true, Ordering::SeqCst) {
            return;
        }
        if let Err(err) = self.recover_running_runs() {
            log::warn!("failed to recover current-state consumer runs: {err:#}");
        }
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            self.worker_started.store(false, Ordering::SeqCst);
            log::error!(
                "current-state consumer worker activation requested without an active tokio runtime"
            );
            return;
        };
        let coordinator = Arc::clone(self);
        handle.spawn(async move {
            let _guard = WorkerStartedGuard {
                coordinator: Arc::clone(&coordinator),
            };
            coordinator.run_loop().await;
        });
    }

    async fn run_loop(self: Arc<Self>) {
        loop {
            if let Err(err) = self.launch_runnable_runs() {
                log::warn!("current-state consumer scheduling failed: {err:#}");
            }
            tokio::select! {
                _ = self.notify.notified() => {}
                _ = sleep(WORKER_POLL_INTERVAL) => {}
            }
        }
    }

    fn launch_runnable_runs(self: &Arc<Self>) -> Result<()> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| anyhow!("current-state consumer lock poisoned"))?;
        let runs = self.runtime_store.with_connection(|conn| {
            conn.execute_batch("BEGIN IMMEDIATE TRANSACTION;")
                .context("starting current-state consumer claim transaction")?;
            let result = (|| {
                let candidates = load_claimable_runs(conn)?;
                let mut claimed = Vec::new();
                let now = unix_timestamp_now();
                for run in candidates {
                    conn.execute(
                        "UPDATE capability_workplane_cursor_runs SET status = ?1, attempts = ?2, started_at_unix = ?3, updated_at_unix = ?4 WHERE run_id = ?5",
                        params![
                            CapabilityEventRunStatus::Running.to_string(),
                            run.record.attempts + 1,
                            sql_i64(now)?,
                            sql_i64(now)?,
                            run.record.run_id,
                        ],
                    )
                    .with_context(|| {
                        format!("marking current-state consumer run `{}` as running", run.record.run_id)
                    })?;
                    let mut claimed_run = run.record.clone();
                    claimed_run.status = CapabilityEventRunStatus::Running;
                    claimed_run.attempts += 1;
                    claimed_run.started_at_unix = Some(now);
                    claimed_run.updated_at_unix = now;
                    claimed.push(StoredRunRecord {
                        record: claimed_run,
                        repo_root: run.repo_root.clone(),
                    });
                }
                Ok(claimed)
            })();

            match result {
                Ok(runs) => {
                    conn.execute_batch("COMMIT;")
                        .context("committing current-state consumer claim transaction")?;
                    Ok(runs)
                }
                Err(err) => {
                    let _ = conn.execute_batch("ROLLBACK;");
                    Err(err)
                }
            }
        })?;

        for run in runs {
            if let Some(init_session_id) = run.record.init_session_id.clone() {
                crate::daemon::shared_init_runtime_coordinator().publish_runtime_event(
                    crate::daemon::RuntimeEventRecord {
                        domain: "current_state_consumer".to_string(),
                        repo_id: run.record.repo_id.clone(),
                        init_session_id: Some(init_session_id),
                        updated_at_unix: run.record.updated_at_unix,
                        task_id: None,
                        run_id: Some(run.record.run_id.clone()),
                        mailbox_name: Some(run.record.consumer_id.clone()),
                    },
                );
            }
            self.spawn_execution_task(run);
        }
        Ok(())
    }

    fn spawn_execution_task(self: &Arc<Self>, run: StoredRunRecord) {
        let coordinator = Arc::clone(self);
        tokio::spawn(async move {
            let completion = coordinator.execute_run(run).await;
            if let Err(err) = coordinator.apply_completion(completion.clone()) {
                log::warn!("failed to persist current-state consumer completion: {err:#}");
            } else if let Err(err) = coordinator.maybe_reclaim_after_repo_idle(&completion).await {
                log::warn!("failed to reclaim idle current-state consumer memory: {err:#}");
            }
            coordinator.notify.notify_waiters();
        });
    }

    async fn execute_run(&self, run: StoredRunRecord) -> RunCompletion {
        let plan = match self
            .runtime_store
            .with_connection(|conn| build_execution_plan(conn, &run.record, &run.repo_root))
        {
            Ok(Some(plan)) => plan,
            Ok(None) => return RunCompletion::NoopCompleted { run: run.record },
            Err(err) => {
                return terminal_or_retry(run.record, err);
            }
        };

        let repo = match resolve_repo_identity(&plan.repo_root) {
            Ok(repo) => repo,
            Err(err) => {
                return terminal_or_retry(plan.record, err.context("resolving repo identity"));
            }
        };
        let host = match DevqlCapabilityHost::builtin(plan.repo_root.clone(), repo) {
            Ok(host) => host,
            Err(err) => {
                return terminal_or_retry(plan.record, err.context("building capability host"));
            }
        };
        let Some(consumer) = find_current_state_consumer(&host, &plan.record) else {
            let capability_id = plan.record.capability_id.clone();
            let consumer_id = plan.record.consumer_id.clone();
            return terminal_or_retry(
                plan.record,
                anyhow!(
                    "current-state consumer `{}` for capability `{}` is not registered",
                    consumer_id,
                    capability_id
                ),
            );
        };
        let context = match host.build_current_state_consumer_context_with_session(
            &plan.record.capability_id,
            plan.record.init_session_id.clone(),
        ) {
            Ok(context) => context,
            Err(err) => {
                return terminal_or_retry(
                    plan.record,
                    err.context("building current-state consumer context"),
                );
            }
        };

        let outcome = AssertUnwindSafe(consumer.reconcile(&plan.request, &context))
            .catch_unwind()
            .await;
        match outcome {
            Ok(Ok(result)) => match validate_consumer_result(&plan.request, &result) {
                Ok(()) => {
                    log::info!(
                        "current-state consumer completed: repo_id={} capability_id={} consumer_id={} reconcile_mode={} from_generation_seq={} to_generation_seq={} metrics={}",
                        plan.record.repo_id,
                        plan.record.capability_id,
                        plan.record.consumer_id,
                        reconcile_mode_for_log(plan.request.reconcile_mode),
                        plan.request.from_generation_seq_exclusive,
                        result.applied_to_generation_seq,
                        result
                            .metrics
                            .as_ref()
                            .map(serde_json::Value::to_string)
                            .unwrap_or_else(|| "{}".to_string()),
                    );
                    RunCompletion::Completed {
                        run: plan.record,
                        applied_to_generation_seq: result.applied_to_generation_seq,
                    }
                }
                Err(err) => terminal_or_retry(plan.record, err),
            },
            Ok(Err(err)) => terminal_or_retry(plan.record, err),
            Err(_) => terminal_or_retry(plan.record, anyhow!("current-state consumer panicked")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IdleReclaimLogFields {
    pub(crate) resident_before_bytes: Option<u64>,
    pub(crate) resident_after_bytes: Option<u64>,
    pub(crate) phys_before_bytes: Option<u64>,
    pub(crate) phys_after_bytes: Option<u64>,
    pub(crate) strategy: &'static str,
    pub(crate) attempt_count: u8,
    pub(crate) released: bool,
}

pub(crate) fn should_attempt_idle_reclaim(completion: &RunCompletion) -> bool {
    matches!(
        completion,
        RunCompletion::Completed { run, .. } if run.reconcile_mode == "full_reconcile"
    )
}

pub(crate) fn build_idle_reclaim_log_fields(
    before: Option<ProcessMemorySnapshot>,
    after: Option<ProcessMemorySnapshot>,
    release_result: PageReleaseResult,
    attempt_count: u8,
) -> IdleReclaimLogFields {
    IdleReclaimLogFields {
        resident_before_bytes: before.as_ref().and_then(|snapshot| snapshot.resident_bytes),
        resident_after_bytes: after.as_ref().and_then(|snapshot| snapshot.resident_bytes),
        phys_before_bytes: before.and_then(|snapshot| snapshot.phys_footprint_bytes),
        phys_after_bytes: after.and_then(|snapshot| snapshot.phys_footprint_bytes),
        strategy: release_result.strategy,
        attempt_count,
        released: release_result.released,
    }
}

fn memory_drop_detected(
    before: Option<&ProcessMemorySnapshot>,
    after: Option<&ProcessMemorySnapshot>,
) -> bool {
    let Some(before) = before else {
        return true;
    };
    let Some(after) = after else {
        return false;
    };

    dropped_metric(before.resident_bytes, after.resident_bytes)
        || dropped_metric(before.phys_footprint_bytes, after.phys_footprint_bytes)
}

fn dropped_metric(before: Option<u64>, after: Option<u64>) -> bool {
    matches!((before, after), (Some(before), Some(after)) if after < before)
}

fn optional_u64_log_value(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_string())
}

fn reconcile_mode_for_log(mode: ReconcileMode) -> &'static str {
    match mode {
        ReconcileMode::MergedDelta => "merged_delta",
        ReconcileMode::FullReconcile => "full_reconcile",
    }
}

fn load_claimable_runs(conn: &rusqlite::Connection) -> Result<Vec<StoredRunRecord>> {
    let now = unix_timestamp_now();
    let candidates = load_runs(
        conn,
        "SELECT run_id, repo_id, repo_root, mailbox_name, capability_id, init_session_id, from_generation_seq, to_generation_seq, reconcile_mode, status, attempts, submitted_at_unix, started_at_unix, updated_at_unix, completed_at_unix, error FROM capability_workplane_cursor_runs WHERE status = ?1 ORDER BY submitted_at_unix ASC",
        params![CapabilityEventRunStatus::Queued.to_string()],
    )?;
    let mut running_lanes = BTreeMap::<String, ()>::new();
    for run in load_runs(
        conn,
        "SELECT run_id, repo_id, repo_root, mailbox_name, capability_id, init_session_id, from_generation_seq, to_generation_seq, reconcile_mode, status, attempts, submitted_at_unix, started_at_unix, updated_at_unix, completed_at_unix, error FROM capability_workplane_cursor_runs WHERE status = ?1",
        params![CapabilityEventRunStatus::Running.to_string()],
    )? {
        running_lanes.insert(run.record.lane_key.clone(), ());
    }

    let mut claimable = Vec::new();
    for run in candidates {
        if running_lanes.contains_key(&run.record.lane_key) {
            continue;
        }
        if run.record.updated_at_unix + retry_backoff_seconds(run.record.attempts) > now {
            continue;
        }
        running_lanes.insert(run.record.lane_key.clone(), ());
        claimable.push(run);
    }
    Ok(claimable)
}

pub(crate) fn terminal_or_retry(
    run: CapabilityEventRunRecord,
    err: impl Into<anyhow::Error>,
) -> RunCompletion {
    let error = format!("{:#}", err.into());
    if run.attempts >= MAX_RUN_ATTEMPTS {
        log::error!(
            "current-state consumer run failed: run_id={} repo_id={} capability_id={} consumer_id={} attempts={} error={}",
            run.run_id,
            run.repo_id,
            run.capability_id,
            run.consumer_id,
            run.attempts,
            error
        );
        RunCompletion::Failed { run, error }
    } else {
        log::warn!(
            "current-state consumer run failed and will retry: run_id={} repo_id={} capability_id={} consumer_id={} attempts={} error={}",
            run.run_id,
            run.repo_id,
            run.capability_id,
            run.consumer_id,
            run.attempts,
            error
        );
        RunCompletion::RetryableFailure { run, error }
    }
}

fn retry_backoff_seconds(attempts: u32) -> u64 {
    match attempts {
        0 | 1 => 0,
        2 => 5,
        3 => 15,
        4 => 30,
        _ => 60,
    }
}

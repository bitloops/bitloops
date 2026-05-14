use std::collections::BTreeMap;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use anyhow::{Context, Result, anyhow};
use futures_util::FutureExt;
use rusqlite::params;
use tokio::time::sleep;

use crate::daemon::capability_events::plan::{
    ExecutionPlan, build_execution_plan, find_current_state_consumer, validate_consumer_result,
};
use crate::daemon::capability_events::queue::{StoredRunRecord, load_runs, sql_i64};
use crate::daemon::types::{
    CapabilityEventRunRecord, CapabilityEventRunStatus, unix_timestamp_now,
};
use crate::daemon::{
    CurrentStateExecutionRoute, CurrentStateWorkerInvocation, current_state_execution_route,
    terminate_current_state_worker_process,
};
use crate::host::capability_host::{DevqlCapabilityHost, ReconcileMode};
use crate::host::devql::resolve_repo_identity;

use super::types::{
    CapabilityEventCoordinator, MAX_RUN_ATTEMPTS, RunCompletion, WORKER_POLL_INTERVAL,
    WorkerStartedGuard,
};

impl CapabilityEventCoordinator {
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
            let completion = Arc::clone(&coordinator).execute_run(run).await;
            if let Err(err) = coordinator.apply_completion(completion) {
                log::warn!("failed to persist current-state consumer completion: {err:#}");
            }
            coordinator.notify.notify_waiters();
        });
    }

    pub(super) async fn execute_run(self: Arc<Self>, run: StoredRunRecord) -> RunCompletion {
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

        let route = current_state_execution_route(
            &plan.record.capability_id,
            &plan.record.consumer_id,
            plan.request.reconcile_mode,
        );
        if matches!(route, CurrentStateExecutionRoute::Subprocess { .. }) {
            return self.execute_run_in_worker(plan, route).await;
        }

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
            Ok(Ok(result)) => completed_run_completion(plan, result),
            Ok(Err(err)) => terminal_or_retry(plan.record, err),
            Err(_) => terminal_or_retry(plan.record, anyhow!("current-state consumer panicked")),
        }
    }

    async fn execute_run_in_worker(
        self: Arc<Self>,
        plan: ExecutionPlan,
        route: CurrentStateExecutionRoute,
    ) -> RunCompletion {
        let CurrentStateExecutionRoute::Subprocess { reason } = route else {
            return terminal_or_retry(plan.record, anyhow!("worker route must be subprocess"));
        };

        let config_path =
            match crate::config::resolve_preferred_daemon_config_path_for_repo(&plan.repo_root) {
                Ok(config_path) => config_path,
                Err(err) => {
                    return terminal_or_retry(
                        plan.record,
                        err.context("resolving daemon config path for current-state worker"),
                    );
                }
            };

        let invocation = CurrentStateWorkerInvocation {
            config_path,
            capability_id: plan.record.capability_id.clone(),
            consumer_id: plan.record.consumer_id.clone(),
            init_session_id: plan.record.init_session_id.clone(),
            parent_pid: Some(std::process::id()),
            request: plan.request.clone(),
        };

        let handle = match self.current_state_worker_runner.spawn(invocation) {
            Ok(handle) => handle,
            Err(err) => {
                return terminal_or_retry(
                    plan.record,
                    err.context("spawning current-state worker subprocess"),
                );
            }
        };
        let pid = handle.pid();
        let mut worker_guard =
            ActiveWorkerRunGuard::register(Arc::clone(&self), plan.record.run_id.clone(), pid);
        log::info!(
            "current-state worker spawned: run_id={} repo_id={} capability_id={} consumer_id={} reconcile_mode={} from_generation_seq={} to_generation_seq={} pid={} init_session_id={} route_reason={}",
            plan.record.run_id,
            plan.record.repo_id,
            plan.record.capability_id,
            plan.record.consumer_id,
            reconcile_mode_for_log(plan.request.reconcile_mode),
            plan.request.from_generation_seq_exclusive,
            plan.request.to_generation_seq_inclusive,
            pid,
            plan.record.init_session_id.as_deref().unwrap_or("<none>"),
            reason,
        );

        let result = handle.wait().await;
        worker_guard.mark_child_exited();

        match result {
            Ok(result) => {
                log::info!(
                    "current-state worker exited successfully: run_id={} repo_id={} capability_id={} consumer_id={} reconcile_mode={} from_generation_seq={} to_generation_seq={} pid={} applied_to_generation_seq={} route_reason={}",
                    plan.record.run_id,
                    plan.record.repo_id,
                    plan.record.capability_id,
                    plan.record.consumer_id,
                    reconcile_mode_for_log(plan.request.reconcile_mode),
                    plan.request.from_generation_seq_exclusive,
                    plan.request.to_generation_seq_inclusive,
                    pid,
                    result.applied_to_generation_seq,
                    reason,
                );
                completed_run_completion(plan, result)
            }
            Err(err) => {
                log::warn!(
                    "current-state worker exited with failure: run_id={} repo_id={} capability_id={} consumer_id={} reconcile_mode={} from_generation_seq={} to_generation_seq={} pid={} route_reason={} error={:#}",
                    plan.record.run_id,
                    plan.record.repo_id,
                    plan.record.capability_id,
                    plan.record.consumer_id,
                    reconcile_mode_for_log(plan.request.reconcile_mode),
                    plan.request.from_generation_seq_exclusive,
                    plan.request.to_generation_seq_inclusive,
                    pid,
                    reason,
                    err,
                );
                terminal_or_retry(
                    plan.record,
                    err.context("waiting for current-state worker subprocess"),
                )
            }
        }
    }

    pub(crate) fn terminate_active_worker_children(&self) -> Result<()> {
        let active = {
            let mut active = self
                .active_worker_children
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            std::mem::take(&mut *active)
        };
        for (run_id, child) in active {
            log::info!(
                "current-state worker termination requested: run_id={} pid={}",
                run_id,
                child.pid,
            );
            terminate_current_state_worker_process(child.pid).with_context(|| {
                format!(
                    "terminating tracked current-state worker for run `{}` (pid {})",
                    run_id, child.pid
                )
            })?;
        }
        Ok(())
    }

    fn register_active_worker(self: &Arc<Self>, run_id: &str, pid: u32) {
        self.active_worker_children
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(run_id.to_string(), super::types::ActiveWorkerChild { pid });
    }

    fn unregister_active_worker(&self, run_id: &str) {
        self.active_worker_children
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(run_id);
    }
}

struct ActiveWorkerRunGuard {
    coordinator: Arc<CapabilityEventCoordinator>,
    run_id: String,
    pid: u32,
    child_exited: bool,
}

impl ActiveWorkerRunGuard {
    fn register(
        coordinator: Arc<CapabilityEventCoordinator>,
        run_id: String,
        pid: u32,
    ) -> ActiveWorkerRunGuard {
        coordinator.register_active_worker(&run_id, pid);
        ActiveWorkerRunGuard {
            coordinator,
            run_id,
            pid,
            child_exited: false,
        }
    }

    fn mark_child_exited(&mut self) {
        self.child_exited = true;
        self.coordinator.unregister_active_worker(&self.run_id);
    }
}

impl Drop for ActiveWorkerRunGuard {
    fn drop(&mut self) {
        self.coordinator.unregister_active_worker(&self.run_id);
        if !self.child_exited {
            let _ = terminate_current_state_worker_process(self.pid);
        }
    }
}

fn completed_run_completion(
    plan: ExecutionPlan,
    result: crate::host::capability_host::CurrentStateConsumerResult,
) -> RunCompletion {
    match validate_consumer_result(&plan.request, &result) {
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
    }
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

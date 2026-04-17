use std::collections::BTreeMap;
use std::panic::AssertUnwindSafe;
#[cfg(test)]
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{Context, Result, anyhow};
use futures_util::FutureExt;
use rusqlite::{OptionalExtension, params};
use tokio::sync::Notify;
use tokio::time::{Duration, sleep};

use crate::config::resolve_repo_runtime_db_path_for_config_root;
use crate::graphql::SubscriptionHub;
use crate::host::capability_host::{DevqlCapabilityHost, SyncArtefactDiff, SyncFileDiff};
use crate::host::devql::{DevqlConfig, SyncSummary, resolve_repo_identity};
use crate::host::runtime_store::DaemonSqliteRuntimeStore;

use super::super::types::{
    CapabilityEventQueueState, CapabilityEventQueueStatus, CapabilityEventRunRecord,
    CapabilityEventRunStatus, unix_timestamp_now,
};
use super::plan::{build_execution_plan, find_current_state_consumer, validate_consumer_result};
use super::queue::{
    ConsumerRunRequest, StoredRunRecord, ensure_consumer_run, insert_artefact_changes,
    insert_file_changes, load_run_by_id, load_runs, next_generation_seq, prune_terminal_runs,
    sql_i64, upsert_consumer_row,
};

const WORKER_POLL_INTERVAL: Duration = Duration::from_secs(2);
const MAX_RUN_ATTEMPTS: u32 = 5;

#[derive(Debug, Clone)]
pub struct CapabilityEventEnqueueResult {
    pub runs: Vec<CapabilityEventRunRecord>,
}

pub(crate) struct SyncGenerationInput<'a> {
    pub(crate) file_diff: SyncFileDiff,
    pub(crate) artefact_diff: SyncArtefactDiff,
    pub(crate) source_task_id: Option<&'a str>,
    pub(crate) init_session_id: Option<&'a str>,
}

#[derive(Debug)]
pub struct CapabilityEventCoordinator {
    runtime_store: DaemonSqliteRuntimeStore,
    lock: Mutex<()>,
    notify: Notify,
    worker_started: AtomicBool,
    subscription_hub: Mutex<Option<Arc<SubscriptionHub>>>,
}

struct WorkerStartedGuard {
    coordinator: Arc<CapabilityEventCoordinator>,
}

impl Drop for WorkerStartedGuard {
    fn drop(&mut self) {
        self.coordinator
            .worker_started
            .store(false, Ordering::SeqCst);
    }
}

#[derive(Debug)]
enum RunCompletion {
    NoopCompleted {
        run: CapabilityEventRunRecord,
    },
    Completed {
        run: CapabilityEventRunRecord,
        applied_to_generation_seq: u64,
    },
    RetryableFailure {
        run: CapabilityEventRunRecord,
        error: String,
    },
    Failed {
        run: CapabilityEventRunRecord,
        error: String,
    },
}

#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn test_shared_instance_at(db_path: PathBuf) -> Arc<CapabilityEventCoordinator> {
    CapabilityEventCoordinator::new_shared_instance(
        DaemonSqliteRuntimeStore::open_at(db_path)
            .expect("opening test daemon runtime store for current-state consumers"),
    )
}

impl CapabilityEventCoordinator {
    pub(crate) fn try_shared() -> Result<Arc<Self>> {
        let daemon_config =
            crate::daemon::resolve_daemon_config(None).context("resolving daemon config")?;
        let runtime_store = DaemonSqliteRuntimeStore::open_at(
            resolve_repo_runtime_db_path_for_config_root(&daemon_config.config_root),
        )
        .context("opening repo runtime workplane store for current-state consumers")?;
        static INSTANCE: OnceLock<Mutex<Arc<CapabilityEventCoordinator>>> = OnceLock::new();
        let slot =
            INSTANCE.get_or_init(|| Mutex::new(Self::new_shared_instance(runtime_store.clone())));
        let coordinator = slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

        #[cfg(test)]
        let mut coordinator = coordinator;

        #[cfg(test)]
        if coordinator.runtime_store.db_path() != runtime_store.db_path() {
            *coordinator = Self::new_shared_instance(runtime_store);
        }

        Ok(Arc::clone(&coordinator))
    }

    pub(crate) fn shared() -> Arc<Self> {
        Self::try_shared().expect("building current-state consumer coordinator")
    }

    pub(crate) fn clear_queued_runs_for_repo(&self, repo_id: &str) -> Result<u64> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| anyhow!("current-state consumer lock poisoned"))?;
        let deleted = self.runtime_store.with_connection(|conn| {
            conn.execute(
                "DELETE FROM capability_workplane_cursor_runs WHERE repo_id = ?1 AND status = ?2",
                params![repo_id, CapabilityEventRunStatus::Queued.to_string()],
            )
            .map(|count| u64::try_from(count).unwrap_or_default())
            .map_err(anyhow::Error::from)
        })?;
        self.notify.notify_waiters();
        Ok(deleted)
    }

    fn new_shared_instance(runtime_store: DaemonSqliteRuntimeStore) -> Arc<Self> {
        Arc::new(Self {
            runtime_store,
            lock: Mutex::new(()),
            notify: Notify::new(),
            worker_started: AtomicBool::new(false),
            subscription_hub: Mutex::new(None),
        })
    }

    pub(crate) fn set_subscription_hub(&self, subscription_hub: Arc<SubscriptionHub>) {
        if let Ok(mut slot) = self.subscription_hub.lock() {
            *slot = Some(subscription_hub);
        }
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
            log::warn!(
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
        let runs = self.runtime_store.with_connection(|conn| {
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
                    if let Some(run) = ensure_consumer_run(
                        conn,
                        ConsumerRunRequest {
                            repo_id: &repo_id,
                            repo_root: &repo_root,
                            capability_id: registration.capability_id,
                            mailbox_name: registration.mailbox_name,
                            handler_id,
                            init_session_id: input.init_session_id,
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

    pub(crate) fn snapshot(&self, repo_id: Option<&str>) -> Result<CapabilityEventQueueStatus> {
        self.runtime_store.with_connection(|conn| {
            let pending_runs = count_runs_with_status(conn, CapabilityEventRunStatus::Queued)?;
            let running_runs = count_runs_with_status(conn, CapabilityEventRunStatus::Running)?;
            let failed_runs = count_runs_with_status(conn, CapabilityEventRunStatus::Failed)?;
            let completed_recent_runs =
                count_runs_with_status(conn, CapabilityEventRunStatus::Completed)?;
            let queue_activity = load_queue_activity(conn)?;
            let current_repo_run = repo_id
                .map(|repo_id| load_current_repo_run(conn, repo_id))
                .transpose()?
                .flatten();

            Ok(CapabilityEventQueueStatus {
                state: CapabilityEventQueueState {
                    version: 1,
                    pending_runs,
                    running_runs,
                    failed_runs,
                    completed_recent_runs,
                    last_action: queue_activity.last_action,
                    last_updated_unix: queue_activity.last_updated_unix,
                },
                persisted: true,
                current_repo_run,
            })
        })
    }

    #[allow(dead_code)]
    pub(crate) fn run(&self, run_id: &str) -> Result<Option<CapabilityEventRunRecord>> {
        self.runtime_store.with_connection(|conn| {
            load_run_by_id(conn, run_id).map(|record| record.map(|r| r.record))
        })
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
            if let Err(err) = coordinator.apply_completion(completion) {
                log::warn!("failed to persist current-state consumer completion: {err:#}");
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
                Ok(()) => RunCompletion::Completed {
                    run: plan.record,
                    applied_to_generation_seq: result.applied_to_generation_seq,
                },
                Err(err) => terminal_or_retry(plan.record, err),
            },
            Ok(Err(err)) => terminal_or_retry(plan.record, err),
            Err(_) => terminal_or_retry(plan.record, anyhow!("current-state consumer panicked")),
        }
    }

    fn apply_completion(&self, completion: RunCompletion) -> Result<()> {
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
        self.runtime_store.with_connection(|conn| {
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

    fn recover_running_runs(&self) -> Result<()> {
        self.runtime_store.with_connection(|conn| {
            conn.execute(
                "UPDATE capability_workplane_cursor_runs SET status = ?1, started_at_unix = NULL, updated_at_unix = ?2 WHERE status = ?3",
                params![
                    CapabilityEventRunStatus::Queued.to_string(),
                    sql_i64(unix_timestamp_now())?,
                    CapabilityEventRunStatus::Running.to_string(),
                ],
            )
            .context("recovering in-flight current-state consumer runs")?;
            Ok(())
        })
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

fn count_runs_with_status(
    conn: &rusqlite::Connection,
    status: CapabilityEventRunStatus,
) -> Result<u64> {
    conn.query_row(
        "SELECT COUNT(*) FROM capability_workplane_cursor_runs WHERE status = ?1",
        params![status.to_string()],
        |row| row.get::<_, i64>(0),
    )
    .map(|value| u64::try_from(value).unwrap_or_default())
    .map_err(anyhow::Error::from)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct QueueActivity {
    last_action: Option<String>,
    last_updated_unix: u64,
}

fn load_queue_activity(conn: &rusqlite::Connection) -> Result<QueueActivity> {
    conn.query_row(
        "SELECT status, updated_at_unix FROM capability_workplane_cursor_runs ORDER BY updated_at_unix DESC, submitted_at_unix DESC LIMIT 1",
        [],
        |row| {
            Ok(QueueActivity {
                last_action: Some(row.get(0)?),
                last_updated_unix: u64::try_from(row.get::<_, i64>(1)?).unwrap_or_default(),
            })
        },
    )
    .optional()
    .map(|row| {
        row.unwrap_or(QueueActivity {
            last_action: None,
            last_updated_unix: 0,
        })
    })
    .map_err(anyhow::Error::from)
}

fn load_current_repo_run(
    conn: &rusqlite::Connection,
    repo_id: &str,
) -> Result<Option<CapabilityEventRunRecord>> {
    if let Some(run) = load_runs(
        conn,
        "SELECT run_id, repo_id, repo_root, mailbox_name, capability_id, init_session_id, from_generation_seq, to_generation_seq, reconcile_mode, status, attempts, submitted_at_unix, started_at_unix, updated_at_unix, completed_at_unix, error FROM capability_workplane_cursor_runs WHERE repo_id = ?1 AND status = ?2 ORDER BY submitted_at_unix ASC LIMIT 1",
        params![repo_id, CapabilityEventRunStatus::Running.to_string()],
    )?
    .into_iter()
    .next()
    {
        return Ok(Some(run.record));
    }

    Ok(load_runs(
        conn,
        "SELECT run_id, repo_id, repo_root, mailbox_name, capability_id, init_session_id, from_generation_seq, to_generation_seq, reconcile_mode, status, attempts, submitted_at_unix, started_at_unix, updated_at_unix, completed_at_unix, error FROM capability_workplane_cursor_runs WHERE repo_id = ?1 AND status = ?2 ORDER BY submitted_at_unix ASC LIMIT 1",
        params![repo_id, CapabilityEventRunStatus::Queued.to_string()],
    )?
    .into_iter()
    .next()
    .map(|run| run.record))
}

fn terminal_or_retry(
    run: CapabilityEventRunRecord,
    err: impl Into<anyhow::Error>,
) -> RunCompletion {
    let error = format!("{:#}", err.into());
    if run.attempts >= MAX_RUN_ATTEMPTS {
        RunCompletion::Failed { run, error }
    } else {
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

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;
    use crate::host::capability_host::{SyncArtefactDiff, SyncFileDiff};
    use crate::host::devql::{DevqlConfig, SyncSummary, resolve_repo_identity};
    use crate::test_support::git_fixtures::{init_test_repo, write_test_daemon_config};

    fn test_runtime_store(dir: &TempDir) -> DaemonSqliteRuntimeStore {
        DaemonSqliteRuntimeStore::open_at(dir.path().join("runtime.sqlite"))
            .expect("open test runtime store")
    }

    fn sample_run(status: CapabilityEventRunStatus) -> CapabilityEventRunRecord {
        CapabilityEventRunRecord {
            run_id: "run-1".to_string(),
            repo_id: "repo-1".to_string(),
            capability_id: "test_harness".to_string(),
            consumer_id: "test_harness.current_state".to_string(),
            handler_id: "test_harness.current_state".to_string(),
            from_generation_seq: 2,
            to_generation_seq: 5,
            reconcile_mode: "merged_delta".to_string(),
            event_kind: "current_state_consumer".to_string(),
            lane_key: "repo-1:test_harness.current_state".to_string(),
            event_payload_json: String::new(),
            init_session_id: None,
            status,
            attempts: 1,
            submitted_at_unix: 10,
            started_at_unix: Some(20),
            updated_at_unix: 30,
            completed_at_unix: None,
            error: Some("stale error".to_string()),
        }
    }

    fn insert_consumer_row(
        store: &DaemonSqliteRuntimeStore,
        repo_id: &str,
        capability_id: &str,
        consumer_id: &str,
        last_applied_generation_seq: Option<u64>,
        last_error: Option<&str>,
        updated_at_unix: u64,
    ) {
        store
            .with_connection(|conn| {
                conn.execute(
                    "INSERT INTO capability_workplane_cursor_mailboxes (repo_id, capability_id, mailbox_name, last_applied_generation_seq, last_error, updated_at_unix) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        repo_id,
                        capability_id,
                        consumer_id,
                        last_applied_generation_seq.map(sql_i64).transpose()?,
                        last_error,
                        sql_i64(updated_at_unix)?,
                    ],
                )?;
                Ok(())
            })
            .expect("insert consumer row");
    }

    fn insert_run_row(store: &DaemonSqliteRuntimeStore, run: &CapabilityEventRunRecord) {
        store
            .with_connection(|conn| {
                conn.execute(
                    "INSERT INTO capability_workplane_cursor_runs (run_id, repo_id, repo_root, mailbox_name, capability_id, from_generation_seq, to_generation_seq, reconcile_mode, status, attempts, submitted_at_unix, started_at_unix, updated_at_unix, completed_at_unix, error) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                    params![
                        run.run_id,
                        run.repo_id,
                        "/tmp/repo",
                        run.consumer_id,
                        run.capability_id,
                        sql_i64(run.from_generation_seq)?,
                        sql_i64(run.to_generation_seq)?,
                        run.reconcile_mode,
                        run.status.to_string(),
                        run.attempts,
                        sql_i64(run.submitted_at_unix)?,
                        run.started_at_unix.map(sql_i64).transpose()?,
                        sql_i64(run.updated_at_unix)?,
                        run.completed_at_unix.map(sql_i64).transpose()?,
                        run.error,
                    ],
                )?;
                Ok(())
            })
            .expect("insert run row");
    }

    fn test_cfg(repo_root: &std::path::Path) -> DevqlConfig {
        let repo = resolve_repo_identity(repo_root).expect("resolve repo identity");
        DevqlConfig::from_env(repo_root.to_path_buf(), repo).expect("build devql config")
    }

    #[test]
    fn record_sync_generation_schedules_consumers_for_successful_empty_sync() {
        let temp = TempDir::new().expect("tempdir");
        let repo_root = temp.path().join("repo");
        fs::create_dir_all(&repo_root).expect("create repo root");
        init_test_repo(&repo_root, "main", "Bitloops Test", "bitloops@example.com");
        write_test_daemon_config(&repo_root);

        let cfg = test_cfg(&repo_root);
        let host = DevqlCapabilityHost::builtin(repo_root.clone(), cfg.repo.clone())
            .expect("build capability host");
        let coordinator =
            CapabilityEventCoordinator::new_shared_instance(test_runtime_store(&temp));
        let cursor_mailbox_count = host
            .workplane_mailboxes()
            .iter()
            .filter(|registration| {
                registration.policy == crate::host::capability_host::CapabilityMailboxPolicy::Cursor
            })
            .count();

        let outcome = coordinator
            .record_sync_generation(
                &host,
                &cfg,
                &SyncSummary {
                    success: true,
                    mode: "auto".to_string(),
                    active_branch: Some("main".to_string()),
                    head_commit_sha: Some("abc123".to_string()),
                    ..SyncSummary::default()
                },
                SyncGenerationInput {
                    file_diff: SyncFileDiff::default(),
                    artefact_diff: SyncArtefactDiff::default(),
                    source_task_id: None,
                    init_session_id: None,
                },
            )
            .expect("record empty sync generation");

        assert!(
            cursor_mailbox_count > 0,
            "expected built-in cursor mailboxes to be registered"
        );
        assert_eq!(outcome.runs.len(), cursor_mailbox_count);

        let snapshot = coordinator
            .snapshot(Some(&cfg.repo.repo_id))
            .expect("snapshot capability event queue");
        assert_eq!(snapshot.state.pending_runs as usize, cursor_mailbox_count);
    }

    #[test]
    fn apply_completion_noop_keeps_existing_cursor_and_clears_error() {
        let temp = TempDir::new().expect("tempdir");
        let store = test_runtime_store(&temp);
        let coordinator = CapabilityEventCoordinator::new_shared_instance(store.clone());
        let run = sample_run(CapabilityEventRunStatus::Running);
        insert_consumer_row(
            &store,
            &run.repo_id,
            &run.capability_id,
            &run.consumer_id,
            Some(7),
            Some("previous failure"),
            29,
        );
        insert_run_row(&store, &run);

        coordinator
            .apply_completion(RunCompletion::NoopCompleted { run: run.clone() })
            .expect("apply noop completion");

        store
            .with_connection(|conn| {
                let cursor = conn.query_row(
                    "SELECT last_applied_generation_seq, last_error
                     FROM capability_workplane_cursor_mailboxes
                     WHERE repo_id = ?1 AND mailbox_name = ?2",
                    params![&run.repo_id, &run.consumer_id],
                    |row| {
                        Ok((
                            row.get::<_, Option<i64>>(0)?,
                            row.get::<_, Option<String>>(1)?,
                        ))
                    },
                )?;
                assert_eq!(cursor.0, Some(7));
                assert_eq!(cursor.1, None);
                Ok(())
            })
            .expect("load consumer cursor");

        let persisted = coordinator
            .run(&run.run_id)
            .expect("load run")
            .expect("persisted run");
        assert_eq!(persisted.status, CapabilityEventRunStatus::Completed);
        assert_eq!(persisted.error, None);
    }

    #[test]
    fn apply_completion_retryable_failure_clears_started_at_when_requeued() {
        let temp = TempDir::new().expect("tempdir");
        let store = test_runtime_store(&temp);
        let coordinator = CapabilityEventCoordinator::new_shared_instance(store.clone());
        let run = sample_run(CapabilityEventRunStatus::Running);
        insert_consumer_row(
            &store,
            &run.repo_id,
            &run.capability_id,
            &run.consumer_id,
            Some(2),
            None,
            29,
        );
        insert_run_row(&store, &run);

        coordinator
            .apply_completion(RunCompletion::RetryableFailure {
                run: run.clone(),
                error: "temporary failure".to_string(),
            })
            .expect("apply retryable failure");

        let persisted = coordinator
            .run(&run.run_id)
            .expect("load run")
            .expect("persisted run");
        assert_eq!(persisted.status, CapabilityEventRunStatus::Queued);
        assert_eq!(persisted.started_at_unix, None);
        assert_eq!(persisted.completed_at_unix, None);
        assert_eq!(persisted.error.as_deref(), Some("temporary failure"));
    }

    #[test]
    fn snapshot_uses_latest_run_status_and_timestamp() {
        let temp = TempDir::new().expect("tempdir");
        let store = test_runtime_store(&temp);
        let coordinator = CapabilityEventCoordinator::new_shared_instance(store.clone());
        let mut older = sample_run(CapabilityEventRunStatus::Running);
        older.run_id = "run-older".to_string();
        older.updated_at_unix = 11;
        older.submitted_at_unix = 9;
        let mut newer = sample_run(CapabilityEventRunStatus::Failed);
        newer.run_id = "run-newer".to_string();
        newer.updated_at_unix = 42;
        newer.submitted_at_unix = 41;
        insert_run_row(&store, &older);
        insert_run_row(&store, &newer);

        let snapshot = coordinator.snapshot(None).expect("snapshot queue");
        assert_eq!(snapshot.state.last_action.as_deref(), Some("failed"));
        assert_eq!(snapshot.state.last_updated_unix, 42);
    }
}

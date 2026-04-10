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
use uuid::Uuid;

use crate::host::capability_host::{
    ChangedArtefact, ChangedFile, CurrentStateConsumer, CurrentStateConsumerRequest,
    CurrentStateConsumerResult, DevqlCapabilityHost, ReconcileMode, RemovedArtefact, RemovedFile,
    SyncArtefactDiff, SyncFileDiff,
};
use crate::host::devql::{DevqlConfig, SyncSummary, resolve_repo_identity};
use crate::host::runtime_store::DaemonSqliteRuntimeStore;

use super::types::{
    CapabilityEventQueueState, CapabilityEventQueueStatus, CapabilityEventRunRecord,
    CapabilityEventRunStatus, unix_timestamp_now,
};

const WORKER_POLL_INTERVAL: Duration = Duration::from_secs(2);
const MAX_RUN_ATTEMPTS: u32 = 5;

#[derive(Debug, Clone)]
pub struct CapabilityEventEnqueueResult {
    pub runs: Vec<CapabilityEventRunRecord>,
}

#[derive(Debug)]
pub struct CapabilityEventCoordinator {
    runtime_store: DaemonSqliteRuntimeStore,
    lock: Mutex<()>,
    notify: Notify,
    worker_started: AtomicBool,
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

#[derive(Debug, Clone)]
struct StoredRunRecord {
    record: CapabilityEventRunRecord,
    repo_root: std::path::PathBuf,
}

#[derive(Debug, Clone)]
struct GenerationRow {
    generation_seq: u64,
    active_branch: Option<String>,
    head_commit_sha: Option<String>,
    requires_full_reconcile: bool,
}

#[derive(Debug, Clone)]
struct FileChangeRow {
    generation_seq: u64,
    path: String,
    change_kind: String,
    language: Option<String>,
    content_id: Option<String>,
}

#[derive(Debug, Clone)]
struct ArtefactChangeRow {
    generation_seq: u64,
    symbol_id: String,
    change_kind: String,
    artefact_id: String,
    path: String,
    canonical_kind: Option<String>,
    name: String,
}

#[derive(Debug, Clone)]
struct ConsumerCursorRow {
    last_applied_generation_seq: Option<u64>,
}

#[derive(Debug, Clone)]
struct ExecutionPlan {
    record: CapabilityEventRunRecord,
    repo_root: std::path::PathBuf,
    request: CurrentStateConsumerRequest,
}

#[derive(Debug)]
enum RunCompletion {
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum MergedFileChange {
    Upsert(ChangedFile),
    Removed(RemovedFile),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MergedArtefactChange {
    Upsert(ChangedArtefact),
    Removed(RemovedArtefact),
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
    pub(crate) fn shared() -> Arc<Self> {
        let runtime_store = DaemonSqliteRuntimeStore::open()
            .expect("opening daemon runtime store for current-state consumers");
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

        Arc::clone(&coordinator)
    }

    fn new_shared_instance(runtime_store: DaemonSqliteRuntimeStore) -> Arc<Self> {
        Arc::new(Self {
            runtime_store,
            lock: Mutex::new(()),
            notify: Notify::new(),
            worker_started: AtomicBool::new(false),
        })
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
        file_diff: SyncFileDiff,
        artefact_diff: SyncArtefactDiff,
        source_task_id: Option<&str>,
    ) -> Result<CapabilityEventEnqueueResult> {
        if !summary.success || summary.mode == "validate" {
            return Ok(CapabilityEventEnqueueResult { runs: Vec::new() });
        }

        let has_changes = !file_diff.added.is_empty()
            || !file_diff.changed.is_empty()
            || !file_diff.removed.is_empty()
            || !artefact_diff.added.is_empty()
            || !artefact_diff.changed.is_empty()
            || !artefact_diff.removed.is_empty();
        if !has_changes {
            return Ok(CapabilityEventEnqueueResult { runs: Vec::new() });
        }

        let registrations = host.current_state_consumers().to_vec();
        let now = unix_timestamp_now();
        let repo_id = cfg.repo.repo_id.clone();
        let repo_root = cfg.repo_root.clone();
        let source_task_id = source_task_id.map(str::to_string);
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
                    "INSERT INTO pack_reconcile_generations (repo_id, generation_seq, source_task_id, sync_mode, active_branch, head_commit_sha, requires_full_reconcile, created_at_unix) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
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
                insert_file_changes(conn, &repo_id, generation_seq, &file_diff)?;
                insert_artefact_changes(conn, &repo_id, generation_seq, &artefact_diff)?;

                let mut scheduled_runs = Vec::new();
                for registration in &registrations {
                    upsert_consumer_row(
                        conn,
                        &repo_id,
                        registration.capability_id,
                        registration.consumer_id,
                        now,
                    )?;
                    if let Some(run) = ensure_consumer_run(
                        conn,
                        &repo_id,
                        &repo_root,
                        registration.capability_id,
                        registration.consumer_id,
                        now,
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

        Ok(CapabilityEventEnqueueResult { runs })
    }

    pub(crate) fn snapshot(&self, repo_id: Option<&str>) -> Result<CapabilityEventQueueStatus> {
        self.runtime_store.with_connection(|conn| {
            let pending_runs = count_runs_with_status(conn, CapabilityEventRunStatus::Queued)?;
            let running_runs = count_runs_with_status(conn, CapabilityEventRunStatus::Running)?;
            let failed_runs = count_runs_with_status(conn, CapabilityEventRunStatus::Failed)?;
            let completed_recent_runs =
                count_runs_with_status(conn, CapabilityEventRunStatus::Completed)?;
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
                    last_action: Some("current_state_consumers_available".to_string()),
                    last_updated_unix: unix_timestamp_now(),
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
                        "UPDATE pack_reconcile_runs SET status = ?1, attempts = ?2, started_at_unix = ?3, updated_at_unix = ?4 WHERE run_id = ?5",
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
            Ok(None) => {
                let applied_to_generation_seq = run.record.to_generation_seq;
                return RunCompletion::Completed {
                    run: run.record,
                    applied_to_generation_seq,
                };
            }
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
        let context = match host.build_current_state_consumer_context() {
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
                    RunCompletion::Completed {
                        run,
                        applied_to_generation_seq,
                    } => {
                        conn.execute(
                            "UPDATE pack_reconcile_consumers SET last_applied_generation_seq = ?1, last_error = NULL, updated_at_unix = ?2 WHERE repo_id = ?3 AND consumer_id = ?4",
                            params![
                                sql_i64(applied_to_generation_seq)?,
                                sql_i64(now)?,
                                run.repo_id,
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
                            "UPDATE pack_reconcile_runs SET from_generation_seq = ?1, to_generation_seq = ?2, reconcile_mode = ?3, status = ?4, updated_at_unix = ?5, completed_at_unix = ?6, error = NULL WHERE run_id = ?7",
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
                            "UPDATE pack_reconcile_consumers SET last_error = ?1, updated_at_unix = ?2 WHERE repo_id = ?3 AND consumer_id = ?4",
                            params![error, sql_i64(now)?, run.repo_id, run.consumer_id],
                        )
                        .with_context(|| {
                            format!(
                                "updating retryable current-state consumer error for run `{}`",
                                run.run_id
                            )
                        })?;
                        conn.execute(
                            "UPDATE pack_reconcile_runs SET status = ?1, updated_at_unix = ?2, completed_at_unix = NULL, error = ?3 WHERE run_id = ?4",
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
                            "UPDATE pack_reconcile_consumers SET last_error = ?1, updated_at_unix = ?2 WHERE repo_id = ?3 AND consumer_id = ?4",
                            params![error, sql_i64(now)?, run.repo_id, run.consumer_id],
                        )
                        .with_context(|| {
                            format!("persisting terminal current-state consumer error for `{}`", run.run_id)
                        })?;
                        conn.execute(
                            "UPDATE pack_reconcile_runs SET status = ?1, updated_at_unix = ?2, completed_at_unix = ?3, error = ?4 WHERE run_id = ?5",
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
        })
    }

    fn recover_running_runs(&self) -> Result<()> {
        self.runtime_store.with_connection(|conn| {
            conn.execute(
                "UPDATE pack_reconcile_runs SET status = ?1, updated_at_unix = ?2 WHERE status = ?3",
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

fn insert_file_changes(
    conn: &rusqlite::Connection,
    repo_id: &str,
    generation_seq: u64,
    diff: &SyncFileDiff,
) -> Result<()> {
    for file in &diff.added {
        insert_file_change_row(
            conn,
            repo_id,
            generation_seq,
            &file.path,
            "added",
            Some(&file.language),
            Some(&file.content_id),
        )?;
    }
    for file in &diff.changed {
        insert_file_change_row(
            conn,
            repo_id,
            generation_seq,
            &file.path,
            "changed",
            Some(&file.language),
            Some(&file.content_id),
        )?;
    }
    for file in &diff.removed {
        insert_file_change_row(
            conn,
            repo_id,
            generation_seq,
            &file.path,
            "removed",
            None,
            None,
        )?;
    }
    Ok(())
}

fn insert_file_change_row(
    conn: &rusqlite::Connection,
    repo_id: &str,
    generation_seq: u64,
    path: &str,
    change_kind: &str,
    language: Option<&str>,
    content_id: Option<&str>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO pack_reconcile_file_changes (repo_id, generation_seq, path, change_kind, language, content_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            repo_id,
            sql_i64(generation_seq)?,
            path,
            change_kind,
            language,
            content_id
        ],
    )
    .with_context(|| format!("inserting file change `{path}` for generation {generation_seq}"))?;
    Ok(())
}

fn insert_artefact_changes(
    conn: &rusqlite::Connection,
    repo_id: &str,
    generation_seq: u64,
    diff: &SyncArtefactDiff,
) -> Result<()> {
    for artefact in &diff.added {
        insert_artefact_change_row(conn, repo_id, generation_seq, artefact, "added")?;
    }
    for artefact in &diff.changed {
        insert_artefact_change_row(conn, repo_id, generation_seq, artefact, "changed")?;
    }
    for artefact in &diff.removed {
        conn.execute(
            "INSERT INTO pack_reconcile_artefact_changes (repo_id, generation_seq, symbol_id, change_kind, artefact_id, path, canonical_kind, name) VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7)",
            params![
                repo_id,
                sql_i64(generation_seq)?,
                artefact.symbol_id,
                "removed",
                artefact.artefact_id,
                artefact.path,
                artefact.path,
            ],
        )
        .with_context(|| {
            format!(
                "inserting removed artefact `{}` for generation {}",
                artefact.symbol_id, generation_seq
            )
        })?;
    }
    Ok(())
}

fn insert_artefact_change_row(
    conn: &rusqlite::Connection,
    repo_id: &str,
    generation_seq: u64,
    artefact: &ChangedArtefact,
    change_kind: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO pack_reconcile_artefact_changes (repo_id, generation_seq, symbol_id, change_kind, artefact_id, path, canonical_kind, name) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            repo_id,
            sql_i64(generation_seq)?,
            artefact.symbol_id,
            change_kind,
            artefact.artefact_id,
            artefact.path,
            artefact.canonical_kind,
            artefact.name,
        ],
    )
    .with_context(|| {
        format!(
            "inserting artefact change `{}` for generation {}",
            artefact.symbol_id, generation_seq
        )
    })?;
    Ok(())
}

fn next_generation_seq(conn: &rusqlite::Connection, repo_id: &str) -> Result<u64> {
    conn.query_row(
        "SELECT COALESCE(MAX(generation_seq), 0) + 1 FROM pack_reconcile_generations WHERE repo_id = ?1",
        params![repo_id],
        |row| row.get::<_, i64>(0),
    )
    .map(|value| u64::try_from(value).unwrap_or_default())
    .map_err(anyhow::Error::from)
}

fn upsert_consumer_row(
    conn: &rusqlite::Connection,
    repo_id: &str,
    capability_id: &str,
    consumer_id: &str,
    now: u64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO pack_reconcile_consumers (repo_id, consumer_id, capability_id, last_applied_generation_seq, last_error, updated_at_unix) VALUES (?1, ?2, ?3, NULL, NULL, ?4) \
         ON CONFLICT (repo_id, consumer_id) DO UPDATE SET capability_id = excluded.capability_id, updated_at_unix = excluded.updated_at_unix",
        params![repo_id, consumer_id, capability_id, sql_i64(now)?],
    )
    .with_context(|| {
        format!(
            "upserting current-state consumer `{consumer_id}` for repo `{repo_id}`"
        )
    })?;
    Ok(())
}

fn ensure_consumer_run(
    conn: &rusqlite::Connection,
    repo_id: &str,
    repo_root: &std::path::Path,
    capability_id: &str,
    consumer_id: &str,
    now: u64,
) -> Result<Option<StoredRunRecord>> {
    let latest_generation = latest_generation_seq(conn, repo_id)?;
    let Some(latest_generation) = latest_generation else {
        return Ok(None);
    };

    let last_applied_generation = load_consumer_cursor(conn, repo_id, consumer_id)?
        .and_then(|cursor| cursor.last_applied_generation_seq)
        .unwrap_or(0);
    if latest_generation <= last_applied_generation {
        return Ok(None);
    }

    if let Some(run) = load_active_run_for_lane(conn, repo_id, consumer_id)? {
        if run.record.status == CapabilityEventRunStatus::Queued {
            let run_id = run.record.run_id.clone();
            conn.execute(
                "UPDATE pack_reconcile_runs SET from_generation_seq = ?1, to_generation_seq = ?2, updated_at_unix = ?3 WHERE run_id = ?4",
                params![
                    sql_i64(last_applied_generation)?,
                    sql_i64(latest_generation)?,
                    sql_i64(now)?,
                    &run_id,
                ],
            )
            .with_context(|| {
                format!(
                    "refreshing queued current-state consumer run `{}`",
                    run_id
                )
            })?;
            let mut refreshed = run.record.clone();
            refreshed.from_generation_seq = last_applied_generation;
            refreshed.to_generation_seq = latest_generation;
            refreshed.updated_at_unix = now;
            return Ok(Some(StoredRunRecord {
                record: refreshed,
                repo_root: run.repo_root,
            }));
        }
        return Ok(None);
    }

    let run_id = format!("current-state-consumer-run-{}", Uuid::new_v4());
    let record = CapabilityEventRunRecord {
        run_id: run_id.clone(),
        repo_id: repo_id.to_string(),
        capability_id: capability_id.to_string(),
        consumer_id: consumer_id.to_string(),
        handler_id: consumer_id.to_string(),
        from_generation_seq: last_applied_generation,
        to_generation_seq: latest_generation,
        reconcile_mode: "merged_delta".to_string(),
        event_kind: "current_state_consumer".to_string(),
        lane_key: build_lane_key(repo_id, consumer_id),
        event_payload_json: String::new(),
        status: CapabilityEventRunStatus::Queued,
        attempts: 0,
        submitted_at_unix: now,
        started_at_unix: None,
        updated_at_unix: now,
        completed_at_unix: None,
        error: None,
    };
    conn.execute(
        "INSERT INTO pack_reconcile_runs (run_id, repo_id, repo_root, consumer_id, capability_id, from_generation_seq, to_generation_seq, reconcile_mode, status, attempts, submitted_at_unix, started_at_unix, updated_at_unix, completed_at_unix, error) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL, ?12, NULL, NULL)",
        params![
            &record.run_id,
            &record.repo_id,
            repo_root.to_string_lossy().to_string(),
            &record.consumer_id,
            &record.capability_id,
            sql_i64(record.from_generation_seq)?,
            sql_i64(record.to_generation_seq)?,
            &record.reconcile_mode,
            record.status.to_string(),
            record.attempts,
            sql_i64(record.submitted_at_unix)?,
            sql_i64(record.updated_at_unix)?,
        ],
    )
    .with_context(|| format!("creating current-state consumer run `{run_id}`"))?;

    Ok(Some(StoredRunRecord {
        record,
        repo_root: repo_root.to_path_buf(),
    }))
}

fn latest_generation_seq(conn: &rusqlite::Connection, repo_id: &str) -> Result<Option<u64>> {
    conn.query_row(
        "SELECT MAX(generation_seq) FROM pack_reconcile_generations WHERE repo_id = ?1",
        params![repo_id],
        |row| row.get::<_, Option<i64>>(0),
    )
    .map(|value| value.and_then(|v| u64::try_from(v).ok()))
    .map_err(anyhow::Error::from)
}

fn load_consumer_cursor(
    conn: &rusqlite::Connection,
    repo_id: &str,
    consumer_id: &str,
) -> Result<Option<ConsumerCursorRow>> {
    conn.query_row(
        "SELECT last_applied_generation_seq FROM pack_reconcile_consumers WHERE repo_id = ?1 AND consumer_id = ?2",
        params![repo_id, consumer_id],
        |row| {
            Ok(ConsumerCursorRow {
                last_applied_generation_seq: row
                    .get::<_, Option<i64>>(0)?
                    .and_then(|value| u64::try_from(value).ok()),
            })
        },
    )
    .optional()
    .map_err(anyhow::Error::from)
}

fn load_active_run_for_lane(
    conn: &rusqlite::Connection,
    repo_id: &str,
    consumer_id: &str,
) -> Result<Option<StoredRunRecord>> {
    load_runs(
        conn,
        "SELECT run_id, repo_id, repo_root, consumer_id, capability_id, from_generation_seq, to_generation_seq, reconcile_mode, status, attempts, submitted_at_unix, started_at_unix, updated_at_unix, completed_at_unix, error FROM pack_reconcile_runs WHERE repo_id = ?1 AND consumer_id = ?2 AND status IN (?3, ?4) ORDER BY CASE status WHEN 'running' THEN 0 ELSE 1 END, submitted_at_unix ASC LIMIT 1",
        params![
            repo_id,
            consumer_id,
            CapabilityEventRunStatus::Running.to_string(),
            CapabilityEventRunStatus::Queued.to_string(),
        ],
    )
    .map(|mut runs| runs.pop())
}

fn load_claimable_runs(conn: &rusqlite::Connection) -> Result<Vec<StoredRunRecord>> {
    let now = unix_timestamp_now();
    let candidates = load_runs(
        conn,
        "SELECT run_id, repo_id, repo_root, consumer_id, capability_id, from_generation_seq, to_generation_seq, reconcile_mode, status, attempts, submitted_at_unix, started_at_unix, updated_at_unix, completed_at_unix, error FROM pack_reconcile_runs WHERE status = ?1 ORDER BY submitted_at_unix ASC",
        params![CapabilityEventRunStatus::Queued.to_string()],
    )?;
    let mut running_lanes = BTreeMap::<String, ()>::new();
    for run in load_runs(
        conn,
        "SELECT run_id, repo_id, repo_root, consumer_id, capability_id, from_generation_seq, to_generation_seq, reconcile_mode, status, attempts, submitted_at_unix, started_at_unix, updated_at_unix, completed_at_unix, error FROM pack_reconcile_runs WHERE status = ?1",
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

fn build_execution_plan(
    conn: &rusqlite::Connection,
    run: &CapabilityEventRunRecord,
    repo_root: &std::path::Path,
) -> Result<Option<ExecutionPlan>> {
    let Some(cursor) = load_consumer_cursor(conn, &run.repo_id, &run.consumer_id)? else {
        return Ok(None);
    };
    let Some(latest_generation_seq) = latest_generation_seq(conn, &run.repo_id)? else {
        return Ok(None);
    };
    let from_generation_seq_exclusive = cursor.last_applied_generation_seq.unwrap_or(0);
    if latest_generation_seq <= from_generation_seq_exclusive {
        return Ok(None);
    }

    let generations = load_generations(
        conn,
        &run.repo_id,
        from_generation_seq_exclusive + 1,
        latest_generation_seq,
    )?;
    if generations.is_empty() {
        return Ok(None);
    }

    let file_changes = load_file_changes(
        conn,
        &run.repo_id,
        from_generation_seq_exclusive + 1,
        latest_generation_seq,
    )?;
    let artefact_changes = load_artefact_changes(
        conn,
        &run.repo_id,
        from_generation_seq_exclusive + 1,
        latest_generation_seq,
    )?;
    let merged_files = merge_file_changes(&file_changes);
    let merged_artefacts = merge_artefact_changes(&artefact_changes);
    let reconcile_mode = determine_reconcile_mode(
        cursor.last_applied_generation_seq,
        &generations,
        merged_files.len(),
        merged_artefacts.len(),
    );
    let (file_upserts, file_removals) = partition_file_changes(merged_files);
    let (artefact_upserts, artefact_removals) = partition_artefact_changes(merged_artefacts);
    let latest_generation = generations
        .last()
        .expect("checked non-empty generations before building execution plan");

    let mut record = run.clone();
    record.from_generation_seq = from_generation_seq_exclusive;
    record.to_generation_seq = latest_generation_seq;
    record.reconcile_mode = reconcile_mode_label(reconcile_mode).to_string();
    let run_id = record.run_id.clone();
    let reconcile_mode_label = record.reconcile_mode.clone();

    conn.execute(
        "UPDATE pack_reconcile_runs SET from_generation_seq = ?1, to_generation_seq = ?2, reconcile_mode = ?3, updated_at_unix = ?4 WHERE run_id = ?5",
        params![
            sql_i64(record.from_generation_seq)?,
            sql_i64(record.to_generation_seq)?,
            &reconcile_mode_label,
            sql_i64(unix_timestamp_now())?,
            &run_id,
        ],
    )
    .with_context(|| {
        format!(
            "refreshing current-state consumer execution bounds for `{}`",
            run_id
        )
    })?;

    Ok(Some(ExecutionPlan {
        record,
        repo_root: repo_root.to_path_buf(),
        request: CurrentStateConsumerRequest {
            repo_id: run.repo_id.clone(),
            repo_root: repo_root.to_path_buf(),
            active_branch: latest_generation.active_branch.clone(),
            head_commit_sha: latest_generation.head_commit_sha.clone(),
            from_generation_seq_exclusive,
            to_generation_seq_inclusive: latest_generation_seq,
            reconcile_mode,
            file_upserts,
            file_removals,
            artefact_upserts,
            artefact_removals,
        },
    }))
}

fn load_generations(
    conn: &rusqlite::Connection,
    repo_id: &str,
    from_generation_seq: u64,
    to_generation_seq: u64,
) -> Result<Vec<GenerationRow>> {
    let mut stmt = conn.prepare(
        "SELECT generation_seq, active_branch, head_commit_sha, requires_full_reconcile FROM pack_reconcile_generations WHERE repo_id = ?1 AND generation_seq >= ?2 AND generation_seq <= ?3 ORDER BY generation_seq ASC",
    )?;
    let rows = stmt
        .query_map(
            params![
                repo_id,
                sql_i64(from_generation_seq)?,
                sql_i64(to_generation_seq)?,
            ],
            |row| {
                Ok(GenerationRow {
                    generation_seq: u64::try_from(row.get::<_, i64>(0)?).unwrap_or_default(),
                    active_branch: row.get(1)?,
                    head_commit_sha: row.get(2)?,
                    requires_full_reconcile: row.get::<_, i64>(3)? != 0,
                })
            },
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn load_file_changes(
    conn: &rusqlite::Connection,
    repo_id: &str,
    from_generation_seq: u64,
    to_generation_seq: u64,
) -> Result<Vec<FileChangeRow>> {
    let mut stmt = conn.prepare(
        "SELECT generation_seq, path, change_kind, language, content_id FROM pack_reconcile_file_changes WHERE repo_id = ?1 AND generation_seq >= ?2 AND generation_seq <= ?3 ORDER BY generation_seq ASC, rowid ASC",
    )?;
    let rows = stmt
        .query_map(
            params![
                repo_id,
                sql_i64(from_generation_seq)?,
                sql_i64(to_generation_seq)?,
            ],
            |row| {
                Ok(FileChangeRow {
                    generation_seq: u64::try_from(row.get::<_, i64>(0)?).unwrap_or_default(),
                    path: row.get(1)?,
                    change_kind: row.get(2)?,
                    language: row.get(3)?,
                    content_id: row.get(4)?,
                })
            },
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn load_artefact_changes(
    conn: &rusqlite::Connection,
    repo_id: &str,
    from_generation_seq: u64,
    to_generation_seq: u64,
) -> Result<Vec<ArtefactChangeRow>> {
    let mut stmt = conn.prepare(
        "SELECT generation_seq, symbol_id, change_kind, artefact_id, path, canonical_kind, name FROM pack_reconcile_artefact_changes WHERE repo_id = ?1 AND generation_seq >= ?2 AND generation_seq <= ?3 ORDER BY generation_seq ASC, rowid ASC",
    )?;
    let rows = stmt
        .query_map(
            params![
                repo_id,
                sql_i64(from_generation_seq)?,
                sql_i64(to_generation_seq)?,
            ],
            |row| {
                Ok(ArtefactChangeRow {
                    generation_seq: u64::try_from(row.get::<_, i64>(0)?).unwrap_or_default(),
                    symbol_id: row.get(1)?,
                    change_kind: row.get(2)?,
                    artefact_id: row.get(3)?,
                    path: row.get(4)?,
                    canonical_kind: row.get(5)?,
                    name: row.get(6)?,
                })
            },
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn merge_file_changes(rows: &[FileChangeRow]) -> Vec<MergedFileChange> {
    let mut merged = BTreeMap::new();
    for row in rows {
        let _ = row.generation_seq;
        match row.change_kind.as_str() {
            "added" | "changed" => {
                if let (Some(language), Some(content_id)) = (&row.language, &row.content_id) {
                    merged.insert(
                        row.path.clone(),
                        MergedFileChange::Upsert(ChangedFile {
                            path: row.path.clone(),
                            language: language.clone(),
                            content_id: content_id.clone(),
                        }),
                    );
                }
            }
            "removed" => {
                merged.insert(
                    row.path.clone(),
                    MergedFileChange::Removed(RemovedFile {
                        path: row.path.clone(),
                    }),
                );
            }
            _ => {}
        }
    }
    merged.into_values().collect()
}

fn merge_artefact_changes(rows: &[ArtefactChangeRow]) -> Vec<MergedArtefactChange> {
    let mut merged = BTreeMap::new();
    for row in rows {
        let _ = row.generation_seq;
        match row.change_kind.as_str() {
            "added" | "changed" => {
                merged.insert(
                    row.symbol_id.clone(),
                    MergedArtefactChange::Upsert(ChangedArtefact {
                        artefact_id: row.artefact_id.clone(),
                        symbol_id: row.symbol_id.clone(),
                        path: row.path.clone(),
                        canonical_kind: row.canonical_kind.clone(),
                        name: row.name.clone(),
                    }),
                );
            }
            "removed" => {
                merged.insert(
                    row.symbol_id.clone(),
                    MergedArtefactChange::Removed(RemovedArtefact {
                        artefact_id: row.artefact_id.clone(),
                        symbol_id: row.symbol_id.clone(),
                        path: row.path.clone(),
                    }),
                );
            }
            _ => {}
        }
    }
    merged.into_values().collect()
}

fn partition_file_changes(merged: Vec<MergedFileChange>) -> (Vec<ChangedFile>, Vec<RemovedFile>) {
    let mut upserts = Vec::new();
    let mut removals = Vec::new();
    for change in merged {
        match change {
            MergedFileChange::Upsert(file) => upserts.push(file),
            MergedFileChange::Removed(file) => removals.push(file),
        }
    }
    (upserts, removals)
}

fn partition_artefact_changes(
    merged: Vec<MergedArtefactChange>,
) -> (Vec<ChangedArtefact>, Vec<RemovedArtefact>) {
    let mut upserts = Vec::new();
    let mut removals = Vec::new();
    for change in merged {
        match change {
            MergedArtefactChange::Upsert(artefact) => upserts.push(artefact),
            MergedArtefactChange::Removed(artefact) => removals.push(artefact),
        }
    }
    (upserts, removals)
}

fn determine_reconcile_mode(
    last_applied_generation_seq: Option<u64>,
    generations: &[GenerationRow],
    merged_file_count: usize,
    merged_artefact_count: usize,
) -> ReconcileMode {
    let Some(last_generation) = generations.last() else {
        return ReconcileMode::MergedDelta;
    };
    let pending_generation_span = last_generation
        .generation_seq
        .saturating_sub(last_applied_generation_seq.unwrap_or(0));
    if last_applied_generation_seq.is_none()
        || generations
            .iter()
            .any(|generation| generation.requires_full_reconcile)
        || pending_generation_span > 64
        || merged_file_count > 2_000
        || merged_artefact_count > 5_000
    {
        ReconcileMode::FullReconcile
    } else {
        ReconcileMode::MergedDelta
    }
}

fn reconcile_mode_label(mode: ReconcileMode) -> &'static str {
    match mode {
        ReconcileMode::MergedDelta => "merged_delta",
        ReconcileMode::FullReconcile => "full_reconcile",
    }
}

fn find_current_state_consumer<'a>(
    host: &'a DevqlCapabilityHost,
    run: &CapabilityEventRunRecord,
) -> Option<&'a Arc<dyn CurrentStateConsumer>> {
    host.current_state_consumers()
        .iter()
        .find(|registration| {
            registration.capability_id == run.capability_id
                && registration.consumer_id == run.consumer_id
        })
        .map(|registration| &registration.handler)
}

fn validate_consumer_result(
    request: &CurrentStateConsumerRequest,
    result: &CurrentStateConsumerResult,
) -> Result<()> {
    if result.applied_to_generation_seq < request.from_generation_seq_exclusive + 1
        || result.applied_to_generation_seq > request.to_generation_seq_inclusive
    {
        anyhow::bail!(
            "consumer applied generation {} outside requested range {}..={}",
            result.applied_to_generation_seq,
            request.from_generation_seq_exclusive + 1,
            request.to_generation_seq_inclusive
        );
    }
    Ok(())
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

fn count_runs_with_status(
    conn: &rusqlite::Connection,
    status: CapabilityEventRunStatus,
) -> Result<u64> {
    conn.query_row(
        "SELECT COUNT(*) FROM pack_reconcile_runs WHERE status = ?1",
        params![status.to_string()],
        |row| row.get::<_, i64>(0),
    )
    .map(|value| u64::try_from(value).unwrap_or_default())
    .map_err(anyhow::Error::from)
}

fn load_current_repo_run(
    conn: &rusqlite::Connection,
    repo_id: &str,
) -> Result<Option<CapabilityEventRunRecord>> {
    if let Some(run) = load_runs(
        conn,
        "SELECT run_id, repo_id, repo_root, consumer_id, capability_id, from_generation_seq, to_generation_seq, reconcile_mode, status, attempts, submitted_at_unix, started_at_unix, updated_at_unix, completed_at_unix, error FROM pack_reconcile_runs WHERE repo_id = ?1 AND status = ?2 ORDER BY submitted_at_unix ASC LIMIT 1",
        params![repo_id, CapabilityEventRunStatus::Running.to_string()],
    )?
    .into_iter()
    .next()
    {
        return Ok(Some(run.record));
    }

    Ok(load_runs(
        conn,
        "SELECT run_id, repo_id, repo_root, consumer_id, capability_id, from_generation_seq, to_generation_seq, reconcile_mode, status, attempts, submitted_at_unix, started_at_unix, updated_at_unix, completed_at_unix, error FROM pack_reconcile_runs WHERE repo_id = ?1 AND status = ?2 ORDER BY submitted_at_unix ASC LIMIT 1",
        params![repo_id, CapabilityEventRunStatus::Queued.to_string()],
    )?
    .into_iter()
    .next()
    .map(|run| run.record))
}

#[allow(dead_code)]
fn load_run_by_id(conn: &rusqlite::Connection, run_id: &str) -> Result<Option<StoredRunRecord>> {
    load_runs(
        conn,
        "SELECT run_id, repo_id, repo_root, consumer_id, capability_id, from_generation_seq, to_generation_seq, reconcile_mode, status, attempts, submitted_at_unix, started_at_unix, updated_at_unix, completed_at_unix, error FROM pack_reconcile_runs WHERE run_id = ?1 LIMIT 1",
        params![run_id],
    )
    .map(|mut runs| runs.pop())
}

fn load_runs<P>(conn: &rusqlite::Connection, sql: &str, params: P) -> Result<Vec<StoredRunRecord>>
where
    P: rusqlite::Params,
{
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map(params, |row| {
            Ok(StoredRunRecord {
                record: CapabilityEventRunRecord {
                    run_id: row.get(0)?,
                    repo_id: row.get(1)?,
                    capability_id: row.get(4)?,
                    consumer_id: row.get(3)?,
                    handler_id: row.get(3)?,
                    from_generation_seq: u64::try_from(row.get::<_, i64>(5)?).unwrap_or_default(),
                    to_generation_seq: u64::try_from(row.get::<_, i64>(6)?).unwrap_or_default(),
                    reconcile_mode: row.get(7)?,
                    event_kind: "current_state_consumer".to_string(),
                    lane_key: build_lane_key(&row.get::<_, String>(1)?, &row.get::<_, String>(3)?),
                    event_payload_json: String::new(),
                    status: parse_run_status(&row.get::<_, String>(8)?),
                    attempts: row.get(9)?,
                    submitted_at_unix: u64::try_from(row.get::<_, i64>(10)?).unwrap_or_default(),
                    started_at_unix: row
                        .get::<_, Option<i64>>(11)?
                        .and_then(|value| u64::try_from(value).ok()),
                    updated_at_unix: u64::try_from(row.get::<_, i64>(12)?).unwrap_or_default(),
                    completed_at_unix: row
                        .get::<_, Option<i64>>(13)?
                        .and_then(|value| u64::try_from(value).ok()),
                    error: row.get(14)?,
                },
                repo_root: row.get::<_, String>(2)?.into(),
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn build_lane_key(repo_id: &str, consumer_id: &str) -> String {
    format!("{repo_id}:{consumer_id}")
}

fn parse_run_status(value: &str) -> CapabilityEventRunStatus {
    match value {
        "running" => CapabilityEventRunStatus::Running,
        "completed" => CapabilityEventRunStatus::Completed,
        "failed" => CapabilityEventRunStatus::Failed,
        "cancelled" => CapabilityEventRunStatus::Cancelled,
        _ => CapabilityEventRunStatus::Queued,
    }
}

fn prune_terminal_runs(conn: &rusqlite::Connection) -> Result<()> {
    conn.execute(
        "DELETE FROM pack_reconcile_runs WHERE run_id IN (
            SELECT run_id FROM pack_reconcile_runs
            WHERE status IN (?1, ?2, ?3)
            ORDER BY COALESCE(completed_at_unix, updated_at_unix) DESC
            LIMIT -1 OFFSET 100
        )",
        params![
            CapabilityEventRunStatus::Completed.to_string(),
            CapabilityEventRunStatus::Failed.to_string(),
            CapabilityEventRunStatus::Cancelled.to_string(),
        ],
    )
    .context("pruning historical current-state consumer runs")?;
    Ok(())
}

fn sql_i64(value: u64) -> Result<i64> {
    i64::try_from(value).context("converting unsigned runtime value to SQLite integer")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_file_changes_keeps_latest_change_per_path() {
        let merged = merge_file_changes(&[
            FileChangeRow {
                generation_seq: 1,
                path: "src/lib.rs".to_string(),
                change_kind: "added".to_string(),
                language: Some("rust".to_string()),
                content_id: Some("a".to_string()),
            },
            FileChangeRow {
                generation_seq: 2,
                path: "src/lib.rs".to_string(),
                change_kind: "changed".to_string(),
                language: Some("rust".to_string()),
                content_id: Some("b".to_string()),
            },
            FileChangeRow {
                generation_seq: 3,
                path: "src/old.rs".to_string(),
                change_kind: "removed".to_string(),
                language: None,
                content_id: None,
            },
        ]);

        assert_eq!(
            merged,
            vec![
                MergedFileChange::Upsert(ChangedFile {
                    path: "src/lib.rs".to_string(),
                    language: "rust".to_string(),
                    content_id: "b".to_string(),
                }),
                MergedFileChange::Removed(RemovedFile {
                    path: "src/old.rs".to_string(),
                }),
            ]
        );
    }

    #[test]
    fn merge_artefact_changes_keeps_latest_change_per_symbol() {
        let merged = merge_artefact_changes(&[
            ArtefactChangeRow {
                generation_seq: 1,
                symbol_id: "symbol-a".to_string(),
                change_kind: "added".to_string(),
                artefact_id: "artefact-a".to_string(),
                path: "src/lib.rs".to_string(),
                canonical_kind: Some("function".to_string()),
                name: "create_user".to_string(),
            },
            ArtefactChangeRow {
                generation_seq: 2,
                symbol_id: "symbol-a".to_string(),
                change_kind: "removed".to_string(),
                artefact_id: "artefact-a".to_string(),
                path: "src/lib.rs".to_string(),
                canonical_kind: None,
                name: "create_user".to_string(),
            },
        ]);

        assert_eq!(
            merged,
            vec![MergedArtefactChange::Removed(RemovedArtefact {
                artefact_id: "artefact-a".to_string(),
                symbol_id: "symbol-a".to_string(),
                path: "src/lib.rs".to_string(),
            })]
        );
    }

    #[test]
    fn determine_reconcile_mode_promotes_full_reconcile_for_first_run_and_thresholds() {
        let generations = vec![GenerationRow {
            generation_seq: 65,
            active_branch: Some("main".to_string()),
            head_commit_sha: Some("abc123".to_string()),
            requires_full_reconcile: false,
        }];

        assert_eq!(
            determine_reconcile_mode(None, &generations, 1, 1),
            ReconcileMode::FullReconcile
        );
        assert_eq!(
            determine_reconcile_mode(Some(0), &generations, 1, 1),
            ReconcileMode::FullReconcile
        );
        assert_eq!(
            determine_reconcile_mode(Some(64), &generations, 1, 1),
            ReconcileMode::MergedDelta
        );
        assert_eq!(
            determine_reconcile_mode(Some(0), &generations, 2_001, 1),
            ReconcileMode::FullReconcile
        );
        assert_eq!(
            determine_reconcile_mode(Some(0), &generations, 1, 5_001),
            ReconcileMode::FullReconcile
        );
    }
}

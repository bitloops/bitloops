use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use anyhow::{Result, anyhow, bail};
use tokio::sync::Notify;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::time::{Duration, sleep};
use uuid::Uuid;

use crate::graphql::{Checkpoint, SubscriptionHub};
use crate::host::capability_host::{SyncArtefactDiff, SyncFileDiff};
use crate::host::devql::{
    DevqlConfig, IngestedCheckpointNotification, IngestionCounters, IngestionObserver,
    IngestionProgressPhase, IngestionProgressUpdate, RepoIdentity, SyncObserver, SyncProgressPhase,
    SyncProgressUpdate, SyncSummary,
};
#[cfg(test)]
use crate::utils::paths::default_global_runtime_db_path;

use super::super::types::{
    DevqlTaskControlResult, DevqlTaskKind, DevqlTaskProgress, DevqlTaskQueueStatus,
    DevqlTaskRecord, DevqlTaskResult, DevqlTaskSource, DevqlTaskSpec, DevqlTaskStatus,
    EmbeddingsBootstrapPhase, EmbeddingsBootstrapProgress, EmbeddingsBootstrapResult,
    RepoTaskControlState, SyncTaskMode, SyncTaskSpec, unix_timestamp_now,
};
use super::queue::{
    changed_tasks, default_progress_for_spec, failed_progress, ingest_progress_from_summary,
    merge_existing_task, next_runnable_task_indexes, project_status, prune_terminal_tasks,
    recompute_queue_positions, sync_progress_from_summary, sync_task_mode_from_host,
    sync_task_mode_to_host,
};
use crate::host::runtime_store::{DaemonSqliteRuntimeStore, PersistedDevqlTaskQueueState};

const WORKER_POLL_INTERVAL: Duration = Duration::from_secs(2);
const PROGRESS_PERSIST_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, Clone)]
pub struct DevqlTaskEnqueueResult {
    pub task: DevqlTaskRecord,
    pub merged: bool,
}

#[derive(Debug)]
pub struct DevqlTaskCoordinator {
    pub(super) runtime_store: DaemonSqliteRuntimeStore,
    pub(super) lock: Mutex<()>,
    pub(super) notify: Notify,
    pub(super) worker_started: AtomicBool,
    pub(super) subscription_hub: Mutex<Option<Arc<SubscriptionHub>>>,
}

struct SyncCoordinatorObserver {
    coordinator: Arc<DevqlTaskCoordinator>,
    task_id: String,
    progress_state: Mutex<ProgressPersistState<SyncProgressUpdate>>,
}

struct IngestCoordinatorObserver {
    coordinator: Arc<DevqlTaskCoordinator>,
    task_id: String,
    repo_name: String,
    progress_state: Mutex<ProgressPersistState<IngestionProgressUpdate>>,
}

#[derive(Debug)]
struct ProgressPersistState<T> {
    last_persisted: Option<T>,
    last_persisted_at: Option<Instant>,
}

impl<T> Default for ProgressPersistState<T> {
    fn default() -> Self {
        Self {
            last_persisted: None,
            last_persisted_at: None,
        }
    }
}

struct WorkerStartedGuard {
    coordinator: Arc<DevqlTaskCoordinator>,
}

impl Drop for WorkerStartedGuard {
    fn drop(&mut self) {
        self.coordinator
            .worker_started
            .store(false, Ordering::SeqCst);
    }
}

impl SyncObserver for SyncCoordinatorObserver {
    fn on_progress(&self, update: SyncProgressUpdate) {
        match self.progress_state.lock() {
            Ok(mut state) => {
                let now = Instant::now();
                if !should_persist_progress(
                    state.last_persisted.as_ref(),
                    &update,
                    state.last_persisted_at,
                    now,
                ) {
                    return;
                }
                state.last_persisted = Some(update.clone());
                state.last_persisted_at = Some(now);
            }
            Err(_) => {
                log::warn!(
                    "failed to acquire sync progress throttle state for task `{}`",
                    self.task_id
                );
            }
        }

        if let Err(err) = self
            .coordinator
            .update_task_progress(&self.task_id, DevqlTaskProgress::Sync(update))
        {
            log::warn!(
                "failed to persist sync progress for task `{}`: {err:#}",
                self.task_id
            );
        }
    }
}

impl IngestionObserver for IngestCoordinatorObserver {
    fn on_progress(&self, update: IngestionProgressUpdate) {
        match self.progress_state.lock() {
            Ok(mut state) => {
                let now = Instant::now();
                if !should_persist_progress(
                    state.last_persisted.as_ref(),
                    &update,
                    state.last_persisted_at,
                    now,
                ) {
                    return;
                }
                state.last_persisted = Some(update.clone());
                state.last_persisted_at = Some(now);
            }
            Err(_) => {
                log::warn!(
                    "failed to acquire ingest progress throttle state for task `{}`",
                    self.task_id
                );
            }
        }

        if let Err(err) = self
            .coordinator
            .update_task_progress(&self.task_id, DevqlTaskProgress::Ingest(update))
        {
            log::warn!(
                "failed to persist ingest progress for task `{}`: {err:#}",
                self.task_id
            );
        }
    }

    fn on_checkpoint_ingested(&self, checkpoint: IngestedCheckpointNotification) {
        self.coordinator.publish_checkpoint(
            self.repo_name.clone(),
            Checkpoint::from_ingested(&checkpoint.checkpoint, checkpoint.commit_sha.as_deref()),
        );
    }
}

impl DevqlTaskCoordinator {
    pub(crate) fn shared() -> Arc<Self> {
        static INSTANCE: OnceLock<Mutex<Arc<DevqlTaskCoordinator>>> = OnceLock::new();
        let slot = INSTANCE.get_or_init(|| {
            let runtime_store = DaemonSqliteRuntimeStore::open()
                .expect("opening daemon runtime store for DevQL tasks");
            Mutex::new(Self::new_shared_instance(runtime_store))
        });
        let coordinator = slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

        #[cfg(test)]
        let mut coordinator = coordinator;

        #[cfg(test)]
        {
            let runtime_db_path = default_global_runtime_db_path();
            if coordinator.runtime_store.db_path() != runtime_db_path.as_path() {
                let runtime_store = DaemonSqliteRuntimeStore::open_at(runtime_db_path)
                    .expect("opening daemon runtime store for DevQL tasks");
                *coordinator = Self::new_shared_instance(runtime_store);
            }
        }

        Arc::clone(&coordinator)
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

    pub(crate) fn activate_worker(
        self: &Arc<Self>,
        config_root: &Path,
        hub: Option<Arc<SubscriptionHub>>,
    ) {
        if let Some(hub) = hub {
            self.register_subscription_hub(hub);
        }
        if self.worker_started.swap(true, Ordering::SeqCst) {
            return;
        }
        if let Err(err) = self.recover_running_tasks() {
            log::warn!("failed to recover queued DevQL tasks: {err:#}");
        }
        if let Err(err) = crate::host::devql::recover_running_producer_spool_jobs(config_root) {
            log::warn!("failed to recover DevQL producer spool jobs: {err:#}");
        }
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            self.worker_started.store(false, Ordering::SeqCst);
            log::warn!("DevQL task worker activation requested without an active tokio runtime");
            return;
        };
        let coordinator = Arc::clone(self);
        let producer_spool_config_root = config_root.to_path_buf();
        handle.spawn(async move {
            let _guard = WorkerStartedGuard {
                coordinator: Arc::clone(&coordinator),
            };
            coordinator.run_loop(producer_spool_config_root).await;
        });
    }

    pub(crate) fn register_subscription_hub(&self, hub: Arc<SubscriptionHub>) {
        if let Ok(mut slot) = self.subscription_hub.lock() {
            *slot = Some(hub);
        }
    }

    pub(crate) fn enqueue(
        &self,
        cfg: &DevqlConfig,
        source: DevqlTaskSource,
        spec: DevqlTaskSpec,
    ) -> Result<DevqlTaskEnqueueResult> {
        let kind = task_kind_from_spec(&spec);
        self.mutate_state(|state| {
            if let Some(task) = merge_existing_task(state, cfg, source, kind, &spec) {
                return Ok(DevqlTaskEnqueueResult { task, merged: true });
            }

            let now = unix_timestamp_now();
            let task = DevqlTaskRecord {
                task_id: format!("{kind}-task-{}", Uuid::new_v4()),
                repo_id: cfg.repo.repo_id.clone(),
                repo_name: cfg.repo.name.clone(),
                repo_provider: cfg.repo.provider.clone(),
                repo_organisation: cfg.repo.organization.clone(),
                repo_identity: cfg.repo.identity.clone(),
                daemon_config_root: cfg.daemon_config_root.clone(),
                repo_root: cfg.repo_root.clone(),
                kind,
                source,
                spec: spec.clone(),
                status: DevqlTaskStatus::Queued,
                submitted_at_unix: now,
                started_at_unix: None,
                updated_at_unix: now,
                completed_at_unix: None,
                queue_position: None,
                tasks_ahead: None,
                progress: default_progress_for_spec(&spec),
                error: None,
                result: None,
            };
            state.tasks.push(task.clone());
            state.last_action = Some("enqueue".to_string());
            Ok(DevqlTaskEnqueueResult {
                task,
                merged: false,
            })
        })
    }

    pub(crate) fn snapshot(&self, repo_id: Option<&str>) -> Result<DevqlTaskQueueStatus> {
        let state = self.load_state()?;
        let persisted = self.runtime_store.devql_task_state_exists()?;
        Ok(project_status(&state, repo_id, persisted))
    }

    pub(crate) fn task(&self, task_id: &str) -> Result<Option<DevqlTaskRecord>> {
        let state = self.load_state()?;
        Ok(state.tasks.into_iter().find(|task| task.task_id == task_id))
    }

    pub(crate) fn tasks(
        &self,
        repo_id: Option<&str>,
        kind: Option<DevqlTaskKind>,
        status: Option<DevqlTaskStatus>,
        limit: Option<usize>,
    ) -> Result<Vec<DevqlTaskRecord>> {
        let state = self.load_state()?;
        let mut tasks = state
            .tasks
            .into_iter()
            .filter(|task| repo_id.is_none_or(|repo_id| task.repo_id == repo_id))
            .filter(|task| kind.is_none_or(|kind| task.kind == kind))
            .filter(|task| status.is_none_or(|status| task.status == status))
            .collect::<Vec<_>>();
        tasks.sort_by(|left, right| {
            right
                .updated_at_unix
                .cmp(&left.updated_at_unix)
                .then_with(|| left.task_id.cmp(&right.task_id))
        });
        if let Some(limit) = limit {
            tasks.truncate(limit);
        }
        Ok(tasks)
    }

    pub(crate) fn pause_repo(
        &self,
        repo_id: &str,
        reason: Option<String>,
    ) -> Result<DevqlTaskControlResult> {
        let repo_id = repo_id.to_string();
        self.mutate_state(|state| {
            let control = state
                .repo_controls
                .entry(repo_id.clone())
                .or_insert_with(|| RepoTaskControlState {
                    repo_id: repo_id.clone(),
                    paused: false,
                    paused_reason: None,
                    updated_at_unix: 0,
                });
            control.paused = true;
            control.paused_reason = reason.clone();
            control.updated_at_unix = unix_timestamp_now();
            state.last_action = Some("paused".to_string());
            Ok(DevqlTaskControlResult {
                message: format!("paused DevQL task queue for {repo_id}"),
                control: control.clone(),
            })
        })
    }

    pub(crate) fn resume_repo(&self, repo_id: &str) -> Result<DevqlTaskControlResult> {
        let repo_id = repo_id.to_string();
        self.mutate_state(|state| {
            let control = state
                .repo_controls
                .entry(repo_id.clone())
                .or_insert_with(|| RepoTaskControlState {
                    repo_id: repo_id.clone(),
                    paused: false,
                    paused_reason: None,
                    updated_at_unix: 0,
                });
            control.paused = false;
            control.paused_reason = None;
            control.updated_at_unix = unix_timestamp_now();
            state.last_action = Some("resumed".to_string());
            Ok(DevqlTaskControlResult {
                message: format!("resumed DevQL task queue for {repo_id}"),
                control: control.clone(),
            })
        })
    }

    pub(crate) fn cancel_task(&self, task_id: &str) -> Result<DevqlTaskRecord> {
        let task_id = task_id.to_string();
        self.mutate_state(|state| {
            let Some(task) = state.tasks.iter_mut().find(|task| task.task_id == task_id) else {
                bail!("unknown task `{task_id}`");
            };
            match task.status {
                DevqlTaskStatus::Queued => {
                    let now = unix_timestamp_now();
                    task.status = DevqlTaskStatus::Cancelled;
                    task.updated_at_unix = now;
                    task.completed_at_unix = Some(now);
                    task.error = None;
                    task.result = None;
                    state.last_action = Some("cancelled".to_string());
                    Ok(task.clone())
                }
                DevqlTaskStatus::Running => {
                    bail!("task `{task_id}` is already running and cannot be cancelled")
                }
                _ => bail!("task `{task_id}` is not queued and cannot be cancelled"),
            }
        })
    }

    async fn run_loop(self: Arc<Self>, producer_spool_config_root: std::path::PathBuf) {
        loop {
            let mut made_progress = false;

            match self.schedule_pending_producer_spool_jobs(&producer_spool_config_root) {
                Ok(progressed) => made_progress |= progressed,
                Err(err) => log::warn!("daemon DevQL producer spool worker error: {err:#}"),
            }
            match self.schedule_pending_tasks() {
                Ok(progressed) => made_progress |= progressed,
                Err(err) => log::warn!("daemon DevQL task worker error: {err:#}"),
            }

            if made_progress {
                continue;
            }
            tokio::select! {
                _ = self.notify.notified() => {},
                _ = sleep(WORKER_POLL_INTERVAL) => {},
            }
        }
    }

    fn schedule_pending_producer_spool_jobs(self: &Arc<Self>, config_root: &Path) -> Result<bool> {
        let jobs = crate::host::devql::claim_next_producer_spool_jobs(config_root)?;
        if jobs.is_empty() {
            return Ok(false);
        }

        for job in jobs {
            let coordinator = Arc::clone(self);
            tokio::spawn(async move {
                if let Err(err) = coordinator.run_producer_spool_job(job).await {
                    log::warn!("DevQL producer spool execution failed: {err:#}");
                }
            });
        }

        Ok(true)
    }

    fn schedule_pending_tasks(self: &Arc<Self>) -> Result<bool> {
        let tasks = self.mutate_state(|state| {
            let indexes = next_runnable_task_indexes(state);
            if indexes.is_empty() {
                return Ok(Vec::new());
            }

            let now = unix_timestamp_now();
            let mut scheduled = Vec::with_capacity(indexes.len());
            for index in indexes {
                let mut task = state.tasks[index].clone();
                task.status = DevqlTaskStatus::Running;
                task.started_at_unix = Some(task.started_at_unix.unwrap_or(now));
                task.updated_at_unix = now;
                task.error = None;
                task.completed_at_unix = None;
                task.result = None;
                state.tasks[index] = task.clone();
                scheduled.push(task);
            }
            state.last_action = Some("running".to_string());
            Ok(scheduled)
        })?;

        if tasks.is_empty() {
            return Ok(false);
        }

        for task in tasks {
            let coordinator = Arc::clone(self);
            tokio::spawn(async move {
                if let Err(err) = coordinator.run_task(task).await {
                    log::warn!("DevQL task execution failed: {err:#}");
                }
            });
        }

        Ok(true)
    }

    async fn run_task(self: Arc<Self>, task: DevqlTaskRecord) -> Result<()> {
        match task.kind {
            DevqlTaskKind::Sync => self.run_sync_task(task).await,
            DevqlTaskKind::Ingest => self.run_ingest_task(task).await,
            DevqlTaskKind::EmbeddingsBootstrap => self.run_embeddings_bootstrap_task(task).await,
        }
    }

    async fn run_producer_spool_job(
        self: Arc<Self>,
        job: crate::host::devql::ProducerSpoolJobRecord,
    ) -> Result<()> {
        let outcome = self.process_producer_spool_job(&job).await;
        match outcome {
            Ok(()) => {
                if let Err(err) =
                    crate::host::devql::delete_producer_spool_job(&job.config_root, &job.job_id)
                {
                    log::warn!(
                        "failed to delete completed DevQL producer spool job `{}`: {err:#}",
                        job.job_id
                    );
                    if let Err(requeue_err) = crate::host::devql::requeue_producer_spool_job(
                        &job.config_root,
                        &job.job_id,
                        &err,
                    ) {
                        log::warn!(
                            "failed to requeue DevQL producer spool job `{}` after delete failure: {requeue_err:#}",
                            job.job_id
                        );
                    }
                }
            }
            Err(err) => {
                log::warn!("DevQL producer spool job `{}` failed: {err:#}", job.job_id);
                if let Err(requeue_err) = crate::host::devql::requeue_producer_spool_job(
                    &job.config_root,
                    &job.job_id,
                    &err,
                ) {
                    log::warn!(
                        "failed to requeue DevQL producer spool job `{}`: {requeue_err:#}",
                        job.job_id
                    );
                }
            }
        }

        self.notify.notify_waiters();
        Ok(())
    }

    async fn process_producer_spool_job(
        &self,
        job: &crate::host::devql::ProducerSpoolJobRecord,
    ) -> Result<()> {
        match &job.payload {
            crate::host::devql::ProducerSpoolJobPayload::Task { source, spec } => {
                let cfg = self.devql_config_from_producer_spool_job(job)?;
                self.enqueue(&cfg, *source, spec.clone())?;
                Ok(())
            }
            crate::host::devql::ProducerSpoolJobPayload::PostCommitRefresh {
                commit_sha,
                changed_files,
            } => {
                let cfg = self.devql_config_from_producer_spool_job(job)?;
                crate::host::checkpoints::strategy::manual_commit::execute_devql_post_commit_refresh(
                    &cfg,
                    commit_sha,
                    changed_files,
                )
                .await
            }
            crate::host::devql::ProducerSpoolJobPayload::PostMergeRefresh {
                head_sha,
                changed_files,
            } => {
                let cfg = self.devql_config_from_producer_spool_job(job)?;
                crate::host::checkpoints::strategy::manual_commit::execute_devql_post_merge_refresh(
                    &cfg,
                    head_sha,
                    changed_files,
                )
                .await
            }
            crate::host::devql::ProducerSpoolJobPayload::PrePushSync {
                remote,
                stdin_lines,
            } => {
                crate::host::checkpoints::strategy::manual_commit::execute_devql_pre_push_sync(
                    &job.repo_root,
                    remote,
                    stdin_lines,
                )
                .await
            }
        }
    }

    fn devql_config_from_producer_spool_job(
        &self,
        job: &crate::host::devql::ProducerSpoolJobRecord,
    ) -> Result<DevqlConfig> {
        let repo = RepoIdentity {
            repo_id: job.repo_id.clone(),
            name: job.repo_name.clone(),
            provider: job.repo_provider.clone(),
            organization: job.repo_organisation.clone(),
            identity: job.repo_identity.clone(),
        };
        DevqlConfig::from_roots(job.config_root.clone(), job.repo_root.clone(), repo)
    }

    async fn run_sync_task(self: Arc<Self>, task: DevqlTaskRecord) -> Result<()> {
        self.update_task_progress(
            &task.task_id,
            DevqlTaskProgress::Sync(SyncProgressUpdate {
                phase: SyncProgressPhase::EnsuringSchema,
                ..task
                    .sync_progress()
                    .cloned()
                    .unwrap_or_else(SyncProgressUpdate::default)
            }),
        )?;

        let repo = RepoIdentity {
            repo_id: task.repo_id.clone(),
            name: task.repo_name.clone(),
            provider: task.repo_provider.clone(),
            organization: task.repo_organisation.clone(),
            identity: task.repo_identity.clone(),
        };
        let cfg = DevqlConfig::from_roots(
            task.daemon_config_root.clone(),
            task.repo_root.clone(),
            repo,
        )?;
        let requested_mode = sync_task_mode_to_host(
            &task
                .sync_spec()
                .map(|spec| &spec.mode)
                .cloned()
                .ok_or_else(|| anyhow!("sync task missing sync spec"))?,
        );

        let schema_outcome = match crate::host::devql::prepare_sync_execution_schema(
            &cfg,
            "queued DevQL sync",
            &requested_mode,
        )
        .await
        {
            Ok(outcome) => outcome,
            Err(err) => {
                self.finish_task_failed(&task.task_id, err)?;
                return Ok(());
            }
        };
        let effective_mode = crate::host::devql::effective_sync_mode_after_schema_preparation(
            requested_mode,
            schema_outcome,
        );
        let effective_spec = sync_task_mode_from_host(&effective_mode);
        if task
            .sync_spec()
            .is_none_or(|spec| spec.mode != effective_spec)
        {
            self.update_sync_mode(&task.task_id, effective_spec)?;
        }

        let observer = SyncCoordinatorObserver {
            coordinator: Arc::clone(&self),
            task_id: task.task_id.clone(),
            progress_state: Mutex::new(ProgressPersistState::default()),
        };

        let host = match crate::host::devql::build_capability_host(&cfg.repo_root, cfg.repo.clone())
        {
            Ok(host) => Some(host),
            Err(err) => {
                log::warn!(
                    "failed to build capability host for sync event dispatch (task_id={}): {err:#}",
                    task.task_id
                );
                None
            }
        };

        match crate::host::devql::run_sync_with_summary_and_observer_and_diffs(
            &cfg,
            effective_mode,
            Some(&observer),
        )
        .await
        {
            Ok((summary, file_diff, artefact_diff)) => {
                if let Some(host) = host.as_ref() {
                    let capability_event_coordinator =
                        crate::daemon::shared_capability_event_coordinator();
                    capability_event_coordinator.activate_worker();
                    if let Err(err) = enqueue_sync_completed_runs(
                        capability_event_coordinator.as_ref(),
                        host,
                        &cfg,
                        &task.task_id,
                        &summary,
                        file_diff,
                        artefact_diff,
                    ) {
                        log::warn!(
                            "failed to enqueue sync current-state consumer runs (task_id={}): {err:#}",
                            task.task_id
                        );
                    }
                }
                self.finish_sync_task_completed(&task.task_id, summary)?
            }
            Err(err) => self.finish_task_failed(&task.task_id, err)?,
        }

        Ok(())
    }

    async fn run_ingest_task(self: Arc<Self>, task: DevqlTaskRecord) -> Result<()> {
        let repo = RepoIdentity {
            repo_id: task.repo_id.clone(),
            name: task.repo_name.clone(),
            provider: task.repo_provider.clone(),
            organization: task.repo_organisation.clone(),
            identity: task.repo_identity.clone(),
        };
        let cfg = DevqlConfig::from_roots(
            task.daemon_config_root.clone(),
            task.repo_root.clone(),
            repo,
        )?;
        let observer = IngestCoordinatorObserver {
            coordinator: Arc::clone(&self),
            task_id: task.task_id.clone(),
            repo_name: task.repo_name.clone(),
            progress_state: Mutex::new(ProgressPersistState::default()),
        };
        let backfill = task.ingest_spec().and_then(|spec| spec.backfill);

        let result = match backfill {
            Some(backfill) => {
                crate::host::devql::execute_ingest_with_backfill_window(
                    &cfg,
                    false,
                    backfill,
                    Some(&observer),
                    Some(crate::daemon::shared_enrichment_coordinator()),
                )
                .await
            }
            None => {
                crate::host::devql::execute_ingest_with_observer(
                    &cfg,
                    false,
                    0,
                    Some(&observer),
                    Some(crate::daemon::shared_enrichment_coordinator()),
                )
                .await
            }
        };

        match result {
            Ok(summary) => self.finish_ingest_task_completed(&task.task_id, summary)?,
            Err(err) => self.finish_task_failed(&task.task_id, err)?,
        }
        Ok(())
    }

    async fn run_embeddings_bootstrap_task(self: Arc<Self>, task: DevqlTaskRecord) -> Result<()> {
        let spec = task
            .embeddings_bootstrap_spec()
            .cloned()
            .ok_or_else(|| anyhow!("embeddings bootstrap task missing spec"))?;
        let catch_up_config_path = spec.config_path.clone();
        let needs_repo_catch_up =
            crate::daemon::embeddings_bootstrap::repo_catch_up_required_for_bootstrap(
                &spec.config_path,
                &spec.profile_name,
            )?;
        let task_id = task.task_id.clone();
        let runtime_store = self.runtime_store.clone();
        let repo_root = task.repo_root.clone();
        let (progress_tx, progress_rx) = mpsc::unbounded_channel();
        let execution = tokio::task::spawn_blocking(move || {
            crate::daemon::embeddings_bootstrap::execute_task_with_progress(
                &runtime_store,
                &repo_root,
                &task_id,
                &spec,
                |progress| {
                    progress_tx
                        .send(progress)
                        .map_err(|_| anyhow!("embeddings bootstrap progress receiver dropped"))?;
                    Ok(())
                },
            )
        });
        let (result_tx, result_rx) = oneshot::channel();
        tokio::spawn(async move {
            let result = execution
                .await
                .map_err(|err| anyhow!("embeddings bootstrap worker join failed: {err:#}"))
                .and_then(|result| result);
            let _ = result_tx.send(result);
        });

        let final_result =
            receive_embeddings_bootstrap_outcome(progress_rx, result_rx, |progress| {
                self.update_task_progress(
                    &task.task_id,
                    DevqlTaskProgress::EmbeddingsBootstrap(progress),
                )
            })
            .await?;

        match final_result {
            Ok(result) => {
                if needs_repo_catch_up {
                    crate::daemon::embeddings_bootstrap::enqueue_repo_catch_up_after_bootstrap(
                        &task.repo_root,
                        &catch_up_config_path,
                    )
                    .await?;
                }
                self.finish_embeddings_bootstrap_task_completed(&task.task_id, result)?
            }
            Err(err) => self.finish_task_failed(&task.task_id, err)?,
        }

        Ok(())
    }

    fn update_sync_mode(&self, task_id: &str, mode: SyncTaskMode) -> Result<()> {
        let task_id = task_id.to_string();
        self.mutate_state(|state| {
            let Some(task) = state.tasks.iter_mut().find(|task| task.task_id == task_id) else {
                return Ok(());
            };
            if let Some(spec) = sync_spec_from_task_spec_mut(&mut task.spec) {
                spec.mode = mode;
            }
            task.updated_at_unix = unix_timestamp_now();
            state.last_action = Some("mode_updated".to_string());
            Ok(())
        })
        .map(|_: ()| ())
    }

    pub(super) fn recover_running_tasks(&self) -> Result<()> {
        self.mutate_state(|state| {
            for task in &mut state.tasks {
                if task.status == DevqlTaskStatus::Running {
                    task.status = DevqlTaskStatus::Queued;
                    task.progress = default_progress_for_spec(&task.spec);
                    task.error = None;
                    task.result = None;
                    task.started_at_unix = None;
                    task.completed_at_unix = None;
                    task.updated_at_unix = unix_timestamp_now();
                }
            }
            state.last_action = Some("recovered_running_tasks".to_string());
            Ok(())
        })
        .map(|_: ()| ())
    }

    fn update_task_progress(&self, task_id: &str, update: DevqlTaskProgress) -> Result<()> {
        let task_id = task_id.to_string();
        self.mutate_state(|state| {
            let Some(task) = state.tasks.iter_mut().find(|task| task.task_id == task_id) else {
                return Ok(());
            };
            task.progress = update.clone();
            task.updated_at_unix = unix_timestamp_now();
            state.last_action = Some(progress_action(&update));
            Ok(())
        })
        .map(|_: ()| ())
    }

    fn finish_sync_task_completed(&self, task_id: &str, summary: SyncSummary) -> Result<()> {
        let task_id = task_id.to_string();
        self.mutate_state(|state| {
            let Some(task) = state.tasks.iter_mut().find(|task| task.task_id == task_id) else {
                return Ok(());
            };
            let now = unix_timestamp_now();
            task.status = DevqlTaskStatus::Completed;
            task.updated_at_unix = now;
            task.completed_at_unix = Some(now);
            task.error = None;
            task.result = Some(DevqlTaskResult::Sync(Box::new(summary.clone())));
            task.progress = DevqlTaskProgress::Sync(sync_progress_from_summary(&summary));
            state.last_action = Some("completed".to_string());
            Ok(())
        })
        .map(|_: ()| ())
    }

    fn finish_ingest_task_completed(
        &self,
        task_id: &str,
        summary: IngestionCounters,
    ) -> Result<()> {
        let task_id = task_id.to_string();
        self.mutate_state(|state| {
            let Some(task) = state.tasks.iter_mut().find(|task| task.task_id == task_id) else {
                return Ok(());
            };
            let now = unix_timestamp_now();
            task.status = DevqlTaskStatus::Completed;
            task.updated_at_unix = now;
            task.completed_at_unix = Some(now);
            task.error = None;
            task.result = Some(DevqlTaskResult::Ingest(summary.clone()));
            task.progress = DevqlTaskProgress::Ingest(ingest_progress_from_summary(&summary));
            state.last_action = Some("completed".to_string());
            Ok(())
        })
        .map(|_: ()| ())
    }

    fn finish_embeddings_bootstrap_task_completed(
        &self,
        task_id: &str,
        result: EmbeddingsBootstrapResult,
    ) -> Result<()> {
        let task_id = task_id.to_string();
        self.mutate_state(|state| {
            let Some(task) = state.tasks.iter_mut().find(|task| task.task_id == task_id) else {
                return Ok(());
            };
            let now = unix_timestamp_now();
            task.status = DevqlTaskStatus::Completed;
            task.updated_at_unix = now;
            task.completed_at_unix = Some(now);
            task.error = None;
            task.result = Some(DevqlTaskResult::EmbeddingsBootstrap(result.clone()));
            task.progress = DevqlTaskProgress::EmbeddingsBootstrap(EmbeddingsBootstrapProgress {
                phase: EmbeddingsBootstrapPhase::Complete,
                asset_name: None,
                bytes_downloaded: 0,
                bytes_total: None,
                version: result.version.clone(),
                message: Some(result.message.clone()),
            });
            state.last_action = Some("completed".to_string());
            Ok(())
        })
        .map(|_: ()| ())
    }

    fn finish_task_failed(&self, task_id: &str, err: anyhow::Error) -> Result<()> {
        let task_id = task_id.to_string();
        let error = format!("{err:#}");
        self.mutate_state(|state| {
            let Some(task) = state.tasks.iter_mut().find(|task| task.task_id == task_id) else {
                return Ok(());
            };
            let now = unix_timestamp_now();
            task.status = DevqlTaskStatus::Failed;
            task.updated_at_unix = now;
            task.completed_at_unix = Some(now);
            task.error = Some(error.clone());
            task.result = None;
            task.progress = failed_progress(&task.progress);
            state.last_action = Some("failed".to_string());
            Ok(())
        })
        .map(|_: ()| ())
    }

    fn load_state(&self) -> Result<PersistedDevqlTaskQueueState> {
        Ok(self
            .runtime_store
            .load_devql_task_queue_state()?
            .unwrap_or_else(PersistedDevqlTaskQueueState::default))
    }

    fn mutate_state<T>(
        &self,
        mutate: impl FnOnce(&mut PersistedDevqlTaskQueueState) -> Result<T>,
    ) -> Result<T> {
        let guard = self
            .lock
            .lock()
            .map_err(|_| anyhow!("DevQL task coordinator lock poisoned"))?;
        let (result, tasks_to_publish) =
            self.runtime_store.mutate_devql_task_queue_state(|state| {
                let previous_tasks = state.tasks.clone();
                let result = mutate(state)?;
                let tasks_to_publish = Self::save_state(state, &previous_tasks)?;
                Ok((result, tasks_to_publish))
            })?;
        drop(guard);
        self.publish_tasks(tasks_to_publish);
        self.notify.notify_waiters();
        Ok(result)
    }

    fn save_state(
        state: &mut PersistedDevqlTaskQueueState,
        previous_tasks: &[DevqlTaskRecord],
    ) -> Result<Vec<DevqlTaskRecord>> {
        state.version = 1;
        state.updated_at_unix = unix_timestamp_now();
        recompute_queue_positions(&mut state.tasks);
        prune_terminal_tasks(&mut state.tasks);
        Ok(changed_tasks(previous_tasks, &state.tasks))
    }

    fn publish_tasks(&self, tasks: Vec<DevqlTaskRecord>) {
        let Some(hub) = self
            .subscription_hub
            .lock()
            .ok()
            .and_then(|slot| slot.clone())
        else {
            return;
        };
        for task in tasks {
            hub.publish_task(task);
        }
    }

    fn publish_checkpoint(&self, repo_name: String, checkpoint: Checkpoint) {
        let Some(hub) = self
            .subscription_hub
            .lock()
            .ok()
            .and_then(|slot| slot.clone())
        else {
            return;
        };
        hub.publish_checkpoint(repo_name, checkpoint);
    }
}

async fn receive_embeddings_bootstrap_outcome<R>(
    mut progress_rx: mpsc::UnboundedReceiver<EmbeddingsBootstrapProgress>,
    mut result_rx: oneshot::Receiver<Result<EmbeddingsBootstrapResult>>,
    mut on_progress: R,
) -> Result<Result<EmbeddingsBootstrapResult>>
where
    R: FnMut(EmbeddingsBootstrapProgress) -> Result<()>,
{
    let mut progress_closed = false;
    let mut final_result = None;

    while final_result.is_none() || !progress_closed {
        tokio::select! {
            maybe_progress = progress_rx.recv(), if !progress_closed => {
                match maybe_progress {
                    Some(progress) => on_progress(progress)?,
                    None => progress_closed = true,
                }
            }
            result = &mut result_rx, if final_result.is_none() => {
                final_result = Some(
                    result.map_err(|_| anyhow!("embeddings bootstrap worker result channel dropped"))?
                );
            }
        }
    }

    final_result.ok_or_else(|| anyhow!("embeddings bootstrap task exited without a result"))
}

fn task_kind_from_spec(spec: &DevqlTaskSpec) -> DevqlTaskKind {
    match spec {
        DevqlTaskSpec::Sync(_) => DevqlTaskKind::Sync,
        DevqlTaskSpec::Ingest(_) => DevqlTaskKind::Ingest,
        DevqlTaskSpec::EmbeddingsBootstrap(_) => DevqlTaskKind::EmbeddingsBootstrap,
    }
}

fn progress_action(update: &DevqlTaskProgress) -> String {
    match update {
        DevqlTaskProgress::Sync(update) => update.phase.as_str().to_string(),
        DevqlTaskProgress::Ingest(update) => match update.phase {
            IngestionProgressPhase::Initializing => "initializing".to_string(),
            IngestionProgressPhase::Extracting => "extracting".to_string(),
            IngestionProgressPhase::Persisting => "persisting".to_string(),
            IngestionProgressPhase::Complete => "complete".to_string(),
            IngestionProgressPhase::Failed => "failed".to_string(),
        },
        DevqlTaskProgress::EmbeddingsBootstrap(update) => update.phase.as_str().to_string(),
    }
}

fn enqueue_sync_completed_runs(
    coordinator: &crate::daemon::CapabilityEventCoordinator,
    host: &crate::host::capability_host::DevqlCapabilityHost,
    cfg: &DevqlConfig,
    source_task_id: &str,
    summary: &SyncSummary,
    file_diff: SyncFileDiff,
    artefact_diff: SyncArtefactDiff,
) -> Result<usize> {
    let runs = coordinator.record_sync_generation(
        host,
        cfg,
        summary,
        file_diff,
        artefact_diff,
        Some(source_task_id),
    )?;
    if runs.runs.is_empty() {
        return Ok(0);
    }
    let run_count = runs.runs.len();
    Ok(run_count)
}

fn should_persist_progress<T: PartialEq>(
    previous: Option<&T>,
    update: &T,
    last_persisted_at: Option<Instant>,
    now: Instant,
) -> bool {
    let Some(previous) = previous else {
        return true;
    };

    let interval_elapsed = last_persisted_at
        .is_none_or(|timestamp| now.duration_since(timestamp) >= PROGRESS_PERSIST_INTERVAL);
    interval_elapsed && previous != update
}

fn sync_spec_from_task_spec_mut(spec: &mut DevqlTaskSpec) -> Option<&mut SyncTaskSpec> {
    match spec {
        DevqlTaskSpec::Sync(spec) => Some(spec),
        DevqlTaskSpec::Ingest(_) | DevqlTaskSpec::EmbeddingsBootstrap(_) => None,
    }
}

#[cfg(test)]
#[path = "coordinator_tests.rs"]
mod tests;

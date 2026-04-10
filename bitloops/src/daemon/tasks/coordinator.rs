use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use anyhow::{Result, anyhow, bail};
use tokio::sync::Notify;
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

    pub(crate) fn activate_worker(self: &Arc<Self>, hub: Option<Arc<SubscriptionHub>>) {
        if let Some(hub) = hub {
            self.register_subscription_hub(hub);
        }
        if self.worker_started.swap(true, Ordering::SeqCst) {
            return;
        }
        if let Err(err) = self.recover_running_tasks() {
            log::warn!("failed to recover queued DevQL tasks: {err:#}");
        }
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            self.worker_started.store(false, Ordering::SeqCst);
            log::warn!("DevQL task worker activation requested without an active tokio runtime");
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

    async fn run_loop(self: Arc<Self>) {
        loop {
            match self.schedule_pending_tasks() {
                Ok(true) => continue,
                Ok(false) => {}
                Err(err) => {
                    log::warn!("daemon DevQL task worker error: {err:#}");
                }
            }
            tokio::select! {
                _ = self.notify.notified() => {},
                _ = sleep(WORKER_POLL_INTERVAL) => {},
            }
        }
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
        }
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

fn task_kind_from_spec(spec: &DevqlTaskSpec) -> DevqlTaskKind {
    match spec {
        DevqlTaskSpec::Sync(_) => DevqlTaskKind::Sync,
        DevqlTaskSpec::Ingest(_) => DevqlTaskKind::Ingest,
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
        DevqlTaskSpec::Ingest(_) => None,
    }
}

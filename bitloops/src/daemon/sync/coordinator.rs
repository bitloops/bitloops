use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use anyhow::Result;
use chrono::Utc;
use tokio::sync::Notify;
use tokio::time::{Duration, sleep};
use uuid::Uuid;

use crate::graphql::SubscriptionHub;
use crate::host::capability_host::{SyncArtefactDiff, SyncCompletedPayload, SyncFileDiff};
use crate::host::devql::{
    DevqlConfig, RepoIdentity, SyncMode, SyncObserver, SyncProgressPhase, SyncProgressUpdate,
    SyncSummary,
};

use super::super::types::{
    SyncQueueStatus, SyncTaskRecord, SyncTaskSource, SyncTaskStatus, unix_timestamp_now,
};
use super::queue::{
    changed_tasks, merge_existing_task, next_pending_task_index, progress_from_summary,
    project_status, prune_terminal_tasks, recompute_queue_positions, sync_task_mode_from_host,
    sync_task_mode_to_host,
};
use crate::host::runtime_store::{DaemonSqliteRuntimeStore, PersistedSyncQueueState};

const WORKER_POLL_INTERVAL: Duration = Duration::from_secs(2);
const PROGRESS_PERSIST_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, Clone)]
pub struct SyncEnqueueResult {
    pub task: SyncTaskRecord,
    pub merged: bool,
}

#[derive(Debug)]
pub struct SyncCoordinator {
    pub(super) runtime_store: DaemonSqliteRuntimeStore,
    pub(super) lock: Mutex<()>,
    pub(super) notify: Notify,
    pub(super) worker_started: AtomicBool,
    pub(super) subscription_hub: Mutex<Option<Arc<SubscriptionHub>>>,
}

struct CoordinatorObserver {
    coordinator: Arc<SyncCoordinator>,
    task_id: String,
    progress_state: Mutex<ProgressPersistState>,
}

#[derive(Debug, Default)]
struct ProgressPersistState {
    last_persisted: Option<SyncProgressUpdate>,
    last_persisted_at: Option<Instant>,
}

struct WorkerStartedGuard {
    coordinator: Arc<SyncCoordinator>,
}

impl Drop for WorkerStartedGuard {
    fn drop(&mut self) {
        self.coordinator
            .worker_started
            .store(false, Ordering::SeqCst);
    }
}

impl SyncObserver for CoordinatorObserver {
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

        if let Err(err) = self.coordinator.update_task_progress(&self.task_id, update) {
            log::warn!(
                "failed to persist sync progress for task `{}`: {err:#}",
                self.task_id
            );
        }
    }
}

impl SyncCoordinator {
    pub(crate) fn shared() -> Arc<Self> {
        let runtime_store =
            DaemonSqliteRuntimeStore::open().expect("opening daemon runtime store for sync queue");
        static INSTANCE: OnceLock<Mutex<Arc<SyncCoordinator>>> = OnceLock::new();
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
            log::warn!("failed to recover queued sync tasks: {err:#}");
        }
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            self.worker_started.store(false, Ordering::SeqCst);
            log::warn!("sync worker activation requested without an active tokio runtime");
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
        source: SyncTaskSource,
        mode: SyncMode,
    ) -> Result<SyncEnqueueResult> {
        let mode = sync_task_mode_from_host(&mode);
        self.mutate_state(|state| {
            if let Some(task) = merge_existing_task(state, cfg, source, &mode) {
                return Ok(SyncEnqueueResult { task, merged: true });
            }

            let now = unix_timestamp_now();
            let task = SyncTaskRecord {
                task_id: format!("sync-task-{}", Uuid::new_v4()),
                repo_id: cfg.repo.repo_id.clone(),
                repo_name: cfg.repo.name.clone(),
                repo_provider: cfg.repo.provider.clone(),
                repo_organisation: cfg.repo.organization.clone(),
                repo_identity: cfg.repo.identity.clone(),
                daemon_config_root: cfg.daemon_config_root.clone(),
                repo_root: cfg.repo_root.clone(),
                source,
                mode,
                status: SyncTaskStatus::Queued,
                submitted_at_unix: now,
                started_at_unix: None,
                updated_at_unix: now,
                completed_at_unix: None,
                queue_position: None,
                tasks_ahead: None,
                progress: SyncProgressUpdate::default(),
                error: None,
                summary: None,
            };
            state.tasks.push(task.clone());
            state.last_action = Some("enqueue".to_string());
            Ok(SyncEnqueueResult {
                task,
                merged: false,
            })
        })
    }

    pub(crate) fn snapshot(&self, repo_id: Option<&str>) -> Result<SyncQueueStatus> {
        let persisted = self.runtime_store.sync_state_exists()?;
        let state = self.load_state()?;
        Ok(project_status(&state, repo_id, persisted))
    }

    pub(crate) fn task(&self, task_id: &str) -> Result<Option<SyncTaskRecord>> {
        let state = self.load_state()?;
        Ok(state.tasks.into_iter().find(|task| task.task_id == task_id))
    }

    pub(crate) fn tasks(
        &self,
        repo_id: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<SyncTaskRecord>> {
        let state = self.load_state()?;
        let mut tasks = state
            .tasks
            .into_iter()
            .filter(|task| repo_id.is_none_or(|repo_id| task.repo_id == repo_id))
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

    async fn run_loop(self: Arc<Self>) {
        loop {
            match self.process_next_task().await {
                Ok(true) => continue,
                Ok(false) => {}
                Err(err) => {
                    log::warn!("daemon sync worker error: {err:#}");
                }
            }
            tokio::select! {
                _ = self.notify.notified() => {},
                _ = sleep(WORKER_POLL_INTERVAL) => {},
            }
        }
    }

    async fn process_next_task(&self) -> Result<bool> {
        let Some(task) = self.take_next_task()? else {
            return Ok(false);
        };

        self.update_task_phase(
            &task.task_id,
            SyncProgressUpdate {
                phase: SyncProgressPhase::EnsuringSchema,
                ..task.progress.clone()
            },
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
        let requested_mode = sync_task_mode_to_host(&task.mode);

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
                return Ok(true);
            }
        };
        let effective_mode = crate::host::devql::effective_sync_mode_after_schema_preparation(
            requested_mode,
            schema_outcome,
        );
        if sync_task_mode_from_host(&effective_mode) != task.mode {
            self.update_task_mode(&task.task_id, sync_task_mode_from_host(&effective_mode))?;
        }

        let observer = CoordinatorObserver {
            coordinator: Arc::clone(&Self::shared()),
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
                    if let Err(err) = enqueue_sync_completed_runs(
                        capability_event_coordinator.as_ref(),
                        host,
                        &cfg,
                        &summary,
                        file_diff,
                        artefact_diff,
                    ) {
                        log::warn!(
                            "failed to enqueue sync capability event runs (task_id={}): {err:#}",
                            task.task_id
                        );
                    }
                }
                self.finish_task_completed(&task.task_id, summary)?
            }
            Err(err) => self.finish_task_failed(&task.task_id, err)?,
        }

        Ok(true)
    }

    fn update_task_mode(
        &self,
        task_id: &str,
        mode: super::super::types::SyncTaskMode,
    ) -> Result<()> {
        let task_id = task_id.to_string();
        self.mutate_state(|state| {
            let Some(task) = state.tasks.iter_mut().find(|task| task.task_id == task_id) else {
                return Ok(());
            };
            task.mode = mode;
            task.updated_at_unix = unix_timestamp_now();
            state.last_action = Some("mode_updated".to_string());
            Ok(())
        })
        .map(|_: ()| ())
    }

    pub(super) fn recover_running_tasks(&self) -> Result<()> {
        self.mutate_state(|state| {
            for task in &mut state.tasks {
                if task.status == SyncTaskStatus::Running {
                    task.status = SyncTaskStatus::Queued;
                    task.progress.phase = SyncProgressPhase::Queued;
                    task.error = None;
                    task.summary = None;
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

    fn take_next_task(&self) -> Result<Option<SyncTaskRecord>> {
        self.mutate_state(|state| {
            let Some(index) = next_pending_task_index(state) else {
                return Ok(None);
            };
            let now = unix_timestamp_now();
            let mut task = state.tasks[index].clone();
            task.status = SyncTaskStatus::Running;
            task.started_at_unix = Some(task.started_at_unix.unwrap_or(now));
            task.updated_at_unix = now;
            task.error = None;
            task.completed_at_unix = None;
            task.progress.phase = SyncProgressPhase::Queued;
            state.tasks[index] = task.clone();
            state.last_action = Some("running".to_string());
            Ok(Some(task))
        })
    }

    fn update_task_progress(&self, task_id: &str, update: SyncProgressUpdate) -> Result<()> {
        let task_id = task_id.to_string();
        self.mutate_state(|state| {
            let Some(task) = state.tasks.iter_mut().find(|task| task.task_id == task_id) else {
                return Ok(());
            };
            task.progress = update.clone();
            task.updated_at_unix = unix_timestamp_now();
            state.last_action = Some(update.phase.as_str().to_string());
            Ok(())
        })
        .map(|_: ()| ())
    }

    fn update_task_phase(&self, task_id: &str, update: SyncProgressUpdate) -> Result<()> {
        self.update_task_progress(task_id, update)
    }

    fn finish_task_completed(&self, task_id: &str, summary: SyncSummary) -> Result<()> {
        let task_id = task_id.to_string();
        self.mutate_state(|state| {
            let Some(task) = state.tasks.iter_mut().find(|task| task.task_id == task_id) else {
                return Ok(());
            };
            let now = unix_timestamp_now();
            task.status = SyncTaskStatus::Completed;
            task.updated_at_unix = now;
            task.completed_at_unix = Some(now);
            task.error = None;
            task.summary = Some(summary.clone());
            task.progress = progress_from_summary(&summary);
            task.progress.phase = SyncProgressPhase::Complete;
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
            task.status = SyncTaskStatus::Failed;
            task.updated_at_unix = now;
            task.completed_at_unix = Some(now);
            task.error = Some(error.clone());
            task.summary = None;
            task.progress.phase = SyncProgressPhase::Failed;
            state.last_action = Some("failed".to_string());
            Ok(())
        })
        .map(|_: ()| ())
    }

    fn load_state(&self) -> Result<PersistedSyncQueueState> {
        Ok(self
            .runtime_store
            .load_sync_queue_state()?
            .unwrap_or_else(PersistedSyncQueueState::default))
    }

    fn mutate_state<T>(
        &self,
        mutate: impl FnOnce(&mut PersistedSyncQueueState) -> Result<T>,
    ) -> Result<T> {
        let guard = self
            .lock
            .lock()
            .map_err(|_| anyhow::anyhow!("sync coordinator lock poisoned"))?;
        let (result, tasks_to_publish) = self.runtime_store.mutate_sync_queue_state(|state| {
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
        state: &mut PersistedSyncQueueState,
        previous_tasks: &[SyncTaskRecord],
    ) -> Result<Vec<SyncTaskRecord>> {
        state.version = 1;
        state.updated_at_unix = unix_timestamp_now();
        recompute_queue_positions(&mut state.tasks);
        prune_terminal_tasks(&mut state.tasks);
        Ok(changed_tasks(previous_tasks, &state.tasks))
    }

    fn publish_tasks(&self, tasks: Vec<SyncTaskRecord>) {
        let Some(hub) = self
            .subscription_hub
            .lock()
            .ok()
            .and_then(|slot| slot.clone())
        else {
            return;
        };
        for task in tasks {
            hub.publish_sync_task(task);
        }
    }
}

fn enqueue_sync_completed_runs(
    coordinator: &crate::daemon::CapabilityEventCoordinator,
    host: &crate::host::capability_host::DevqlCapabilityHost,
    cfg: &DevqlConfig,
    summary: &SyncSummary,
    file_diff: SyncFileDiff,
    artefact_diff: SyncArtefactDiff,
) -> Result<usize> {
    if !summary.success || summary.mode == "validate" {
        return Ok(0);
    }

    let payload = SyncCompletedPayload {
        repo_id: cfg.repo.repo_id.clone(),
        repo_root: cfg.repo_root.clone(),
        active_branch: summary.active_branch.clone(),
        head_commit_sha: summary.head_commit_sha.clone(),
        sync_mode: summary.mode.clone(),
        sync_completed_at: Utc::now().to_rfc3339(),
        files: file_diff,
        artefacts: artefact_diff,
    };
    let runs = crate::daemon::capability_events::build_sync_completed_runs(host, &payload)?;
    if runs.is_empty() {
        return Ok(0);
    }
    let run_count = runs.len();
    coordinator.enqueue_runs(runs)?;
    Ok(run_count)
}

fn should_persist_progress(
    previous: Option<&SyncProgressUpdate>,
    update: &SyncProgressUpdate,
    last_persisted_at: Option<Instant>,
    now: Instant,
) -> bool {
    let Some(previous) = previous else {
        return true;
    };

    if previous.phase != update.phase {
        return true;
    }

    let interval_elapsed = last_persisted_at
        .is_none_or(|timestamp| now.duration_since(timestamp) >= PROGRESS_PERSIST_INTERVAL);
    let completed_all_paths =
        update.paths_total > 0 && update.paths_completed >= update.paths_total;

    completed_all_paths || (interval_elapsed && previous != update)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    use crate::daemon::types::SyncTaskMode;
    use crate::host::devql::{SyncMode, resolve_repo_identity};
    use crate::test_support::git_fixtures::init_test_repo;
    use tempfile::TempDir;

    fn test_coordinator(root: &Path) -> SyncCoordinator {
        SyncCoordinator {
            runtime_store: DaemonSqliteRuntimeStore::open_at(root.join("runtime.sqlite"))
                .expect("open test runtime store"),
            lock: Mutex::new(()),
            notify: Notify::new(),
            worker_started: AtomicBool::new(false),
            subscription_hub: Mutex::new(None),
        }
    }

    fn seeded_cfg() -> (TempDir, DevqlConfig) {
        let dir = TempDir::new().expect("temp dir");
        init_test_repo(dir.path(), "main", "Bitloops Test", "bitloops@example.com");
        let repo = resolve_repo_identity(dir.path()).expect("resolve repo identity");
        let cfg = DevqlConfig::from_roots(dir.path().to_path_buf(), dir.path().to_path_buf(), repo)
            .expect("build devql config");
        (dir, cfg)
    }

    fn sample_task(cfg: &DevqlConfig, task_id: &str) -> SyncTaskRecord {
        SyncTaskRecord {
            task_id: task_id.to_string(),
            repo_id: cfg.repo.repo_id.clone(),
            repo_name: cfg.repo.name.clone(),
            repo_provider: cfg.repo.provider.clone(),
            repo_organisation: cfg.repo.organization.clone(),
            repo_identity: cfg.repo.identity.clone(),
            daemon_config_root: cfg.daemon_config_root.clone(),
            repo_root: cfg.repo_root.clone(),
            source: SyncTaskSource::ManualCli,
            mode: SyncTaskMode::Full,
            status: SyncTaskStatus::Queued,
            submitted_at_unix: 1,
            started_at_unix: None,
            updated_at_unix: 1,
            completed_at_unix: None,
            queue_position: Some(1),
            tasks_ahead: Some(0),
            progress: SyncProgressUpdate::default(),
            error: None,
            summary: None,
        }
    }

    #[test]
    fn enqueue_merges_pending_path_tasks_for_same_repo() {
        let (dir, cfg) = seeded_cfg();
        let coordinator = test_coordinator(dir.path());

        let first = coordinator
            .enqueue(
                &cfg,
                SyncTaskSource::Watcher,
                SyncMode::Paths(vec!["src/lib.rs".to_string()]),
            )
            .expect("enqueue first task");
        let second = coordinator
            .enqueue(
                &cfg,
                SyncTaskSource::Watcher,
                SyncMode::Paths(vec!["src/main.rs".to_string()]),
            )
            .expect("enqueue second task");

        assert!(!first.merged);
        assert!(second.merged);
        assert_eq!(first.task.task_id, second.task.task_id);

        let tasks = coordinator
            .tasks(Some(cfg.repo.repo_id.as_str()), None)
            .expect("list tasks");
        assert_eq!(tasks.len(), 1);
        match &tasks[0].mode {
            SyncTaskMode::Paths { paths } => {
                assert_eq!(
                    paths,
                    &vec!["src/lib.rs".to_string(), "src/main.rs".to_string()]
                );
            }
            other => panic!("expected paths mode, got {other:?}"),
        }
    }

    #[test]
    fn enqueue_full_task_absorbs_later_path_task_for_same_repo() {
        let (dir, cfg) = seeded_cfg();
        let coordinator = test_coordinator(dir.path());

        let first = coordinator
            .enqueue(&cfg, SyncTaskSource::ManualCli, SyncMode::Full)
            .expect("enqueue full task");
        let second = coordinator
            .enqueue(
                &cfg,
                SyncTaskSource::Watcher,
                SyncMode::Paths(vec!["src/lib.rs".to_string()]),
            )
            .expect("enqueue path task");

        assert!(!first.merged);
        assert!(second.merged);
        assert_eq!(first.task.task_id, second.task.task_id);
    }

    #[test]
    fn recover_running_tasks_requeues_them() {
        let (dir, cfg) = seeded_cfg();
        let coordinator = test_coordinator(dir.path());
        let task = SyncTaskRecord {
            task_id: "sync-task-1".to_string(),
            repo_id: cfg.repo.repo_id.clone(),
            repo_name: cfg.repo.name.clone(),
            repo_provider: cfg.repo.provider.clone(),
            repo_organisation: cfg.repo.organization.clone(),
            repo_identity: cfg.repo.identity.clone(),
            daemon_config_root: cfg.daemon_config_root.clone(),
            repo_root: cfg.repo_root.clone(),
            source: SyncTaskSource::ManualCli,
            mode: SyncTaskMode::Full,
            status: SyncTaskStatus::Running,
            submitted_at_unix: 1,
            started_at_unix: Some(2),
            updated_at_unix: 3,
            completed_at_unix: None,
            queue_position: Some(1),
            tasks_ahead: Some(0),
            progress: SyncProgressUpdate {
                phase: SyncProgressPhase::ExtractingPaths,
                ..SyncProgressUpdate::default()
            },
            error: Some("boom".to_string()),
            summary: None,
        };
        coordinator
            .runtime_store
            .mutate_sync_queue_state(|state| {
                *state = PersistedSyncQueueState {
                    version: 1,
                    tasks: vec![task],
                    last_action: Some("running".to_string()),
                    updated_at_unix: 3,
                };
                Ok(())
            })
            .expect("seed queue state");

        coordinator
            .recover_running_tasks()
            .expect("recover running tasks");
        let task = coordinator
            .task("sync-task-1")
            .expect("load task")
            .expect("task must exist");
        assert_eq!(task.status, SyncTaskStatus::Queued);
        assert_eq!(task.progress.phase, SyncProgressPhase::Queued);
        assert!(task.error.is_none());
        assert!(task.started_at_unix.is_none());
    }

    #[test]
    fn update_task_mode_rewrites_the_persisted_task_mode() {
        let (dir, cfg) = seeded_cfg();
        let coordinator = test_coordinator(dir.path());
        let mut task = sample_task(&cfg, "sync-task-1");
        task.status = SyncTaskStatus::Running;
        coordinator
            .runtime_store
            .mutate_sync_queue_state(|state| {
                *state = PersistedSyncQueueState {
                    version: 1,
                    tasks: vec![task],
                    last_action: Some("running".to_string()),
                    updated_at_unix: 1,
                };
                Ok(())
            })
            .expect("seed queue state");

        coordinator
            .update_task_mode("sync-task-1", SyncTaskMode::Repair)
            .expect("update task mode");

        let task = coordinator
            .task("sync-task-1")
            .expect("load task")
            .expect("task must exist");
        assert_eq!(task.mode, SyncTaskMode::Repair);
    }

    #[test]
    fn changed_tasks_only_returns_new_or_modified_records() {
        let (_dir, cfg) = seeded_cfg();
        let unchanged = sample_task(&cfg, "sync-task-1");
        let mut changed_before = sample_task(&cfg, "sync-task-2");
        changed_before.queue_position = Some(2);
        changed_before.tasks_ahead = Some(1);
        let previous = vec![unchanged.clone(), changed_before.clone()];

        let mut changed_after = changed_before;
        changed_after.status = SyncTaskStatus::Running;
        changed_after.started_at_unix = Some(2);
        changed_after.updated_at_unix = 2;
        changed_after.progress.phase = SyncProgressPhase::InspectingWorkspace;

        let mut added = sample_task(&cfg, "sync-task-3");
        added.queue_position = Some(3);
        added.tasks_ahead = Some(2);

        let changed = changed_tasks(
            &previous,
            &[unchanged, changed_after.clone(), added.clone()],
        );
        assert_eq!(changed, vec![changed_after, added]);
    }

    #[test]
    fn progress_persistence_throttles_same_phase_updates() {
        let now = Instant::now();
        let previous = SyncProgressUpdate {
            phase: SyncProgressPhase::ExtractingPaths,
            current_path: Some("src/a.rs".to_string()),
            paths_total: 10,
            paths_completed: 1,
            paths_remaining: 9,
            ..SyncProgressUpdate::default()
        };
        let next = SyncProgressUpdate {
            current_path: Some("src/b.rs".to_string()),
            paths_completed: 2,
            paths_remaining: 8,
            ..previous.clone()
        };

        assert!(!should_persist_progress(
            Some(&previous),
            &next,
            Some(now),
            now + Duration::from_millis(500),
        ));
        assert!(should_persist_progress(
            Some(&previous),
            &next,
            Some(now),
            now + PROGRESS_PERSIST_INTERVAL,
        ));
    }

    #[test]
    fn progress_persistence_always_keeps_phase_changes() {
        let now = Instant::now();
        let previous = SyncProgressUpdate {
            phase: SyncProgressPhase::ExtractingPaths,
            ..SyncProgressUpdate::default()
        };
        let next = SyncProgressUpdate {
            phase: SyncProgressPhase::MaterialisingPaths,
            ..SyncProgressUpdate::default()
        };

        assert!(should_persist_progress(
            Some(&previous),
            &next,
            Some(now),
            now + Duration::from_millis(10),
        ));
    }

    #[test]
    fn enqueue_sync_completed_runs_persists_matching_capability_events() {
        let (dir, cfg) = seeded_cfg();
        let host = crate::host::devql::build_capability_host(&cfg.repo_root, cfg.repo.clone())
            .expect("build capability host");
        let capability_event_store_path = dir.path().join("capability-events.sqlite");
        let capability_event_coordinator =
            crate::daemon::capability_events::test_shared_instance_at(
                capability_event_store_path.clone(),
            );
        let summary = SyncSummary {
            success: true,
            mode: "full".to_string(),
            active_branch: Some("main".to_string()),
            head_commit_sha: Some("abc123".to_string()),
            ..SyncSummary::default()
        };

        let enqueued = enqueue_sync_completed_runs(
            capability_event_coordinator.as_ref(),
            &host,
            &cfg,
            &summary,
            SyncFileDiff::default(),
            SyncArtefactDiff::default(),
        )
        .expect("enqueue capability event runs");

        assert!(enqueued >= 1, "expected at least one capability event run");
        let state = crate::host::runtime_store::DaemonSqliteRuntimeStore::open_at(
            capability_event_store_path,
        )
        .expect("open runtime store")
        .load_capability_event_queue_state()
        .expect("load capability event queue state")
        .expect("state should exist");
        assert!(
            state
                .runs
                .iter()
                .any(|run| run.repo_id == cfg.repo.repo_id && run.event_kind == "sync_completed")
        );
    }

    #[test]
    fn enqueue_sync_completed_runs_skips_validate_mode() {
        let (dir, cfg) = seeded_cfg();
        let host = crate::host::devql::build_capability_host(&cfg.repo_root, cfg.repo.clone())
            .expect("build capability host");
        let capability_event_store_path = dir.path().join("capability-events.sqlite");
        let capability_event_coordinator =
            crate::daemon::capability_events::test_shared_instance_at(
                capability_event_store_path.clone(),
            );
        let summary = SyncSummary {
            success: true,
            mode: "validate".to_string(),
            active_branch: Some("main".to_string()),
            head_commit_sha: Some("abc123".to_string()),
            ..SyncSummary::default()
        };

        let enqueued = enqueue_sync_completed_runs(
            capability_event_coordinator.as_ref(),
            &host,
            &cfg,
            &summary,
            SyncFileDiff::default(),
            SyncArtefactDiff::default(),
        )
        .expect("enqueue capability event runs");

        assert_eq!(enqueued, 0);
        let state = crate::host::runtime_store::DaemonSqliteRuntimeStore::open_at(
            capability_event_store_path,
        )
        .expect("open runtime store")
        .load_capability_event_queue_state()
        .expect("load capability event queue state");
        assert!(
            state.is_none(),
            "validate mode should not enqueue capability event runs"
        );
    }
}

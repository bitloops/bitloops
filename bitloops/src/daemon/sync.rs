use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;
use tokio::time::{Duration, sleep};
use uuid::Uuid;

use crate::graphql::SubscriptionHub;
use crate::host::devql::{
    DevqlConfig, RepoIdentity, SyncMode, SyncObserver, SyncProgressPhase, SyncProgressUpdate,
    SyncSummary,
};

use super::process_is_running;
use super::state_store::{read_json, write_json};
use super::types::{
    SYNC_STATE_FILE_NAME, SYNC_STATE_LOCK_FILE_NAME, SyncQueueState, SyncQueueStatus, SyncTaskMode,
    SyncTaskRecord, SyncTaskSource, SyncTaskStatus, global_daemon_dir_fallback, unix_timestamp_now,
};

const WORKER_POLL_INTERVAL: Duration = Duration::from_secs(2);
const MAX_TERMINAL_TASKS: usize = 64;

#[derive(Debug, Clone)]
pub struct SyncEnqueueResult {
    pub task: SyncTaskRecord,
    pub merged: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedSyncQueueState {
    version: u8,
    tasks: Vec<SyncTaskRecord>,
    last_action: Option<String>,
    updated_at_unix: u64,
}

impl Default for PersistedSyncQueueState {
    fn default() -> Self {
        Self {
            version: 1,
            tasks: Vec::new(),
            last_action: Some("initialized".to_string()),
            updated_at_unix: 0,
        }
    }
}

#[derive(Debug)]
pub struct SyncCoordinator {
    state_path: PathBuf,
    state_lock_path: PathBuf,
    lock: Mutex<()>,
    notify: Notify,
    worker_started: AtomicBool,
    subscription_hub: Mutex<Option<Arc<SubscriptionHub>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StateLockOwner {
    pid: u32,
    token: String,
}

#[derive(Debug)]
struct StateFileLockGuard {
    lock_path: PathBuf,
    owner: StateLockOwner,
}

impl Drop for StateFileLockGuard {
    fn drop(&mut self) {
        if matches!(
            read_state_lock_owner(&self.lock_path),
            Ok(Some(owner)) if owner == self.owner
        ) {
            let _ = fs::remove_file(&self.lock_path);
        }
    }
}

struct CoordinatorObserver {
    coordinator: Arc<SyncCoordinator>,
    task_id: String,
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
        static INSTANCE: OnceLock<Arc<SyncCoordinator>> = OnceLock::new();
        Arc::clone(INSTANCE.get_or_init(|| {
            let coordinator = Arc::new(Self {
                state_path: sync_state_path(),
                state_lock_path: sync_state_lock_path(),
                lock: Mutex::new(()),
                notify: Notify::new(),
                worker_started: AtomicBool::new(false),
                subscription_hub: Mutex::new(None),
            });
            coordinator.ensure_state_file();
            coordinator
        }))
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
                config_root: cfg.config_root.clone(),
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
        let persisted = self.state_path.exists();
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

    fn ensure_state_file(&self) {
        if self.state_path.exists() {
            return;
        }
        let _ = write_json(&self.state_path, &PersistedSyncQueueState::default());
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
        let cfg = DevqlConfig::from_roots(task.config_root.clone(), task.repo_root.clone(), repo)?;

        if let Err(err) = crate::host::devql::execute_init_schema(&cfg, "queued DevQL sync").await {
            self.finish_task_failed(&task.task_id, err)?;
            return Ok(true);
        }

        let observer = CoordinatorObserver {
            coordinator: Arc::clone(&Self::shared()),
            task_id: task.task_id.clone(),
        };

        match crate::host::devql::run_sync_with_summary_and_observer(
            &cfg,
            sync_task_mode_to_host(&task.mode),
            Some(&observer),
        )
        .await
        {
            Ok(summary) => self.finish_task_completed(&task.task_id, summary)?,
            Err(err) => self.finish_task_failed(&task.task_id, err)?,
        }

        Ok(true)
    }

    fn recover_running_tasks(&self) -> Result<()> {
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
        Ok(read_json::<PersistedSyncQueueState>(&self.state_path)?
            .unwrap_or_else(PersistedSyncQueueState::default))
    }

    fn mutate_state<T>(
        &self,
        mutate: impl FnOnce(&mut PersistedSyncQueueState) -> Result<T>,
    ) -> Result<T> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| anyhow::anyhow!("sync coordinator lock poisoned"))?;
        let _file_lock = acquire_state_file_lock(&self.state_lock_path)?;
        let mut state = self.load_state()?;
        let previous_tasks = state.tasks.clone();
        let result = mutate(&mut state)?;
        let tasks_to_publish = self.save_state(&mut state, &previous_tasks)?;
        drop(_file_lock);
        drop(_guard);
        self.publish_tasks(tasks_to_publish);
        self.notify.notify_waiters();
        Ok(result)
    }

    fn save_state(
        &self,
        state: &mut PersistedSyncQueueState,
        previous_tasks: &[SyncTaskRecord],
    ) -> Result<Vec<SyncTaskRecord>> {
        state.version = 1;
        state.updated_at_unix = unix_timestamp_now();
        recompute_queue_positions(&mut state.tasks);
        prune_terminal_tasks(&mut state.tasks);
        write_json(&self.state_path, state)?;
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

fn changed_tasks(previous: &[SyncTaskRecord], current: &[SyncTaskRecord]) -> Vec<SyncTaskRecord> {
    let previous_by_id = previous
        .iter()
        .map(|task| (task.task_id.as_str(), task))
        .collect::<std::collections::HashMap<_, _>>();
    current
        .iter()
        .filter(|task| {
            previous_by_id
                .get(task.task_id.as_str())
                .is_none_or(|previous| *previous != *task)
        })
        .cloned()
        .collect()
}

fn sync_state_path() -> PathBuf {
    global_daemon_dir_fallback().join(SYNC_STATE_FILE_NAME)
}

fn sync_state_lock_path() -> PathBuf {
    global_daemon_dir_fallback().join(SYNC_STATE_LOCK_FILE_NAME)
}

fn acquire_state_file_lock(lock_path: &Path) -> Result<StateFileLockGuard> {
    let owner = StateLockOwner {
        pid: std::process::id(),
        token: Uuid::new_v4().to_string(),
    };

    if try_write_state_lock(lock_path, &owner)? {
        return Ok(StateFileLockGuard {
            lock_path: lock_path.to_path_buf(),
            owner,
        });
    }

    if clear_stale_state_lock(lock_path)? && try_write_state_lock(lock_path, &owner)? {
        return Ok(StateFileLockGuard {
            lock_path: lock_path.to_path_buf(),
            owner,
        });
    }

    bail!("sync queue lock already held at {}", lock_path.display())
}

fn try_write_state_lock(lock_path: &Path, owner: &StateLockOwner) -> Result<bool> {
    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("creating sync queue lock directory {}", parent.display()))?;
    }
    let payload = format!("{}\n{}\n", owner.pid, owner.token);
    let mut file = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(lock_path)
    {
        Ok(file) => file,
        Err(err) if err.kind() == ErrorKind::AlreadyExists => return Ok(false),
        Err(err) => {
            return Err(err)
                .with_context(|| format!("creating sync queue lock {}", lock_path.display()));
        }
    };
    file.write_all(payload.as_bytes())
        .with_context(|| format!("writing sync queue lock {}", lock_path.display()))?;
    file.sync_all()
        .with_context(|| format!("syncing sync queue lock {}", lock_path.display()))?;
    Ok(true)
}

fn clear_stale_state_lock(lock_path: &Path) -> Result<bool> {
    let Some(owner) = read_state_lock_owner(lock_path)? else {
        return Ok(false);
    };
    if process_is_running(owner.pid)? {
        return Ok(false);
    }
    fs::remove_file(lock_path)
        .or_else(|err| {
            if err.kind() == ErrorKind::NotFound {
                Ok(())
            } else {
                Err(err)
            }
        })
        .with_context(|| format!("removing stale sync queue lock {}", lock_path.display()))?;
    Ok(true)
}

fn read_state_lock_owner(lock_path: &Path) -> Result<Option<StateLockOwner>> {
    let payload = match fs::read_to_string(lock_path) {
        Ok(payload) => payload,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("reading {}", lock_path.display())),
    };
    let mut lines = payload.lines();
    let Some(pid) = lines.next() else {
        return Ok(None);
    };
    let Some(token) = lines.next() else {
        return Ok(None);
    };
    let Ok(pid) = pid.trim().parse::<u32>() else {
        return Ok(None);
    };
    Ok(Some(StateLockOwner {
        pid,
        token: token.trim().to_string(),
    }))
}

fn merge_existing_task(
    state: &mut PersistedSyncQueueState,
    cfg: &DevqlConfig,
    _source: SyncTaskSource,
    mode: &SyncTaskMode,
) -> Option<SyncTaskRecord> {
    if *mode != SyncTaskMode::Validate
        && let Some(existing) = state.tasks.iter_mut().find(|task| {
            task.repo_id == cfg.repo.repo_id
                && matches!(
                    task.status,
                    SyncTaskStatus::Queued | SyncTaskStatus::Running
                )
                && match (&task.mode, mode) {
                    (SyncTaskMode::Repair, _) => true,
                    (existing_mode, incoming_mode)
                        if is_full_like(existing_mode) && is_weaker_than_repair(incoming_mode) =>
                    {
                        true
                    }
                    _ => false,
                }
        })
    {
        existing.updated_at_unix = unix_timestamp_now();
        existing.error = None;
        return Some(existing.clone());
    }

    if let SyncTaskMode::Paths { paths } = mode
        && let Some(existing) = state.tasks.iter_mut().find(|task| {
            task.repo_id == cfg.repo.repo_id
                && task.status == SyncTaskStatus::Queued
                && matches!(task.mode, SyncTaskMode::Paths { .. })
        })
    {
        if let SyncTaskMode::Paths {
            paths: existing_paths,
        } = &mut existing.mode
        {
            existing_paths.extend(paths.iter().cloned());
            existing_paths.sort();
            existing_paths.dedup();
        }
        existing.updated_at_unix = unix_timestamp_now();
        existing.error = None;
        return Some(existing.clone());
    }

    None
}

fn next_pending_task_index(state: &PersistedSyncQueueState) -> Option<usize> {
    state
        .tasks
        .iter()
        .enumerate()
        .filter(|(_, task)| task.status == SyncTaskStatus::Queued)
        .min_by_key(|(_, task)| pending_sort_key(task))
        .map(|(index, _)| index)
}

fn pending_sort_key(task: &SyncTaskRecord) -> (u8, u64, String) {
    (
        if matches!(task.mode, SyncTaskMode::Validate) {
            1
        } else {
            0
        },
        task.submitted_at_unix,
        task.task_id.clone(),
    )
}

fn recompute_queue_positions(tasks: &mut [SyncTaskRecord]) {
    for task in tasks.iter_mut() {
        task.queue_position = None;
        task.tasks_ahead = None;
    }

    let mut order = tasks
        .iter()
        .enumerate()
        .filter(|(_, task)| {
            matches!(
                task.status,
                SyncTaskStatus::Running | SyncTaskStatus::Queued
            )
        })
        .map(|(index, task)| {
            (
                index,
                task.status == SyncTaskStatus::Running,
                pending_sort_key(task),
            )
        })
        .collect::<Vec<_>>();
    order.sort_by(
        |(_, left_running, left_key), (_, right_running, right_key)| {
            let left_running = *left_running;
            let right_running = *right_running;
            left_running
                .cmp(&right_running)
                .reverse()
                .then_with(|| left_key.cmp(right_key))
        },
    );

    for (index, (task_index, _, _)) in order.into_iter().enumerate() {
        let position = (index as u64) + 1;
        tasks[task_index].queue_position = Some(position);
        tasks[task_index].tasks_ahead = Some(position.saturating_sub(1));
    }
}

fn prune_terminal_tasks(tasks: &mut Vec<SyncTaskRecord>) {
    let mut terminal = tasks
        .iter()
        .filter(|task| {
            matches!(
                task.status,
                SyncTaskStatus::Completed | SyncTaskStatus::Failed | SyncTaskStatus::Cancelled
            )
        })
        .cloned()
        .collect::<Vec<_>>();
    terminal.sort_by(|left, right| right.updated_at_unix.cmp(&left.updated_at_unix));
    terminal.truncate(MAX_TERMINAL_TASKS);

    let terminal_ids = terminal
        .into_iter()
        .map(|task| task.task_id)
        .collect::<std::collections::HashSet<_>>();
    tasks.retain(|task| {
        !matches!(
            task.status,
            SyncTaskStatus::Completed | SyncTaskStatus::Failed | SyncTaskStatus::Cancelled
        ) || terminal_ids.contains(&task.task_id)
    });
}

fn project_status(
    state: &PersistedSyncQueueState,
    repo_id: Option<&str>,
    persisted: bool,
) -> SyncQueueStatus {
    let pending_tasks = state
        .tasks
        .iter()
        .filter(|task| task.status == SyncTaskStatus::Queued)
        .count() as u64;
    let running_tasks = state
        .tasks
        .iter()
        .filter(|task| task.status == SyncTaskStatus::Running)
        .count() as u64;
    let failed_tasks = state
        .tasks
        .iter()
        .filter(|task| task.status == SyncTaskStatus::Failed)
        .count() as u64;
    let completed_recent_tasks = state
        .tasks
        .iter()
        .filter(|task| task.status == SyncTaskStatus::Completed)
        .count() as u64;
    let current_repo_task = repo_id.and_then(|repo_id| select_repo_task(&state.tasks, repo_id));

    SyncQueueStatus {
        state: SyncQueueState {
            version: state.version,
            pending_tasks,
            running_tasks,
            failed_tasks,
            completed_recent_tasks,
            last_action: state.last_action.clone(),
            last_updated_unix: state.updated_at_unix,
        },
        persisted,
        current_repo_task,
    }
}

fn select_repo_task(tasks: &[SyncTaskRecord], repo_id: &str) -> Option<SyncTaskRecord> {
    tasks
        .iter()
        .filter(|task| task.repo_id == repo_id && task.status == SyncTaskStatus::Running)
        .max_by_key(|task| task.updated_at_unix)
        .cloned()
        .or_else(|| {
            tasks
                .iter()
                .filter(|task| task.repo_id == repo_id && task.status == SyncTaskStatus::Queued)
                .min_by_key(|task| task.queue_position.unwrap_or(u64::MAX))
                .cloned()
        })
        .or_else(|| {
            tasks
                .iter()
                .filter(|task| task.repo_id == repo_id)
                .max_by_key(|task| task.updated_at_unix)
                .cloned()
        })
}

fn sync_task_mode_from_host(mode: &SyncMode) -> SyncTaskMode {
    match mode {
        SyncMode::Auto => SyncTaskMode::Auto,
        SyncMode::Full => SyncTaskMode::Full,
        SyncMode::Paths(paths) => SyncTaskMode::Paths {
            paths: normalize_paths(paths),
        },
        SyncMode::Repair => SyncTaskMode::Repair,
        SyncMode::Validate => SyncTaskMode::Validate,
    }
}

fn sync_task_mode_to_host(mode: &SyncTaskMode) -> SyncMode {
    match mode {
        SyncTaskMode::Auto => SyncMode::Auto,
        SyncTaskMode::Full => SyncMode::Full,
        SyncTaskMode::Paths { paths } => SyncMode::Paths(paths.clone()),
        SyncTaskMode::Repair => SyncMode::Repair,
        SyncTaskMode::Validate => SyncMode::Validate,
    }
}

fn normalize_paths(paths: &[String]) -> Vec<String> {
    let mut paths = paths
        .iter()
        .map(|path| normalize_repo_path(path))
        .filter(|path| !path.is_empty())
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    paths
}

fn normalize_repo_path(path: &str) -> String {
    let mut normalized = path.trim().replace('\\', "/");
    while normalized.starts_with("./") {
        normalized = normalized[2..].to_string();
    }
    normalized.trim_start_matches('/').to_string()
}

fn is_full_like(mode: &SyncTaskMode) -> bool {
    matches!(mode, SyncTaskMode::Auto | SyncTaskMode::Full)
}

fn is_weaker_than_repair(mode: &SyncTaskMode) -> bool {
    matches!(
        mode,
        SyncTaskMode::Auto | SyncTaskMode::Full | SyncTaskMode::Paths { .. }
    )
}

fn progress_from_summary(summary: &SyncSummary) -> SyncProgressUpdate {
    let total = summary.paths_unchanged
        + summary.paths_added
        + summary.paths_changed
        + summary.paths_removed;
    SyncProgressUpdate {
        phase: SyncProgressPhase::Complete,
        current_path: None,
        paths_total: total,
        paths_completed: total,
        paths_remaining: 0,
        paths_unchanged: summary.paths_unchanged,
        paths_added: summary.paths_added,
        paths_changed: summary.paths_changed,
        paths_removed: summary.paths_removed,
        cache_hits: summary.cache_hits,
        cache_misses: summary.cache_misses,
        parse_errors: summary.parse_errors,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::devql::{SyncMode, resolve_repo_identity};
    use crate::test_support::git_fixtures::init_test_repo;
    use tempfile::TempDir;

    fn test_coordinator(root: &Path) -> SyncCoordinator {
        let coordinator = SyncCoordinator {
            state_path: root.join("sync.json"),
            state_lock_path: root.join("sync.lock"),
            lock: Mutex::new(()),
            notify: Notify::new(),
            worker_started: AtomicBool::new(false),
            subscription_hub: Mutex::new(None),
        };
        coordinator.ensure_state_file();
        coordinator
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
            config_root: cfg.config_root.clone(),
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
            config_root: cfg.config_root.clone(),
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
        write_json(
            &coordinator.state_path,
            &PersistedSyncQueueState {
                version: 1,
                tasks: vec![task],
                last_action: Some("running".to_string()),
                updated_at_unix: 3,
            },
        )
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
}

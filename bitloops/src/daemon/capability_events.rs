use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
#[cfg(test)]
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{Context, Result, bail};
use futures_util::FutureExt;
use serde::Deserialize;
use tokio::sync::Notify;
use tokio::time::{Duration, sleep};
use uuid::Uuid;

#[path = "capability_events/queue.rs"]
mod queue;

use crate::host::capability_host::{
    ChangedArtefact, ChangedFile, HostEvent, HostEventHandler, HostEventKind, RemovedArtefact,
    RemovedFile, SyncArtefactDiff, SyncCompletedPayload, SyncFileDiff,
};
use crate::host::runtime_store::{DaemonSqliteRuntimeStore, PersistedCapabilityEventQueueState};

use self::queue::{next_pending_run_index, project_status, prune_terminal_runs};
use super::types::{
    CapabilityEventQueueStatus, CapabilityEventRunRecord, CapabilityEventRunStatus,
    unix_timestamp_now,
};

const WORKER_POLL_INTERVAL: Duration = Duration::from_secs(2);

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

#[derive(Debug, Deserialize)]
struct SyncCompletedPayloadEnvelope {
    repo_id: String,
    repo_root: std::path::PathBuf,
    active_branch: Option<String>,
    head_commit_sha: Option<String>,
    sync_mode: String,
    sync_completed_at: String,
    files: SyncFileDiffEnvelope,
    artefacts: SyncArtefactDiffEnvelope,
}

#[derive(Debug, Deserialize)]
struct SyncFileDiffEnvelope {
    added: Vec<ChangedFileEnvelope>,
    changed: Vec<ChangedFileEnvelope>,
    removed: Vec<RemovedFileEnvelope>,
}

#[derive(Debug, Deserialize)]
struct ChangedFileEnvelope {
    path: String,
    language: String,
    content_id: String,
}

#[derive(Debug, Deserialize)]
struct RemovedFileEnvelope {
    path: String,
}

#[derive(Debug, Deserialize)]
struct SyncArtefactDiffEnvelope {
    added: Vec<ChangedArtefactEnvelope>,
    changed: Vec<ChangedArtefactEnvelope>,
    removed: Vec<RemovedArtefactEnvelope>,
}

#[derive(Debug, Deserialize)]
struct ChangedArtefactEnvelope {
    artefact_id: String,
    symbol_id: String,
    path: String,
    canonical_kind: Option<String>,
    name: String,
}

#[derive(Debug, Deserialize)]
struct RemovedArtefactEnvelope {
    artefact_id: String,
    symbol_id: String,
    path: String,
}

impl From<SyncCompletedPayloadEnvelope> for SyncCompletedPayload {
    fn from(value: SyncCompletedPayloadEnvelope) -> Self {
        Self {
            repo_id: value.repo_id,
            repo_root: value.repo_root,
            active_branch: value.active_branch,
            head_commit_sha: value.head_commit_sha,
            sync_mode: value.sync_mode,
            sync_completed_at: value.sync_completed_at,
            files: SyncFileDiff {
                added: value
                    .files
                    .added
                    .into_iter()
                    .map(|file| ChangedFile {
                        path: file.path,
                        language: file.language,
                        content_id: file.content_id,
                    })
                    .collect(),
                changed: value
                    .files
                    .changed
                    .into_iter()
                    .map(|file| ChangedFile {
                        path: file.path,
                        language: file.language,
                        content_id: file.content_id,
                    })
                    .collect(),
                removed: value
                    .files
                    .removed
                    .into_iter()
                    .map(|file| RemovedFile { path: file.path })
                    .collect(),
            },
            artefacts: SyncArtefactDiff {
                added: value
                    .artefacts
                    .added
                    .into_iter()
                    .map(|artefact| ChangedArtefact {
                        artefact_id: artefact.artefact_id,
                        symbol_id: artefact.symbol_id,
                        path: artefact.path,
                        canonical_kind: artefact.canonical_kind,
                        name: artefact.name,
                    })
                    .collect(),
                changed: value
                    .artefacts
                    .changed
                    .into_iter()
                    .map(|artefact| ChangedArtefact {
                        artefact_id: artefact.artefact_id,
                        symbol_id: artefact.symbol_id,
                        path: artefact.path,
                        canonical_kind: artefact.canonical_kind,
                        name: artefact.name,
                    })
                    .collect(),
                removed: value
                    .artefacts
                    .removed
                    .into_iter()
                    .map(|artefact| RemovedArtefact {
                        artefact_id: artefact.artefact_id,
                        symbol_id: artefact.symbol_id,
                        path: artefact.path,
                    })
                    .collect(),
            },
        }
    }
}

pub(crate) fn build_sync_completed_runs(
    host: &crate::host::capability_host::DevqlCapabilityHost,
    payload: &SyncCompletedPayload,
) -> Result<Vec<CapabilityEventRunRecord>> {
    let payload_json = sync_completed_payload_json(payload)?;
    let handlers = describe_handlers(host);
    let now = unix_timestamp_now();
    let runs = handlers
        .into_iter()
        .filter(|descriptor| descriptor.event_kind == HostEventKind::SyncCompleted)
        .map(|descriptor| CapabilityEventRunRecord {
            capability_id: descriptor.capability_id.clone(),
            handler_id: descriptor.handler_id.clone(),
            run_id: format!("capability-event-run-{}", Uuid::new_v4()),
            repo_id: payload.repo_id.clone(),
            event_kind: "sync_completed".to_string(),
            lane_key: build_lane_key(
                &payload.repo_id,
                &descriptor.capability_id,
                &descriptor.handler_id,
            ),
            event_payload_json: payload_json.clone(),
            status: CapabilityEventRunStatus::Queued,
            attempts: 0,
            submitted_at_unix: now,
            started_at_unix: None,
            updated_at_unix: now,
            completed_at_unix: None,
            error: None,
        })
        .collect::<Vec<_>>();
    Ok(runs)
}

#[cfg(test)]
pub(crate) fn test_shared_instance_at(db_path: PathBuf) -> Arc<CapabilityEventCoordinator> {
    CapabilityEventCoordinator::new_shared_instance(
        DaemonSqliteRuntimeStore::open_at(db_path)
            .expect("opening test daemon runtime store for capability event queue"),
    )
}

#[allow(dead_code)]
impl CapabilityEventCoordinator {
    pub(crate) fn shared() -> Arc<Self> {
        let runtime_store = DaemonSqliteRuntimeStore::open()
            .expect("opening daemon runtime store for capability event queue");
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
        self.spawn_worker_if_possible();
    }

    fn spawn_worker_if_possible(self: &Arc<Self>) {
        if self.worker_started.swap(true, Ordering::SeqCst) {
            return;
        }
        if let Err(err) = self.recover_running_runs() {
            log::warn!("failed to recover queued capability event runs: {err:#}");
        }
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            self.worker_started.store(false, Ordering::SeqCst);
            log::warn!(
                "capability event worker activation requested without an active tokio runtime"
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

    pub(crate) fn enqueue_run(
        &self,
        run: CapabilityEventRunRecord,
    ) -> Result<CapabilityEventEnqueueResult> {
        self.enqueue_runs(vec![run])
    }

    pub(crate) fn enqueue_runs(
        &self,
        runs: Vec<CapabilityEventRunRecord>,
    ) -> Result<CapabilityEventEnqueueResult> {
        if runs.is_empty() {
            return Ok(CapabilityEventEnqueueResult { runs });
        }

        let mut persisted_runs = runs;
        self.mutate_state(|state| {
            let now = unix_timestamp_now();
            for run in &mut persisted_runs {
                normalise_run_for_enqueue(run, now)?;
                state.runs.push(run.clone());
            }
            state.last_action = Some("enqueue".to_string());
            Ok(())
        })?;

        Ok(CapabilityEventEnqueueResult {
            runs: persisted_runs,
        })
    }

    pub(crate) fn snapshot(&self, repo_id: Option<&str>) -> Result<CapabilityEventQueueStatus> {
        let persisted = self.runtime_store.capability_event_state_exists()?;
        let state = self.load_state()?;
        Ok(project_status(&state, repo_id, persisted))
    }

    pub(crate) fn run(&self, run_id: &str) -> Result<Option<CapabilityEventRunRecord>> {
        let state = self.load_state()?;
        Ok(state.runs.into_iter().find(|run| run.run_id == run_id))
    }

    async fn run_loop(self: Arc<Self>) {
        loop {
            match self.schedule_ready_runs().await {
                Ok(true) => continue,
                Ok(false) => {}
                Err(err) => {
                    log::warn!("daemon capability event worker error: {err:#}");
                }
            }
            tokio::select! {
                _ = self.notify.notified() => {},
                _ = sleep(WORKER_POLL_INTERVAL) => {},
            }
        }
    }

    async fn schedule_ready_runs(self: &Arc<Self>) -> Result<bool> {
        let ready_runs = self.mutate_state(|state| {
            let mut ready_runs = Vec::new();
            while let Some(index) = next_pending_run_index(state) {
                let now = unix_timestamp_now();
                let mut run = state.runs[index].clone();
                run.status = CapabilityEventRunStatus::Running;
                run.attempts = run.attempts.saturating_add(1);
                run.started_at_unix = Some(run.started_at_unix.unwrap_or(now));
                run.updated_at_unix = now;
                run.completed_at_unix = None;
                run.error = None;
                state.runs[index] = run.clone();
                state.last_action = Some("running".to_string());
                ready_runs.push(run);
            }
            Ok(ready_runs)
        })?;

        if ready_runs.is_empty() {
            return Ok(false);
        }

        for run in ready_runs {
            self.spawn_execution_task(run);
        }
        Ok(true)
    }

    fn spawn_execution_task(self: &Arc<Self>, run: CapabilityEventRunRecord) {
        let coordinator = Arc::clone(self);
        tokio::spawn(async move {
            let completion = coordinator.execute_run(&run).await;
            if let Err(err) = coordinator.finish_run(&run.run_id, completion).await {
                log::warn!(
                    "failed to finalize capability event run `{}`: {err:#}",
                    run.run_id
                );
            }
        });
    }

    async fn execute_run(&self, run: &CapabilityEventRunRecord) -> RunCompletion {
        let (event, repo_root) = match build_host_event(run) {
            Ok(event) => event,
            Err(err) => {
                return RunCompletion::failed(err);
            }
        };

        let repo_identity = match crate::host::devql::resolve_repo_identity(&repo_root) {
            Ok(repo) => repo,
            Err(err) => return RunCompletion::failed(err),
        };

        let host = match crate::host::devql::build_capability_host(&repo_root, repo_identity) {
            Ok(host) => host,
            Err(err) => return RunCompletion::failed(err),
        };

        let Some(handler) = resolve_handler_for_run(&host, run) else {
            return RunCompletion::failed(anyhow::anyhow!(
                "no matching handler found for capability event run `{}`",
                run.run_id
            ));
        };

        let handler_context = match host.build_event_handler_context() {
            Ok(context) => Arc::new(context),
            Err(err) => return RunCompletion::failed(err),
        };

        let outcome = AssertUnwindSafe(handler.handle(&event, &handler_context))
            .catch_unwind()
            .await;
        match outcome {
            Ok(Ok(())) => RunCompletion::completed(),
            Ok(Err(err)) => RunCompletion::failed(err),
            Err(_) => RunCompletion::failed(anyhow::anyhow!(
                "capability event handler panicked for run `{}`",
                run.run_id
            )),
        }
    }

    async fn finish_run(&self, run_id: &str, completion: RunCompletion) -> Result<()> {
        let run_id = run_id.to_string();
        self.mutate_state(|state| {
            let Some(run) = state.runs.iter_mut().find(|run| run.run_id == run_id) else {
                return Ok(());
            };
            let now = unix_timestamp_now();
            run.status = completion.status;
            run.updated_at_unix = now;
            run.completed_at_unix = Some(now);
            run.error = completion.error;
            state.last_action = Some(match run.status {
                CapabilityEventRunStatus::Completed => "completed".to_string(),
                CapabilityEventRunStatus::Failed => "failed".to_string(),
                CapabilityEventRunStatus::Cancelled => "cancelled".to_string(),
                CapabilityEventRunStatus::Queued | CapabilityEventRunStatus::Running => {
                    "running".to_string()
                }
            });
            Ok(())
        })?;
        self.notify.notify_waiters();
        Ok(())
    }

    fn recover_running_runs(&self) -> Result<()> {
        self.mutate_state(|state| {
            for run in &mut state.runs {
                if run.status == CapabilityEventRunStatus::Running {
                    run.status = CapabilityEventRunStatus::Queued;
                    run.started_at_unix = None;
                    run.completed_at_unix = None;
                    run.error = None;
                    run.updated_at_unix = unix_timestamp_now();
                }
            }
            state.last_action = Some("recovered_running_runs".to_string());
            Ok(())
        })
    }

    fn load_state(&self) -> Result<PersistedCapabilityEventQueueState> {
        Ok(self
            .runtime_store
            .load_capability_event_queue_state()?
            .unwrap_or_else(PersistedCapabilityEventQueueState::default))
    }

    fn mutate_state<T>(
        &self,
        mutate: impl FnOnce(&mut PersistedCapabilityEventQueueState) -> Result<T>,
    ) -> Result<T> {
        let guard = self
            .lock
            .lock()
            .map_err(|_| anyhow::anyhow!("capability event coordinator lock poisoned"))?;
        let result = self
            .runtime_store
            .mutate_capability_event_queue_state(|state| {
                let output = mutate(state)?;
                save_state(state)?;
                Ok(output)
            })?;
        drop(guard);
        self.notify.notify_waiters();
        Ok(result)
    }
}

#[derive(Debug)]
struct RunCompletion {
    status: CapabilityEventRunStatus,
    error: Option<String>,
}

impl RunCompletion {
    fn completed() -> Self {
        Self {
            status: CapabilityEventRunStatus::Completed,
            error: None,
        }
    }

    fn failed(err: anyhow::Error) -> Self {
        Self {
            status: CapabilityEventRunStatus::Failed,
            error: Some(format!("{err:#}")),
        }
    }
}

#[allow(dead_code)]
fn normalise_run_for_enqueue(run: &mut CapabilityEventRunRecord, now: u64) -> Result<()> {
    if run.run_id.is_empty() {
        run.run_id = format!("capability-event-run-{}", Uuid::new_v4());
    }
    if run.lane_key.is_empty() {
        run.lane_key = build_lane_key(&run.repo_id, &run.capability_id, &run.handler_id);
    }
    if run.submitted_at_unix == 0 {
        run.submitted_at_unix = now;
    }
    if run.updated_at_unix == 0 {
        run.updated_at_unix = now;
    }
    if run.status != CapabilityEventRunStatus::Queued {
        run.status = CapabilityEventRunStatus::Queued;
    }
    run.started_at_unix = None;
    run.completed_at_unix = None;
    run.error = None;
    if run.event_payload_json.trim().is_empty() {
        bail!(
            "capability event run `{}` is missing event payload JSON",
            run.run_id
        );
    }
    Ok(())
}

#[allow(dead_code)]
fn build_lane_key(repo_id: &str, capability_id: &str, handler_id: &str) -> String {
    format!("{repo_id}:{capability_id}:{handler_id}")
}

fn build_host_event(run: &CapabilityEventRunRecord) -> Result<(HostEvent, std::path::PathBuf)> {
    match run.event_kind.as_str() {
        "sync_completed" => {
            let payload =
                serde_json::from_str::<SyncCompletedPayloadEnvelope>(&run.event_payload_json)
                    .with_context(|| {
                        format!(
                            "parsing sync_completed payload for capability event run `{}`",
                            run.run_id
                        )
                    })?;
            let repo_root = payload.repo_root.clone();
            Ok((HostEvent::SyncCompleted(payload.into()), repo_root))
        }
        other => bail!(
            "unsupported capability event kind `{other}` for run `{}`",
            run.run_id
        ),
    }
}

struct CapabilityEventHandlerDescriptor {
    capability_id: String,
    handler_id: String,
    event_kind: HostEventKind,
    handler: Arc<dyn HostEventHandler>,
}

fn resolve_handler_for_run(
    host: &crate::host::capability_host::DevqlCapabilityHost,
    run: &CapabilityEventRunRecord,
) -> Option<Arc<dyn HostEventHandler>> {
    let descriptors = describe_handlers(host);
    descriptors.into_iter().find_map(|descriptor| {
        if descriptor.capability_id == run.capability_id
            && descriptor.handler_id == run.handler_id
            && descriptor.event_kind == event_kind_from_name(run.event_kind.as_str())
        {
            return Some(descriptor.handler);
        }
        None
    })
}

fn describe_handlers(
    host: &crate::host::capability_host::DevqlCapabilityHost,
) -> Vec<CapabilityEventHandlerDescriptor> {
    let mut per_capability_index: HashMap<String, usize> = HashMap::new();
    host.event_handlers()
        .iter()
        .map(|handler| {
            let capability_id = handler.capability_id().to_string();
            let index = per_capability_index
                .entry(capability_id.clone())
                .or_insert(0);
            let handler_id = format!("{capability_id}#{index}");
            *index += 1;
            CapabilityEventHandlerDescriptor {
                capability_id,
                handler_id,
                event_kind: handler.event_kind(),
                handler: Arc::clone(handler),
            }
        })
        .collect()
}

fn event_kind_from_name(name: &str) -> HostEventKind {
    match name {
        "sync_completed" => HostEventKind::SyncCompleted,
        _ => HostEventKind::SyncCompleted,
    }
}

fn save_state(state: &mut PersistedCapabilityEventQueueState) -> Result<()> {
    state.version = 1;
    state.updated_at_unix = unix_timestamp_now();
    prune_terminal_runs(&mut state.runs);
    Ok(())
}

fn sync_completed_payload_json(payload: &SyncCompletedPayload) -> Result<String> {
    let files = serde_json::json!({
        "added": payload.files.added.iter().map(|file| serde_json::json!({
            "path": file.path,
            "language": file.language,
            "content_id": file.content_id,
        })).collect::<Vec<_>>(),
        "changed": payload.files.changed.iter().map(|file| serde_json::json!({
            "path": file.path,
            "language": file.language,
            "content_id": file.content_id,
        })).collect::<Vec<_>>(),
        "removed": payload.files.removed.iter().map(|file| serde_json::json!({
            "path": file.path,
        })).collect::<Vec<_>>(),
    });
    let artefacts = serde_json::json!({
        "added": payload.artefacts.added.iter().map(|artefact| serde_json::json!({
            "artefact_id": artefact.artefact_id,
            "symbol_id": artefact.symbol_id,
            "path": artefact.path,
            "canonical_kind": artefact.canonical_kind,
            "name": artefact.name,
        })).collect::<Vec<_>>(),
        "changed": payload.artefacts.changed.iter().map(|artefact| serde_json::json!({
            "artefact_id": artefact.artefact_id,
            "symbol_id": artefact.symbol_id,
            "path": artefact.path,
            "canonical_kind": artefact.canonical_kind,
            "name": artefact.name,
        })).collect::<Vec<_>>(),
        "removed": payload.artefacts.removed.iter().map(|artefact| serde_json::json!({
            "artefact_id": artefact.artefact_id,
            "symbol_id": artefact.symbol_id,
            "path": artefact.path,
        })).collect::<Vec<_>>(),
    });

    Ok(serde_json::json!({
        "repo_id": payload.repo_id,
        "repo_root": payload.repo_root,
        "active_branch": payload.active_branch,
        "head_commit_sha": payload.head_commit_sha,
        "sync_mode": payload.sync_mode,
        "sync_completed_at": payload.sync_completed_at,
        "files": files,
        "artefacts": artefacts,
    })
    .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::CapabilityEventRunStatus;
    use crate::host::runtime_store::DaemonSqliteRuntimeStore;
    use crate::test_support::process_state::with_env_var;
    use tempfile::TempDir;

    fn queued_run(
        run_id: &str,
        repo_id: &str,
        capability_id: &str,
        handler_id: &str,
        submitted_at_unix: u64,
        status: CapabilityEventRunStatus,
    ) -> CapabilityEventRunRecord {
        CapabilityEventRunRecord {
            run_id: run_id.to_string(),
            repo_id: repo_id.to_string(),
            capability_id: capability_id.to_string(),
            handler_id: handler_id.to_string(),
            event_kind: "sync_completed".to_string(),
            lane_key: build_lane_key(repo_id, capability_id, handler_id),
            event_payload_json: serde_json::json!({
                "repo_id": repo_id,
                "repo_root": "/tmp/repo",
                "active_branch": "main",
                "head_commit_sha": "abc123",
                "sync_mode": "full",
                "sync_completed_at": "2026-04-06T00:00:00Z",
                "files": {"added": [], "changed": [], "removed": []},
                "artefacts": {"added": [], "changed": [], "removed": []},
            })
            .to_string(),
            status,
            attempts: 0,
            submitted_at_unix,
            started_at_unix: None,
            updated_at_unix: submitted_at_unix,
            completed_at_unix: None,
            error: None,
        }
    }

    #[test]
    fn next_pending_run_skips_running_lane_and_preserves_fifo() {
        let state = PersistedCapabilityEventQueueState {
            version: 1,
            runs: vec![
                queued_run(
                    "run-a-1",
                    "repo-1",
                    "test_harness",
                    "test_harness#0",
                    1,
                    CapabilityEventRunStatus::Running,
                ),
                queued_run(
                    "run-a-2",
                    "repo-1",
                    "test_harness",
                    "test_harness#0",
                    2,
                    CapabilityEventRunStatus::Queued,
                ),
                queued_run(
                    "run-b-1",
                    "repo-1",
                    "semantic_clones",
                    "semantic_clones#0",
                    3,
                    CapabilityEventRunStatus::Queued,
                ),
            ],
            last_action: Some("running".to_string()),
            updated_at_unix: 3,
        };

        let index = next_pending_run_index(&state).expect("expected runnable run");
        assert_eq!(state.runs[index].run_id, "run-b-1");
    }

    #[test]
    fn different_lanes_remain_runnable_when_one_lane_is_running() {
        let state = PersistedCapabilityEventQueueState {
            version: 1,
            runs: vec![
                queued_run(
                    "run-a-1",
                    "repo-1",
                    "test_harness",
                    "test_harness#0",
                    1,
                    CapabilityEventRunStatus::Running,
                ),
                queued_run(
                    "run-b-1",
                    "repo-1",
                    "knowledge",
                    "knowledge#0",
                    2,
                    CapabilityEventRunStatus::Queued,
                ),
            ],
            last_action: Some("running".to_string()),
            updated_at_unix: 2,
        };

        let index = next_pending_run_index(&state).expect("expected runnable run");
        assert_eq!(state.runs[index].run_id, "run-b-1");
    }

    #[test]
    fn failed_lane_does_not_block_other_lanes() {
        let state = PersistedCapabilityEventQueueState {
            version: 1,
            runs: vec![
                queued_run(
                    "run-a-1",
                    "repo-1",
                    "test_harness",
                    "test_harness#0",
                    1,
                    CapabilityEventRunStatus::Failed,
                ),
                queued_run(
                    "run-b-1",
                    "repo-1",
                    "semantic_clones",
                    "semantic_clones#0",
                    2,
                    CapabilityEventRunStatus::Queued,
                ),
            ],
            last_action: Some("failed".to_string()),
            updated_at_unix: 2,
        };

        let index = next_pending_run_index(&state).expect("expected runnable run");
        assert_eq!(state.runs[index].run_id, "run-b-1");
    }

    #[test]
    fn project_status_counts_runs_and_selects_current_repo_run() {
        let state = PersistedCapabilityEventQueueState {
            version: 1,
            runs: vec![
                queued_run(
                    "run-a-1",
                    "repo-1",
                    "test_harness",
                    "test_harness#0",
                    1,
                    CapabilityEventRunStatus::Completed,
                ),
                queued_run(
                    "run-a-2",
                    "repo-1",
                    "test_harness",
                    "test_harness#0",
                    2,
                    CapabilityEventRunStatus::Failed,
                ),
                queued_run(
                    "run-a-3",
                    "repo-1",
                    "test_harness",
                    "test_harness#0",
                    3,
                    CapabilityEventRunStatus::Queued,
                ),
                queued_run(
                    "run-b-1",
                    "repo-2",
                    "knowledge",
                    "knowledge#0",
                    4,
                    CapabilityEventRunStatus::Running,
                ),
            ],
            last_action: Some("running".to_string()),
            updated_at_unix: 4,
        };

        let projected = project_status(&state, Some("repo-1"), true);
        assert_eq!(projected.state.pending_runs, 1);
        assert_eq!(projected.state.running_runs, 1);
        assert_eq!(projected.state.failed_runs, 1);
        assert_eq!(projected.state.completed_recent_runs, 1);
        assert!(projected.persisted);
        assert_eq!(
            projected
                .current_repo_run
                .as_ref()
                .map(|run| run.run_id.as_str()),
            Some("run-a-3")
        );
    }

    #[test]
    fn capability_event_queue_state_persists_in_sqlite() {
        let state_dir = TempDir::new().expect("tempdir");
        crate::test_support::process_state::with_env_var(
            "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
            Some(state_dir.path().to_string_lossy().as_ref()),
            || {
                let store = DaemonSqliteRuntimeStore::open().expect("open daemon runtime store");
                store
                    .mutate_capability_event_queue_state(|state| {
                        state.version = 9;
                        state.runs.push(queued_run(
                            "run-1",
                            "repo-1",
                            "test_harness",
                            "test_harness#0",
                            1,
                            CapabilityEventRunStatus::Queued,
                        ));
                        Ok(())
                    })
                    .expect("save capability event queue state");
                let loaded = store
                    .load_capability_event_queue_state()
                    .expect("load capability event queue state")
                    .expect("state exists");
                assert_eq!(loaded.version, 9);
                assert_eq!(loaded.runs.len(), 1);
            },
        );
    }

    #[test]
    fn coordinator_enqueue_run_persists_and_snapshots_state() {
        let state_dir = TempDir::new().expect("tempdir");
        with_env_var(
            "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
            Some(state_dir.path().to_string_lossy().as_ref()),
            || {
                let coordinator = CapabilityEventCoordinator::shared();
                let run = queued_run(
                    "run-1",
                    "repo-1",
                    "test_harness",
                    "test_harness#0",
                    1,
                    CapabilityEventRunStatus::Queued,
                );

                let outcome = coordinator.enqueue_run(run).expect("enqueue run");
                assert_eq!(outcome.runs.len(), 1);
                let run_id = outcome.runs[0].run_id.clone();
                let loaded = coordinator
                    .run(&run_id)
                    .expect("load run by id")
                    .expect("run must exist");
                assert_eq!(loaded.run_id, run_id);
                assert_eq!(loaded.status, CapabilityEventRunStatus::Queued);

                let snapshot = coordinator
                    .snapshot(Some("repo-1"))
                    .expect("snapshot capability event status");
                assert_eq!(snapshot.state.pending_runs, 1);
                assert_eq!(
                    snapshot
                        .current_repo_run
                        .as_ref()
                        .map(|run| run.run_id.as_str()),
                    Some("run-1")
                );
            },
        );
    }

    #[test]
    fn snapshot_does_not_recover_running_runs_without_activation() {
        let state_dir = TempDir::new().expect("tempdir");
        with_env_var(
            "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
            Some(state_dir.path().to_string_lossy().as_ref()),
            || {
                let store = DaemonSqliteRuntimeStore::open().expect("open daemon runtime store");
                let started_at_unix = 42_u64;
                let updated_at_unix = 99_u64;
                store
                    .mutate_capability_event_queue_state(|state| {
                        state.version = 1;
                        state.last_action = Some("running".to_string());
                        state.updated_at_unix = updated_at_unix;
                        let mut run = queued_run(
                            "run-running",
                            "repo-1",
                            "test_harness",
                            "test_harness#0",
                            1,
                            CapabilityEventRunStatus::Running,
                        );
                        run.started_at_unix = Some(started_at_unix);
                        run.updated_at_unix = updated_at_unix;
                        state.runs = vec![run];
                        Ok(())
                    })
                    .expect("seed running capability event run");

                let coordinator = CapabilityEventCoordinator::shared();
                let snapshot = coordinator
                    .snapshot(Some("repo-1"))
                    .expect("snapshot capability event status");
                assert_eq!(snapshot.state.running_runs, 1);

                let loaded = store
                    .load_capability_event_queue_state()
                    .expect("load capability event queue state")
                    .expect("state exists");
                let run = loaded
                    .runs
                    .iter()
                    .find(|run| run.run_id == "run-running")
                    .expect("running run exists");
                assert_eq!(run.status, CapabilityEventRunStatus::Running);
                assert_eq!(run.started_at_unix, Some(started_at_unix));
                assert_eq!(loaded.last_action.as_deref(), Some("running"));
                assert_eq!(loaded.updated_at_unix, updated_at_unix);
            },
        );
    }

    #[test]
    fn activate_worker_recovers_running_runs() {
        let state_dir = TempDir::new().expect("tempdir");
        with_env_var(
            "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
            Some(state_dir.path().to_string_lossy().as_ref()),
            || {
                let store = DaemonSqliteRuntimeStore::open().expect("open daemon runtime store");
                store
                    .mutate_capability_event_queue_state(|state| {
                        state.version = 1;
                        state.last_action = Some("running".to_string());
                        let mut run = queued_run(
                            "run-running",
                            "repo-1",
                            "test_harness",
                            "test_harness#0",
                            1,
                            CapabilityEventRunStatus::Running,
                        );
                        run.started_at_unix = Some(12);
                        run.error = Some("previous failure".to_string());
                        state.runs = vec![run];
                        Ok(())
                    })
                    .expect("seed running capability event run");

                let coordinator = CapabilityEventCoordinator::shared();
                coordinator.activate_worker();
                let snapshot = coordinator
                    .snapshot(Some("repo-1"))
                    .expect("snapshot capability event status");
                assert_eq!(snapshot.state.pending_runs, 1);
                assert_eq!(snapshot.state.running_runs, 0);

                let loaded = store
                    .load_capability_event_queue_state()
                    .expect("load capability event queue state")
                    .expect("state exists");
                let run = loaded
                    .runs
                    .iter()
                    .find(|run| run.run_id == "run-running")
                    .expect("running run exists");
                assert_eq!(run.status, CapabilityEventRunStatus::Queued);
                assert_eq!(run.started_at_unix, None);
                assert_eq!(run.completed_at_unix, None);
                assert_eq!(run.error, None);
                assert_eq!(
                    loaded.last_action.as_deref(),
                    Some("recovered_running_runs")
                );
            },
        );
    }
}

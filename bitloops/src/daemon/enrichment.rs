use anyhow::Result;
use serde::Deserializer;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use tokio::sync::{Mutex, Notify};
use tokio::time::{Duration, sleep};
use uuid::Uuid;

use crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind;
use crate::capability_packs::semantic_clones::features as semantic_features;
use crate::host::devql::RepoIdentity;
use crate::host::runtime_store::DaemonSqliteRuntimeStore;

use super::types::{EnrichmentQueueMode, EnrichmentQueueStatus, unix_timestamp_now};

#[path = "enrichment/execution.rs"]
mod execution;
#[path = "enrichment/queue.rs"]
mod queue;
#[path = "enrichment/worker_count.rs"]
mod worker_count;

use execution::execute_job;
use queue::{next_pending_job_index, project_status};
use worker_count::configured_enrichment_worker_count;

const MAX_ENRICHMENT_JOB_ARTEFACTS: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnrichmentJobStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EnrichmentJobKind {
    SemanticSummaries {
        #[serde(alias = "inputs", deserialize_with = "deserialize_job_artefact_ids")]
        artefact_ids: Vec<String>,
        input_hashes: BTreeMap<String, String>,
        batch_key: String,
    },
    SymbolEmbeddings {
        #[serde(alias = "inputs", deserialize_with = "deserialize_job_artefact_ids")]
        artefact_ids: Vec<String>,
        input_hashes: BTreeMap<String, String>,
        batch_key: String,
        #[serde(default)]
        representation_kind: EmbeddingRepresentationKind,
    },
    CloneEdgesRebuild {},
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrichmentJob {
    pub id: String,
    pub repo_id: String,
    pub repo_root: PathBuf,
    pub config_root: PathBuf,
    pub branch: String,
    pub status: EnrichmentJobStatus,
    pub attempts: u32,
    pub error: Option<String>,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
    pub job: EnrichmentJobKind,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EnrichmentQueueState {
    pub version: u8,
    pub paused_semantic: bool,
    pub paused_embeddings: bool,
    pub active_branch_by_repo: BTreeMap<String, String>,
    pub jobs: Vec<EnrichmentJob>,
    pub retried_failed_jobs: u64,
    pub last_action: Option<String>,
    pub paused_reason: Option<String>,
    pub updated_at_unix: u64,
}

#[derive(Debug, Clone)]
pub struct EnrichmentControlResult {
    pub message: String,
    pub state: super::types::EnrichmentQueueState,
}

#[derive(Debug, Clone)]
pub struct EnrichmentJobTarget {
    config_root: PathBuf,
    repo_root: PathBuf,
    repo_id: String,
    branch: String,
}

impl EnrichmentJobTarget {
    pub fn new(config_root: PathBuf, repo_root: PathBuf, repo_id: String, branch: String) -> Self {
        Self {
            config_root,
            repo_root,
            repo_id,
            branch,
        }
    }

    pub(super) fn from_job(job: &EnrichmentJob) -> Self {
        Self {
            config_root: job.config_root.clone(),
            repo_root: job.repo_root.clone(),
            repo_id: job.repo_id.clone(),
            branch: job.branch.clone(),
        }
    }
}

#[derive(Debug)]
pub struct EnrichmentCoordinator {
    runtime_store: DaemonSqliteRuntimeStore,
    lock: Mutex<()>,
    notify: Notify,
}

#[derive(Debug, Clone)]
enum FollowUpJob {
    SymbolEmbeddings {
        target: EnrichmentJobTarget,
        artefact_ids: Vec<String>,
        input_hashes: BTreeMap<String, String>,
        representation_kind: EmbeddingRepresentationKind,
    },
    CloneEdgesRebuild {
        target: EnrichmentJobTarget,
    },
}

#[derive(Debug, Clone)]
struct JobExecutionOutcome {
    error: Option<String>,
    follow_ups: Vec<FollowUpJob>,
}

impl JobExecutionOutcome {
    fn ok() -> Self {
        Self {
            error: None,
            follow_ups: Vec::new(),
        }
    }

    fn failed(err: anyhow::Error) -> Self {
        Self {
            error: Some(format!("{err:#}")),
            follow_ups: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum PersistedEnrichmentJobInput {
    ArtefactId(String),
    LegacyInput(Box<semantic_features::SemanticFeatureInput>),
}

fn deserialize_job_artefact_ids<'de, D>(
    deserializer: D,
) -> std::result::Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let inputs = Vec::<PersistedEnrichmentJobInput>::deserialize(deserializer)?;
    Ok(inputs
        .into_iter()
        .map(|input| match input {
            PersistedEnrichmentJobInput::ArtefactId(artefact_id) => artefact_id,
            PersistedEnrichmentJobInput::LegacyInput(input) => input.artefact_id.clone(),
        })
        .collect())
}

impl EnrichmentCoordinator {
    pub(crate) fn shared() -> Arc<Self> {
        static INSTANCE: OnceLock<Arc<EnrichmentCoordinator>> = OnceLock::new();
        Arc::clone(INSTANCE.get_or_init(|| {
            let coordinator = Arc::new(Self {
                runtime_store: DaemonSqliteRuntimeStore::open()
                    .expect("opening daemon runtime store for enrichment queue"),
                lock: Mutex::new(()),
                notify: Notify::new(),
            });
            coordinator.ensure_state_file();
            coordinator.requeue_running_jobs();
            let worker_count = configured_enrichment_worker_count();
            if worker_count > 1 {
                log::info!("starting {worker_count} enrichment workers");
            }
            for _ in 0..worker_count {
                coordinator.spawn_worker_if_possible();
            }
            coordinator
        }))
    }

    pub async fn enqueue_semantic_summaries(
        &self,
        target: EnrichmentJobTarget,
        inputs: Vec<semantic_features::SemanticFeatureInput>,
        input_hashes: BTreeMap<String, String>,
    ) -> Result<()> {
        if inputs.is_empty() {
            return Ok(());
        }

        let _guard = self.lock.lock().await;
        let mut state = self.load_state()?;
        state
            .active_branch_by_repo
            .insert(target.repo_id.clone(), target.branch.clone());
        for chunk in inputs.chunks(MAX_ENRICHMENT_JOB_ARTEFACTS) {
            let artefact_ids = chunk
                .iter()
                .map(|input| input.artefact_id.clone())
                .collect::<Vec<_>>();
            let input_hashes = select_input_hashes_for_artefact_ids(&input_hashes, &artefact_ids);
            upsert_pending_semantic_job(&mut state, &target, artefact_ids, input_hashes);
        }
        state.last_action = Some("enqueue_semantic".to_string());
        self.save_state(&mut state)?;
        self.notify.notify_waiters();
        Ok(())
    }

    pub async fn enqueue_symbol_embeddings(
        &self,
        target: EnrichmentJobTarget,
        inputs: Vec<semantic_features::SemanticFeatureInput>,
        input_hashes: BTreeMap<String, String>,
        representation_kind: EmbeddingRepresentationKind,
    ) -> Result<()> {
        if inputs.is_empty() {
            return Ok(());
        }

        let _guard = self.lock.lock().await;
        let mut state = self.load_state()?;
        state
            .active_branch_by_repo
            .insert(target.repo_id.clone(), target.branch.clone());
        for chunk in inputs.chunks(MAX_ENRICHMENT_JOB_ARTEFACTS) {
            let artefact_ids = chunk
                .iter()
                .map(|input| input.artefact_id.clone())
                .collect::<Vec<_>>();
            let input_hashes = select_input_hashes_for_artefact_ids(&input_hashes, &artefact_ids);
            upsert_pending_embedding_job(
                &mut state,
                &target,
                artefact_ids,
                input_hashes,
                representation_kind,
            );
        }
        state.last_action = Some("enqueue_embeddings".to_string());
        self.save_state(&mut state)?;
        self.notify.notify_waiters();
        Ok(())
    }

    pub async fn enqueue_clone_edges_rebuild(&self, target: EnrichmentJobTarget) -> Result<()> {
        let _guard = self.lock.lock().await;
        let mut state = self.load_state()?;
        state
            .active_branch_by_repo
            .insert(target.repo_id.clone(), target.branch.clone());
        if Self::has_pending_or_running_semantic_jobs(&state, &target.repo_id)
            || Self::has_pending_or_running_embedding_jobs(&state, &target.repo_id)
        {
            state.last_action = Some("defer_clone_edges_rebuild".to_string());
            self.save_state(&mut state)?;
            return Ok(());
        }
        let has_existing = state.jobs.iter().any(|job| {
            job.repo_id == target.repo_id
                && matches!(
                    (&job.status, &job.job),
                    (
                        EnrichmentJobStatus::Pending | EnrichmentJobStatus::Running,
                        EnrichmentJobKind::CloneEdgesRebuild { .. }
                    )
                )
        });
        if !has_existing {
            state.jobs.push(EnrichmentJob {
                id: format!("clone-edges-rebuild-{}", Uuid::new_v4()),
                repo_id: target.repo_id,
                repo_root: target.repo_root,
                config_root: target.config_root,
                branch: target.branch,
                status: EnrichmentJobStatus::Pending,
                attempts: 0,
                error: None,
                created_at_unix: unix_timestamp_now(),
                updated_at_unix: unix_timestamp_now(),
                job: EnrichmentJobKind::CloneEdgesRebuild {},
            });
            state.last_action = Some("enqueue_clone_edges_rebuild".to_string());
            self.save_state(&mut state)?;
            self.notify.notify_waiters();
        }
        Ok(())
    }

    fn has_pending_or_running_semantic_jobs(state: &EnrichmentQueueState, repo_id: &str) -> bool {
        state.jobs.iter().any(|job| {
            job.repo_id == repo_id
                && matches!(
                    (&job.status, &job.job),
                    (
                        EnrichmentJobStatus::Pending | EnrichmentJobStatus::Running,
                        EnrichmentJobKind::SemanticSummaries { .. }
                    )
                )
        })
    }

    fn has_pending_or_running_embedding_jobs(state: &EnrichmentQueueState, repo_id: &str) -> bool {
        state.jobs.iter().any(|job| {
            job.repo_id == repo_id
                && matches!(
                    (&job.status, &job.job),
                    (
                        EnrichmentJobStatus::Pending | EnrichmentJobStatus::Running,
                        EnrichmentJobKind::SymbolEmbeddings { .. }
                    )
                )
        })
    }

    fn ensure_state_file(&self) {
        if self
            .runtime_store
            .enrichment_state_exists()
            .unwrap_or(false)
        {
            return;
        }
        let mut state = default_state();
        let _ = self.save_state(&mut state);
    }

    fn requeue_running_jobs(&self) {
        let Ok(Some(mut state)) = self.runtime_store.load_enrichment_queue_state() else {
            return;
        };

        let mut recovered = 0usize;
        for job in &mut state.jobs {
            if job.status == EnrichmentJobStatus::Running {
                job.status = EnrichmentJobStatus::Pending;
                job.updated_at_unix = unix_timestamp_now();
                recovered += 1;
            }
        }

        if recovered == 0 {
            return;
        }

        state.last_action = Some("requeue_running".to_string());
        if let Err(err) = self.runtime_store.save_enrichment_queue_state(&state) {
            log::warn!("failed to requeue stale running enrichment jobs: {err:#}");
            return;
        }
        log::warn!("requeued {recovered} stale running enrichment jobs on daemon startup");
    }

    fn spawn_worker_if_possible(self: &Arc<Self>) {
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let coordinator = Arc::clone(self);
            handle.spawn(async move {
                coordinator.run_loop().await;
            });
        }
    }

    async fn run_loop(self: Arc<Self>) {
        loop {
            match self.process_next_job().await {
                Ok(true) => continue,
                Ok(false) => {}
                Err(err) => {
                    log::warn!("daemon enrichment worker error: {err:#}");
                }
            }
            tokio::select! {
                _ = self.notify.notified() => {},
                _ = sleep(Duration::from_secs(2)) => {},
            }
        }
    }

    async fn process_next_job(&self) -> Result<bool> {
        let job = {
            let _guard = self.lock.lock().await;
            let mut state = self.load_state()?;
            let Some(index) = next_pending_job_index(&state, &self.runtime_store)? else {
                return Ok(false);
            };
            let mut job = state.jobs[index].clone();
            job.status = EnrichmentJobStatus::Running;
            job.attempts += 1;
            job.updated_at_unix = unix_timestamp_now();
            state.jobs[index] = job.clone();
            state.last_action = Some("running".to_string());
            self.save_state(&mut state)?;
            job
        };

        let outcome = execute_job(&job).await;

        {
            let _guard = self.lock.lock().await;
            let mut state = self.load_state()?;
            if let Some(current) = state.jobs.iter_mut().find(|queued| queued.id == job.id) {
                current.updated_at_unix = unix_timestamp_now();
                if let Some(error) = outcome.error.as_ref() {
                    current.status = EnrichmentJobStatus::Failed;
                    current.error = Some(error.clone());
                } else {
                    current.status = EnrichmentJobStatus::Completed;
                    current.error = None;
                }
            }
            state.last_action = Some(match outcome.error {
                Some(_) => "failed".to_string(),
                None => "completed".to_string(),
            });
            self.save_state(&mut state)?;
        }

        for follow_up in outcome.follow_ups {
            self.enqueue_follow_up(follow_up).await?;
        }

        Ok(true)
    }

    async fn enqueue_follow_up(&self, follow_up: FollowUpJob) -> Result<()> {
        match follow_up {
            FollowUpJob::SymbolEmbeddings {
                target,
                artefact_ids,
                input_hashes,
                representation_kind,
            } => {
                self.enqueue_symbol_embeddings_from_artefact_ids(
                    target,
                    artefact_ids,
                    input_hashes,
                    representation_kind,
                )
                .await
            }
            FollowUpJob::CloneEdgesRebuild { target } => {
                self.enqueue_clone_edges_rebuild(target).await
            }
        }
    }

    fn load_state(&self) -> Result<EnrichmentQueueState> {
        Ok(self
            .runtime_store
            .load_enrichment_queue_state()?
            .unwrap_or_else(default_state))
    }

    fn save_state(&self, state: &mut EnrichmentQueueState) -> Result<()> {
        state.version = 1;
        state.updated_at_unix = unix_timestamp_now();
        self.runtime_store.save_enrichment_queue_state(state)
    }

    async fn enqueue_symbol_embeddings_from_artefact_ids(
        &self,
        target: EnrichmentJobTarget,
        artefact_ids: Vec<String>,
        input_hashes: BTreeMap<String, String>,
        representation_kind: EmbeddingRepresentationKind,
    ) -> Result<()> {
        if artefact_ids.is_empty() {
            return Ok(());
        }

        let _guard = self.lock.lock().await;
        let mut state = self.load_state()?;
        state
            .active_branch_by_repo
            .insert(target.repo_id.clone(), target.branch.clone());
        for chunk in artefact_ids.chunks(MAX_ENRICHMENT_JOB_ARTEFACTS) {
            let artefact_ids = chunk.to_vec();
            let input_hashes = select_input_hashes_for_artefact_ids(&input_hashes, &artefact_ids);
            upsert_pending_embedding_job(
                &mut state,
                &target,
                artefact_ids,
                input_hashes,
                representation_kind,
            );
        }
        state.last_action = Some("enqueue_embeddings".to_string());
        self.save_state(&mut state)?;
        self.notify.notify_waiters();
        Ok(())
    }
}

pub fn snapshot() -> Result<EnrichmentQueueStatus> {
    let runtime_store = DaemonSqliteRuntimeStore::open()?;
    let state = runtime_store
        .load_enrichment_queue_state()?
        .unwrap_or_else(default_state);
    let gate = crate::daemon::embeddings_bootstrap::gate_status_for_enrichment_queue(
        &runtime_store,
        state.jobs.iter().map(|job| job.config_root.clone()),
    )?;
    Ok(EnrichmentQueueStatus {
        state: project_status(&state),
        persisted: runtime_store.enrichment_state_exists()?,
        embeddings_gate: gate,
    })
}

pub fn pause_enrichments(reason: Option<String>) -> Result<EnrichmentControlResult> {
    let runtime_store = DaemonSqliteRuntimeStore::open()?;
    let mut state = runtime_store
        .load_enrichment_queue_state()?
        .unwrap_or_else(default_state);
    state.paused_embeddings = true;
    state.paused_semantic = true;
    state.paused_reason = reason.clone();
    state.last_action = Some("paused".to_string());
    runtime_store.save_enrichment_queue_state(&state)?;
    let mut projected = project_status(&state);
    projected.mode = EnrichmentQueueMode::Paused;
    projected.last_action = Some("paused".to_string());
    projected.paused_reason = reason.clone();
    Ok(EnrichmentControlResult {
        message: reason
            .map(|reason| format!("Enrichment queue paused: {reason}"))
            .unwrap_or_else(|| "Enrichment queue paused.".to_string()),
        state: projected,
    })
}

pub fn resume_enrichments() -> Result<EnrichmentControlResult> {
    let runtime_store = DaemonSqliteRuntimeStore::open()?;
    let mut state = runtime_store
        .load_enrichment_queue_state()?
        .unwrap_or_else(default_state);
    state.paused_embeddings = false;
    state.paused_semantic = false;
    state.paused_reason = None;
    state.last_action = Some("resumed".to_string());
    runtime_store.save_enrichment_queue_state(&state)?;
    Ok(EnrichmentControlResult {
        message: "Enrichment queue resumed.".to_string(),
        state: project_status(&state),
    })
}

pub fn retry_failed_enrichments() -> Result<EnrichmentControlResult> {
    let runtime_store = DaemonSqliteRuntimeStore::open()?;
    let mut state = runtime_store
        .load_enrichment_queue_state()?
        .unwrap_or_else(default_state);
    let mut retried = 0u64;
    for job in &mut state.jobs {
        if job.status == EnrichmentJobStatus::Failed {
            job.status = EnrichmentJobStatus::Pending;
            job.error = None;
            job.updated_at_unix = unix_timestamp_now();
            retried += 1;
        }
    }
    state.retried_failed_jobs += retried;
    state.last_action = Some("retry_failed".to_string());
    runtime_store.save_enrichment_queue_state(&state)?;
    let mut projected = project_status(&state);
    projected.retried_failed_jobs = state.retried_failed_jobs;
    projected.last_action = Some("retry_failed".to_string());
    Ok(EnrichmentControlResult {
        message: format!("Requeued {retried} failed enrichment jobs."),
        state: projected,
    })
}

fn default_state() -> EnrichmentQueueState {
    EnrichmentQueueState {
        version: 1,
        last_action: Some("initialized".to_string()),
        ..EnrichmentQueueState::default()
    }
}

fn fallback_repo_identity(repo_root: &Path, repo_id: &str) -> RepoIdentity {
    let name = repo_root
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("repository")
        .to_string();
    RepoIdentity {
        provider: "git".to_string(),
        organization: "local".to_string(),
        name: name.clone(),
        identity: format!("git/local/{name}"),
        repo_id: repo_id.to_string(),
    }
}

fn build_batch_key(artefact_ids: &[String]) -> String {
    artefact_ids.join("|")
}

fn select_input_hashes_for_artefact_ids(
    input_hashes: &BTreeMap<String, String>,
    artefact_ids: &[String],
) -> BTreeMap<String, String> {
    artefact_ids
        .iter()
        .filter_map(|artefact_id| {
            input_hashes
                .get(artefact_id)
                .map(|hash| (artefact_id.clone(), hash.clone()))
        })
        .collect()
}

fn upsert_pending_semantic_job(
    state: &mut EnrichmentQueueState,
    target: &EnrichmentJobTarget,
    artefact_ids: Vec<String>,
    input_hashes: BTreeMap<String, String>,
) {
    let batch_key = build_batch_key(&artefact_ids);
    if let Some(existing) = state.jobs.iter_mut().find(|job| {
        job.repo_id == target.repo_id
            && job.branch == target.branch
            && matches!(
                (&job.status, &job.job),
                (
                    EnrichmentJobStatus::Pending,
                    EnrichmentJobKind::SemanticSummaries {
                        batch_key: existing_key,
                        ..
                    },
                ) if existing_key == &batch_key
            )
    }) {
        existing.updated_at_unix = unix_timestamp_now();
        existing.error = None;
        existing.job = EnrichmentJobKind::SemanticSummaries {
            artefact_ids,
            input_hashes,
            batch_key,
        };
        return;
    }

    state.jobs.push(EnrichmentJob {
        id: format!("semantic-job-{}", Uuid::new_v4()),
        repo_id: target.repo_id.clone(),
        repo_root: target.repo_root.clone(),
        config_root: target.config_root.clone(),
        branch: target.branch.clone(),
        status: EnrichmentJobStatus::Pending,
        attempts: 0,
        error: None,
        created_at_unix: unix_timestamp_now(),
        updated_at_unix: unix_timestamp_now(),
        job: EnrichmentJobKind::SemanticSummaries {
            artefact_ids,
            input_hashes,
            batch_key,
        },
    });
}

fn upsert_pending_embedding_job(
    state: &mut EnrichmentQueueState,
    target: &EnrichmentJobTarget,
    artefact_ids: Vec<String>,
    input_hashes: BTreeMap<String, String>,
    representation_kind: EmbeddingRepresentationKind,
) {
    let batch_key = build_batch_key(&artefact_ids);
    if let Some(existing) = state.jobs.iter_mut().find(|job| {
        job.repo_id == target.repo_id
            && job.branch == target.branch
            && matches!(
                (&job.status, &job.job),
                (
                    EnrichmentJobStatus::Pending,
                    EnrichmentJobKind::SymbolEmbeddings {
                        batch_key: existing_key,
                        representation_kind: existing_representation_kind,
                        ..
                    },
                ) if existing_key == &batch_key
                    && existing_representation_kind == &representation_kind
            )
    }) {
        existing.updated_at_unix = unix_timestamp_now();
        existing.error = None;
        existing.job = EnrichmentJobKind::SymbolEmbeddings {
            artefact_ids,
            input_hashes,
            batch_key,
            representation_kind,
        };
        return;
    }

    state.jobs.push(EnrichmentJob {
        id: format!("embedding-job-{}", Uuid::new_v4()),
        repo_id: target.repo_id.clone(),
        repo_root: target.repo_root.clone(),
        config_root: target.config_root.clone(),
        branch: target.branch.clone(),
        status: EnrichmentJobStatus::Pending,
        attempts: 0,
        error: None,
        created_at_unix: unix_timestamp_now(),
        updated_at_unix: unix_timestamp_now(),
        job: EnrichmentJobKind::SymbolEmbeddings {
            artefact_ids,
            input_hashes,
            batch_key,
            representation_kind,
        },
    });
}

#[cfg(test)]
#[path = "enrichment_tests.rs"]
mod tests;

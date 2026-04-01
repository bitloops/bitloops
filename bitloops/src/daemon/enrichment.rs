use anyhow::Result;
use serde::Deserializer;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use tokio::sync::{Mutex, Notify};
use tokio::time::{Duration, sleep};
use uuid::Uuid;

use crate::capability_packs::semantic_clones::features as semantic_features;
use crate::config::SemanticCloneEmbeddingMode;
use crate::daemon::state_store::{read_json, write_json};
use crate::host::devql::RepoIdentity;

use super::types::{
    EnrichmentQueueMode, EnrichmentQueueStatus, global_daemon_dir_fallback, unix_timestamp_now,
};

#[path = "enrichment/execution.rs"]
mod execution;
#[path = "enrichment/queue.rs"]
mod queue;

use execution::execute_job;
use queue::{job_is_paused, next_pending_job_index, project_status};

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
        embedding_mode: SemanticCloneEmbeddingMode,
    },
    SymbolEmbeddings {
        #[serde(alias = "inputs", deserialize_with = "deserialize_job_artefact_ids")]
        artefact_ids: Vec<String>,
        input_hashes: BTreeMap<String, String>,
        batch_key: String,
        embedding_mode: SemanticCloneEmbeddingMode,
    },
    CloneEdgesRebuild {
        embedding_mode: SemanticCloneEmbeddingMode,
    },
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
    state_path: PathBuf,
    lock: Mutex<()>,
    notify: Notify,
}

#[derive(Debug, Clone)]
enum FollowUpJob {
    SymbolEmbeddings {
        target: EnrichmentJobTarget,
        artefact_ids: Vec<String>,
        input_hashes: BTreeMap<String, String>,
        embedding_mode: SemanticCloneEmbeddingMode,
    },
    CloneEdgesRebuild {
        target: EnrichmentJobTarget,
        embedding_mode: SemanticCloneEmbeddingMode,
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
    LegacyInput(semantic_features::SemanticFeatureInput),
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
            PersistedEnrichmentJobInput::LegacyInput(input) => input.artefact_id,
        })
        .collect())
}

impl EnrichmentCoordinator {
    pub(crate) fn shared() -> Arc<Self> {
        static INSTANCE: OnceLock<Arc<EnrichmentCoordinator>> = OnceLock::new();
        Arc::clone(INSTANCE.get_or_init(|| {
            let coordinator = Arc::new(Self {
                state_path: enrichment_state_path(),
                lock: Mutex::new(()),
                notify: Notify::new(),
            });
            coordinator.ensure_state_file();
            coordinator.spawn_worker_if_possible();
            coordinator
        }))
    }

    pub async fn enqueue_semantic_summaries(
        &self,
        target: EnrichmentJobTarget,
        inputs: Vec<semantic_features::SemanticFeatureInput>,
        input_hashes: BTreeMap<String, String>,
        embedding_mode: SemanticCloneEmbeddingMode,
    ) -> Result<()> {
        if inputs.is_empty() {
            return Ok(());
        }
        let artefact_ids = inputs
            .iter()
            .map(|input| input.artefact_id.clone())
            .collect::<Vec<_>>();

        let _guard = self.lock.lock().await;
        let mut state = self.load_state()?;
        state
            .active_branch_by_repo
            .insert(target.repo_id.clone(), target.branch.clone());
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
                embedding_mode,
            };
        } else {
            state.jobs.push(EnrichmentJob {
                id: format!("semantic-job-{}", Uuid::new_v4()),
                repo_id: target.repo_id,
                repo_root: target.repo_root,
                config_root: target.config_root,
                branch: target.branch,
                status: EnrichmentJobStatus::Pending,
                attempts: 0,
                error: None,
                created_at_unix: unix_timestamp_now(),
                updated_at_unix: unix_timestamp_now(),
                job: EnrichmentJobKind::SemanticSummaries {
                    artefact_ids,
                    input_hashes,
                    batch_key,
                    embedding_mode,
                },
            });
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
        embedding_mode: SemanticCloneEmbeddingMode,
    ) -> Result<()> {
        if inputs.is_empty() {
            return Ok(());
        }
        let artefact_ids = inputs
            .iter()
            .map(|input| input.artefact_id.clone())
            .collect::<Vec<_>>();

        let _guard = self.lock.lock().await;
        let mut state = self.load_state()?;
        state
            .active_branch_by_repo
            .insert(target.repo_id.clone(), target.branch.clone());
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
                            ..
                        },
                    ) if existing_key == &batch_key
                )
        }) {
            existing.updated_at_unix = unix_timestamp_now();
            existing.error = None;
            existing.job = EnrichmentJobKind::SymbolEmbeddings {
                artefact_ids,
                input_hashes,
                batch_key,
                embedding_mode,
            };
        } else {
            state.jobs.push(EnrichmentJob {
                id: format!("embedding-job-{}", Uuid::new_v4()),
                repo_id: target.repo_id,
                repo_root: target.repo_root,
                config_root: target.config_root,
                branch: target.branch,
                status: EnrichmentJobStatus::Pending,
                attempts: 0,
                error: None,
                created_at_unix: unix_timestamp_now(),
                updated_at_unix: unix_timestamp_now(),
                job: EnrichmentJobKind::SymbolEmbeddings {
                    artefact_ids,
                    input_hashes,
                    batch_key,
                    embedding_mode,
                },
            });
        }
        state.last_action = Some("enqueue_embeddings".to_string());
        self.save_state(&mut state)?;
        self.notify.notify_waiters();
        Ok(())
    }

    pub async fn enqueue_clone_edges_rebuild(
        &self,
        target: EnrichmentJobTarget,
        embedding_mode: SemanticCloneEmbeddingMode,
    ) -> Result<()> {
        let _guard = self.lock.lock().await;
        let mut state = self.load_state()?;
        state
            .active_branch_by_repo
            .insert(target.repo_id.clone(), target.branch.clone());
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
                job: EnrichmentJobKind::CloneEdgesRebuild { embedding_mode },
            });
            state.last_action = Some("enqueue_clone_edges_rebuild".to_string());
            self.save_state(&mut state)?;
            self.notify.notify_waiters();
        }
        Ok(())
    }

    fn ensure_state_file(&self) {
        if self.state_path.exists() {
            return;
        }
        let mut state = default_state();
        let _ = self.save_state(&mut state);
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
            let Some(index) = next_pending_job_index(&state) else {
                return Ok(false);
            };
            if job_is_paused(&state, &state.jobs[index].job) {
                return Ok(false);
            }
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
                embedding_mode,
            } => {
                self.enqueue_symbol_embeddings_from_artefact_ids(
                    target,
                    artefact_ids,
                    input_hashes,
                    embedding_mode,
                )
                .await
            }
            FollowUpJob::CloneEdgesRebuild {
                target,
                embedding_mode,
            } => {
                self.enqueue_clone_edges_rebuild(target, embedding_mode)
                    .await
            }
        }
    }

    fn load_state(&self) -> Result<EnrichmentQueueState> {
        Ok(read_json::<EnrichmentQueueState>(&self.state_path)?.unwrap_or_else(default_state))
    }

    fn save_state(&self, state: &mut EnrichmentQueueState) -> Result<()> {
        state.version = 1;
        state.updated_at_unix = unix_timestamp_now();
        write_json(&self.state_path, state)
    }

    async fn enqueue_symbol_embeddings_from_artefact_ids(
        &self,
        target: EnrichmentJobTarget,
        artefact_ids: Vec<String>,
        input_hashes: BTreeMap<String, String>,
        embedding_mode: SemanticCloneEmbeddingMode,
    ) -> Result<()> {
        if artefact_ids.is_empty() {
            return Ok(());
        }

        let _guard = self.lock.lock().await;
        let mut state = self.load_state()?;
        state
            .active_branch_by_repo
            .insert(target.repo_id.clone(), target.branch.clone());
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
                            ..
                        },
                    ) if existing_key == &batch_key
                )
        }) {
            existing.updated_at_unix = unix_timestamp_now();
            existing.error = None;
            existing.job = EnrichmentJobKind::SymbolEmbeddings {
                artefact_ids,
                input_hashes,
                batch_key,
                embedding_mode,
            };
        } else {
            state.jobs.push(EnrichmentJob {
                id: format!("embedding-job-{}", Uuid::new_v4()),
                repo_id: target.repo_id,
                repo_root: target.repo_root,
                config_root: target.config_root,
                branch: target.branch,
                status: EnrichmentJobStatus::Pending,
                attempts: 0,
                error: None,
                created_at_unix: unix_timestamp_now(),
                updated_at_unix: unix_timestamp_now(),
                job: EnrichmentJobKind::SymbolEmbeddings {
                    artefact_ids,
                    input_hashes,
                    batch_key,
                    embedding_mode,
                },
            });
        }
        state.last_action = Some("enqueue_embeddings".to_string());
        self.save_state(&mut state)?;
        self.notify.notify_waiters();
        Ok(())
    }
}

pub fn snapshot() -> Result<EnrichmentQueueStatus> {
    let state =
        read_json::<EnrichmentQueueState>(&enrichment_state_path())?.unwrap_or_else(default_state);
    Ok(EnrichmentQueueStatus {
        state: project_status(&state),
        persisted: enrichment_state_path().exists(),
    })
}

pub fn pause_enrichments(reason: Option<String>) -> Result<EnrichmentControlResult> {
    let path = enrichment_state_path();
    let mut state = read_json::<EnrichmentQueueState>(&path)?.unwrap_or_else(default_state);
    state.paused_embeddings = true;
    state.paused_semantic = true;
    state.paused_reason = reason.clone();
    state.last_action = Some("paused".to_string());
    write_json(&path, &state)?;
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
    let path = enrichment_state_path();
    let mut state = read_json::<EnrichmentQueueState>(&path)?.unwrap_or_else(default_state);
    state.paused_embeddings = false;
    state.paused_semantic = false;
    state.paused_reason = None;
    state.last_action = Some("resumed".to_string());
    write_json(&path, &state)?;
    Ok(EnrichmentControlResult {
        message: "Enrichment queue resumed.".to_string(),
        state: project_status(&state),
    })
}

pub fn retry_failed_enrichments() -> Result<EnrichmentControlResult> {
    let path = enrichment_state_path();
    let mut state = read_json::<EnrichmentQueueState>(&path)?.unwrap_or_else(default_state);
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
    write_json(&path, &state)?;
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

fn enrichment_state_path() -> PathBuf {
    global_daemon_dir_fallback().join(super::types::ENRICHMENT_STATE_FILE_NAME)
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_input() -> semantic_features::SemanticFeatureInput {
        semantic_features::SemanticFeatureInput {
            artefact_id: "artefact-1".to_string(),
            symbol_id: Some("symbol-1".to_string()),
            repo_id: "repo-1".to_string(),
            blob_sha: "blob-1".to_string(),
            path: "src/service.rs".to_string(),
            language: "rust".to_string(),
            canonical_kind: "function".to_string(),
            language_kind: "function".to_string(),
            symbol_fqn: "src/service.rs::load_user".to_string(),
            name: "load_user".to_string(),
            signature: Some("fn load_user(id: &str)".to_string()),
            modifiers: vec!["pub".to_string()],
            body: "load_user_impl(id)".to_string(),
            docstring: Some("Loads a user.".to_string()),
            parent_kind: None,
            dependency_signals: vec!["calls:user_store::load".to_string()],
            content_hash: Some("content-hash".to_string()),
        }
    }

    #[test]
    fn enrichment_job_kind_serializes_lightweight_artefact_ids() {
        let job = EnrichmentJobKind::SemanticSummaries {
            artefact_ids: vec!["artefact-1".to_string()],
            input_hashes: BTreeMap::from([("artefact-1".to_string(), "hash-1".to_string())]),
            batch_key: "artefact-1".to_string(),
            embedding_mode: SemanticCloneEmbeddingMode::SemanticAwareOnce,
        };

        let value = serde_json::to_value(job).expect("serialize job kind");
        assert_eq!(
            value.get("kind").and_then(|value| value.as_str()),
            Some("semantic_summaries")
        );
        assert_eq!(
            value
                .get("artefact_ids")
                .and_then(|value| value.as_array())
                .map(|values| values.len()),
            Some(1)
        );
        assert!(value.get("inputs").is_none());
    }

    #[test]
    fn enrichment_job_kind_deserializes_legacy_inputs_into_artefact_ids() {
        let input = sample_input();
        let job = serde_json::from_value::<EnrichmentJobKind>(json!({
            "kind": "semantic_summaries",
            "inputs": [input],
            "input_hashes": { "artefact-1": "hash-1" },
            "batch_key": "artefact-1",
            "embedding_mode": "semantic_aware_once"
        }))
        .expect("deserialize legacy job kind");

        match job {
            EnrichmentJobKind::SemanticSummaries { artefact_ids, .. } => {
                assert_eq!(artefact_ids, vec!["artefact-1".to_string()]);
            }
            other => panic!("expected semantic summaries job, got {other:?}"),
        }
    }
}

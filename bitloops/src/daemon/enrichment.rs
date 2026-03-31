use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use tokio::sync::{Mutex, Notify};
use tokio::time::{Duration, sleep};
use uuid::Uuid;

use crate::capability_packs::semantic_clones::embeddings::EmbeddingProviderConfig;
use crate::capability_packs::semantic_clones::extension_descriptor::{
    build_semantic_summary_provider, build_symbol_embedding_provider,
};
use crate::capability_packs::semantic_clones::features as semantic_features;
use crate::capability_packs::semantic_clones::features::SemanticSummaryProviderConfig;
use crate::capability_packs::semantic_clones::{
    clear_repo_symbol_embedding_rows, load_semantic_summary_snapshot, persist_semantic_summary_row,
    upsert_symbol_embedding_rows,
};
use crate::config::{
    BITLOOPS_CONFIG_RELATIVE_PATH, SemanticCloneEmbeddingMode,
    resolve_embedding_capability_config_for_repo, resolve_store_backend_config_for_repo,
    resolve_store_semantic_config_for_repo,
};
use crate::daemon::state_store::{read_json, write_json};
use crate::host::devql::{DevqlConfig, RelationalStorage, RepoIdentity, resolve_repo_identity};

use super::types::{
    EnrichmentQueueMode, EnrichmentQueueStatus, global_daemon_dir_fallback, unix_timestamp_now,
};

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
        inputs: Vec<semantic_features::SemanticFeatureInput>,
        input_hashes: BTreeMap<String, String>,
        batch_key: String,
        embedding_mode: SemanticCloneEmbeddingMode,
    },
    SymbolEmbeddings {
        inputs: Vec<semantic_features::SemanticFeatureInput>,
        input_hashes: BTreeMap<String, String>,
        batch_key: String,
        embedding_mode: SemanticCloneEmbeddingMode,
    },
    CloneRebuild {
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

#[derive(Debug)]
pub struct EnrichmentCoordinator {
    state_path: PathBuf,
    lock: Mutex<()>,
    notify: Notify,
}

#[derive(Debug, Clone)]
enum FollowUpJob {
    SymbolEmbeddings {
        config_root: PathBuf,
        repo_root: PathBuf,
        repo_id: String,
        branch: String,
        inputs: Vec<semantic_features::SemanticFeatureInput>,
        input_hashes: BTreeMap<String, String>,
        embedding_mode: SemanticCloneEmbeddingMode,
    },
    CloneRebuild {
        config_root: PathBuf,
        repo_root: PathBuf,
        repo_id: String,
        branch: String,
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
        config_root: PathBuf,
        repo_root: PathBuf,
        repo_id: String,
        branch: String,
        inputs: Vec<semantic_features::SemanticFeatureInput>,
        input_hashes: BTreeMap<String, String>,
        embedding_mode: SemanticCloneEmbeddingMode,
    ) -> Result<()> {
        if inputs.is_empty() {
            return Ok(());
        }

        let _guard = self.lock.lock().await;
        let mut state = self.load_state()?;
        state.active_branch_by_repo.insert(repo_id.clone(), branch.clone());
        let batch_key = build_batch_key(&inputs);
        if let Some(existing) = state.jobs.iter_mut().find(|job| {
            job.repo_id == repo_id
                && job.branch == branch
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
                inputs,
                input_hashes,
                batch_key,
                embedding_mode,
            };
        } else {
            state.jobs.push(EnrichmentJob {
                id: format!("semantic-job-{}", Uuid::new_v4()),
                repo_id,
                repo_root,
                config_root,
                branch,
                status: EnrichmentJobStatus::Pending,
                attempts: 0,
                error: None,
                created_at_unix: unix_timestamp_now(),
                updated_at_unix: unix_timestamp_now(),
                job: EnrichmentJobKind::SemanticSummaries {
                    inputs,
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
        config_root: PathBuf,
        repo_root: PathBuf,
        repo_id: String,
        branch: String,
        inputs: Vec<semantic_features::SemanticFeatureInput>,
        input_hashes: BTreeMap<String, String>,
        embedding_mode: SemanticCloneEmbeddingMode,
    ) -> Result<()> {
        if inputs.is_empty() {
            return Ok(());
        }

        let _guard = self.lock.lock().await;
        let mut state = self.load_state()?;
        state.active_branch_by_repo.insert(repo_id.clone(), branch.clone());
        let batch_key = build_batch_key(&inputs);
        if let Some(existing) = state.jobs.iter_mut().find(|job| {
            job.repo_id == repo_id
                && job.branch == branch
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
                inputs,
                input_hashes,
                batch_key,
                embedding_mode,
            };
        } else {
            state.jobs.push(EnrichmentJob {
                id: format!("embedding-job-{}", Uuid::new_v4()),
                repo_id,
                repo_root,
                config_root,
                branch,
                status: EnrichmentJobStatus::Pending,
                attempts: 0,
                error: None,
                created_at_unix: unix_timestamp_now(),
                updated_at_unix: unix_timestamp_now(),
                job: EnrichmentJobKind::SymbolEmbeddings {
                    inputs,
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

    pub async fn enqueue_clone_rebuild(
        &self,
        config_root: PathBuf,
        repo_root: PathBuf,
        repo_id: String,
        branch: String,
        embedding_mode: SemanticCloneEmbeddingMode,
    ) -> Result<()> {
        let _guard = self.lock.lock().await;
        let mut state = self.load_state()?;
        state.active_branch_by_repo.insert(repo_id.clone(), branch.clone());
        let has_existing = state.jobs.iter().any(|job| {
            job.repo_id == repo_id
                && matches!(
                    (&job.status, &job.job),
                    (
                        EnrichmentJobStatus::Pending | EnrichmentJobStatus::Running,
                        EnrichmentJobKind::CloneRebuild { .. }
                    )
                )
        });
        if !has_existing {
            state.jobs.push(EnrichmentJob {
                id: format!("clone-rebuild-{}", Uuid::new_v4()),
                repo_id,
                repo_root,
                config_root,
                branch,
                status: EnrichmentJobStatus::Pending,
                attempts: 0,
                error: None,
                created_at_unix: unix_timestamp_now(),
                updated_at_unix: unix_timestamp_now(),
                job: EnrichmentJobKind::CloneRebuild { embedding_mode },
            });
            state.last_action = Some("enqueue_clone_rebuild".to_string());
            self.save_state(&mut state)?;
            self.notify.notify_waiters();
        }
        Ok(())
    }

    fn ensure_state_file(&self) {
        if self.state_path.exists() {
            return;
        }
        let mut state = EnrichmentQueueState {
            version: 1,
            last_action: Some("initialized".to_string()),
            ..EnrichmentQueueState::default()
        };
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
                config_root,
                repo_root,
                repo_id,
                branch,
                inputs,
                input_hashes,
                embedding_mode,
            } => {
                self.enqueue_symbol_embeddings(
                    config_root,
                    repo_root,
                    repo_id,
                    branch,
                    inputs,
                    input_hashes,
                    embedding_mode,
                )
                .await
            }
            FollowUpJob::CloneRebuild {
                config_root,
                repo_root,
                repo_id,
                branch,
                embedding_mode,
            } => {
                self.enqueue_clone_rebuild(
                    config_root,
                    repo_root,
                    repo_id,
                    branch,
                    embedding_mode,
                )
                .await
            }
        }
    }

    fn load_state(&self) -> Result<EnrichmentQueueState> {
        Ok(read_json::<EnrichmentQueueState>(&self.state_path)?.unwrap_or(EnrichmentQueueState {
            version: 1,
            last_action: Some("initialized".to_string()),
            ..EnrichmentQueueState::default()
        }))
    }

    fn save_state(&self, state: &mut EnrichmentQueueState) -> Result<()> {
        state.version = 1;
        state.updated_at_unix = unix_timestamp_now();
        write_json(&self.state_path, state)
    }
}

pub fn snapshot() -> Result<EnrichmentQueueStatus> {
    let state = read_json::<EnrichmentQueueState>(&enrichment_state_path())?.unwrap_or(
        EnrichmentQueueState {
            version: 1,
            last_action: Some("initialized".to_string()),
            ..EnrichmentQueueState::default()
        },
    );
    Ok(EnrichmentQueueStatus {
        state: project_status(&state),
        persisted: enrichment_state_path().exists(),
    })
}

pub fn pause_enrichments(reason: Option<String>) -> Result<EnrichmentControlResult> {
    let path = enrichment_state_path();
    let mut state = read_json::<EnrichmentQueueState>(&path)?.unwrap_or(EnrichmentQueueState {
        version: 1,
        last_action: Some("initialized".to_string()),
        ..EnrichmentQueueState::default()
    });
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
    let mut state = read_json::<EnrichmentQueueState>(&path)?.unwrap_or(EnrichmentQueueState {
        version: 1,
        last_action: Some("initialized".to_string()),
        ..EnrichmentQueueState::default()
    });
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
    let mut state = read_json::<EnrichmentQueueState>(&path)?.unwrap_or(EnrichmentQueueState {
        version: 1,
        last_action: Some("initialized".to_string()),
        ..EnrichmentQueueState::default()
    });
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

fn next_pending_job_index(state: &EnrichmentQueueState) -> Option<usize> {
    state
        .jobs
        .iter()
        .enumerate()
        .filter(|(_, job)| job.status == EnrichmentJobStatus::Pending)
        .min_by_key(|(_, job)| {
            let active_branch = state.active_branch_by_repo.get(&job.repo_id);
            let branch_rank = match active_branch {
                Some(active_branch) if active_branch == &job.branch => 0usize,
                Some(_) => 1usize,
                None => 0usize,
            };
            (branch_rank, job_kind_priority(&job.job), job.created_at_unix)
        })
        .map(|(index, _)| index)
}

fn job_is_paused(state: &EnrichmentQueueState, job: &EnrichmentJobKind) -> bool {
    match job {
        EnrichmentJobKind::SemanticSummaries { .. } => state.paused_semantic,
        EnrichmentJobKind::SymbolEmbeddings { .. } | EnrichmentJobKind::CloneRebuild { .. } => {
            state.paused_embeddings
        }
    }
}

fn job_kind_priority(job: &EnrichmentJobKind) -> usize {
    match job {
        EnrichmentJobKind::SemanticSummaries { .. } => 0,
        EnrichmentJobKind::SymbolEmbeddings { .. } => 1,
        EnrichmentJobKind::CloneRebuild { .. } => 2,
    }
}

async fn execute_job(job: &EnrichmentJob) -> JobExecutionOutcome {
    let repo = resolve_repo_identity(&job.repo_root)
        .unwrap_or_else(|_| fallback_repo_identity(&job.repo_root, &job.repo_id));
    let cfg = match DevqlConfig::from_roots(job.config_root.clone(), job.repo_root.clone(), repo) {
        Ok(cfg) => cfg,
        Err(err) => return JobExecutionOutcome::failed(err),
    };
    let backends = match resolve_store_backend_config_for_repo(&job.config_root) {
        Ok(backends) => backends,
        Err(err) => return JobExecutionOutcome::failed(err),
    };
    let relational = match RelationalStorage::connect(&cfg, &backends.relational, "daemon enrichment worker").await {
        Ok(relational) => relational,
        Err(err) => return JobExecutionOutcome::failed(err),
    };

    match &job.job {
        EnrichmentJobKind::SemanticSummaries {
            inputs,
            input_hashes,
            embedding_mode,
            ..
        } => {
            execute_semantic_job(&relational, job, inputs, input_hashes, *embedding_mode).await
        }
        EnrichmentJobKind::SymbolEmbeddings {
            inputs,
            input_hashes,
            embedding_mode,
            ..
        } => execute_embedding_job(&relational, job, inputs, input_hashes, *embedding_mode).await,
        EnrichmentJobKind::CloneRebuild { embedding_mode } => {
            execute_clone_rebuild_job(&cfg, &relational, job, *embedding_mode).await
        }
    }
}

async fn execute_semantic_job(
    relational: &RelationalStorage,
    job: &EnrichmentJob,
    inputs: &[semantic_features::SemanticFeatureInput],
    input_hashes: &BTreeMap<String, String>,
    embedding_mode: SemanticCloneEmbeddingMode,
) -> JobExecutionOutcome {
    let semantic_cfg = resolve_store_semantic_config_for_repo(&job.config_root);
    let summary_provider = match build_semantic_summary_provider(&SemanticSummaryProviderConfig {
        semantic_provider: semantic_cfg.semantic_provider,
        semantic_model: semantic_cfg.semantic_model,
        semantic_api_key: semantic_cfg.semantic_api_key,
        semantic_base_url: semantic_cfg.semantic_base_url,
    }) {
        Ok(provider) => provider,
        Err(err) => {
            let mut outcome = JobExecutionOutcome::failed(err);
            if embedding_mode == SemanticCloneEmbeddingMode::SemanticAwareOnce {
                outcome.follow_ups.push(FollowUpJob::SymbolEmbeddings {
                    config_root: job.config_root.clone(),
                    repo_root: job.repo_root.clone(),
                    repo_id: job.repo_id.clone(),
                    branch: job.branch.clone(),
                    inputs: inputs.to_vec(),
                    input_hashes: input_hashes.clone(),
                    embedding_mode,
                });
            }
            return outcome;
        }
    };

    let mut summary_changed = false;
    for input in inputs {
        let Some(expected_hash) = input_hashes.get(&input.artefact_id) else {
            continue;
        };
        let current = match load_semantic_summary_snapshot(relational, &input.artefact_id).await {
            Ok(snapshot) => snapshot,
            Err(err) => return JobExecutionOutcome::failed(err),
        };
        let Some(current) = current else {
            continue;
        };
        if current.semantic_features_input_hash != *expected_hash {
            continue;
        }

        let input = input.clone();
        let summary_provider = Arc::clone(&summary_provider);
        let rows = match tokio::task::spawn_blocking(move || {
            semantic_features::build_semantic_feature_rows(&input, summary_provider.as_ref())
        })
        .await
        .context("building queued semantic summary rows on blocking worker")
        {
            Ok(rows) => rows,
            Err(err) => {
                let mut outcome = JobExecutionOutcome::failed(err);
                if embedding_mode == SemanticCloneEmbeddingMode::SemanticAwareOnce {
                    outcome.follow_ups.push(FollowUpJob::SymbolEmbeddings {
                        config_root: job.config_root.clone(),
                        repo_root: job.repo_root.clone(),
                        repo_id: job.repo_id.clone(),
                        branch: job.branch.clone(),
                        inputs: inputs.to_vec(),
                        input_hashes: input_hashes.clone(),
                        embedding_mode,
                    });
                }
                return outcome;
            }
        };

        if current.summary != rows.semantics.summary {
            summary_changed = true;
        }
        if let Err(err) = persist_semantic_summary_row(
            relational,
            &rows.semantics,
            expected_hash,
        )
        .await
        {
            let mut outcome = JobExecutionOutcome::failed(err);
            if embedding_mode == SemanticCloneEmbeddingMode::SemanticAwareOnce {
                outcome.follow_ups.push(FollowUpJob::SymbolEmbeddings {
                    config_root: job.config_root.clone(),
                    repo_root: job.repo_root.clone(),
                    repo_id: job.repo_id.clone(),
                    branch: job.branch.clone(),
                    inputs: inputs.to_vec(),
                    input_hashes: input_hashes.clone(),
                    embedding_mode,
                });
            }
            return outcome;
        }
    }

    let mut outcome = JobExecutionOutcome::ok();
    match embedding_mode {
        SemanticCloneEmbeddingMode::SemanticAwareOnce => {
            outcome.follow_ups.push(FollowUpJob::SymbolEmbeddings {
                config_root: job.config_root.clone(),
                repo_root: job.repo_root.clone(),
                repo_id: job.repo_id.clone(),
                branch: job.branch.clone(),
                inputs: inputs.to_vec(),
                input_hashes: input_hashes.clone(),
                embedding_mode,
            });
        }
        SemanticCloneEmbeddingMode::RefreshOnUpgrade if summary_changed => {
            outcome.follow_ups.push(FollowUpJob::SymbolEmbeddings {
                config_root: job.config_root.clone(),
                repo_root: job.repo_root.clone(),
                repo_id: job.repo_id.clone(),
                branch: job.branch.clone(),
                inputs: inputs.to_vec(),
                input_hashes: input_hashes.clone(),
                embedding_mode,
            });
        }
        _ => {}
    }
    outcome
}

async fn execute_embedding_job(
    relational: &RelationalStorage,
    job: &EnrichmentJob,
    inputs: &[semantic_features::SemanticFeatureInput],
    input_hashes: &BTreeMap<String, String>,
    embedding_mode: SemanticCloneEmbeddingMode,
) -> JobExecutionOutcome {
    if embedding_mode == SemanticCloneEmbeddingMode::Off {
        return match clear_embedding_outputs(relational, &job.repo_id).await {
            Ok(()) => JobExecutionOutcome::ok(),
            Err(err) => JobExecutionOutcome::failed(err),
        };
    }

    let current_inputs = match filter_current_inputs(relational, inputs, input_hashes).await {
        Ok(filtered) => filtered,
        Err(err) => return JobExecutionOutcome::failed(err),
    };
    if current_inputs.is_empty() {
        return JobExecutionOutcome::ok();
    }

    let capability = resolve_embedding_capability_config_for_repo(&job.config_root);
    let provider_config = EmbeddingProviderConfig {
        daemon_config_path: job.config_root.join(BITLOOPS_CONFIG_RELATIVE_PATH),
        embedding_profile: capability.semantic_clones.embedding_profile,
        runtime_command: capability.embeddings.runtime.command,
        runtime_args: capability.embeddings.runtime.args,
        startup_timeout_secs: capability.embeddings.runtime.startup_timeout_secs,
        request_timeout_secs: capability.embeddings.runtime.request_timeout_secs,
        warnings: capability.embeddings.warnings,
    };

    let provider = match build_symbol_embedding_provider(&provider_config, Some(&job.repo_root)) {
        Ok(provider) => provider,
        Err(err) => {
            let error = format!("{err:#}");
            return match clear_embedding_outputs(relational, &job.repo_id).await {
                Ok(()) => JobExecutionOutcome {
                    error: Some(error),
                    follow_ups: Vec::new(),
                },
                Err(clear_err) => JobExecutionOutcome::failed(clear_err),
            };
        }
    };
    let Some(provider) = provider else {
        return match clear_embedding_outputs(relational, &job.repo_id).await {
            Ok(()) => JobExecutionOutcome::ok(),
            Err(err) => JobExecutionOutcome::failed(err),
        };
    };

    let provider =
        Arc::<dyn crate::adapters::model_providers::embeddings::EmbeddingProvider>::from(provider);
    if let Err(err) = upsert_symbol_embedding_rows(relational, &current_inputs, provider).await {
        let error = format!("{err:#}");
        return match clear_embedding_outputs(relational, &job.repo_id).await {
            Ok(()) => JobExecutionOutcome {
                error: Some(error),
                follow_ups: Vec::new(),
            },
            Err(clear_err) => JobExecutionOutcome::failed(clear_err),
        };
    }

    let mut outcome = JobExecutionOutcome::ok();
    outcome.follow_ups.push(FollowUpJob::CloneRebuild {
        config_root: job.config_root.clone(),
        repo_root: job.repo_root.clone(),
        repo_id: job.repo_id.clone(),
        branch: job.branch.clone(),
        embedding_mode,
    });
    outcome
}

async fn execute_clone_rebuild_job(
    _cfg: &DevqlConfig,
    relational: &RelationalStorage,
    job: &EnrichmentJob,
    embedding_mode: SemanticCloneEmbeddingMode,
) -> JobExecutionOutcome {
    if embedding_mode == SemanticCloneEmbeddingMode::Off {
        return match clear_embedding_outputs(relational, &job.repo_id).await {
            Ok(()) => JobExecutionOutcome::ok(),
            Err(err) => JobExecutionOutcome::failed(err),
        };
    }

    let capability = resolve_embedding_capability_config_for_repo(&job.config_root);
    if capability.semantic_clones.embedding_profile.is_none() {
        return match clear_embedding_outputs(relational, &job.repo_id).await {
            Ok(()) => JobExecutionOutcome::ok(),
            Err(err) => JobExecutionOutcome::failed(err),
        };
    }

    match crate::capability_packs::semantic_clones::pipeline::rebuild_symbol_clone_edges(
        relational,
        &job.repo_id,
    )
    .await
    {
        Ok(_) => JobExecutionOutcome::ok(),
        Err(err) => JobExecutionOutcome::failed(err),
    }
}

async fn clear_embedding_outputs(relational: &RelationalStorage, repo_id: &str) -> Result<()> {
    clear_repo_symbol_embedding_rows(relational, repo_id).await?;
    crate::capability_packs::semantic_clones::pipeline::delete_repo_symbol_clone_edges(
        relational,
        repo_id,
    )
    .await
}

async fn filter_current_inputs(
    relational: &RelationalStorage,
    inputs: &[semantic_features::SemanticFeatureInput],
    input_hashes: &BTreeMap<String, String>,
) -> Result<Vec<semantic_features::SemanticFeatureInput>> {
    let mut filtered = Vec::with_capacity(inputs.len());
    for input in inputs {
        let Some(expected_hash) = input_hashes.get(&input.artefact_id) else {
            continue;
        };
        let Some(snapshot) = load_semantic_summary_snapshot(relational, &input.artefact_id).await?
        else {
            continue;
        };
        if snapshot.semantic_features_input_hash == *expected_hash {
            filtered.push(input.clone());
        }
    }
    Ok(filtered)
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

fn build_batch_key(inputs: &[semantic_features::SemanticFeatureInput]) -> String {
    inputs
        .iter()
        .map(|input| input.artefact_id.as_str())
        .collect::<Vec<_>>()
        .join("|")
}

fn project_status(state: &EnrichmentQueueState) -> super::types::EnrichmentQueueState {
    let pending_semantic_jobs = count_jobs(
        state,
        EnrichmentJobStatus::Pending,
        |job| matches!(job, EnrichmentJobKind::SemanticSummaries { .. }),
    );
    let pending_embedding_jobs = count_jobs(
        state,
        EnrichmentJobStatus::Pending,
        |job| matches!(job, EnrichmentJobKind::SymbolEmbeddings { .. }),
    );
    let pending_clone_rebuild_jobs = count_jobs(
        state,
        EnrichmentJobStatus::Pending,
        |job| matches!(job, EnrichmentJobKind::CloneRebuild { .. }),
    );
    let running_semantic_jobs = count_jobs(
        state,
        EnrichmentJobStatus::Running,
        |job| matches!(job, EnrichmentJobKind::SemanticSummaries { .. }),
    );
    let running_embedding_jobs = count_jobs(
        state,
        EnrichmentJobStatus::Running,
        |job| matches!(job, EnrichmentJobKind::SymbolEmbeddings { .. }),
    );
    let running_clone_rebuild_jobs = count_jobs(
        state,
        EnrichmentJobStatus::Running,
        |job| matches!(job, EnrichmentJobKind::CloneRebuild { .. }),
    );
    let failed_semantic_jobs = count_jobs(
        state,
        EnrichmentJobStatus::Failed,
        |job| matches!(job, EnrichmentJobKind::SemanticSummaries { .. }),
    );
    let failed_embedding_jobs = count_jobs(
        state,
        EnrichmentJobStatus::Failed,
        |job| matches!(job, EnrichmentJobKind::SymbolEmbeddings { .. }),
    );
    let failed_clone_rebuild_jobs = count_jobs(
        state,
        EnrichmentJobStatus::Failed,
        |job| matches!(job, EnrichmentJobKind::CloneRebuild { .. }),
    );

    super::types::EnrichmentQueueState {
        version: state.version,
        mode: if state.paused_embeddings || state.paused_semantic {
            EnrichmentQueueMode::Paused
        } else {
            EnrichmentQueueMode::Running
        },
        pending_jobs: pending_semantic_jobs + pending_embedding_jobs + pending_clone_rebuild_jobs,
        pending_semantic_jobs,
        pending_embedding_jobs,
        pending_clone_rebuild_jobs,
        running_jobs: running_semantic_jobs + running_embedding_jobs + running_clone_rebuild_jobs,
        running_semantic_jobs,
        running_embedding_jobs,
        running_clone_rebuild_jobs,
        failed_jobs: failed_semantic_jobs + failed_embedding_jobs + failed_clone_rebuild_jobs,
        failed_semantic_jobs,
        failed_embedding_jobs,
        failed_clone_rebuild_jobs,
        retried_failed_jobs: state.retried_failed_jobs,
        last_action: state.last_action.clone(),
        last_updated_unix: state.updated_at_unix,
        paused_reason: state.paused_reason.clone(),
    }
}

fn count_jobs(
    state: &EnrichmentQueueState,
    status: EnrichmentJobStatus,
    predicate: impl Fn(&EnrichmentJobKind) -> bool,
) -> u64 {
    state
        .jobs
        .iter()
        .filter(|job| job.status == status && predicate(&job.job))
        .count() as u64
}

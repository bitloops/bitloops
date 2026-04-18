use anyhow::Result;
use serde::Deserializer;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use tokio::sync::{Mutex, Notify};
use tokio::time::{Duration, sleep};

#[cfg(test)]
use crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CAPABILITY_ID;
use crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind;
use crate::capability_packs::semantic_clones::features as semantic_features;
#[cfg(test)]
use crate::capability_packs::semantic_clones::types::{
    SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX, SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX, SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
};
use crate::config::resolve_repo_runtime_db_path_for_config_root;
use crate::graphql::SubscriptionHub;
use crate::host::runtime_store::{DaemonSqliteRuntimeStore, WorkplaneJobStatus};
use rusqlite::params;

use super::types::{
    BlockedMailboxStatus, EnrichmentQueueMode, EnrichmentQueueStatus, unix_timestamp_now,
};

#[path = "enrichment/execution.rs"]
mod execution;
#[path = "enrichment/worker_count.rs"]
pub(crate) mod worker_count;
#[path = "enrichment/workplane.rs"]
mod workplane;

#[cfg(test)]
use self::workplane::load_workplane_jobs_by_status;
use self::workplane::{
    WorkplaneJobCompletionDisposition, claim_next_workplane_job, compact_and_prune_workplane_jobs,
    current_workplane_mailbox_blocked_statuses,
    current_workplane_mailbox_blocked_statuses_for_repo, default_state,
    enqueue_workplane_clone_rebuild, enqueue_workplane_embedding_jobs,
    enqueue_workplane_embedding_repo_backfill_job, enqueue_workplane_summary_jobs,
    iter_workplane_job_config_roots, last_failed_embedding_job_from_workplane,
    persist_workplane_job_completion, project_workplane_status, retry_failed_workplane_jobs,
    sql_i64,
};
use worker_count::{
    EnrichmentWorkerBudgets, EnrichmentWorkerPool, configured_enrichment_worker_budgets_for_repo,
};

#[cfg(test)]
const MAX_SEMANTIC_ENRICHMENT_JOB_ARTEFACTS: usize = 32;
const WORKPLANE_PENDING_COMPACTION_MIN_AGE_SECS: u64 = 900;
const WORKPLANE_PENDING_COMPACTION_MIN_COUNT: u64 = 10_000;
const WORKPLANE_TERMINAL_RETENTION_SECS: u64 = 7 * 24 * 60 * 60;
const WORKPLANE_TERMINAL_ROW_LIMIT: u64 = 1_000;

#[derive(Debug, Clone, Default)]
struct WorkplaneMailboxReadiness {
    blocked: bool,
    reason: Option<String>,
}

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
pub struct EnrichmentControlState {
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
    init_session_id: Option<String>,
}

impl EnrichmentJobTarget {
    pub fn new(config_root: PathBuf, repo_root: PathBuf) -> Self {
        Self {
            config_root,
            repo_root,
            init_session_id: None,
        }
    }

    pub fn with_init_session_id(mut self, init_session_id: Option<String>) -> Self {
        self.init_session_id = init_session_id;
        self
    }
}

#[derive(Debug)]
pub struct EnrichmentCoordinator {
    runtime_store: DaemonSqliteRuntimeStore,
    workplane_store: DaemonSqliteRuntimeStore,
    daemon_config_root: PathBuf,
    lock: Mutex<()>,
    notify: Notify,
    state_initialised: AtomicBool,
    maintenance_started: AtomicBool,
    started_worker_counts: std::sync::Mutex<EnrichmentWorkerBudgets>,
    subscription_hub: std::sync::Mutex<Option<Arc<SubscriptionHub>>>,
}

#[derive(Debug, Clone)]
enum FollowUpJob {
    SemanticSummaries {
        target: EnrichmentJobTarget,
        artefact_ids: Vec<String>,
    },
    RepoBackfillEmbeddings {
        target: EnrichmentJobTarget,
        artefact_ids: Vec<String>,
        representation_kind: EmbeddingRepresentationKind,
    },
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
        let coordinator =
            Arc::clone(
                INSTANCE.get_or_init(|| {
                    let daemon_config = crate::daemon::resolve_daemon_config(None)
                        .expect("resolving daemon config");
                    Arc::new(Self {
                        runtime_store: DaemonSqliteRuntimeStore::open()
                            .expect("opening daemon runtime store for enrichment controls"),
                        workplane_store: DaemonSqliteRuntimeStore::open_at(
                            resolve_repo_runtime_db_path_for_config_root(
                                &daemon_config.config_root,
                            ),
                        )
                        .expect("opening repo runtime workplane store for enrichment queue"),
                        daemon_config_root: daemon_config.config_root.clone(),
                        lock: Mutex::new(()),
                        notify: Notify::new(),
                        state_initialised: AtomicBool::new(false),
                        maintenance_started: AtomicBool::new(false),
                        started_worker_counts: std::sync::Mutex::new(
                            EnrichmentWorkerBudgets::default(),
                        ),
                        subscription_hub: std::sync::Mutex::new(None),
                    })
                }),
            );
        coordinator.ensure_started();
        coordinator
    }

    pub(crate) fn set_subscription_hub(&self, subscription_hub: Arc<SubscriptionHub>) {
        if let Ok(mut slot) = self.subscription_hub.lock() {
            *slot = Some(subscription_hub);
        }
    }

    pub(crate) fn ensure_started(self: &Arc<Self>) {
        if !self.state_initialised.swap(true, Ordering::AcqRel) {
            self.ensure_state_file();
            self.requeue_running_jobs();
            let _ = compact_and_prune_workplane_jobs(&self.workplane_store);
        }
        self.ensure_maintenance_loop();
        self.ensure_worker_capacity();
    }

    fn ensure_maintenance_loop(self: &Arc<Self>) {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        if self.maintenance_started.swap(true, Ordering::AcqRel) {
            return;
        }
        let coordinator = Arc::clone(self);
        handle.spawn(async move {
            coordinator.maintenance_loop().await;
        });
    }

    async fn maintenance_loop(self: Arc<Self>) {
        loop {
            sleep(Duration::from_secs(60)).await;
            if let Err(err) = self.run_maintenance_pass().await {
                log::warn!("capability workplane maintenance failed: {err:#}");
            }
        }
    }

    async fn run_maintenance_pass(&self) -> Result<()> {
        let _guard = self.lock.lock().await;
        compact_and_prune_workplane_jobs(&self.workplane_store)
    }

    fn ensure_worker_capacity(self: &Arc<Self>) {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let budgets = effective_worker_budgets(&self.workplane_store, &self.daemon_config_root)
            .unwrap_or_else(|err| {
                log::warn!(
                    "failed to resolve effective enrichment worker budgets from `{}`: {err:#}",
                    self.daemon_config_root.display()
                );
                configured_enrichment_worker_budgets_for_repo(&self.daemon_config_root)
            });
        let Ok(mut started_worker_counts) = self.started_worker_counts.lock() else {
            log::warn!("failed to lock enrichment worker counts; skipping worker-capacity update");
            return;
        };
        for pool in [
            EnrichmentWorkerPool::SummaryRefresh,
            EnrichmentWorkerPool::Embeddings,
            EnrichmentWorkerPool::CloneRebuild,
        ] {
            let current_count = started_worker_counts.for_pool(pool);
            let desired_count = budgets.for_pool(pool);
            if desired_count <= current_count {
                continue;
            }
            let additional_workers = desired_count - current_count;
            if additional_workers > 0 {
                log::info!(
                    "starting {} additional enrichment workers for pool {} (total {})",
                    additional_workers,
                    pool.as_str(),
                    desired_count
                );
            }
            started_worker_counts.set_for_pool(pool, desired_count);
            for _ in 0..additional_workers {
                let coordinator = Arc::clone(self);
                handle.spawn(async move {
                    coordinator.run_loop(pool).await;
                });
            }
        }
    }

    pub async fn enqueue_semantic_summaries(
        &self,
        target: EnrichmentJobTarget,
        inputs: Vec<semantic_features::SemanticFeatureInput>,
        input_hashes: BTreeMap<String, String>,
    ) -> Result<()> {
        let _ = input_hashes;
        enqueue_workplane_summary_jobs(
            &target,
            inputs.into_iter().map(|input| input.artefact_id).collect(),
        )?;
        let _guard = self.lock.lock().await;
        let mut state = self.load_state()?;
        state.last_action = Some("enqueue_semantic".to_string());
        self.save_state(&mut state)?;
        self.notify.notify_waiters();
        publish_workplane_runtime_event(
            &target,
            crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
        )?;
        Ok(())
    }

    pub async fn enqueue_symbol_embeddings(
        &self,
        target: EnrichmentJobTarget,
        inputs: Vec<semantic_features::SemanticFeatureInput>,
        input_hashes: BTreeMap<String, String>,
        representation_kind: EmbeddingRepresentationKind,
    ) -> Result<()> {
        let _ = input_hashes;
        enqueue_workplane_embedding_jobs(
            &target,
            inputs.into_iter().map(|input| input.artefact_id).collect(),
            representation_kind,
        )?;
        let _guard = self.lock.lock().await;
        let mut state = self.load_state()?;
        state.last_action = Some("enqueue_embeddings".to_string());
        self.save_state(&mut state)?;
        self.notify.notify_waiters();
        let mailbox_name = match representation_kind {
            EmbeddingRepresentationKind::Code => {
                crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX
            }
            EmbeddingRepresentationKind::Summary => {
                crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX
            }
        };
        publish_workplane_runtime_event(&target, mailbox_name)?;
        Ok(())
    }

    pub async fn enqueue_clone_edges_rebuild(&self, target: EnrichmentJobTarget) -> Result<()> {
        enqueue_workplane_clone_rebuild(&target)?;
        let _guard = self.lock.lock().await;
        let mut state = self.load_state()?;
        state.last_action = Some("enqueue_clone_edges_rebuild".to_string());
        self.save_state(&mut state)?;
        self.notify.notify_waiters();
        publish_workplane_runtime_event(
            &target,
            crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
        )?;
        Ok(())
    }

    pub async fn prune_pending_single_artefact_jobs_after_reconcile(
        &self,
        repo_id: &str,
        relational: &crate::host::devql::RelationalStorage,
    ) -> Result<u64> {
        let repo_id_sql = crate::host::devql::esc_pg(repo_id);
        let existing_artefact_ids = relational
            .query_rows(&format!(
                "SELECT DISTINCT artefact_id FROM artefacts WHERE repo_id = '{repo_id_sql}' \
UNION \
SELECT DISTINCT artefact_id FROM artefacts_current WHERE repo_id = '{repo_id_sql}'"
            ))
            .await?
            .into_iter()
            .filter_map(|row| {
                row.as_object()
                    .and_then(|row| row.get("artefact_id"))
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
            })
            .collect::<HashSet<_>>();

        let _guard = self.lock.lock().await;
        let deleted = self.workplane_store.with_connection(|conn| {
            let mut stmt = conn.prepare(
                "SELECT job_id, payload FROM capability_workplane_jobs WHERE repo_id = ?1 AND status = ?2",
            )?;
            let rows = stmt.query_map(
                params![repo_id, WorkplaneJobStatus::Pending.as_str()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )?;
            let mut job_ids = Vec::new();
            for row in rows {
                let (job_id, payload_raw) = row?;
                let payload = serde_json::from_str::<serde_json::Value>(&payload_raw)
                    .unwrap_or(serde_json::Value::Null);
                if crate::capability_packs::semantic_clones::workplane::payload_is_repo_backfill(
                    &payload,
                ) {
                    continue;
                }
                let Some(artefact_id) =
                    crate::capability_packs::semantic_clones::workplane::payload_artefact_id(
                        &payload,
                    )
                else {
                    continue;
                };
                if !existing_artefact_ids.contains(&artefact_id) {
                    job_ids.push(job_id);
                }
            }
            for job_id in &job_ids {
                conn.execute(
                    "DELETE FROM capability_workplane_jobs WHERE job_id = ?1",
                    params![job_id],
                )?;
            }
            Ok(u64::try_from(job_ids.len()).unwrap_or_default())
        })?;
        Ok(deleted)
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
        let recovered = self
            .workplane_store
            .with_connection(|conn| {
                conn.execute(
                    "UPDATE capability_workplane_jobs
                     SET status = ?1,
                         started_at_unix = NULL,
                         updated_at_unix = ?2,
                         lease_owner = NULL,
                         lease_expires_at_unix = NULL
                     WHERE status = ?3",
                    params![
                        WorkplaneJobStatus::Pending.as_str(),
                        sql_i64(unix_timestamp_now())?,
                        WorkplaneJobStatus::Running.as_str(),
                    ],
                )
                .map_err(anyhow::Error::from)
            })
            .unwrap_or_default();
        if recovered == 0 {
            return;
        }
        let mut state = self.load_state().unwrap_or_else(|_| default_state());
        state.last_action = Some("requeue_running".to_string());
        let _ = self.save_state(&mut state);
        log::warn!("requeued {recovered} stale running enrichment jobs on daemon startup");
    }

    async fn run_loop(self: Arc<Self>, pool: EnrichmentWorkerPool) {
        loop {
            self.ensure_worker_capacity();
            match self.process_next_job(pool).await {
                Ok(true) => continue,
                Ok(false) => {}
                Err(err) => {
                    log::warn!(
                        "daemon enrichment worker error for pool {}: {err:#}",
                        pool.as_str()
                    );
                }
            }
            tokio::select! {
                _ = self.notify.notified() => {},
                _ = sleep(Duration::from_secs(2)) => {},
            }
        }
    }

    async fn process_next_job(&self, pool: EnrichmentWorkerPool) -> Result<bool> {
        let job = {
            let _guard = self.lock.lock().await;
            let mut state = self.load_state()?;
            let Some(job) =
                claim_next_workplane_job(&self.workplane_store, &self.runtime_store, &state, pool)?
            else {
                return Ok(false);
            };
            state.last_action = Some(format!("running:{}", pool.as_str()));
            self.save_state(&mut state)?;
            job
        };
        publish_job_runtime_event(&job);

        let outcome = execution::execute_workplane_job(&job).await;

        {
            let _guard = self.lock.lock().await;
            let mut state = self.load_state()?;
            let disposition =
                persist_workplane_job_completion(&self.workplane_store, &job, &outcome)?;
            state.last_action = Some(match disposition {
                WorkplaneJobCompletionDisposition::Completed => "completed".to_string(),
                WorkplaneJobCompletionDisposition::Failed => "failed".to_string(),
                WorkplaneJobCompletionDisposition::RetryScheduled { .. } => {
                    "retry_scheduled".to_string()
                }
            });
            self.save_state(&mut state)?;
        }
        publish_job_runtime_event(&job);

        for follow_up in outcome.follow_ups {
            self.enqueue_follow_up(follow_up).await?;
        }

        Ok(true)
    }

    async fn enqueue_follow_up(&self, follow_up: FollowUpJob) -> Result<()> {
        match follow_up {
            FollowUpJob::SemanticSummaries {
                target,
                artefact_ids,
            } => {
                self.enqueue_semantic_summary_workplane_jobs(target, artefact_ids)
                    .await
            }
            FollowUpJob::RepoBackfillEmbeddings {
                target,
                artefact_ids,
                representation_kind,
            } => {
                self.enqueue_repo_backfill_embedding_job(target, artefact_ids, representation_kind)
                    .await
            }
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

    fn load_state(&self) -> Result<EnrichmentControlState> {
        Ok(self
            .runtime_store
            .load_enrichment_queue_state()?
            .unwrap_or_else(default_state))
    }

    fn save_state(&self, state: &mut EnrichmentControlState) -> Result<()> {
        state.version = 1;
        state.jobs.clear();
        state.active_branch_by_repo.clear();
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
        let _ = input_hashes;
        enqueue_workplane_embedding_jobs(&target, artefact_ids, representation_kind)?;
        let mut state = self.load_state()?;
        state.last_action = Some("enqueue_embeddings".to_string());
        self.save_state(&mut state)?;
        self.notify.notify_waiters();
        let mailbox_name = match representation_kind {
            EmbeddingRepresentationKind::Code => {
                crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX
            }
            EmbeddingRepresentationKind::Summary => {
                crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX
            }
        };
        publish_workplane_runtime_event(&target, mailbox_name)?;
        Ok(())
    }

    async fn enqueue_repo_backfill_embedding_job(
        &self,
        target: EnrichmentJobTarget,
        artefact_ids: Vec<String>,
        representation_kind: EmbeddingRepresentationKind,
    ) -> Result<()> {
        enqueue_workplane_embedding_repo_backfill_job(&target, artefact_ids, representation_kind)?;
        let mut state = self.load_state()?;
        state.last_action = Some("enqueue_embeddings".to_string());
        self.save_state(&mut state)?;
        self.notify.notify_waiters();
        let mailbox_name = match representation_kind {
            EmbeddingRepresentationKind::Code => {
                crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX
            }
            EmbeddingRepresentationKind::Summary => {
                crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX
            }
        };
        publish_workplane_runtime_event(&target, mailbox_name)?;
        Ok(())
    }

    async fn enqueue_semantic_summary_workplane_jobs(
        &self,
        target: EnrichmentJobTarget,
        artefact_ids: Vec<String>,
    ) -> Result<()> {
        enqueue_workplane_summary_jobs(&target, artefact_ids)?;
        let mut state = self.load_state()?;
        state.last_action = Some("enqueue_semantic".to_string());
        self.save_state(&mut state)?;
        self.notify.notify_waiters();
        publish_workplane_runtime_event(
            &target,
            crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
        )?;
        Ok(())
    }
}

fn publish_workplane_runtime_event(target: &EnrichmentJobTarget, mailbox_name: &str) -> Result<()> {
    let Some(init_session_id) = target.init_session_id.clone() else {
        return Ok(());
    };
    let repo_id = crate::host::devql::resolve_repo_identity(&target.repo_root)?.repo_id;
    crate::daemon::shared_init_runtime_coordinator().publish_runtime_event(
        crate::daemon::RuntimeEventRecord {
            domain: "workplane".to_string(),
            repo_id,
            init_session_id: Some(init_session_id),
            updated_at_unix: unix_timestamp_now(),
            task_id: None,
            run_id: None,
            mailbox_name: Some(mailbox_name.to_string()),
        },
    );
    Ok(())
}

fn publish_job_runtime_event(job: &crate::host::runtime_store::WorkplaneJobRecord) {
    let Some(init_session_id) = job.init_session_id.clone() else {
        return;
    };
    crate::daemon::shared_init_runtime_coordinator().publish_runtime_event(
        crate::daemon::RuntimeEventRecord {
            domain: "workplane".to_string(),
            repo_id: job.repo_id.clone(),
            init_session_id: Some(init_session_id),
            updated_at_unix: unix_timestamp_now(),
            task_id: None,
            run_id: None,
            mailbox_name: Some(job.mailbox_name.clone()),
        },
    );
}

pub fn snapshot() -> Result<EnrichmentQueueStatus> {
    let runtime_store = DaemonSqliteRuntimeStore::open()?;
    let daemon_config = crate::daemon::resolve_daemon_config(None)?;
    let workplane_store = DaemonSqliteRuntimeStore::open_at(
        resolve_repo_runtime_db_path_for_config_root(&daemon_config.config_root),
    )?;
    let state = runtime_store
        .load_enrichment_queue_state()?
        .unwrap_or_else(default_state);
    let projected = project_workplane_status(
        &workplane_store,
        &state,
        effective_worker_budgets(&workplane_store, &daemon_config.config_root)?,
    )?;
    let gate = crate::daemon::embeddings_bootstrap::gate_status_for_enrichment_queue(
        &runtime_store,
        iter_workplane_job_config_roots(&workplane_store)?,
    )?;
    Ok(EnrichmentQueueStatus {
        state: projected,
        persisted: runtime_store.enrichment_state_exists()?,
        embeddings_gate: gate,
        blocked_mailboxes: current_workplane_mailbox_blocked_statuses(
            &workplane_store,
            &runtime_store,
        )?,
        last_failed_embedding: last_failed_embedding_job_from_workplane(&workplane_store)?,
    })
}

pub(crate) fn blocked_mailboxes_for_repo(
    workplane_store: &DaemonSqliteRuntimeStore,
    runtime_store: &DaemonSqliteRuntimeStore,
    repo_id: &str,
) -> Result<Vec<BlockedMailboxStatus>> {
    current_workplane_mailbox_blocked_statuses_for_repo(workplane_store, runtime_store, repo_id)
}

fn retry_failed_jobs_in_store(workplane_store: &DaemonSqliteRuntimeStore) -> Result<u64> {
    let retried = retry_failed_workplane_jobs(workplane_store)?;
    compact_and_prune_workplane_jobs(workplane_store)?;
    Ok(retried)
}

pub fn pause_enrichments(reason: Option<String>) -> Result<EnrichmentControlResult> {
    let runtime_store = DaemonSqliteRuntimeStore::open()?;
    let daemon_config = crate::daemon::resolve_daemon_config(None)?;
    let workplane_store = DaemonSqliteRuntimeStore::open_at(
        resolve_repo_runtime_db_path_for_config_root(&daemon_config.config_root),
    )?;
    let mut state = runtime_store
        .load_enrichment_queue_state()?
        .unwrap_or_else(default_state);
    state.paused_embeddings = true;
    state.paused_semantic = true;
    state.paused_reason = reason.clone();
    state.last_action = Some("paused".to_string());
    runtime_store.save_enrichment_queue_state(&state)?;
    let mut projected = project_workplane_status(
        &workplane_store,
        &state,
        effective_worker_budgets(&workplane_store, &daemon_config.config_root)?,
    )?;
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
    let daemon_config = crate::daemon::resolve_daemon_config(None)?;
    let workplane_store = DaemonSqliteRuntimeStore::open_at(
        resolve_repo_runtime_db_path_for_config_root(&daemon_config.config_root),
    )?;
    Ok(EnrichmentControlResult {
        message: "Enrichment queue resumed.".to_string(),
        state: project_workplane_status(
            &workplane_store,
            &state,
            effective_worker_budgets(&workplane_store, &daemon_config.config_root)?,
        )?,
    })
}

pub fn retry_failed_enrichments() -> Result<EnrichmentControlResult> {
    let runtime_store = DaemonSqliteRuntimeStore::open()?;
    let daemon_config = crate::daemon::resolve_daemon_config(None)?;
    let workplane_store = DaemonSqliteRuntimeStore::open_at(
        resolve_repo_runtime_db_path_for_config_root(&daemon_config.config_root),
    )?;
    let mut state = runtime_store
        .load_enrichment_queue_state()?
        .unwrap_or_else(default_state);
    let retried = retry_failed_jobs_in_store(&workplane_store)?;
    state.retried_failed_jobs += retried;
    state.last_action = Some("retry_failed".to_string());
    runtime_store.save_enrichment_queue_state(&state)?;
    let mut projected = project_workplane_status(
        &workplane_store,
        &state,
        effective_worker_budgets(&workplane_store, &daemon_config.config_root)?,
    )?;
    projected.retried_failed_jobs = state.retried_failed_jobs;
    projected.last_action = Some("retry_failed".to_string());
    Ok(EnrichmentControlResult {
        message: format!("Requeued {retried} failed enrichment jobs."),
        state: projected,
    })
}

fn effective_worker_budgets(
    workplane_store: &DaemonSqliteRuntimeStore,
    fallback_config_root: &std::path::Path,
) -> Result<EnrichmentWorkerBudgets> {
    let mut budgets = configured_enrichment_worker_budgets_for_repo(fallback_config_root);
    for config_root in iter_workplane_job_config_roots(workplane_store)? {
        let next = configured_enrichment_worker_budgets_for_repo(&config_root);
        budgets.summary_refresh = budgets.summary_refresh.max(next.summary_refresh);
        budgets.embeddings = budgets.embeddings.max(next.embeddings);
        budgets.clone_rebuild = budgets.clone_rebuild.max(next.clone_rebuild);
    }
    Ok(budgets)
}

#[cfg(test)]
#[path = "enrichment_tests.rs"]
mod tests;

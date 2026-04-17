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
    SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
};
use crate::config::resolve_repo_runtime_db_path_for_config_root;
use crate::host::runtime_store::{DaemonSqliteRuntimeStore, WorkplaneJobStatus};
use rusqlite::params;

use super::types::{EnrichmentQueueMode, EnrichmentQueueStatus, unix_timestamp_now};

#[path = "enrichment/execution.rs"]
mod execution;
#[path = "enrichment/worker_count.rs"]
mod worker_count;
#[path = "enrichment/workplane.rs"]
mod workplane;

#[cfg(test)]
use self::workplane::load_workplane_jobs_by_status;
use self::workplane::{
    claim_next_workplane_job, compact_and_prune_workplane_jobs,
    current_workplane_mailbox_blocked_statuses, default_state, enqueue_workplane_clone_rebuild,
    enqueue_workplane_embedding_jobs, enqueue_workplane_summary_jobs,
    iter_workplane_job_config_roots, last_failed_embedding_job_from_workplane,
    persist_workplane_job_completion, project_workplane_status, retry_failed_workplane_jobs,
    sql_i64,
};
use worker_count::configured_enrichment_worker_count;

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
}

impl EnrichmentJobTarget {
    pub fn new(config_root: PathBuf, repo_root: PathBuf) -> Self {
        Self {
            config_root,
            repo_root,
        }
    }
}

#[derive(Debug)]
pub struct EnrichmentCoordinator {
    runtime_store: DaemonSqliteRuntimeStore,
    workplane_store: DaemonSqliteRuntimeStore,
    lock: Mutex<()>,
    notify: Notify,
    state_initialised: AtomicBool,
    workers_started: AtomicBool,
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
        let coordinator = Arc::clone(INSTANCE.get_or_init(|| {
            let daemon_config =
                crate::daemon::resolve_daemon_config(None).expect("resolving daemon config");
            Arc::new(Self {
                runtime_store: DaemonSqliteRuntimeStore::open()
                    .expect("opening daemon runtime store for enrichment controls"),
                workplane_store: DaemonSqliteRuntimeStore::open_at(
                    resolve_repo_runtime_db_path_for_config_root(&daemon_config.config_root),
                )
                .expect("opening repo runtime workplane store for enrichment queue"),
                lock: Mutex::new(()),
                notify: Notify::new(),
                state_initialised: AtomicBool::new(false),
                workers_started: AtomicBool::new(false),
            })
        }));
        coordinator.ensure_started();
        coordinator
    }

    pub(crate) fn ensure_started(self: &Arc<Self>) {
        if !self.state_initialised.swap(true, Ordering::AcqRel) {
            self.ensure_state_file();
            self.requeue_running_jobs();
            if let Err(err) = compact_and_prune_workplane_jobs(&self.workplane_store) {
                log::warn!("failed to compact enrichment workplane jobs during startup: {err:#}");
            }
        }
        self.start_workers_if_possible();
    }

    fn start_workers_if_possible(self: &Arc<Self>) {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            log::error!("enrichment worker activation requested without an active tokio runtime");
            return;
        };
        if self.workers_started.swap(true, Ordering::AcqRel) {
            return;
        }

        let worker_count = configured_enrichment_worker_count();
        if worker_count > 1 {
            log::info!("starting {worker_count} enrichment workers");
        }
        for _ in 0..worker_count {
            let coordinator = Arc::clone(self);
            handle.spawn(async move {
                coordinator.run_loop().await;
            });
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
        compact_and_prune_workplane_jobs(&self.workplane_store)?;
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
        compact_and_prune_workplane_jobs(&self.workplane_store)?;
        self.notify.notify_waiters();
        Ok(())
    }

    pub async fn enqueue_clone_edges_rebuild(&self, target: EnrichmentJobTarget) -> Result<()> {
        enqueue_workplane_clone_rebuild(&target)?;
        let _guard = self.lock.lock().await;
        let mut state = self.load_state()?;
        state.last_action = Some("enqueue_clone_edges_rebuild".to_string());
        self.save_state(&mut state)?;
        compact_and_prune_workplane_jobs(&self.workplane_store)?;
        self.notify.notify_waiters();
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
        if deleted > 0 {
            compact_and_prune_workplane_jobs(&self.workplane_store)?;
        }
        Ok(deleted)
    }

    fn ensure_state_file(&self) {
        match self.runtime_store.enrichment_state_exists() {
            Ok(true) => return,
            Ok(false) => {}
            Err(err) => {
                log::warn!(
                    "failed to check persisted enrichment queue state during startup: {err:#}"
                );
            }
        }
        let mut state = default_state();
        if let Err(err) = self.save_state(&mut state) {
            log::warn!("failed to initialise persisted enrichment queue state: {err:#}");
        }
    }

    fn requeue_running_jobs(&self) {
        let recovered = match self.workplane_store.with_connection(|conn| {
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
        }) {
            Ok(recovered) => recovered,
            Err(err) => {
                log::warn!(
                    "failed to recover stale running enrichment jobs during startup: {err:#}"
                );
                return;
            }
        };
        if recovered == 0 {
            return;
        }
        let mut state = match self.load_state() {
            Ok(state) => state,
            Err(err) => {
                log::warn!(
                    "failed to load enrichment queue state during startup recovery: {err:#}"
                );
                default_state()
            }
        };
        state.last_action = Some("requeue_running".to_string());
        if let Err(err) = self.save_state(&mut state) {
            log::warn!("failed to persist enrichment queue recovery state: {err:#}");
        }
        log::warn!("requeued {recovered} stale running enrichment jobs on daemon startup");
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
            compact_and_prune_workplane_jobs(&self.workplane_store)?;
            let Some(job) =
                claim_next_workplane_job(&self.workplane_store, &self.runtime_store, &state)?
            else {
                return Ok(false);
            };
            state.last_action = Some("running".to_string());
            self.save_state(&mut state)?;
            job
        };

        let outcome = execution::execute_workplane_job(&job).await;

        {
            let _guard = self.lock.lock().await;
            let mut state = self.load_state()?;
            persist_workplane_job_completion(&self.workplane_store, &job, &outcome)?;
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
        compact_and_prune_workplane_jobs(&self.workplane_store)?;
        self.notify.notify_waiters();
        Ok(())
    }
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
    let projected = project_workplane_status(&workplane_store, &state)?;
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
    let mut projected = project_workplane_status(&workplane_store, &state)?;
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
        state: project_workplane_status(&workplane_store, &state)?,
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
    let retried = retry_failed_workplane_jobs(&workplane_store)?;
    state.retried_failed_jobs += retried;
    state.last_action = Some("retry_failed".to_string());
    runtime_store.save_enrichment_queue_state(&state)?;
    let mut projected = project_workplane_status(&workplane_store, &state)?;
    projected.retried_failed_jobs = state.retried_failed_jobs;
    projected.last_action = Some("retry_failed".to_string());
    Ok(EnrichmentControlResult {
        message: format!("Requeued {retried} failed enrichment jobs."),
        state: projected,
    })
}

#[cfg(test)]
#[path = "enrichment_tests.rs"]
mod tests;

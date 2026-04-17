use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use tokio::runtime::Handle;
use uuid::Uuid;

use crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind;
use crate::capability_packs::semantic_clones::runtime_config::embedding_slot_for_representation;
use crate::capability_packs::semantic_clones::types::{
    SEMANTIC_CLONES_CAPABILITY_ID, SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
    SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX, SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
};
use crate::capability_packs::semantic_clones::workplane::{
    payload_artefact_id, payload_is_repo_backfill, payload_repo_backfill_artefact_ids,
    payload_work_item_count,
};
use crate::cli::inference::{
    PreparedSummarySetupAction, PreparedSummarySetupPlan, SummarySetupExecutionResult,
    SummarySetupOutcome, SummarySetupPhase, SummarySetupProgress,
    execute_prepared_summary_setup_with_progress,
};
use crate::config::resolve_semantic_clones_config_for_repo;
use crate::graphql::SubscriptionHub;
use crate::host::capability_host::gateways::CapabilityMailboxStatus;
use crate::host::devql::{DevqlConfig, SyncMode};
use crate::host::relational_store::{DefaultRelationalStore, RelationalStore};
use crate::host::runtime_store::{DaemonSqliteRuntimeStore, RepoSqliteRuntimeStore};
use crate::runtime_presentation::{
    RETRY_FAILED_ENRICHMENTS_COMMAND, lane_activity_label, mailbox_label, task_kind_label,
    warning_summary, workplane_pool_label, workplane_warning_message,
};

const CURRENT_CODE_EMBEDDINGS_TABLE: &str = "symbol_embeddings_current";
const CURRENT_SUMMARY_SEMANTICS_TABLE: &str = "symbol_semantics_current";

use super::enrichment::worker_count::configured_enrichment_worker_budgets_for_repo;
use super::types::{
    BlockedMailboxStatus, DevqlTaskRecord, DevqlTaskSpec, DevqlTaskStatus,
    EmbeddingsBootstrapGateStatus, EmbeddingsBootstrapTaskSpec, IngestTaskSpec, InitSessionRecord,
    InitSessionState, InitSessionTerminalStatus, StartInitSessionSelections,
    SummaryBootstrapAction, SummaryBootstrapProgress, SummaryBootstrapRequest,
    SummaryBootstrapResultRecord, SummaryBootstrapRunRecord, SummaryBootstrapState,
    SummaryBootstrapStatus, SyncTaskMode, SyncTaskSpec, unix_timestamp_now,
};

pub(crate) type PersistedInitSessionState = InitSessionState;
pub(crate) type PersistedSummaryBootstrapState = SummaryBootstrapState;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeEventRecord {
    pub domain: String,
    pub repo_id: String,
    pub init_session_id: Option<String>,
    pub updated_at_unix: u64,
    pub task_id: Option<String>,
    pub run_id: Option<String>,
    pub mailbox_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitSessionHandle {
    pub init_session_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct InitRuntimeWorkplaneMailboxSnapshot {
    pub mailbox_name: String,
    pub display_name: String,
    pub pending_jobs: u64,
    pub running_jobs: u64,
    pub failed_jobs: u64,
    pub completed_recent_jobs: u64,
    pub pending_cursor_runs: u64,
    pub running_cursor_runs: u64,
    pub failed_cursor_runs: u64,
    pub completed_recent_cursor_runs: u64,
    pub intent_active: bool,
    pub blocked_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InitRuntimeWorkplaneSnapshot {
    pub pending_jobs: u64,
    pub running_jobs: u64,
    pub failed_jobs: u64,
    pub completed_recent_jobs: u64,
    pub pools: Vec<InitRuntimeWorkplanePoolSnapshot>,
    pub mailboxes: Vec<InitRuntimeWorkplaneMailboxSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InitRuntimeWorkplanePoolSnapshot {
    pub pool_name: String,
    pub display_name: String,
    pub worker_budget: u64,
    pub active_workers: u64,
    pub pending_jobs: u64,
    pub running_jobs: u64,
    pub failed_jobs: u64,
    pub completed_recent_jobs: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct InitRuntimeLaneProgressView {
    pub completed: u64,
    pub total: u64,
    pub remaining: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct InitRuntimeLaneQueueView {
    pub queued: u64,
    pub running: u64,
    pub failed: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct InitRuntimeLaneWarningView {
    pub component_label: String,
    pub message: String,
    pub retry_command: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct InitRuntimeLaneView {
    pub status: String,
    pub waiting_reason: Option<String>,
    pub detail: Option<String>,
    pub activity_label: Option<String>,
    pub task_id: Option<String>,
    pub run_id: Option<String>,
    pub progress: Option<InitRuntimeLaneProgressView>,
    pub queue: InitRuntimeLaneQueueView,
    pub warnings: Vec<InitRuntimeLaneWarningView>,
    pub pending_count: u64,
    pub running_count: u64,
    pub failed_count: u64,
    pub completed_count: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct InitRuntimeSessionView {
    pub init_session_id: String,
    pub status: String,
    pub waiting_reason: Option<String>,
    pub warning_summary: Option<String>,
    pub follow_up_sync_required: bool,
    pub run_sync: bool,
    pub run_ingest: bool,
    pub embeddings_selected: bool,
    pub summaries_selected: bool,
    pub initial_sync_task_id: Option<String>,
    pub ingest_task_id: Option<String>,
    pub follow_up_sync_task_id: Option<String>,
    pub embeddings_bootstrap_task_id: Option<String>,
    pub summary_bootstrap_run_id: Option<String>,
    pub terminal_error: Option<String>,
    pub top_pipeline_lane: InitRuntimeLaneView,
    pub embeddings_lane: InitRuntimeLaneView,
    pub summaries_lane: InitRuntimeLaneView,
}

#[derive(Debug, Clone, Serialize)]
pub struct InitRuntimeSnapshot {
    pub repo_id: String,
    pub task_queue: super::DevqlTaskQueueStatus,
    pub current_state_consumer: super::CapabilityEventQueueStatus,
    pub workplane: InitRuntimeWorkplaneSnapshot,
    pub blocked_mailboxes: Vec<BlockedMailboxStatus>,
    pub embeddings_readiness_gate: Option<EmbeddingsBootstrapGateStatus>,
    pub summaries_bootstrap: Option<SummaryBootstrapRunRecord>,
    pub current_init_session: Option<InitRuntimeSessionView>,
}

#[derive(Debug, Default, Clone, Copy)]
struct StatusCounts {
    pending: u64,
    running: u64,
    failed: u64,
    completed: u64,
}

impl StatusCounts {
    fn queued(self) -> u64 {
        self.pending
    }

    fn has_pending_or_running(self) -> bool {
        self.pending > 0 || self.running > 0
    }
}

#[derive(Debug, Default, Clone)]
struct SessionWorkplaneStats {
    current_state: StatusCounts,
    embedding_jobs: StatusCounts,
    summary_jobs: StatusCounts,
    code_embedding_jobs: SessionMailboxStats,
    summary_embedding_jobs: SessionMailboxStats,
    clone_rebuild_jobs: SessionMailboxStats,
    summary_refresh_jobs: SessionMailboxStats,
    failed_current_state_detail: Option<String>,
    blocked_embedding_reason: Option<String>,
    blocked_summary_reason: Option<String>,
}

#[derive(Debug, Default, Clone)]
struct SessionMailboxStats {
    counts: StatusCounts,
    latest_error: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct RuntimeLaneProgressState {
    embeddings: Option<InitRuntimeLaneProgressView>,
    summaries: Option<InitRuntimeLaneProgressView>,
}

#[derive(Debug, Clone, Default)]
struct SummaryFreshnessState {
    eligible_artefact_ids: BTreeSet<String>,
    fresh_model_backed_artefact_ids: BTreeSet<String>,
}

impl SummaryFreshnessState {
    fn artefact_needs_refresh(&self, artefact_id: &str) -> bool {
        self.eligible_artefact_ids.contains(artefact_id)
            && !self.fresh_model_backed_artefact_ids.contains(artefact_id)
    }

    fn outstanding_work_item_count(&self) -> u64 {
        self.eligible_artefact_ids
            .difference(&self.fresh_model_backed_artefact_ids)
            .count() as u64
    }

    fn outstanding_work_item_count_for_artefacts(&self, artefact_ids: &[String]) -> u64 {
        artefact_ids
            .iter()
            .filter(|artefact_id| self.artefact_needs_refresh(artefact_id.as_str()))
            .count() as u64
    }
}

impl SessionWorkplaneStats {
    fn warning_failed_jobs_total(&self) -> u64 {
        self.code_embedding_jobs.counts.failed
            + self.summary_embedding_jobs.counts.failed
            + self.clone_rebuild_jobs.counts.failed
            + self.summary_refresh_jobs.counts.failed
    }

    fn active_embedding_mailbox(&self) -> Option<&'static str> {
        [
            (
                SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
                &self.code_embedding_jobs.counts,
            ),
            (
                SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
                &self.summary_embedding_jobs.counts,
            ),
            (
                SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
                &self.clone_rebuild_jobs.counts,
            ),
        ]
        .into_iter()
        .find_map(|(mailbox_name, counts)| {
            (counts.running > 0 || counts.pending > 0 || counts.failed > 0).then_some(mailbox_name)
        })
    }

    fn summary_warnings(&self) -> Vec<InitRuntimeLaneWarningView> {
        mailbox_warning(
            SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
            &self.summary_refresh_jobs,
        )
        .into_iter()
        .collect()
    }

    fn embedding_warnings(&self) -> Vec<InitRuntimeLaneWarningView> {
        [
            mailbox_warning(
                SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
                &self.code_embedding_jobs,
            ),
            mailbox_warning(
                SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
                &self.summary_embedding_jobs,
            ),
            mailbox_warning(
                SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
                &self.clone_rebuild_jobs,
            ),
        ]
        .into_iter()
        .flatten()
        .collect()
    }
}

fn mailbox_warning(
    mailbox_name: &str,
    mailbox: &SessionMailboxStats,
) -> Option<InitRuntimeLaneWarningView> {
    (mailbox.counts.failed > 0).then(|| InitRuntimeLaneWarningView {
        component_label: mailbox_label(mailbox_name).to_string(),
        message: workplane_warning_message(mailbox.counts.failed, mailbox.latest_error.as_deref()),
        retry_command: RETRY_FAILED_ENRICHMENTS_COMMAND.to_string(),
    })
}

#[derive(Debug)]
pub struct InitRuntimeCoordinator {
    runtime_store: DaemonSqliteRuntimeStore,
    subscription_hub: Mutex<Option<Arc<SubscriptionHub>>>,
}

impl InitRuntimeCoordinator {
    pub(crate) fn shared() -> Arc<Self> {
        static INSTANCE: OnceLock<Arc<InitRuntimeCoordinator>> = OnceLock::new();
        Arc::clone(INSTANCE.get_or_init(|| {
            Arc::new(Self {
                runtime_store: DaemonSqliteRuntimeStore::open()
                    .expect("opening daemon runtime store for init runtime orchestration"),
                subscription_hub: Mutex::new(None),
            })
        }))
    }

    pub(crate) fn set_subscription_hub(&self, subscription_hub: Arc<SubscriptionHub>) {
        if let Ok(mut slot) = self.subscription_hub.lock() {
            *slot = Some(subscription_hub);
        }
    }

    pub(crate) fn start_session(
        self: &Arc<Self>,
        cfg: &DevqlConfig,
        selections: StartInitSessionSelections,
    ) -> Result<InitSessionHandle> {
        let init_session_id = format!("init-session-{}", Uuid::new_v4());
        let now = unix_timestamp_now();
        let mut session = InitSessionRecord {
            init_session_id: init_session_id.clone(),
            repo_id: cfg.repo.repo_id.clone(),
            repo_root: cfg.repo_root.clone(),
            daemon_config_root: cfg.daemon_config_root.clone(),
            selections: selections.clone(),
            initial_sync_task_id: None,
            ingest_task_id: None,
            embeddings_bootstrap_task_id: None,
            summary_bootstrap_run_id: None,
            follow_up_sync_required: false,
            follow_up_sync_task_id: None,
            submitted_at_unix: now,
            updated_at_unix: now,
            terminal_status: None,
            terminal_error: None,
        };

        if selections.run_sync {
            let queued = super::shared_devql_task_coordinator().enqueue_with_init_session(
                cfg,
                super::DevqlTaskSource::Init,
                DevqlTaskSpec::Sync(SyncTaskSpec {
                    mode: SyncTaskMode::Auto,
                    post_commit_snapshot: None,
                }),
                Some(init_session_id.clone()),
            )?;
            session.initial_sync_task_id = Some(queued.task.task_id);
        } else if selections.run_ingest {
            let queued = super::shared_devql_task_coordinator().enqueue_with_init_session(
                cfg,
                super::DevqlTaskSource::Init,
                DevqlTaskSpec::Ingest(IngestTaskSpec {
                    backfill: selections.ingest_backfill,
                }),
                Some(init_session_id.clone()),
            )?;
            session.ingest_task_id = Some(queued.task.task_id);
        }

        if let Some(request) = selections.embeddings_bootstrap.clone() {
            let queued = super::shared_devql_task_coordinator().enqueue_with_init_session(
                cfg,
                super::DevqlTaskSource::Init,
                DevqlTaskSpec::EmbeddingsBootstrap(EmbeddingsBootstrapTaskSpec {
                    config_path: request.config_path,
                    profile_name: request.profile_name,
                }),
                Some(init_session_id.clone()),
            )?;
            session.embeddings_bootstrap_task_id = Some(queued.task.task_id);
        }

        if let Some(request) = selections.summaries_bootstrap.clone() {
            let run_id = format!("summary-bootstrap-run-{}", Uuid::new_v4());
            let run = SummaryBootstrapRunRecord {
                run_id: run_id.clone(),
                repo_id: cfg.repo.repo_id.clone(),
                repo_root: cfg.repo_root.clone(),
                init_session_id: init_session_id.clone(),
                request: request.clone(),
                status: SummaryBootstrapStatus::Queued,
                progress: SummaryBootstrapProgress::default(),
                result: None,
                error: None,
                submitted_at_unix: now,
                started_at_unix: None,
                updated_at_unix: now,
                completed_at_unix: None,
            };
            session.summary_bootstrap_run_id = Some(run_id.clone());
            self.runtime_store.mutate_summary_bootstrap_state(|state| {
                state.runs.push(run.clone());
                state.last_action = Some("queued".to_string());
                state.updated_at_unix = now;
                Ok(())
            })?;
            self.spawn_summary_bootstrap_worker(cfg.repo_root.clone(), request, run_id.clone());
            self.publish_event(RuntimeEventRecord {
                domain: "summary_bootstrap".to_string(),
                repo_id: cfg.repo.repo_id.clone(),
                init_session_id: Some(init_session_id.clone()),
                updated_at_unix: now,
                task_id: None,
                run_id: Some(run_id),
                mailbox_name: None,
            });
        }

        self.runtime_store.mutate_init_session_state(|state| {
            state.sessions.push(session.clone());
            state.last_action = Some("started".to_string());
            state.updated_at_unix = now;
            Ok(())
        })?;

        self.publish_event(RuntimeEventRecord {
            domain: "init_session".to_string(),
            repo_id: cfg.repo.repo_id.clone(),
            init_session_id: Some(init_session_id.clone()),
            updated_at_unix: now,
            task_id: session.initial_sync_task_id.clone(),
            run_id: session.summary_bootstrap_run_id.clone(),
            mailbox_name: None,
        });

        Ok(InitSessionHandle { init_session_id })
    }

    pub(crate) fn handle_task_update(self: &Arc<Self>, task: DevqlTaskRecord) -> Result<()> {
        let Some(init_session_id) = task.init_session_id.clone() else {
            return Ok(());
        };

        let mut enqueue_ingest = None::<(PathBuf, PathBuf, String, Option<usize>)>;
        self.runtime_store.mutate_init_session_state(|state| {
            let Some(session) = state
                .sessions
                .iter_mut()
                .find(|session| session.init_session_id == init_session_id)
            else {
                return Ok(());
            };
            session.updated_at_unix = unix_timestamp_now();
            if task.task_id == session.initial_sync_task_id.clone().unwrap_or_default()
                && task.status == DevqlTaskStatus::Completed
            {
                if session.selections.run_ingest && session.ingest_task_id.is_none() {
                    enqueue_ingest = Some((
                        session.daemon_config_root.clone(),
                        session.repo_root.clone(),
                        session.init_session_id.clone(),
                        session.selections.ingest_backfill,
                    ));
                }
                if session.follow_up_sync_task_id.is_none()
                    && !session.follow_up_sync_required
                    && session_requires_semantic_follow_up(session)
                    && !self.semantic_bootstraps_ready(session)?
                {
                    session.follow_up_sync_required = true;
                }
            }
            state.last_action = Some("task_updated".to_string());
            state.updated_at_unix = unix_timestamp_now();
            Ok(())
        })?;

        if let Some((daemon_config_root, repo_root, init_session_id, backfill)) = enqueue_ingest {
            let repo = crate::host::devql::resolve_repo_identity(&repo_root)
                .context("resolving repo identity for staged init ingest")?;
            let cfg = DevqlConfig::from_roots(daemon_config_root, repo_root, repo)?;
            let queued = super::shared_devql_task_coordinator().enqueue_with_init_session(
                &cfg,
                super::DevqlTaskSource::Init,
                DevqlTaskSpec::Ingest(IngestTaskSpec { backfill }),
                Some(init_session_id.clone()),
            )?;
            self.runtime_store.mutate_init_session_state(|state| {
                let Some(session) = state
                    .sessions
                    .iter_mut()
                    .find(|session| session.init_session_id == init_session_id)
                else {
                    return Ok(());
                };
                session.ingest_task_id = Some(queued.task.task_id.clone());
                session.updated_at_unix = unix_timestamp_now();
                state.last_action = Some("staged_ingest_enqueued".to_string());
                state.updated_at_unix = unix_timestamp_now();
                Ok(())
            })?;
        }

        self.maybe_enqueue_follow_up_sync(init_session_id.as_str())?;
        self.publish_event(RuntimeEventRecord {
            domain: "task_queue".to_string(),
            repo_id: task.repo_id.clone(),
            init_session_id: Some(init_session_id),
            updated_at_unix: task.updated_at_unix,
            task_id: Some(task.task_id),
            run_id: None,
            mailbox_name: None,
        });
        Ok(())
    }

    pub(crate) fn snapshot_for_repo(&self, cfg: &DevqlConfig) -> Result<InitRuntimeSnapshot> {
        let repo_store =
            RepoSqliteRuntimeStore::open_for_roots(&cfg.daemon_config_root, &cfg.repo_root)?;
        let repo_id = cfg.repo.repo_id.clone();
        let task_queue = super::shared_devql_task_coordinator().snapshot(Some(&repo_id))?;
        let current_state_consumer =
            super::shared_capability_event_coordinator().snapshot(Some(&repo_id))?;
        let mailboxes = repo_store.load_capability_workplane_mailbox_status(
            SEMANTIC_CLONES_CAPABILITY_ID,
            [
                SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
                SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
                SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
                SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
            ],
        )?;
        let blocked_mailboxes =
            repo_blocked_mailboxes(repo_store.db_path().to_path_buf(), repo_store.repo_id())?;
        let workplane =
            workplane_snapshot_from_mailboxes(&cfg.repo_root, &mailboxes, &blocked_mailboxes);
        let embeddings_readiness_gate =
            super::embeddings_bootstrap::gate_status_for_enrichment_queue(
                &self.runtime_store,
                vec![cfg.daemon_config_root.clone()],
            )?;
        let summaries_bootstrap = self.current_summary_bootstrap_run(&repo_id)?;
        let current_session = self.current_session_view(cfg, &repo_store, &blocked_mailboxes)?;

        Ok(InitRuntimeSnapshot {
            repo_id,
            task_queue,
            current_state_consumer,
            workplane,
            blocked_mailboxes,
            embeddings_readiness_gate,
            summaries_bootstrap,
            current_init_session: current_session,
        })
    }

    fn current_summary_bootstrap_run(
        &self,
        repo_id: &str,
    ) -> Result<Option<SummaryBootstrapRunRecord>> {
        Ok(self
            .runtime_store
            .load_summary_bootstrap_state()?
            .unwrap_or_default()
            .runs
            .into_iter()
            .filter(|run| run.repo_id == repo_id)
            .max_by_key(|run| (run.updated_at_unix, run.submitted_at_unix)))
    }

    fn current_session_view(
        &self,
        cfg: &DevqlConfig,
        repo_store: &RepoSqliteRuntimeStore,
        blocked_mailboxes: &[BlockedMailboxStatus],
    ) -> Result<Option<InitRuntimeSessionView>> {
        let state = self
            .runtime_store
            .load_init_session_state()?
            .unwrap_or_default();
        let Some(session) = state
            .sessions
            .into_iter()
            .filter(|session| session.repo_id == cfg.repo.repo_id)
            .max_by_key(|session| (session.updated_at_unix, session.submitted_at_unix))
        else {
            return Ok(None);
        };

        let stats = load_session_workplane_stats(
            &cfg.repo_root,
            repo_store,
            &cfg.repo.repo_id,
            &session.init_session_id,
        )?;
        let summary_run =
            load_summary_run_for_session(&self.runtime_store, &session.init_session_id)?;
        let initial_sync = load_task_by_id(session.initial_sync_task_id.as_deref())?;
        let ingest_task = load_task_by_id(session.ingest_task_id.as_deref())?;
        let follow_up_sync = load_task_by_id(session.follow_up_sync_task_id.as_deref())?;
        let embeddings_task = load_task_by_id(session.embeddings_bootstrap_task_id.as_deref())?;
        let lane_progress =
            match load_runtime_lane_progress(&cfg.repo_root, &cfg.repo.repo_id, &session, &stats) {
                Ok(progress) => progress,
                Err(err) => {
                    log::debug!(
                        "failed to load runtime lane progress for repo `{}`: {err:#}",
                        cfg.repo.repo_id
                    );
                    RuntimeLaneProgressState::default()
                }
            };

        let top_pipeline_lane = derive_top_pipeline_lane(
            &session,
            initial_sync.as_ref(),
            ingest_task.as_ref(),
            follow_up_sync.as_ref(),
            stats.current_state,
        );
        let embeddings_lane = derive_embeddings_lane(
            &session,
            initial_sync.as_ref(),
            follow_up_sync.as_ref(),
            embeddings_task.as_ref(),
            &stats,
            lane_progress.embeddings.clone(),
        );
        let summaries_lane = derive_summaries_lane(
            &session,
            initial_sync.as_ref(),
            follow_up_sync.as_ref(),
            summary_run.as_ref(),
            &stats,
            lane_progress.summaries.clone(),
        );

        let fatal_failure_detail = session_fatal_failure_detail(
            initial_sync.as_ref(),
            ingest_task.as_ref(),
            follow_up_sync.as_ref(),
            embeddings_task.as_ref(),
            summary_run.as_ref(),
            &stats,
        );
        let has_fatal_failure = fatal_failure_detail.is_some();
        let has_remaining_work = session_has_remaining_work(
            initial_sync.as_ref(),
            ingest_task.as_ref(),
            follow_up_sync.as_ref(),
            embeddings_task.as_ref(),
            summary_run.as_ref(),
            &stats,
        );
        let warning_failures = stats.warning_failed_jobs_total();
        let has_warnings = warning_failures > 0;
        let semantic_bootstraps_terminal =
            semantic_bootstraps_terminal(&session, embeddings_task.as_ref(), summary_run.as_ref());
        let bootstrap_waiting = semantic_bootstrap_still_outstanding_after_initial_sync(
            &session,
            initial_sync.as_ref(),
            embeddings_task.as_ref(),
            summary_run.as_ref(),
        );
        let follow_up_pending = session.follow_up_sync_required
            && semantic_follow_up_pending(
                &session,
                initial_sync.as_ref(),
                follow_up_sync.as_ref(),
                embeddings_task.as_ref(),
                summary_run.as_ref(),
            );
        let follow_up_satisfied = !follow_up_pending;
        let selected_top_level_terminal =
            selected_top_level_terminal(&session, initial_sync.as_ref(), ingest_task.as_ref());
        let blocked_embedding = stats.blocked_embedding_reason.clone();
        let blocked_summary = stats.blocked_summary_reason.clone();
        let waiting_reason = if has_fatal_failure {
            Some("failed".to_string())
        } else if !selected_top_level_terminal {
            Some("waiting_for_top_level_work".to_string())
        } else if stats.current_state.pending > 0 || stats.current_state.running > 0 {
            Some("waiting_for_current_state_consumer".to_string())
        } else if blocked_embedding.is_some() || blocked_summary.is_some() {
            Some("waiting_on_blocked_mailbox".to_string())
        } else if stats.embedding_jobs.pending > 0
            || stats.embedding_jobs.running > 0
            || stats.summary_jobs.pending > 0
            || stats.summary_jobs.running > 0
        {
            Some("waiting_for_workplane".to_string())
        } else if bootstrap_waiting {
            semantic_bootstrap_waiting_reason(
                &session,
                embeddings_task.as_ref(),
                summary_run.as_ref(),
            )
            .map(str::to_string)
        } else if follow_up_pending {
            Some("waiting_for_follow_up_sync".to_string())
        } else if !semantic_bootstraps_terminal {
            Some("waiting_for_bootstrap".to_string())
        } else if blocked_mailboxes.iter().any(|blocked| {
            blocked.mailbox_name == SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX
                && (stats.embedding_jobs.pending > 0 || stats.embedding_jobs.running > 0)
        }) {
            Some("waiting_on_blocked_mailbox".to_string())
        } else {
            None
        };

        let completed = !has_fatal_failure
            && selected_top_level_terminal
            && semantic_bootstraps_terminal
            && follow_up_satisfied
            && stats.current_state.pending == 0
            && stats.current_state.running == 0
            && stats.embedding_jobs.pending == 0
            && stats.embedding_jobs.running == 0
            && stats.summary_jobs.pending == 0
            && stats.summary_jobs.running == 0
            && blocked_embedding.is_none()
            && blocked_summary.is_none();
        let terminal_failed = has_fatal_failure && !has_remaining_work;
        let status = derive_session_status(
            has_fatal_failure,
            has_remaining_work,
            completed,
            waiting_reason.as_deref(),
            has_warnings,
        )
        .to_string();

        if matches!(
            status.as_str(),
            "completed" | "completed_with_warnings" | "failed"
        ) {
            self.runtime_store.mutate_init_session_state(|state| {
                if let Some(record) = state
                    .sessions
                    .iter_mut()
                    .find(|record| record.init_session_id == session.init_session_id)
                {
                    record.terminal_status = Some(if status == "completed" {
                        InitSessionTerminalStatus::Completed
                    } else if status == "completed_with_warnings" {
                        InitSessionTerminalStatus::CompletedWithWarnings
                    } else {
                        InitSessionTerminalStatus::Failed
                    });
                    if terminal_failed {
                        record.terminal_error = fatal_failure_detail
                            .clone()
                            .or(record.terminal_error.clone())
                            .or_else(|| Some("init session failed".to_string()));
                    }
                    record.updated_at_unix = unix_timestamp_now();
                    state.last_action = Some(status.clone());
                    state.updated_at_unix = unix_timestamp_now();
                }
                Ok(())
            })?;
        }

        Ok(Some(InitRuntimeSessionView {
            init_session_id: session.init_session_id,
            status: status.clone(),
            waiting_reason,
            warning_summary: has_warnings.then(|| warning_summary(warning_failures)),
            follow_up_sync_required: follow_up_pending,
            run_sync: session.selections.run_sync,
            run_ingest: session.selections.run_ingest,
            embeddings_selected: session.selections.embeddings_bootstrap.is_some(),
            summaries_selected: session.selections.summaries_bootstrap.is_some(),
            initial_sync_task_id: session.initial_sync_task_id,
            ingest_task_id: session.ingest_task_id,
            follow_up_sync_task_id: session.follow_up_sync_task_id,
            embeddings_bootstrap_task_id: session.embeddings_bootstrap_task_id,
            summary_bootstrap_run_id: session.summary_bootstrap_run_id,
            terminal_error: if status == "failed" {
                fatal_failure_detail.or(session.terminal_error)
            } else {
                None
            },
            top_pipeline_lane,
            embeddings_lane,
            summaries_lane,
        }))
    }

    fn maybe_enqueue_follow_up_sync(&self, init_session_id: &str) -> Result<()> {
        let state = self
            .runtime_store
            .load_init_session_state()?
            .unwrap_or_default();
        let Some(session) = state
            .sessions
            .into_iter()
            .find(|session| session.init_session_id == init_session_id)
        else {
            return Ok(());
        };
        if !session.follow_up_sync_required {
            return Ok(());
        }
        let initial_sync = load_task_by_id(session.initial_sync_task_id.as_deref())?;
        let ingest_task = load_task_by_id(session.ingest_task_id.as_deref())?;
        let follow_up_sync = load_task_by_id(session.follow_up_sync_task_id.as_deref())?;
        let embeddings_task = load_task_by_id(session.embeddings_bootstrap_task_id.as_deref())?;
        let summary_run =
            load_summary_run_for_session(&self.runtime_store, &session.init_session_id)?;
        if !selected_top_level_terminal(&session, initial_sync.as_ref(), ingest_task.as_ref()) {
            return Ok(());
        }
        if task_failed(initial_sync.as_ref())
            || task_failed(ingest_task.as_ref())
            || task_failed(follow_up_sync.as_ref())
            || task_failed(embeddings_task.as_ref())
            || summary_run.as_ref().is_some_and(summary_run_failed)
        {
            return Ok(());
        }
        if running_task(follow_up_sync.as_ref()).is_some() {
            return Ok(());
        }
        if !semantic_follow_up_ready_for_sync(
            &session,
            initial_sync.as_ref(),
            follow_up_sync.as_ref(),
            embeddings_task.as_ref(),
            summary_run.as_ref(),
        ) {
            return Ok(());
        }
        let repo = crate::host::devql::resolve_repo_identity(&session.repo_root)
            .context("resolving repo identity for follow-up init sync")?;
        let cfg = DevqlConfig::from_roots(
            session.daemon_config_root.clone(),
            session.repo_root.clone(),
            repo,
        )?;
        let queued = super::shared_devql_task_coordinator().enqueue_with_init_session(
            &cfg,
            super::DevqlTaskSource::Init,
            DevqlTaskSpec::Sync(SyncTaskSpec {
                mode: match SyncMode::Auto {
                    SyncMode::Auto => SyncTaskMode::Auto,
                    SyncMode::Full => SyncTaskMode::Full,
                    SyncMode::Paths(paths) => SyncTaskMode::Paths { paths },
                    SyncMode::Repair => SyncTaskMode::Repair,
                    SyncMode::Validate => SyncTaskMode::Validate,
                },
                post_commit_snapshot: None,
            }),
            Some(session.init_session_id.clone()),
        )?;
        self.runtime_store.mutate_init_session_state(|state| {
            let Some(record) = state
                .sessions
                .iter_mut()
                .find(|record| record.init_session_id == session.init_session_id)
            else {
                return Ok(());
            };
            record.follow_up_sync_task_id = Some(queued.task.task_id.clone());
            record.updated_at_unix = unix_timestamp_now();
            state.last_action = Some("follow_up_sync_enqueued".to_string());
            state.updated_at_unix = unix_timestamp_now();
            Ok(())
        })?;
        Ok(())
    }

    fn semantic_bootstraps_ready(&self, session: &InitSessionRecord) -> Result<bool> {
        let embeddings_task = load_task_by_id(session.embeddings_bootstrap_task_id.as_deref())?;
        let summary_run =
            load_summary_run_for_session(&self.runtime_store, &session.init_session_id)?;
        Ok(semantic_bootstraps_ready(
            session,
            embeddings_task.as_ref(),
            summary_run.as_ref(),
        ))
    }

    fn publish_event(&self, event: RuntimeEventRecord) {
        let Some(hub) = self
            .subscription_hub
            .lock()
            .ok()
            .and_then(|slot| slot.clone())
        else {
            return;
        };
        hub.publish_runtime_event(event);
    }

    pub(crate) fn publish_runtime_event(&self, event: RuntimeEventRecord) {
        self.publish_event(event);
    }

    fn spawn_summary_bootstrap_worker(
        self: &Arc<Self>,
        repo_root: PathBuf,
        request: SummaryBootstrapRequest,
        run_id: String,
    ) {
        let coordinator = Arc::clone(self);
        let handle = Handle::try_current().ok();
        let Some(handle) = handle else {
            log::warn!("summary bootstrap worker requested without an active tokio runtime");
            return;
        };
        handle.spawn(async move {
            let blocking_coordinator = Arc::clone(&coordinator);
            let blocking_run_id = run_id.clone();
            let worker = tokio::task::spawn_blocking(move || {
                let plan = prepared_summary_setup_plan_from_request(&request);
                let now = unix_timestamp_now();
                blocking_coordinator
                    .runtime_store
                    .mutate_summary_bootstrap_state(|state| {
                        if let Some(run) = state
                            .runs
                            .iter_mut()
                            .find(|run| run.run_id == blocking_run_id)
                        {
                            run.status = SummaryBootstrapStatus::Running;
                            run.started_at_unix = Some(now);
                            run.updated_at_unix = now;
                            state.last_action = Some("running".to_string());
                            state.updated_at_unix = now;
                        }
                        Ok(())
                    })?;
                execute_prepared_summary_setup_with_progress(&repo_root, plan, |progress| {
                    blocking_coordinator.update_summary_progress(&blocking_run_id, progress)
                })
            });

            match worker.await {
                Ok(Ok(result)) => {
                    if let Err(err) = coordinator.finish_summary_bootstrap(&run_id, Ok(result)) {
                        log::warn!("failed to persist summary bootstrap completion: {err:#}");
                    }
                }
                Ok(Err(err)) => {
                    if let Err(persist_err) =
                        coordinator.finish_summary_bootstrap(&run_id, Err(format!("{err:#}")))
                    {
                        log::warn!(
                            "failed to persist summary bootstrap failure after worker error: {persist_err:#}"
                        );
                    }
                }
                Err(err) => {
                    if let Err(persist_err) = coordinator.finish_summary_bootstrap(
                        &run_id,
                        Err(format!("summary bootstrap worker join failed: {err:#}")),
                    ) {
                        log::warn!(
                            "failed to persist summary bootstrap failure after join error: {persist_err:#}"
                        );
                    }
                }
            }
        });
    }

    fn update_summary_progress(&self, run_id: &str, progress: SummarySetupProgress) -> Result<()> {
        let progress = summary_progress_from_cli(progress);
        let mut event = None::<RuntimeEventRecord>;
        self.runtime_store.mutate_summary_bootstrap_state(|state| {
            if let Some(run) = state.runs.iter_mut().find(|run| run.run_id == run_id) {
                run.progress = progress.clone();
                run.updated_at_unix = unix_timestamp_now();
                state.last_action = Some(progress.phase.to_string());
                state.updated_at_unix = run.updated_at_unix;
                event = Some(RuntimeEventRecord {
                    domain: "summary_bootstrap".to_string(),
                    repo_id: run.repo_id.clone(),
                    init_session_id: Some(run.init_session_id.clone()),
                    updated_at_unix: run.updated_at_unix,
                    task_id: None,
                    run_id: Some(run.run_id.clone()),
                    mailbox_name: None,
                });
            }
            Ok(())
        })?;
        if let Some(event) = event {
            self.publish_event(event);
        }
        Ok(())
    }

    fn finish_summary_bootstrap(
        &self,
        run_id: &str,
        result: std::result::Result<SummarySetupExecutionResult, String>,
    ) -> Result<()> {
        let mut session_id = None::<String>;
        let mut repo_id = None::<String>;
        self.runtime_store.mutate_summary_bootstrap_state(|state| {
            if let Some(run) = state.runs.iter_mut().find(|run| run.run_id == run_id) {
                let now = unix_timestamp_now();
                run.updated_at_unix = now;
                run.completed_at_unix = Some(now);
                match &result {
                    Ok(result) => {
                        run.status = SummaryBootstrapStatus::Completed;
                        run.progress.phase = super::SummaryBootstrapPhase::Complete;
                        run.progress.message = Some(result.message.clone());
                        run.result = Some(summary_result_from_cli(result));
                        run.error = None;
                        state.last_action = Some("completed".to_string());
                    }
                    Err(error) => {
                        run.status = SummaryBootstrapStatus::Failed;
                        run.error = Some(error.clone());
                        run.progress.message = Some(error.clone());
                        state.last_action = Some("failed".to_string());
                    }
                }
                state.updated_at_unix = now;
                session_id = Some(run.init_session_id.clone());
                repo_id = Some(run.repo_id.clone());
            }
            Ok(())
        })?;
        if let Some(session_id) = session_id.as_deref() {
            self.maybe_enqueue_follow_up_sync(session_id)?;
        }
        if let (Some(repo_id), Some(session_id)) = (repo_id, session_id) {
            self.publish_event(RuntimeEventRecord {
                domain: "summary_bootstrap".to_string(),
                repo_id,
                init_session_id: Some(session_id),
                updated_at_unix: unix_timestamp_now(),
                task_id: None,
                run_id: Some(run_id.to_string()),
                mailbox_name: None,
            });
        }
        Ok(())
    }
}

fn prepared_summary_setup_plan_from_request(
    request: &SummaryBootstrapRequest,
) -> PreparedSummarySetupPlan {
    PreparedSummarySetupPlan::new(match request.action {
        SummaryBootstrapAction::InstallRuntimeOnly => {
            PreparedSummarySetupAction::InstallRuntimeOnly {
                message: request.message.clone().unwrap_or_default(),
            }
        }
        SummaryBootstrapAction::InstallRuntimeOnlyPendingProbe => {
            PreparedSummarySetupAction::InstallRuntimeOnlyPendingProbe {
                message: request.message.clone().unwrap_or_default(),
            }
        }
        SummaryBootstrapAction::ConfigureLocal => PreparedSummarySetupAction::ConfigureLocal {
            model_name: request.model_name.clone().unwrap_or_default(),
        },
        SummaryBootstrapAction::ConfigureCloud => PreparedSummarySetupAction::ConfigureCloud {
            gateway_url_override: request.gateway_url_override.clone(),
        },
    })
}

fn summary_progress_from_cli(progress: SummarySetupProgress) -> SummaryBootstrapProgress {
    SummaryBootstrapProgress {
        phase: match progress.phase {
            SummarySetupPhase::Queued => super::SummaryBootstrapPhase::Queued,
            SummarySetupPhase::ResolvingRelease => super::SummaryBootstrapPhase::ResolvingRelease,
            SummarySetupPhase::DownloadingRuntime => {
                super::SummaryBootstrapPhase::DownloadingRuntime
            }
            SummarySetupPhase::ExtractingRuntime => super::SummaryBootstrapPhase::ExtractingRuntime,
            SummarySetupPhase::RewritingRuntime => super::SummaryBootstrapPhase::RewritingRuntime,
            SummarySetupPhase::WritingProfile => super::SummaryBootstrapPhase::WritingProfile,
        },
        asset_name: progress.asset_name,
        bytes_downloaded: progress.bytes_downloaded,
        bytes_total: progress.bytes_total,
        version: progress.version,
        message: progress.message,
    }
}

fn summary_result_from_cli(result: &SummarySetupExecutionResult) -> SummaryBootstrapResultRecord {
    SummaryBootstrapResultRecord {
        outcome_kind: match &result.outcome {
            SummarySetupOutcome::InstalledRuntimeOnly => "installed_runtime_only".to_string(),
            SummarySetupOutcome::Configured { .. } => "configured".to_string(),
        },
        model_name: match &result.outcome {
            SummarySetupOutcome::InstalledRuntimeOnly => None,
            SummarySetupOutcome::Configured { model_name } => Some(model_name.clone()),
        },
        message: result.message.clone(),
    }
}

fn load_task_by_id(task_id: Option<&str>) -> Result<Option<DevqlTaskRecord>> {
    let Some(task_id) = task_id else {
        return Ok(None);
    };
    super::shared_devql_task_coordinator().task(task_id)
}

fn load_summary_run_for_session(
    runtime_store: &DaemonSqliteRuntimeStore,
    init_session_id: &str,
) -> Result<Option<SummaryBootstrapRunRecord>> {
    Ok(runtime_store
        .load_summary_bootstrap_state()?
        .unwrap_or_default()
        .runs
        .into_iter()
        .filter(|run| run.init_session_id == init_session_id)
        .max_by_key(|run| (run.updated_at_unix, run.submitted_at_unix)))
}

fn session_requires_semantic_follow_up(session: &InitSessionRecord) -> bool {
    session.selections.embeddings_bootstrap.is_some()
        || session.selections.summaries_bootstrap.is_some()
}

fn task_failed(task: Option<&DevqlTaskRecord>) -> bool {
    task.is_some_and(|task| {
        matches!(
            task.status,
            DevqlTaskStatus::Failed | DevqlTaskStatus::Cancelled
        )
    })
}

fn summary_run_failed(run: &SummaryBootstrapRunRecord) -> bool {
    run.status == SummaryBootstrapStatus::Failed
}

fn semantic_bootstraps_terminal(
    session: &InitSessionRecord,
    embeddings_task: Option<&DevqlTaskRecord>,
    summary_run: Option<&SummaryBootstrapRunRecord>,
) -> bool {
    let embeddings_terminal = if session.selections.embeddings_bootstrap.is_some() {
        embeddings_task.is_some_and(|task| {
            matches!(
                task.status,
                DevqlTaskStatus::Completed | DevqlTaskStatus::Failed | DevqlTaskStatus::Cancelled
            )
        })
    } else {
        true
    };
    let summaries_terminal = if session.selections.summaries_bootstrap.is_some() {
        summary_run.is_some_and(|run| {
            matches!(
                run.status,
                SummaryBootstrapStatus::Completed | SummaryBootstrapStatus::Failed
            )
        })
    } else {
        true
    };
    embeddings_terminal && summaries_terminal
}

fn semantic_bootstraps_ready(
    session: &InitSessionRecord,
    embeddings_task: Option<&DevqlTaskRecord>,
    summary_run: Option<&SummaryBootstrapRunRecord>,
) -> bool {
    let embeddings_ready = if session.selections.embeddings_bootstrap.is_some() {
        embeddings_task.is_some_and(|task| task.status == DevqlTaskStatus::Completed)
    } else {
        true
    };
    let summaries_ready = if session.selections.summaries_bootstrap.is_some() {
        summary_run.is_some_and(|run| run.status == SummaryBootstrapStatus::Completed)
    } else {
        true
    };
    embeddings_ready && summaries_ready
}

fn semantic_bootstrap_waiting_reason(
    session: &InitSessionRecord,
    embeddings_task: Option<&DevqlTaskRecord>,
    summary_run: Option<&SummaryBootstrapRunRecord>,
) -> Option<&'static str> {
    let embeddings_waiting = session.selections.embeddings_bootstrap.is_some()
        && !embeddings_task.is_some_and(|task| task.status == DevqlTaskStatus::Completed);
    let summaries_waiting = session.selections.summaries_bootstrap.is_some()
        && !summary_run.is_some_and(|run| run.status == SummaryBootstrapStatus::Completed);

    match (embeddings_waiting, summaries_waiting) {
        (true, false) => Some("waiting_for_embeddings_bootstrap"),
        (false, true) => Some("waiting_for_summary_bootstrap"),
        (true, true) => Some("waiting_for_semantic_bootstrap"),
        (false, false) => None,
    }
}

fn completed_task_at(task: Option<&DevqlTaskRecord>) -> Option<u64> {
    task.filter(|task| task.status == DevqlTaskStatus::Completed)
        .and_then(|task| task.completed_at_unix.or(Some(task.updated_at_unix)))
}

fn completed_summary_run_at(run: Option<&SummaryBootstrapRunRecord>) -> Option<u64> {
    run.filter(|run| run.status == SummaryBootstrapStatus::Completed)
        .and_then(|run| run.completed_at_unix.or(Some(run.updated_at_unix)))
}

fn latest_completed_sync_at(
    initial_sync: Option<&DevqlTaskRecord>,
    follow_up_sync: Option<&DevqlTaskRecord>,
) -> Option<u64> {
    completed_task_at(initial_sync).max(completed_task_at(follow_up_sync))
}

fn embeddings_bootstrap_outstanding_after_initial_sync(
    session: &InitSessionRecord,
    initial_sync: Option<&DevqlTaskRecord>,
    embeddings_task: Option<&DevqlTaskRecord>,
) -> bool {
    session.selections.embeddings_bootstrap.is_some()
        && completed_task_at(initial_sync).is_some()
        && completed_task_at(embeddings_task).is_none()
}

fn summary_bootstrap_outstanding_after_initial_sync(
    session: &InitSessionRecord,
    initial_sync: Option<&DevqlTaskRecord>,
    summary_run: Option<&SummaryBootstrapRunRecord>,
) -> bool {
    session.selections.summaries_bootstrap.is_some()
        && completed_task_at(initial_sync).is_some()
        && completed_summary_run_at(summary_run).is_none()
}

fn embeddings_follow_up_pending(
    session: &InitSessionRecord,
    initial_sync: Option<&DevqlTaskRecord>,
    follow_up_sync: Option<&DevqlTaskRecord>,
    embeddings_task: Option<&DevqlTaskRecord>,
) -> bool {
    if session.selections.embeddings_bootstrap.is_none() {
        return false;
    }
    let Some(bootstrap_completed_at) = completed_task_at(embeddings_task) else {
        return false;
    };
    let Some(sync_completed_at) = latest_completed_sync_at(initial_sync, follow_up_sync) else {
        return false;
    };
    bootstrap_completed_at > sync_completed_at
}

fn summaries_follow_up_pending(
    session: &InitSessionRecord,
    initial_sync: Option<&DevqlTaskRecord>,
    follow_up_sync: Option<&DevqlTaskRecord>,
    summary_run: Option<&SummaryBootstrapRunRecord>,
) -> bool {
    if session.selections.summaries_bootstrap.is_none() {
        return false;
    }
    let Some(bootstrap_completed_at) = completed_summary_run_at(summary_run) else {
        return false;
    };
    let Some(sync_completed_at) = latest_completed_sync_at(initial_sync, follow_up_sync) else {
        return false;
    };
    bootstrap_completed_at > sync_completed_at
}

fn semantic_bootstrap_still_outstanding_after_initial_sync(
    session: &InitSessionRecord,
    initial_sync: Option<&DevqlTaskRecord>,
    embeddings_task: Option<&DevqlTaskRecord>,
    summary_run: Option<&SummaryBootstrapRunRecord>,
) -> bool {
    embeddings_bootstrap_outstanding_after_initial_sync(session, initial_sync, embeddings_task)
        || summary_bootstrap_outstanding_after_initial_sync(session, initial_sync, summary_run)
}

fn semantic_follow_up_ready_for_sync(
    session: &InitSessionRecord,
    initial_sync: Option<&DevqlTaskRecord>,
    follow_up_sync: Option<&DevqlTaskRecord>,
    embeddings_task: Option<&DevqlTaskRecord>,
    summary_run: Option<&SummaryBootstrapRunRecord>,
) -> bool {
    embeddings_follow_up_pending(session, initial_sync, follow_up_sync, embeddings_task)
        || summaries_follow_up_pending(session, initial_sync, follow_up_sync, summary_run)
}

fn semantic_follow_up_pending(
    session: &InitSessionRecord,
    initial_sync: Option<&DevqlTaskRecord>,
    follow_up_sync: Option<&DevqlTaskRecord>,
    embeddings_task: Option<&DevqlTaskRecord>,
    summary_run: Option<&SummaryBootstrapRunRecord>,
) -> bool {
    running_task(follow_up_sync).is_some()
        || semantic_bootstrap_still_outstanding_after_initial_sync(
            session,
            initial_sync,
            embeddings_task,
            summary_run,
        )
        || semantic_follow_up_ready_for_sync(
            session,
            initial_sync,
            follow_up_sync,
            embeddings_task,
            summary_run,
        )
}

fn selected_top_level_terminal(
    session: &InitSessionRecord,
    initial_sync: Option<&DevqlTaskRecord>,
    ingest_task: Option<&DevqlTaskRecord>,
) -> bool {
    let sync_terminal = if session.selections.run_sync {
        initial_sync.is_some_and(|task| {
            matches!(
                task.status,
                DevqlTaskStatus::Completed | DevqlTaskStatus::Failed | DevqlTaskStatus::Cancelled
            )
        })
    } else {
        true
    };
    let ingest_terminal = if session.selections.run_ingest {
        ingest_task.is_some_and(|task| {
            matches!(
                task.status,
                DevqlTaskStatus::Completed | DevqlTaskStatus::Failed | DevqlTaskStatus::Cancelled
            )
        })
    } else {
        true
    };
    sync_terminal && ingest_terminal
}

fn derive_top_pipeline_lane(
    session: &InitSessionRecord,
    initial_sync: Option<&DevqlTaskRecord>,
    ingest_task: Option<&DevqlTaskRecord>,
    follow_up_sync: Option<&DevqlTaskRecord>,
    current_state: StatusCounts,
) -> InitRuntimeLaneView {
    if !session.selections.run_sync && !session.selections.run_ingest {
        return skipped_lane();
    }
    if let Some(task) = active_task(follow_up_sync) {
        return lane_from_task(
            task,
            Some("follow_up_sync".to_string()),
            current_state,
            None,
            Vec::new(),
        );
    }
    if let Some(task) = active_task(ingest_task) {
        return lane_from_task(
            task,
            Some("ingest".to_string()),
            current_state,
            None,
            Vec::new(),
        );
    }
    if let Some(task) = active_task(initial_sync) {
        return lane_from_task(
            task,
            Some("sync".to_string()),
            current_state,
            None,
            Vec::new(),
        );
    }
    if let Some(task) = follow_up_sync
        && task_failed(Some(task))
    {
        return failed_lane(
            Some("Running a follow-up sync failed".to_string()),
            current_state,
            Some(task.task_id.clone()),
            None,
            None,
            Vec::new(),
        );
    }
    if let Some(task) = ingest_task
        && task_failed(Some(task))
    {
        return failed_lane(
            Some("Ingesting commit history failed".to_string()),
            current_state,
            Some(task.task_id.clone()),
            None,
            None,
            Vec::new(),
        );
    }
    if let Some(task) = initial_sync
        && task_failed(Some(task))
    {
        return failed_lane(
            Some("Syncing repository failed".to_string()),
            current_state,
            Some(task.task_id.clone()),
            None,
            None,
            Vec::new(),
        );
    }
    if current_state.failed > 0 {
        return failed_lane(
            Some("Applying codebase updates failed".to_string()),
            current_state,
            None,
            None,
            None,
            Vec::new(),
        );
    }
    if current_state.pending > 0 || current_state.running > 0 {
        return runtime_lane("waiting", None, current_state, Vec::new())
            .with_waiting_reason("waiting_for_current_state_consumer")
            .with_activity_label("Applying codebase updates");
    }
    completed_lane()
}

fn derive_session_status(
    has_failure: bool,
    has_remaining_work: bool,
    completed: bool,
    waiting_reason: Option<&str>,
    has_warnings: bool,
) -> &'static str {
    if has_failure && has_remaining_work {
        "failing"
    } else if has_failure {
        "failed"
    } else if completed && has_warnings {
        "completed_with_warnings"
    } else if completed {
        "completed"
    } else if waiting_reason.is_some_and(|reason| reason.starts_with("waiting")) {
        "waiting"
    } else {
        "running"
    }
}

fn derive_embeddings_lane(
    session: &InitSessionRecord,
    initial_sync: Option<&DevqlTaskRecord>,
    follow_up_sync: Option<&DevqlTaskRecord>,
    embeddings_task: Option<&DevqlTaskRecord>,
    stats: &SessionWorkplaneStats,
    progress: Option<InitRuntimeLaneProgressView>,
) -> InitRuntimeLaneView {
    if session.selections.embeddings_bootstrap.is_none() {
        return skipped_lane();
    }
    let warnings = stats.embedding_warnings();
    if let Some(task) = active_task(embeddings_task) {
        return lane_from_task(
            task,
            Some("embeddings_bootstrap".to_string()),
            stats.embedding_jobs,
            progress,
            warnings,
        );
    }
    if let Some(task) = embeddings_task
        && task_failed(Some(task))
    {
        return failed_lane(
            Some("Preparing the embeddings runtime failed".to_string()),
            stats.embedding_jobs,
            Some(task.task_id.clone()),
            None,
            progress,
            warnings,
        );
    }
    if let Some(reason) = stats.blocked_embedding_reason.clone() {
        return runtime_lane("waiting", progress, stats.embedding_jobs, warnings)
            .with_waiting_reason("blocked_mailbox")
            .with_activity_label(
                stats
                    .active_embedding_mailbox()
                    .map(mailbox_label)
                    .unwrap_or("Building the semantic search index")
                    .to_string(),
            )
            .with_detail(reason);
    }
    if stats.embedding_jobs.pending > 0 || stats.embedding_jobs.running > 0 {
        return runtime_lane(
            if stats.embedding_jobs.running > 0 {
                "running"
            } else {
                "queued"
            },
            progress,
            stats.embedding_jobs,
            warnings,
        )
        .with_activity_label(
            stats
                .active_embedding_mailbox()
                .map(mailbox_label)
                .unwrap_or("Building the semantic search index")
                .to_string(),
        );
    }
    if embeddings_bootstrap_outstanding_after_initial_sync(session, initial_sync, embeddings_task) {
        return runtime_lane("waiting", progress, stats.embedding_jobs, warnings)
            .with_waiting_reason("waiting_for_embeddings_bootstrap")
            .with_activity_label("Preparing the embeddings runtime");
    }
    if embeddings_follow_up_pending(session, initial_sync, follow_up_sync, embeddings_task) {
        return runtime_lane("waiting", progress, stats.embedding_jobs, warnings)
            .with_waiting_reason("waiting_for_follow_up_sync")
            .with_activity_label("Running a follow-up sync");
    }
    if !warnings.is_empty() {
        return runtime_lane("warning", progress, stats.embedding_jobs, warnings)
            .with_activity_label("Building the semantic search index");
    }
    if progress_has_remaining(progress.as_ref()) {
        return runtime_lane("waiting", progress, stats.embedding_jobs, warnings)
            .with_waiting_reason("waiting_for_workplane")
            .with_activity_label("Building the semantic search index");
    }
    completed_lane_with_progress(progress)
}

fn derive_summaries_lane(
    session: &InitSessionRecord,
    initial_sync: Option<&DevqlTaskRecord>,
    follow_up_sync: Option<&DevqlTaskRecord>,
    summary_run: Option<&SummaryBootstrapRunRecord>,
    stats: &SessionWorkplaneStats,
    progress: Option<InitRuntimeLaneProgressView>,
) -> InitRuntimeLaneView {
    if session.selections.summaries_bootstrap.is_none() {
        return skipped_lane();
    }
    let warnings = stats.summary_warnings();
    if let Some(run) = summary_run {
        if run.status == SummaryBootstrapStatus::Running
            || run.status == SummaryBootstrapStatus::Queued
        {
            return runtime_lane(
                if run.status == SummaryBootstrapStatus::Queued {
                    "queued"
                } else {
                    "running"
                },
                progress,
                stats.summary_jobs,
                warnings,
            )
            .with_activity_label("Preparing summary generation")
            .with_run_id_option(Some(run.run_id.clone()));
        }
        if run.status == SummaryBootstrapStatus::Failed {
            return failed_lane(
                Some("Preparing summary generation failed".to_string()),
                stats.summary_jobs,
                None,
                Some(run.run_id.clone()),
                progress,
                warnings,
            );
        }
    }
    if let Some(reason) = stats.blocked_summary_reason.clone() {
        return runtime_lane("waiting", progress, stats.summary_jobs, warnings)
            .with_waiting_reason("blocked_mailbox")
            .with_activity_label("Generating summaries")
            .with_run_id_option(summary_run.map(|run| run.run_id.clone()))
            .with_detail(reason);
    }
    if stats.summary_jobs.pending > 0 || stats.summary_jobs.running > 0 {
        return runtime_lane(
            if stats.summary_jobs.running > 0 {
                "running"
            } else {
                "queued"
            },
            progress,
            stats.summary_jobs,
            warnings,
        )
        .with_activity_label("Generating summaries")
        .with_run_id_option(summary_run.map(|run| run.run_id.clone()));
    }
    if summary_bootstrap_outstanding_after_initial_sync(session, initial_sync, summary_run) {
        return runtime_lane("waiting", progress, stats.summary_jobs, warnings)
            .with_waiting_reason("waiting_for_summary_bootstrap")
            .with_activity_label("Preparing summary generation")
            .with_run_id_option(summary_run.map(|run| run.run_id.clone()));
    }
    if summaries_follow_up_pending(session, initial_sync, follow_up_sync, summary_run) {
        return runtime_lane("waiting", progress, stats.summary_jobs, warnings)
            .with_waiting_reason("waiting_for_follow_up_sync")
            .with_activity_label("Running a follow-up sync")
            .with_run_id_option(summary_run.map(|run| run.run_id.clone()));
    }
    if !warnings.is_empty() {
        return runtime_lane("warning", progress, stats.summary_jobs, warnings)
            .with_activity_label("Generating summaries")
            .with_run_id_option(summary_run.map(|run| run.run_id.clone()));
    }
    if progress_has_remaining(progress.as_ref()) {
        return runtime_lane("waiting", progress, stats.summary_jobs, warnings)
            .with_waiting_reason("waiting_for_workplane")
            .with_activity_label("Generating summaries")
            .with_run_id_option(summary_run.map(|run| run.run_id.clone()));
    }
    completed_lane_with_progress(progress)
}

fn lane_from_task(
    task: &DevqlTaskRecord,
    detail: Option<String>,
    counts: StatusCounts,
    progress: Option<InitRuntimeLaneProgressView>,
    warnings: Vec<InitRuntimeLaneWarningView>,
) -> InitRuntimeLaneView {
    let activity_label = detail
        .as_deref()
        .map(lane_activity_label)
        .map(str::to_string)
        .or_else(|| Some(task_kind_label(&task.kind.to_string()).to_string()));
    let status = match task.status {
        DevqlTaskStatus::Queued => "queued",
        DevqlTaskStatus::Running => "running",
        DevqlTaskStatus::Completed => "completed",
        DevqlTaskStatus::Failed | DevqlTaskStatus::Cancelled => "failed",
    };
    let lane = runtime_lane(status, progress, counts, warnings)
        .with_activity_label_option(activity_label)
        .with_task_id_option(Some(task.task_id.clone()));
    if let Some(detail) = detail {
        lane.with_detail(lane_activity_label(&detail).to_string())
    } else {
        lane
    }
}

fn failed_lane(
    detail: Option<String>,
    counts: StatusCounts,
    task_id: Option<String>,
    run_id: Option<String>,
    progress: Option<InitRuntimeLaneProgressView>,
    warnings: Vec<InitRuntimeLaneWarningView>,
) -> InitRuntimeLaneView {
    let lane = runtime_lane("failed", progress, counts, warnings)
        .with_waiting_reason("failed")
        .with_activity_label_option(detail.clone())
        .with_task_id_option(task_id)
        .with_run_id_option(run_id);
    if let Some(detail) = detail {
        lane.with_detail(detail)
    } else {
        lane
    }
}

fn completed_lane() -> InitRuntimeLaneView {
    completed_lane_with_progress(None)
}

fn completed_lane_with_progress(
    progress: Option<InitRuntimeLaneProgressView>,
) -> InitRuntimeLaneView {
    runtime_lane("completed", progress, StatusCounts::default(), Vec::new())
}

fn skipped_lane() -> InitRuntimeLaneView {
    runtime_lane("skipped", None, StatusCounts::default(), Vec::new())
}

impl InitRuntimeLaneView {
    fn with_activity_label(mut self, activity_label: impl Into<String>) -> Self {
        let activity_label = activity_label.into();
        self.detail = Some(activity_label.clone());
        self.activity_label = Some(activity_label);
        self
    }

    fn with_activity_label_option(mut self, activity_label: Option<String>) -> Self {
        if let Some(activity_label) = activity_label {
            self = self.with_activity_label(activity_label);
        }
        self
    }

    fn with_waiting_reason(mut self, waiting_reason: impl Into<String>) -> Self {
        self.waiting_reason = Some(waiting_reason.into());
        self
    }

    fn with_task_id_option(mut self, task_id: Option<String>) -> Self {
        self.task_id = task_id;
        self
    }

    fn with_run_id_option(mut self, run_id: Option<String>) -> Self {
        self.run_id = run_id;
        self
    }

    fn with_detail(mut self, detail: String) -> Self {
        self.detail = Some(detail);
        self
    }
}

fn runtime_lane(
    status: &str,
    progress: Option<InitRuntimeLaneProgressView>,
    counts: StatusCounts,
    warnings: Vec<InitRuntimeLaneWarningView>,
) -> InitRuntimeLaneView {
    InitRuntimeLaneView {
        status: status.to_string(),
        waiting_reason: None,
        detail: None,
        activity_label: None,
        task_id: None,
        run_id: None,
        progress,
        queue: InitRuntimeLaneQueueView {
            queued: counts.queued(),
            running: counts.running,
            failed: counts.failed,
        },
        warnings,
        pending_count: counts.pending,
        running_count: counts.running,
        failed_count: counts.failed,
        completed_count: counts.completed,
    }
}

fn progress_has_remaining(progress: Option<&InitRuntimeLaneProgressView>) -> bool {
    progress.is_some_and(|progress| progress.remaining > 0)
}

fn derive_embeddings_completed_count(
    code_total: u64,
    code_completed_current: u64,
    code_queue: StatusCounts,
    summary_total: u64,
    summary_completed_current: u64,
    summaries_completed_current: u64,
    summary_queue: StatusCounts,
) -> u64 {
    let code_completed = code_total
        .saturating_sub(code_queue.pending + code_queue.running + code_queue.failed)
        .max(code_completed_current.min(code_total));
    let summary_completed = summaries_completed_current
        .saturating_sub(summary_queue.pending + summary_queue.running + summary_queue.failed)
        .max(summary_completed_current.min(summary_total))
        .min(summary_total)
        .min(summaries_completed_current);

    code_completed + summary_completed
}

fn active_task(task: Option<&DevqlTaskRecord>) -> Option<&DevqlTaskRecord> {
    task.filter(|task| {
        matches!(
            task.status,
            DevqlTaskStatus::Queued | DevqlTaskStatus::Running
        )
    })
}

fn running_task(task: Option<&DevqlTaskRecord>) -> Option<&DevqlTaskRecord> {
    task.filter(|task| task.status == DevqlTaskStatus::Running)
}

fn workplane_snapshot_from_mailboxes(
    repo_root: &Path,
    mailboxes: &BTreeMap<String, CapabilityMailboxStatus>,
    blocked_mailboxes: &[BlockedMailboxStatus],
) -> InitRuntimeWorkplaneSnapshot {
    let budgets = configured_enrichment_worker_budgets_for_repo(repo_root);
    let blocked_by_mailbox = blocked_mailboxes
        .iter()
        .map(|blocked| (blocked.mailbox_name.as_str(), blocked.reason.as_str()))
        .collect::<BTreeMap<_, _>>();
    let mut snapshot_mailboxes = mailboxes
        .iter()
        .map(
            |(mailbox_name, status)| InitRuntimeWorkplaneMailboxSnapshot {
                mailbox_name: mailbox_name.clone(),
                display_name: mailbox_label(mailbox_name).to_string(),
                pending_jobs: status.pending_jobs,
                running_jobs: status.running_jobs,
                failed_jobs: status.failed_jobs,
                completed_recent_jobs: status.completed_recent_jobs,
                pending_cursor_runs: status.pending_cursor_runs,
                running_cursor_runs: status.running_cursor_runs,
                failed_cursor_runs: status.failed_cursor_runs,
                completed_recent_cursor_runs: status.completed_recent_cursor_runs,
                intent_active: status.intent_active,
                blocked_reason: blocked_by_mailbox
                    .get(mailbox_name.as_str())
                    .map(|reason| (*reason).to_string()),
            },
        )
        .collect::<Vec<_>>();
    snapshot_mailboxes.sort_by(|left, right| left.mailbox_name.cmp(&right.mailbox_name));
    let summary_mailbox = snapshot_mailboxes
        .iter()
        .find(|mailbox| mailbox.mailbox_name == SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX);
    let code_embedding_mailbox = snapshot_mailboxes
        .iter()
        .find(|mailbox| mailbox.mailbox_name == SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX);
    let summary_embedding_mailbox = snapshot_mailboxes
        .iter()
        .find(|mailbox| mailbox.mailbox_name == SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX);
    let clone_rebuild_mailbox = snapshot_mailboxes
        .iter()
        .find(|mailbox| mailbox.mailbox_name == SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX);
    let pools = vec![
        InitRuntimeWorkplanePoolSnapshot {
            pool_name: "summary_refresh".to_string(),
            display_name: workplane_pool_label("summary_refresh").to_string(),
            worker_budget: budgets.summary_refresh as u64,
            active_workers: summary_mailbox
                .map(|mailbox| mailbox.running_jobs)
                .unwrap_or_default(),
            pending_jobs: summary_mailbox
                .map(|mailbox| mailbox.pending_jobs)
                .unwrap_or_default(),
            running_jobs: summary_mailbox
                .map(|mailbox| mailbox.running_jobs)
                .unwrap_or_default(),
            failed_jobs: summary_mailbox
                .map(|mailbox| mailbox.failed_jobs)
                .unwrap_or_default(),
            completed_recent_jobs: summary_mailbox
                .map(|mailbox| mailbox.completed_recent_jobs)
                .unwrap_or_default(),
        },
        InitRuntimeWorkplanePoolSnapshot {
            pool_name: "embeddings".to_string(),
            display_name: workplane_pool_label("embeddings").to_string(),
            worker_budget: budgets.embeddings as u64,
            active_workers: code_embedding_mailbox
                .map(|mailbox| mailbox.running_jobs)
                .unwrap_or_default()
                + summary_embedding_mailbox
                    .map(|mailbox| mailbox.running_jobs)
                    .unwrap_or_default(),
            pending_jobs: code_embedding_mailbox
                .map(|mailbox| mailbox.pending_jobs)
                .unwrap_or_default()
                + summary_embedding_mailbox
                    .map(|mailbox| mailbox.pending_jobs)
                    .unwrap_or_default(),
            running_jobs: code_embedding_mailbox
                .map(|mailbox| mailbox.running_jobs)
                .unwrap_or_default()
                + summary_embedding_mailbox
                    .map(|mailbox| mailbox.running_jobs)
                    .unwrap_or_default(),
            failed_jobs: code_embedding_mailbox
                .map(|mailbox| mailbox.failed_jobs)
                .unwrap_or_default()
                + summary_embedding_mailbox
                    .map(|mailbox| mailbox.failed_jobs)
                    .unwrap_or_default(),
            completed_recent_jobs: code_embedding_mailbox
                .map(|mailbox| mailbox.completed_recent_jobs)
                .unwrap_or_default()
                + summary_embedding_mailbox
                    .map(|mailbox| mailbox.completed_recent_jobs)
                    .unwrap_or_default(),
        },
        InitRuntimeWorkplanePoolSnapshot {
            pool_name: "clone_rebuild".to_string(),
            display_name: workplane_pool_label("clone_rebuild").to_string(),
            worker_budget: budgets.clone_rebuild as u64,
            active_workers: clone_rebuild_mailbox
                .map(|mailbox| mailbox.running_jobs)
                .unwrap_or_default(),
            pending_jobs: clone_rebuild_mailbox
                .map(|mailbox| mailbox.pending_jobs)
                .unwrap_or_default(),
            running_jobs: clone_rebuild_mailbox
                .map(|mailbox| mailbox.running_jobs)
                .unwrap_or_default(),
            failed_jobs: clone_rebuild_mailbox
                .map(|mailbox| mailbox.failed_jobs)
                .unwrap_or_default(),
            completed_recent_jobs: clone_rebuild_mailbox
                .map(|mailbox| mailbox.completed_recent_jobs)
                .unwrap_or_default(),
        },
    ];
    InitRuntimeWorkplaneSnapshot {
        pending_jobs: snapshot_mailboxes
            .iter()
            .map(|mailbox| mailbox.pending_jobs)
            .sum(),
        running_jobs: snapshot_mailboxes
            .iter()
            .map(|mailbox| mailbox.running_jobs)
            .sum(),
        failed_jobs: snapshot_mailboxes
            .iter()
            .map(|mailbox| mailbox.failed_jobs)
            .sum(),
        completed_recent_jobs: snapshot_mailboxes
            .iter()
            .map(|mailbox| mailbox.completed_recent_jobs)
            .sum(),
        pools,
        mailboxes: snapshot_mailboxes,
    }
}

fn load_session_workplane_stats(
    repo_root: &Path,
    repo_store: &RepoSqliteRuntimeStore,
    repo_id: &str,
    init_session_id: &str,
) -> Result<SessionWorkplaneStats> {
    let sqlite = repo_store.connect_repo_sqlite()?;
    let summary_freshness = load_summary_freshness_state_for_repo(repo_root, repo_id)
        .unwrap_or_else(|err| {
            log::debug!(
                "failed to load summary freshness state for repo `{repo_id}` at `{}`: {err:#}",
                repo_root.display()
            );
            SummaryFreshnessState::default()
        });
    sqlite.with_connection(|conn| {
        let mut stats = SessionWorkplaneStats::default();

        let mut cursor_stmt = conn.prepare(
            "SELECT status, COUNT(*)
             FROM capability_workplane_cursor_runs
             WHERE repo_id = ?1 AND init_session_id = ?2
             GROUP BY status",
        )?;
        let cursor_rows = cursor_stmt.query_map([repo_id, init_session_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        for row in cursor_rows {
            let (status, count) = row?;
            let count = u64::try_from(count).unwrap_or_default();
            match status.as_str() {
                "queued" => stats.current_state.pending += count,
                "running" => stats.current_state.running += count,
                "completed" => stats.current_state.completed += count,
                "failed" | "cancelled" => stats.current_state.failed += count,
                _ => {}
            }
        }
        stats.failed_current_state_detail = conn
            .query_row(
                "SELECT run_id, error
                 FROM capability_workplane_cursor_runs
                 WHERE repo_id = ?1
                   AND init_session_id = ?2
                   AND status IN ('failed', 'cancelled')
                 ORDER BY updated_at_unix DESC
                 LIMIT 1",
                params![repo_id, init_session_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .optional()?
            .map(|(run_id, error)| {
                format!(
                    "Applying codebase updates failed for run `{run_id}`{}",
                    error
                        .as_deref()
                        .map(|error| format!(": {error}"))
                        .unwrap_or_default()
                )
            });

        let mut job_stmt = conn.prepare(
            "SELECT mailbox_name, status, payload
             FROM capability_workplane_jobs
             WHERE repo_id = ?1 AND init_session_id = ?2",
        )?;
        let job_rows = job_stmt.query_map([repo_id, init_session_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        for row in job_rows {
            let (mailbox_name, status, payload_json) = row?;
            let target = mailbox_stats_mut(&mut stats, mailbox_name.as_str());
            let payload = serde_json::from_str::<serde_json::Value>(&payload_json)
                .unwrap_or(serde_json::Value::Null);
            let count = effective_session_work_item_count(
                mailbox_name.as_str(),
                status.as_str(),
                &payload,
                &summary_freshness,
            );
            match status.as_str() {
                "pending" => target.counts.pending += count,
                "running" => target.counts.running += count,
                "completed" => target.counts.completed += count,
                "failed" => target.counts.failed += count,
                _ => {}
            }
        }
        stats.summary_jobs = stats.summary_refresh_jobs.counts;
        stats.embedding_jobs = merge_status_counts([
            stats.code_embedding_jobs.counts,
            stats.summary_embedding_jobs.counts,
            stats.clone_rebuild_jobs.counts,
        ]);
        stats.summary_refresh_jobs.latest_error = latest_mailbox_error(
            conn,
            repo_id,
            init_session_id,
            SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
        )?;
        stats.code_embedding_jobs.latest_error = latest_mailbox_error(
            conn,
            repo_id,
            init_session_id,
            SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
        )?;
        stats.summary_embedding_jobs.latest_error = latest_mailbox_error(
            conn,
            repo_id,
            init_session_id,
            SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
        )?;
        stats.clone_rebuild_jobs.latest_error = latest_mailbox_error(
            conn,
            repo_id,
            init_session_id,
            SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
        )?;

        for blocked in repo_blocked_mailboxes(repo_store.db_path().to_path_buf(), repo_id)? {
            if blocked.mailbox_name == SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX {
                if stats.summary_refresh_jobs.counts.has_pending_or_running() {
                    stats
                        .blocked_summary_reason
                        .get_or_insert(blocked.reason.clone());
                }
                continue;
            }
            if stats_for_mailbox(&stats, blocked.mailbox_name.as_str()).has_pending_or_running() {
                stats
                    .blocked_embedding_reason
                    .get_or_insert(blocked.reason.clone());
            }
        }

        Ok(stats)
    })
}

fn session_has_remaining_work(
    initial_sync: Option<&DevqlTaskRecord>,
    ingest_task: Option<&DevqlTaskRecord>,
    follow_up_sync: Option<&DevqlTaskRecord>,
    embeddings_task: Option<&DevqlTaskRecord>,
    summary_run: Option<&SummaryBootstrapRunRecord>,
    stats: &SessionWorkplaneStats,
) -> bool {
    active_task(initial_sync).is_some()
        || active_task(ingest_task).is_some()
        || active_task(follow_up_sync).is_some()
        || active_task(embeddings_task).is_some()
        || summary_run.is_some_and(|run| {
            matches!(
                run.status,
                SummaryBootstrapStatus::Queued | SummaryBootstrapStatus::Running
            )
        })
        || stats.current_state.has_pending_or_running()
        || stats.embedding_jobs.has_pending_or_running()
        || stats.summary_jobs.has_pending_or_running()
}

fn session_fatal_failure_detail(
    initial_sync: Option<&DevqlTaskRecord>,
    ingest_task: Option<&DevqlTaskRecord>,
    follow_up_sync: Option<&DevqlTaskRecord>,
    embeddings_task: Option<&DevqlTaskRecord>,
    summary_run: Option<&SummaryBootstrapRunRecord>,
    stats: &SessionWorkplaneStats,
) -> Option<String> {
    if let Some(task) = initial_sync
        && task_failed(Some(task))
    {
        return Some(task_failure_detail("Syncing repository", task));
    }
    if let Some(task) = ingest_task
        && task_failed(Some(task))
    {
        return Some(task_failure_detail("Ingesting commit history", task));
    }
    if let Some(task) = follow_up_sync
        && task_failed(Some(task))
    {
        return Some(task_failure_detail("Running a follow-up sync", task));
    }
    if let Some(task) = embeddings_task
        && task_failed(Some(task))
    {
        return Some(task_failure_detail(
            "Preparing the embeddings runtime",
            task,
        ));
    }
    if let Some(run) = summary_run
        && summary_run_failed(run)
    {
        return Some(summary_bootstrap_failure_detail(run));
    }
    if let Some(detail) = stats.failed_current_state_detail.clone() {
        return Some(detail);
    }
    None
}

fn task_failure_detail(label: &str, task: &DevqlTaskRecord) -> String {
    let error = task
        .error
        .as_deref()
        .map(str::to_string)
        .unwrap_or_else(|| format!("task ended with status {}", task.status));
    format!("{label} failed: {error}")
}

fn summary_bootstrap_failure_detail(run: &SummaryBootstrapRunRecord) -> String {
    format!(
        "Preparing summary generation failed{}",
        run.error
            .as_deref()
            .map(|error| format!(": {error}"))
            .unwrap_or_default()
    )
}

fn load_runtime_lane_progress(
    repo_root: &Path,
    repo_id: &str,
    session: &InitSessionRecord,
    stats: &SessionWorkplaneStats,
) -> Result<RuntimeLaneProgressState> {
    let relational =
        DefaultRelationalStore::open_local_for_repo_root_preferring_bound_config(repo_root)?;
    let total_eligible = count_eligible_current_artefacts(&relational, repo_id)?;
    let summaries_completed = count_current_model_backed_summary_artefacts(&relational, repo_id)?;
    let code_embeddings_completed =
        count_current_embedding_artefacts(&relational, repo_id, "code")?;
    let summary_embeddings_completed =
        count_current_embedding_artefacts(&relational, repo_id, "summary")?;
    let semantic_clones = resolve_semantic_clones_config_for_repo(repo_root);

    let code_embeddings_enabled =
        embedding_slot_for_representation(&semantic_clones, EmbeddingRepresentationKind::Code)
            .is_some();
    let summary_embeddings_enabled =
        embedding_slot_for_representation(&semantic_clones, EmbeddingRepresentationKind::Summary)
            .is_some();
    let code_embeddings_total = u64::from(code_embeddings_enabled) * total_eligible;
    let summary_embeddings_total = u64::from(summary_embeddings_enabled) * total_eligible;
    let embeddings_total = code_embeddings_total + summary_embeddings_total;
    let embeddings_completed = derive_embeddings_completed_count(
        code_embeddings_total,
        code_embeddings_completed,
        stats.code_embedding_jobs.counts,
        summary_embeddings_total,
        summary_embeddings_completed,
        summaries_completed,
        stats.summary_embedding_jobs.counts,
    )
    .min(embeddings_total);
    let summaries_total = if session.selections.summaries_bootstrap.is_some() {
        total_eligible
    } else {
        0
    };

    Ok(RuntimeLaneProgressState {
        embeddings: (session.selections.embeddings_bootstrap.is_some() && embeddings_total > 0)
            .then(|| InitRuntimeLaneProgressView {
                completed: embeddings_completed,
                total: embeddings_total,
                remaining: embeddings_total.saturating_sub(embeddings_completed),
            }),
        summaries: (session.selections.summaries_bootstrap.is_some() && summaries_total > 0).then(
            || InitRuntimeLaneProgressView {
                completed: summaries_completed.min(summaries_total),
                total: summaries_total,
                remaining: summaries_total.saturating_sub(summaries_completed),
            },
        ),
    })
}

fn count_eligible_current_artefacts(
    relational: &DefaultRelationalStore,
    repo_id: &str,
) -> Result<u64> {
    query_progress_count(
        relational,
        &format!(
            "SELECT COUNT(DISTINCT a.artefact_id) AS total \
             FROM artefacts_current a \
             JOIN current_file_state cfs ON cfs.repo_id = a.repo_id AND cfs.path = a.path \
             WHERE a.repo_id = '{}' \
               AND cfs.analysis_mode = 'code' \
               AND LOWER(COALESCE(a.canonical_kind, COALESCE(a.language_kind, 'symbol'))) <> 'import'",
            escape_sql_string(repo_id),
        ),
    )
}

fn count_current_embedding_artefacts(
    relational: &DefaultRelationalStore,
    repo_id: &str,
    representation_kind: &str,
) -> Result<u64> {
    query_progress_count(
        relational,
        &format!(
            "SELECT COUNT(DISTINCT a.artefact_id) AS total \
             FROM artefacts_current a \
             JOIN current_file_state cfs ON cfs.repo_id = a.repo_id AND cfs.path = a.path \
             JOIN {CURRENT_CODE_EMBEDDINGS_TABLE} e ON e.repo_id = a.repo_id AND e.artefact_id = a.artefact_id \
             WHERE a.repo_id = '{}' \
               AND cfs.analysis_mode = 'code' \
               AND LOWER(COALESCE(a.canonical_kind, COALESCE(a.language_kind, 'symbol'))) <> 'import' \
               AND LOWER(COALESCE(e.representation_kind, 'code')) = '{}'",
            escape_sql_string(repo_id),
            escape_sql_string(representation_kind),
        ),
    )
}

fn count_current_model_backed_summary_artefacts(
    relational: &DefaultRelationalStore,
    repo_id: &str,
) -> Result<u64> {
    query_progress_count(
        relational,
        &format!(
            "SELECT COUNT(DISTINCT a.artefact_id) AS total \
             FROM artefacts_current a \
             JOIN current_file_state cfs ON cfs.repo_id = a.repo_id AND cfs.path = a.path \
             JOIN symbol_features_current f ON f.repo_id = a.repo_id AND f.artefact_id = a.artefact_id AND f.content_id = a.content_id \
             JOIN {CURRENT_SUMMARY_SEMANTICS_TABLE} s ON s.repo_id = a.repo_id AND s.artefact_id = a.artefact_id AND s.content_id = a.content_id \
             WHERE a.repo_id = '{}' \
               AND cfs.analysis_mode = 'code' \
               AND LOWER(COALESCE(a.canonical_kind, COALESCE(a.language_kind, 'symbol'))) <> 'import' \
               AND s.semantic_features_input_hash = f.semantic_features_input_hash \
               AND ( \
                    (s.llm_summary IS NOT NULL AND TRIM(s.llm_summary) <> '') \
                    OR (s.source_model IS NOT NULL AND TRIM(s.source_model) <> '') \
               )",
            escape_sql_string(repo_id),
        ),
    )
}

fn load_summary_freshness_state(
    relational: &DefaultRelationalStore,
    repo_id: &str,
) -> Result<SummaryFreshnessState> {
    let eligible_artefact_ids = query_progress_ids(
        relational,
        &format!(
            "SELECT DISTINCT a.artefact_id \
             FROM artefacts_current a \
             JOIN current_file_state cfs ON cfs.repo_id = a.repo_id AND cfs.path = a.path \
             WHERE a.repo_id = '{}' \
               AND cfs.analysis_mode = 'code' \
               AND LOWER(COALESCE(a.canonical_kind, COALESCE(a.language_kind, 'symbol'))) <> 'import'",
            escape_sql_string(repo_id),
        ),
    )?;
    let fresh_model_backed_artefact_ids = query_progress_ids(
        relational,
        &format!(
            "SELECT DISTINCT a.artefact_id \
             FROM artefacts_current a \
             JOIN current_file_state cfs ON cfs.repo_id = a.repo_id AND cfs.path = a.path \
             JOIN symbol_features_current f ON f.repo_id = a.repo_id AND f.artefact_id = a.artefact_id AND f.content_id = a.content_id \
             JOIN {CURRENT_SUMMARY_SEMANTICS_TABLE} s ON s.repo_id = a.repo_id AND s.artefact_id = a.artefact_id AND s.content_id = a.content_id \
             WHERE a.repo_id = '{}' \
               AND cfs.analysis_mode = 'code' \
               AND LOWER(COALESCE(a.canonical_kind, COALESCE(a.language_kind, 'symbol'))) <> 'import' \
               AND s.semantic_features_input_hash = f.semantic_features_input_hash \
               AND ( \
                    (s.llm_summary IS NOT NULL AND TRIM(s.llm_summary) <> '') \
                    OR (s.source_model IS NOT NULL AND TRIM(s.source_model) <> '') \
               )",
            escape_sql_string(repo_id),
        ),
    )?;

    Ok(SummaryFreshnessState {
        eligible_artefact_ids,
        fresh_model_backed_artefact_ids,
    })
}

fn query_progress_count(relational: &DefaultRelationalStore, sql: &str) -> Result<u64> {
    let sqlite = relational.local_sqlite_pool()?;
    let count =
        sqlite.with_connection(|conn| Ok(conn.query_row(sql, [], |row| row.get::<_, i64>(0))?));
    match count {
        Ok(value) => Ok(u64::try_from(value).unwrap_or_default()),
        Err(err) if missing_progress_table(&err) => Ok(0),
        Err(err) => Err(err),
    }
}

fn query_progress_ids(relational: &DefaultRelationalStore, sql: &str) -> Result<BTreeSet<String>> {
    let sqlite = relational.local_sqlite_pool()?;
    let values = sqlite.with_connection(|conn| {
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut ids = BTreeSet::new();
        for row in rows {
            ids.insert(row?);
        }
        Ok(ids)
    });
    match values {
        Ok(ids) => Ok(ids),
        Err(err) if missing_progress_table(&err) => Ok(BTreeSet::new()),
        Err(err) => Err(err),
    }
}

fn missing_progress_table(err: &anyhow::Error) -> bool {
    let message = err.to_string();
    message.contains("no such table:") || message.contains("does not exist")
}

fn escape_sql_string(value: &str) -> String {
    value.replace('\'', "''")
}

fn mailbox_stats_mut<'a>(
    stats: &'a mut SessionWorkplaneStats,
    mailbox_name: &str,
) -> &'a mut SessionMailboxStats {
    match mailbox_name {
        SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX => &mut stats.summary_refresh_jobs,
        SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX => &mut stats.code_embedding_jobs,
        SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX => &mut stats.summary_embedding_jobs,
        SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX => &mut stats.clone_rebuild_jobs,
        _ => &mut stats.clone_rebuild_jobs,
    }
}

fn merge_status_counts<const N: usize>(counts: [StatusCounts; N]) -> StatusCounts {
    counts
        .into_iter()
        .fold(StatusCounts::default(), |mut acc, counts| {
            acc.pending += counts.pending;
            acc.running += counts.running;
            acc.failed += counts.failed;
            acc.completed += counts.completed;
            acc
        })
}

fn latest_mailbox_error(
    conn: &rusqlite::Connection,
    repo_id: &str,
    init_session_id: &str,
    mailbox_name: &str,
) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT last_error
         FROM capability_workplane_jobs
         WHERE repo_id = ?1
           AND init_session_id = ?2
           AND status = 'failed'
           AND mailbox_name = ?3
         ORDER BY updated_at_unix DESC
         LIMIT 1",
        params![repo_id, init_session_id, mailbox_name],
        |row| row.get::<_, Option<String>>(0),
    )
    .optional()
    .map(|error| error.flatten())
}

fn repo_blocked_mailboxes(db_path: PathBuf, repo_id: &str) -> Result<Vec<BlockedMailboxStatus>> {
    let runtime_store = DaemonSqliteRuntimeStore::open()?;
    let workplane_store = DaemonSqliteRuntimeStore::open_at(db_path)?;
    super::enrichment::blocked_mailboxes_for_repo(&workplane_store, &runtime_store, repo_id)
}

fn load_summary_freshness_state_for_repo(
    repo_root: &Path,
    repo_id: &str,
) -> Result<SummaryFreshnessState> {
    let relational =
        DefaultRelationalStore::open_local_for_repo_root_preferring_bound_config(repo_root)?;
    load_summary_freshness_state(&relational, repo_id)
}

fn effective_session_work_item_count(
    mailbox_name: &str,
    status: &str,
    payload: &serde_json::Value,
    summary_freshness: &SummaryFreshnessState,
) -> u64 {
    if mailbox_name != SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX {
        return payload_work_item_count(payload, mailbox_name);
    }
    match status {
        "pending" | "running" | "failed" => {
            summary_effective_work_item_count(payload, summary_freshness)
        }
        _ => payload_work_item_count(payload, mailbox_name),
    }
}

fn summary_effective_work_item_count(
    payload: &serde_json::Value,
    summary_freshness: &SummaryFreshnessState,
) -> u64 {
    if payload_is_repo_backfill(payload) {
        return payload_repo_backfill_artefact_ids(payload)
            .map(|artefact_ids| {
                summary_freshness.outstanding_work_item_count_for_artefacts(&artefact_ids)
            })
            .unwrap_or_else(|| summary_freshness.outstanding_work_item_count());
    }
    payload_artefact_id(payload)
        .map(|artefact_id| u64::from(summary_freshness.artefact_needs_refresh(&artefact_id)))
        .unwrap_or_else(|| {
            payload_work_item_count(payload, SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX)
        })
}

fn stats_for_mailbox(stats: &SessionWorkplaneStats, mailbox_name: &str) -> StatusCounts {
    match mailbox_name {
        SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX => stats.summary_refresh_jobs.counts,
        SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX => stats.code_embedding_jobs.counts,
        SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX => stats.summary_embedding_jobs.counts,
        SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX => stats.clone_rebuild_jobs.counts,
        _ => StatusCounts::default(),
    }
}

#[cfg(test)]
fn is_summary_mailbox(mailbox_name: &str) -> bool {
    mailbox_name == SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX
}

#[cfg(test)]
mod tests {
    use super::{
        InitRuntimeLaneProgressView, SessionMailboxStats, SessionWorkplaneStats, StatusCounts,
        SummaryFreshnessState, derive_embeddings_completed_count, derive_session_status,
        derive_summaries_lane, derive_top_pipeline_lane, is_summary_mailbox,
        semantic_bootstrap_waiting_reason, semantic_follow_up_ready_for_sync,
        summary_effective_work_item_count,
    };
    use crate::capability_packs::semantic_clones::types::{
        SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX, SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
        SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX, SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
    };
    use crate::daemon::{
        DevqlTaskKind, DevqlTaskRecord, DevqlTaskSource, DevqlTaskStatus,
        EmbeddingsBootstrapTaskSpec, InitEmbeddingsBootstrapRequest, InitSessionRecord,
        StartInitSessionSelections, SummaryBootstrapAction, SummaryBootstrapProgress,
        SummaryBootstrapRequest, SummaryBootstrapRunRecord, SummaryBootstrapStatus, SyncTaskMode,
        SyncTaskSpec,
    };
    use serde_json::json;
    use std::path::PathBuf;

    fn completed_sync_task(task_id: &str, completed_at_unix: u64) -> DevqlTaskRecord {
        DevqlTaskRecord {
            task_id: task_id.to_string(),
            repo_id: "repo-1".to_string(),
            repo_name: "repo".to_string(),
            repo_provider: "local".to_string(),
            repo_organisation: "local".to_string(),
            repo_identity: "repo".to_string(),
            daemon_config_root: PathBuf::from("/tmp/config-1"),
            repo_root: PathBuf::from("/tmp/repo-1"),
            init_session_id: Some("init-session-1".to_string()),
            kind: DevqlTaskKind::Sync,
            source: DevqlTaskSource::Init,
            spec: crate::daemon::DevqlTaskSpec::Sync(SyncTaskSpec {
                mode: SyncTaskMode::Full,
                post_commit_snapshot: None,
            }),
            status: DevqlTaskStatus::Completed,
            submitted_at_unix: 1,
            started_at_unix: Some(1),
            updated_at_unix: completed_at_unix,
            completed_at_unix: Some(completed_at_unix),
            queue_position: None,
            tasks_ahead: None,
            error: None,
            progress: crate::daemon::DevqlTaskProgress::Sync(Default::default()),
            result: None,
        }
    }

    #[test]
    fn summary_lane_classification_only_includes_summary_refresh_mailbox() {
        assert!(is_summary_mailbox(SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX));
        assert!(!is_summary_mailbox(
            SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX
        ));
        assert!(!is_summary_mailbox(SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX));
        assert!(!is_summary_mailbox(SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX));
    }

    #[test]
    fn summary_effective_work_item_count_ignores_fresh_artefact_jobs() {
        let freshness = SummaryFreshnessState {
            eligible_artefact_ids: ["artefact-1".to_string()].into_iter().collect(),
            fresh_model_backed_artefact_ids: ["artefact-1".to_string()].into_iter().collect(),
        };

        let count =
            summary_effective_work_item_count(&json!({ "artefact_id": "artefact-1" }), &freshness);

        assert_eq!(count, 0);
    }

    #[test]
    fn summary_effective_work_item_count_uses_outstanding_repo_backfill_work() {
        let freshness = SummaryFreshnessState {
            eligible_artefact_ids: [
                "artefact-1".to_string(),
                "artefact-2".to_string(),
                "artefact-3".to_string(),
            ]
            .into_iter()
            .collect(),
            fresh_model_backed_artefact_ids: ["artefact-1".to_string()].into_iter().collect(),
        };

        let count = summary_effective_work_item_count(
            &json!({
                "kind": "repo_backfill",
                "work_item_count": 3,
                "artefact_ids": ["artefact-1", "artefact-2", "artefact-3"]
            }),
            &freshness,
        );

        assert_eq!(count, 2);
    }

    #[test]
    fn embeddings_completed_count_uses_queue_backlog_until_current_projection_catches_up() {
        let completed = derive_embeddings_completed_count(
            278,
            2,
            StatusCounts {
                pending: 0,
                running: 0,
                failed: 0,
                completed: 278,
            },
            278,
            6,
            278,
            StatusCounts {
                pending: 226,
                running: 1,
                failed: 1,
                completed: 50,
            },
        );

        assert_eq!(completed, 328);
    }

    #[test]
    fn embeddings_completed_count_never_exceeds_available_summaries() {
        let completed = derive_embeddings_completed_count(
            278,
            278,
            StatusCounts {
                pending: 0,
                running: 0,
                failed: 0,
                completed: 278,
            },
            278,
            10,
            40,
            StatusCounts {
                pending: 5,
                running: 0,
                failed: 0,
                completed: 35,
            },
        );

        assert_eq!(completed, 313);
    }

    #[test]
    fn summaries_lane_reports_summary_mailbox_blockage_without_waiting_for_embeddings() {
        let session = InitSessionRecord {
            init_session_id: "init-session-1".to_string(),
            repo_id: "repo-1".to_string(),
            repo_root: PathBuf::from("/tmp/repo-1"),
            daemon_config_root: PathBuf::from("/tmp/config-1"),
            selections: StartInitSessionSelections {
                run_sync: true,
                run_ingest: false,
                ingest_backfill: None,
                embeddings_bootstrap: Some(InitEmbeddingsBootstrapRequest {
                    config_path: PathBuf::from("/tmp/config-1/config.toml"),
                    profile_name: "local_code".to_string(),
                }),
                summaries_bootstrap: Some(SummaryBootstrapRequest {
                    action: SummaryBootstrapAction::ConfigureCloud,
                    message: None,
                    model_name: None,
                    gateway_url_override: None,
                }),
            },
            initial_sync_task_id: None,
            ingest_task_id: None,
            embeddings_bootstrap_task_id: Some("bootstrap-task-1".to_string()),
            summary_bootstrap_run_id: Some("summary-run-1".to_string()),
            follow_up_sync_required: false,
            follow_up_sync_task_id: None,
            submitted_at_unix: 1,
            updated_at_unix: 1,
            terminal_status: None,
            terminal_error: None,
        };
        let summary_run = SummaryBootstrapRunRecord {
            run_id: "summary-run-1".to_string(),
            repo_id: "repo-1".to_string(),
            repo_root: PathBuf::from("/tmp/repo-1"),
            init_session_id: "init-session-1".to_string(),
            request: SummaryBootstrapRequest {
                action: SummaryBootstrapAction::ConfigureCloud,
                message: None,
                model_name: None,
                gateway_url_override: None,
            },
            status: SummaryBootstrapStatus::Completed,
            progress: SummaryBootstrapProgress::default(),
            result: None,
            error: None,
            submitted_at_unix: 1,
            started_at_unix: Some(1),
            updated_at_unix: 1,
            completed_at_unix: Some(1),
        };
        let stats = SessionWorkplaneStats {
            summary_jobs: StatusCounts {
                pending: 1,
                running: 0,
                failed: 0,
                completed: 0,
            },
            blocked_summary_reason: Some(
                "embedding slot `summary_embeddings` is not configured yet".to_string(),
            ),
            ..SessionWorkplaneStats::default()
        };

        let lane = derive_summaries_lane(&session, None, None, Some(&summary_run), &stats, None);

        assert_eq!(lane.status, "waiting");
        assert_eq!(lane.waiting_reason.as_deref(), Some("blocked_mailbox"));
        assert_eq!(
            lane.detail.as_deref(),
            Some("embedding slot `summary_embeddings` is not configured yet")
        );
        assert_eq!(lane.pending_count, 1);
    }

    #[test]
    fn semantic_bootstrap_waiting_reason_distinguishes_embeddings_only() {
        let session = InitSessionRecord {
            init_session_id: "init-session-1".to_string(),
            repo_id: "repo-1".to_string(),
            repo_root: PathBuf::from("/tmp/repo-1"),
            daemon_config_root: PathBuf::from("/tmp/config-1"),
            selections: StartInitSessionSelections {
                run_sync: true,
                run_ingest: false,
                ingest_backfill: None,
                embeddings_bootstrap: Some(InitEmbeddingsBootstrapRequest {
                    config_path: PathBuf::from("/tmp/config-1/config.toml"),
                    profile_name: "local_code".to_string(),
                }),
                summaries_bootstrap: Some(SummaryBootstrapRequest {
                    action: SummaryBootstrapAction::ConfigureCloud,
                    message: None,
                    model_name: None,
                    gateway_url_override: None,
                }),
            },
            initial_sync_task_id: None,
            ingest_task_id: None,
            embeddings_bootstrap_task_id: Some("bootstrap-task-1".to_string()),
            summary_bootstrap_run_id: Some("summary-run-1".to_string()),
            follow_up_sync_required: true,
            follow_up_sync_task_id: None,
            submitted_at_unix: 1,
            updated_at_unix: 1,
            terminal_status: None,
            terminal_error: None,
        };
        let embeddings_task = DevqlTaskRecord {
            task_id: "bootstrap-task-1".to_string(),
            repo_id: "repo-1".to_string(),
            repo_name: "repo".to_string(),
            repo_provider: "local".to_string(),
            repo_organisation: "local".to_string(),
            repo_identity: "repo".to_string(),
            daemon_config_root: PathBuf::from("/tmp/config-1"),
            repo_root: PathBuf::from("/tmp/repo-1"),
            init_session_id: Some("init-session-1".to_string()),
            kind: DevqlTaskKind::EmbeddingsBootstrap,
            source: DevqlTaskSource::Init,
            spec: crate::daemon::DevqlTaskSpec::EmbeddingsBootstrap(EmbeddingsBootstrapTaskSpec {
                config_path: PathBuf::from("/tmp/config-1/config.toml"),
                profile_name: "local_code".to_string(),
            }),
            status: DevqlTaskStatus::Running,
            submitted_at_unix: 1,
            started_at_unix: Some(1),
            updated_at_unix: 1,
            completed_at_unix: None,
            queue_position: None,
            tasks_ahead: None,
            error: None,
            progress: crate::daemon::DevqlTaskProgress::EmbeddingsBootstrap(
                crate::daemon::EmbeddingsBootstrapProgress::default(),
            ),
            result: None,
        };
        let summary_run = SummaryBootstrapRunRecord {
            run_id: "summary-run-1".to_string(),
            repo_id: "repo-1".to_string(),
            repo_root: PathBuf::from("/tmp/repo-1"),
            init_session_id: "init-session-1".to_string(),
            request: SummaryBootstrapRequest {
                action: SummaryBootstrapAction::ConfigureCloud,
                message: None,
                model_name: None,
                gateway_url_override: None,
            },
            status: SummaryBootstrapStatus::Completed,
            progress: SummaryBootstrapProgress::default(),
            result: None,
            error: None,
            submitted_at_unix: 1,
            started_at_unix: Some(1),
            updated_at_unix: 1,
            completed_at_unix: Some(1),
        };

        assert_eq!(
            semantic_bootstrap_waiting_reason(&session, Some(&embeddings_task), Some(&summary_run)),
            Some("waiting_for_embeddings_bootstrap")
        );
    }

    #[test]
    fn summaries_lane_waits_for_follow_up_sync_after_summary_bootstrap_finishes_late() {
        let session = InitSessionRecord {
            init_session_id: "init-session-1".to_string(),
            repo_id: "repo-1".to_string(),
            repo_root: PathBuf::from("/tmp/repo-1"),
            daemon_config_root: PathBuf::from("/tmp/config-1"),
            selections: StartInitSessionSelections {
                run_sync: true,
                run_ingest: false,
                ingest_backfill: None,
                embeddings_bootstrap: Some(InitEmbeddingsBootstrapRequest {
                    config_path: PathBuf::from("/tmp/config-1/config.toml"),
                    profile_name: "local_code".to_string(),
                }),
                summaries_bootstrap: Some(SummaryBootstrapRequest {
                    action: SummaryBootstrapAction::ConfigureCloud,
                    message: None,
                    model_name: None,
                    gateway_url_override: None,
                }),
            },
            initial_sync_task_id: Some("sync-task-1".to_string()),
            ingest_task_id: None,
            embeddings_bootstrap_task_id: Some("bootstrap-task-1".to_string()),
            summary_bootstrap_run_id: Some("summary-run-1".to_string()),
            follow_up_sync_required: true,
            follow_up_sync_task_id: None,
            submitted_at_unix: 1,
            updated_at_unix: 1,
            terminal_status: None,
            terminal_error: None,
        };
        let initial_sync = completed_sync_task("sync-task-1", 10);
        let summary_run = SummaryBootstrapRunRecord {
            run_id: "summary-run-1".to_string(),
            repo_id: "repo-1".to_string(),
            repo_root: PathBuf::from("/tmp/repo-1"),
            init_session_id: "init-session-1".to_string(),
            request: SummaryBootstrapRequest {
                action: SummaryBootstrapAction::ConfigureCloud,
                message: None,
                model_name: None,
                gateway_url_override: None,
            },
            status: SummaryBootstrapStatus::Completed,
            progress: SummaryBootstrapProgress::default(),
            result: None,
            error: None,
            submitted_at_unix: 1,
            started_at_unix: Some(1),
            updated_at_unix: 12,
            completed_at_unix: Some(12),
        };

        let lane = derive_summaries_lane(
            &session,
            Some(&initial_sync),
            None,
            Some(&summary_run),
            &SessionWorkplaneStats::default(),
            None,
        );

        assert_eq!(lane.status, "waiting");
        assert_eq!(
            lane.waiting_reason.as_deref(),
            Some("waiting_for_follow_up_sync")
        );
        assert_eq!(
            lane.activity_label.as_deref(),
            Some("Running a follow-up sync")
        );
    }

    #[test]
    fn summaries_lane_becomes_warning_after_failed_jobs_drain() {
        let session = InitSessionRecord {
            init_session_id: "init-session-1".to_string(),
            repo_id: "repo-1".to_string(),
            repo_root: PathBuf::from("/tmp/repo-1"),
            daemon_config_root: PathBuf::from("/tmp/config-1"),
            selections: StartInitSessionSelections {
                run_sync: true,
                run_ingest: false,
                ingest_backfill: None,
                embeddings_bootstrap: Some(InitEmbeddingsBootstrapRequest {
                    config_path: PathBuf::from("/tmp/config-1/config.toml"),
                    profile_name: "local_code".to_string(),
                }),
                summaries_bootstrap: Some(SummaryBootstrapRequest {
                    action: SummaryBootstrapAction::ConfigureCloud,
                    message: None,
                    model_name: None,
                    gateway_url_override: None,
                }),
            },
            initial_sync_task_id: Some("sync-task-1".to_string()),
            ingest_task_id: None,
            embeddings_bootstrap_task_id: Some("bootstrap-task-1".to_string()),
            summary_bootstrap_run_id: Some("summary-run-1".to_string()),
            follow_up_sync_required: false,
            follow_up_sync_task_id: None,
            submitted_at_unix: 1,
            updated_at_unix: 1,
            terminal_status: None,
            terminal_error: None,
        };
        let initial_sync = completed_sync_task("sync-task-1", 10);
        let summary_run = SummaryBootstrapRunRecord {
            run_id: "summary-run-1".to_string(),
            repo_id: "repo-1".to_string(),
            repo_root: PathBuf::from("/tmp/repo-1"),
            init_session_id: "init-session-1".to_string(),
            request: SummaryBootstrapRequest {
                action: SummaryBootstrapAction::ConfigureCloud,
                message: None,
                model_name: None,
                gateway_url_override: None,
            },
            status: SummaryBootstrapStatus::Completed,
            progress: SummaryBootstrapProgress::default(),
            result: None,
            error: None,
            submitted_at_unix: 1,
            started_at_unix: Some(1),
            updated_at_unix: 10,
            completed_at_unix: Some(10),
        };
        let stats = SessionWorkplaneStats {
            summary_jobs: StatusCounts {
                pending: 0,
                running: 0,
                failed: 1,
                completed: 9,
            },
            summary_refresh_jobs: SessionMailboxStats {
                counts: StatusCounts {
                    pending: 0,
                    running: 0,
                    failed: 1,
                    completed: 9,
                },
                latest_error: Some("summary provider timed out".to_string()),
            },
            ..SessionWorkplaneStats::default()
        };

        let lane = derive_summaries_lane(
            &session,
            Some(&initial_sync),
            None,
            Some(&summary_run),
            &stats,
            Some(InitRuntimeLaneProgressView {
                completed: 277,
                total: 278,
                remaining: 1,
            }),
        );

        assert_eq!(lane.status, "warning");
        assert_eq!(lane.warnings.len(), 1);
    }

    #[test]
    fn summary_follow_up_can_start_before_embeddings_bootstrap_finishes() {
        let session = InitSessionRecord {
            init_session_id: "init-session-1".to_string(),
            repo_id: "repo-1".to_string(),
            repo_root: PathBuf::from("/tmp/repo-1"),
            daemon_config_root: PathBuf::from("/tmp/config-1"),
            selections: StartInitSessionSelections {
                run_sync: true,
                run_ingest: false,
                ingest_backfill: None,
                embeddings_bootstrap: Some(InitEmbeddingsBootstrapRequest {
                    config_path: PathBuf::from("/tmp/config-1/config.toml"),
                    profile_name: "local_code".to_string(),
                }),
                summaries_bootstrap: Some(SummaryBootstrapRequest {
                    action: SummaryBootstrapAction::ConfigureCloud,
                    message: None,
                    model_name: None,
                    gateway_url_override: None,
                }),
            },
            initial_sync_task_id: Some("sync-task-1".to_string()),
            ingest_task_id: None,
            embeddings_bootstrap_task_id: Some("bootstrap-task-1".to_string()),
            summary_bootstrap_run_id: Some("summary-run-1".to_string()),
            follow_up_sync_required: true,
            follow_up_sync_task_id: None,
            submitted_at_unix: 1,
            updated_at_unix: 1,
            terminal_status: None,
            terminal_error: None,
        };
        let initial_sync = completed_sync_task("sync-task-1", 10);
        let embeddings_task = DevqlTaskRecord {
            task_id: "bootstrap-task-1".to_string(),
            repo_id: "repo-1".to_string(),
            repo_name: "repo".to_string(),
            repo_provider: "local".to_string(),
            repo_organisation: "local".to_string(),
            repo_identity: "repo".to_string(),
            daemon_config_root: PathBuf::from("/tmp/config-1"),
            repo_root: PathBuf::from("/tmp/repo-1"),
            init_session_id: Some("init-session-1".to_string()),
            kind: DevqlTaskKind::EmbeddingsBootstrap,
            source: DevqlTaskSource::Init,
            spec: crate::daemon::DevqlTaskSpec::EmbeddingsBootstrap(EmbeddingsBootstrapTaskSpec {
                config_path: PathBuf::from("/tmp/config-1/config.toml"),
                profile_name: "local_code".to_string(),
            }),
            status: DevqlTaskStatus::Running,
            submitted_at_unix: 1,
            started_at_unix: Some(1),
            updated_at_unix: 11,
            completed_at_unix: None,
            queue_position: None,
            tasks_ahead: None,
            error: None,
            progress: crate::daemon::DevqlTaskProgress::EmbeddingsBootstrap(
                crate::daemon::EmbeddingsBootstrapProgress::default(),
            ),
            result: None,
        };
        let summary_run = SummaryBootstrapRunRecord {
            run_id: "summary-run-1".to_string(),
            repo_id: "repo-1".to_string(),
            repo_root: PathBuf::from("/tmp/repo-1"),
            init_session_id: "init-session-1".to_string(),
            request: SummaryBootstrapRequest {
                action: SummaryBootstrapAction::ConfigureCloud,
                message: None,
                model_name: None,
                gateway_url_override: None,
            },
            status: SummaryBootstrapStatus::Completed,
            progress: SummaryBootstrapProgress::default(),
            result: None,
            error: None,
            submitted_at_unix: 1,
            started_at_unix: Some(1),
            updated_at_unix: 12,
            completed_at_unix: Some(12),
        };

        assert!(semantic_follow_up_ready_for_sync(
            &session,
            Some(&initial_sync),
            None,
            Some(&embeddings_task),
            Some(&summary_run),
        ));
    }

    #[test]
    fn embeddings_can_trigger_a_second_follow_up_after_summary_follow_up_completes() {
        let session = InitSessionRecord {
            init_session_id: "init-session-1".to_string(),
            repo_id: "repo-1".to_string(),
            repo_root: PathBuf::from("/tmp/repo-1"),
            daemon_config_root: PathBuf::from("/tmp/config-1"),
            selections: StartInitSessionSelections {
                run_sync: true,
                run_ingest: false,
                ingest_backfill: None,
                embeddings_bootstrap: Some(InitEmbeddingsBootstrapRequest {
                    config_path: PathBuf::from("/tmp/config-1/config.toml"),
                    profile_name: "local_code".to_string(),
                }),
                summaries_bootstrap: Some(SummaryBootstrapRequest {
                    action: SummaryBootstrapAction::ConfigureCloud,
                    message: None,
                    model_name: None,
                    gateway_url_override: None,
                }),
            },
            initial_sync_task_id: Some("sync-task-1".to_string()),
            ingest_task_id: None,
            embeddings_bootstrap_task_id: Some("bootstrap-task-1".to_string()),
            summary_bootstrap_run_id: Some("summary-run-1".to_string()),
            follow_up_sync_required: true,
            follow_up_sync_task_id: Some("follow-up-sync-1".to_string()),
            submitted_at_unix: 1,
            updated_at_unix: 1,
            terminal_status: None,
            terminal_error: None,
        };
        let initial_sync = completed_sync_task("sync-task-1", 10);
        let follow_up_sync = completed_sync_task("follow-up-sync-1", 14);
        let embeddings_task = DevqlTaskRecord {
            task_id: "bootstrap-task-1".to_string(),
            repo_id: "repo-1".to_string(),
            repo_name: "repo".to_string(),
            repo_provider: "local".to_string(),
            repo_organisation: "local".to_string(),
            repo_identity: "repo".to_string(),
            daemon_config_root: PathBuf::from("/tmp/config-1"),
            repo_root: PathBuf::from("/tmp/repo-1"),
            init_session_id: Some("init-session-1".to_string()),
            kind: DevqlTaskKind::EmbeddingsBootstrap,
            source: DevqlTaskSource::Init,
            spec: crate::daemon::DevqlTaskSpec::EmbeddingsBootstrap(EmbeddingsBootstrapTaskSpec {
                config_path: PathBuf::from("/tmp/config-1/config.toml"),
                profile_name: "local_code".to_string(),
            }),
            status: DevqlTaskStatus::Completed,
            submitted_at_unix: 1,
            started_at_unix: Some(1),
            updated_at_unix: 20,
            completed_at_unix: Some(20),
            queue_position: None,
            tasks_ahead: None,
            error: None,
            progress: crate::daemon::DevqlTaskProgress::EmbeddingsBootstrap(
                crate::daemon::EmbeddingsBootstrapProgress::default(),
            ),
            result: None,
        };
        let summary_run = SummaryBootstrapRunRecord {
            run_id: "summary-run-1".to_string(),
            repo_id: "repo-1".to_string(),
            repo_root: PathBuf::from("/tmp/repo-1"),
            init_session_id: "init-session-1".to_string(),
            request: SummaryBootstrapRequest {
                action: SummaryBootstrapAction::ConfigureCloud,
                message: None,
                model_name: None,
                gateway_url_override: None,
            },
            status: SummaryBootstrapStatus::Completed,
            progress: SummaryBootstrapProgress::default(),
            result: None,
            error: None,
            submitted_at_unix: 1,
            started_at_unix: Some(1),
            updated_at_unix: 12,
            completed_at_unix: Some(12),
        };

        assert!(semantic_follow_up_ready_for_sync(
            &session,
            Some(&initial_sync),
            Some(&follow_up_sync),
            Some(&embeddings_task),
            Some(&summary_run),
        ));
    }

    #[test]
    fn top_pipeline_lane_reports_failed_sync_task() {
        let session = InitSessionRecord {
            init_session_id: "init-session-1".to_string(),
            repo_id: "repo-1".to_string(),
            repo_root: PathBuf::from("/tmp/repo-1"),
            daemon_config_root: PathBuf::from("/tmp/config-1"),
            selections: StartInitSessionSelections {
                run_sync: true,
                run_ingest: false,
                ingest_backfill: None,
                embeddings_bootstrap: None,
                summaries_bootstrap: None,
            },
            initial_sync_task_id: Some("sync-task-1".to_string()),
            ingest_task_id: None,
            embeddings_bootstrap_task_id: None,
            summary_bootstrap_run_id: None,
            follow_up_sync_required: false,
            follow_up_sync_task_id: None,
            submitted_at_unix: 1,
            updated_at_unix: 1,
            terminal_status: None,
            terminal_error: None,
        };
        let sync_task = DevqlTaskRecord {
            task_id: "sync-task-1".to_string(),
            repo_id: "repo-1".to_string(),
            repo_name: "repo".to_string(),
            repo_provider: "local".to_string(),
            repo_organisation: "local".to_string(),
            repo_identity: "repo".to_string(),
            daemon_config_root: PathBuf::from("/tmp/config-1"),
            repo_root: PathBuf::from("/tmp/repo-1"),
            init_session_id: Some("init-session-1".to_string()),
            kind: DevqlTaskKind::Sync,
            source: DevqlTaskSource::Init,
            spec: crate::daemon::DevqlTaskSpec::Sync(crate::daemon::SyncTaskSpec {
                mode: crate::daemon::SyncTaskMode::Full,
                post_commit_snapshot: None,
            }),
            status: DevqlTaskStatus::Failed,
            submitted_at_unix: 1,
            started_at_unix: Some(1),
            updated_at_unix: 2,
            completed_at_unix: Some(2),
            queue_position: None,
            tasks_ahead: None,
            error: Some("sync failed".to_string()),
            progress: crate::daemon::DevqlTaskProgress::Sync(Default::default()),
            result: None,
        };

        let lane = derive_top_pipeline_lane(
            &session,
            Some(&sync_task),
            None,
            None,
            StatusCounts::default(),
        );

        assert_eq!(lane.status, "failed");
        assert_eq!(lane.waiting_reason.as_deref(), Some("failed"));
        assert_eq!(lane.detail.as_deref(), Some("Syncing repository failed"));
        assert_eq!(lane.task_id.as_deref(), Some("sync-task-1"));
    }

    #[test]
    fn session_status_only_becomes_failed_after_claimed_work_drains() {
        assert_eq!(
            derive_session_status(true, true, false, None, false),
            "failing"
        );
        assert_eq!(
            derive_session_status(true, false, false, None, false),
            "failed"
        );
        assert_eq!(
            derive_session_status(
                false,
                false,
                false,
                Some("waiting_for_current_state_consumer"),
                false,
            ),
            "waiting"
        );
        assert_eq!(
            derive_session_status(false, false, false, None, false),
            "running"
        );
        assert_eq!(
            derive_session_status(false, false, true, None, false),
            "completed"
        );
        assert_eq!(
            derive_session_status(false, false, true, None, true),
            "completed_with_warnings"
        );
    }
}

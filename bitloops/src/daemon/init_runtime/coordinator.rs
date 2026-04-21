use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{Context, Result};
use uuid::Uuid;

use crate::capability_packs::semantic_clones::types::{
    SEMANTIC_CLONES_CAPABILITY_ID, SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
    SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX, SEMANTIC_CLONES_IDENTITY_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX, SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
};
use crate::daemon::types::{
    DevqlTaskRecord, DevqlTaskSpec, DevqlTaskStatus, EmbeddingsBootstrapTaskSpec, IngestTaskSpec,
    InitSessionRecord, InitSessionTerminalStatus, StartInitSessionSelections,
    SummaryBootstrapRunRecord, SyncTaskMode, SyncTaskSpec, unix_timestamp_now,
};
use crate::graphql::SubscriptionHub;
use crate::host::devql::{DevqlConfig, SyncMode};
use crate::host::runtime_store::{DaemonSqliteRuntimeStore, RepoSqliteRuntimeStore};
use crate::runtime_presentation::warning_summary;

use super::lanes::{
    SummaryEmbeddingsLaneContext, derive_code_embeddings_lane, derive_ingest_lane,
    derive_session_status, derive_summaries_lane, derive_summary_embeddings_lane, derive_sync_lane,
    running_task,
};
use super::orchestration::{
    record_task_completion_seq, selected_top_level_terminal,
    semantic_bootstrap_still_outstanding_after_initial_sync, semantic_bootstrap_waiting_reason,
    semantic_bootstraps_ready, semantic_bootstraps_terminal, semantic_follow_up_pending,
    semantic_follow_up_ready_for_sync, session_fatal_failure_detail, session_has_remaining_work,
    session_requires_semantic_follow_up, summary_run_failed, task_failed,
};
use super::progress::load_runtime_lane_progress;
use super::session_stats::load_session_workplane_stats;
use super::stats::{RuntimeLaneProgressState, SummaryInMemoryBatchProgress};
use super::tasks::{
    load_summary_task_by_id, load_task_by_id, summary_run_from_task, summary_run_from_task_ref,
};
use super::types::{
    InitRuntimeSessionView, InitRuntimeSnapshot, InitSessionHandle, RuntimeEventRecord,
};
use super::workplane::{repo_blocked_mailboxes, workplane_snapshot_from_mailboxes};

#[derive(Debug)]
pub struct InitRuntimeCoordinator {
    pub(crate) runtime_store: DaemonSqliteRuntimeStore,
    pub(crate) subscription_hub: Mutex<Option<Arc<SubscriptionHub>>>,
    pub(crate) summary_in_memory_batches: Mutex<BTreeMap<String, SummaryInMemoryBatchProgress>>,
}

impl InitRuntimeCoordinator {
    pub(crate) fn shared() -> Arc<Self> {
        static INSTANCE: OnceLock<Arc<InitRuntimeCoordinator>> = OnceLock::new();
        Arc::clone(INSTANCE.get_or_init(|| {
            Arc::new(Self {
                runtime_store: DaemonSqliteRuntimeStore::open()
                    .expect("opening daemon runtime store for init runtime orchestration"),
                subscription_hub: Mutex::new(None),
                summary_in_memory_batches: Mutex::new(BTreeMap::new()),
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
            summary_bootstrap_task_id: None,
            follow_up_sync_required: false,
            follow_up_sync_task_id: None,
            next_completion_seq: 0,
            initial_sync_completion_seq: None,
            embeddings_bootstrap_completion_seq: None,
            summary_bootstrap_completion_seq: None,
            follow_up_sync_completion_seq: None,
            submitted_at_unix: now,
            updated_at_unix: now,
            terminal_status: None,
            terminal_error: None,
        };

        if selections.run_sync {
            let queued = crate::daemon::shared_devql_task_coordinator().enqueue_with_init_session(
                cfg,
                crate::daemon::DevqlTaskSource::Init,
                DevqlTaskSpec::Sync(SyncTaskSpec {
                    mode: SyncTaskMode::Auto,
                    post_commit_snapshot: None,
                }),
                Some(init_session_id.clone()),
            )?;
            session.initial_sync_task_id = Some(queued.task.task_id);
        } else if selections.run_ingest {
            let queued = crate::daemon::shared_devql_task_coordinator().enqueue_with_init_session(
                cfg,
                crate::daemon::DevqlTaskSource::Init,
                DevqlTaskSpec::Ingest(IngestTaskSpec {
                    backfill: selections.ingest_backfill,
                }),
                Some(init_session_id.clone()),
            )?;
            session.ingest_task_id = Some(queued.task.task_id);
        }

        if let Some(request) = selections.embeddings_bootstrap.clone() {
            let queued = crate::daemon::shared_devql_task_coordinator().enqueue_with_init_session(
                cfg,
                crate::daemon::DevqlTaskSource::Init,
                DevqlTaskSpec::EmbeddingsBootstrap(EmbeddingsBootstrapTaskSpec {
                    config_path: request.config_path,
                    profile_name: request.profile_name,
                    mode: request.mode,
                    gateway_url_override: request.gateway_url_override,
                    api_key_env: request.api_key_env,
                }),
                Some(init_session_id.clone()),
            )?;
            session.embeddings_bootstrap_task_id = Some(queued.task.task_id);
        }

        if let Some(request) = selections.summaries_bootstrap.clone() {
            let queued = crate::daemon::shared_devql_task_coordinator().enqueue_with_init_session(
                cfg,
                crate::daemon::DevqlTaskSource::Init,
                DevqlTaskSpec::SummaryBootstrap(request),
                Some(init_session_id.clone()),
            )?;
            session.summary_bootstrap_task_id = Some(queued.task.task_id);
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
            run_id: None,
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
            record_task_completion_seq(session, &task);
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
                if !session.follow_up_sync_required
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
            let queued = crate::daemon::shared_devql_task_coordinator().enqueue_with_init_session(
                &cfg,
                crate::daemon::DevqlTaskSource::Init,
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
        let task_queue = crate::daemon::shared_devql_task_coordinator().snapshot(Some(&repo_id))?;
        let current_state_consumer =
            crate::daemon::shared_capability_event_coordinator().snapshot(Some(&repo_id))?;
        let mailboxes = repo_store.load_capability_workplane_mailbox_status(
            SEMANTIC_CLONES_CAPABILITY_ID,
            [
                SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
                SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
                SEMANTIC_CLONES_IDENTITY_EMBEDDING_MAILBOX,
                SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
                SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
            ],
        )?;
        let blocked_mailboxes =
            repo_blocked_mailboxes(repo_store.db_path().to_path_buf(), repo_store.repo_id())?;
        let workplane =
            workplane_snapshot_from_mailboxes(&cfg.repo_root, &mailboxes, &blocked_mailboxes);
        let embeddings_readiness_gate =
            crate::daemon::embeddings_bootstrap::gate_status_for_enrichment_queue(
                &self.runtime_store,
                vec![cfg.daemon_config_root.clone()],
            )?;
        let summaries_bootstrap = self.current_summary_bootstrap_run(&repo_id)?;
        let current_session = self.current_session_view(cfg, &repo_store)?;

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
        Ok(crate::daemon::shared_devql_task_coordinator()
            .tasks(
                Some(repo_id),
                Some(crate::daemon::DevqlTaskKind::SummaryBootstrap),
                None,
                Some(1),
            )?
            .into_iter()
            .find_map(summary_run_from_task))
    }

    fn current_session_view(
        &self,
        cfg: &DevqlConfig,
        repo_store: &RepoSqliteRuntimeStore,
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
        let summary_task = load_summary_task_by_id(session.summary_bootstrap_task_id.as_deref())?;
        let summary_run = summary_task.as_ref().and_then(summary_run_from_task_ref);
        let initial_sync = load_task_by_id(session.initial_sync_task_id.as_deref())?;
        let ingest_task = load_task_by_id(session.ingest_task_id.as_deref())?;
        let follow_up_sync = load_task_by_id(session.follow_up_sync_task_id.as_deref())?;
        let embeddings_task = load_task_by_id(session.embeddings_bootstrap_task_id.as_deref())?;
        let summary_in_memory_completed =
            self.summary_in_memory_completed(&cfg.repo.repo_id, &session.init_session_id);
        let lane_progress = match load_runtime_lane_progress(
            &cfg.repo_root,
            &cfg.repo.repo_id,
            &session,
            &stats,
            summary_in_memory_completed,
        ) {
            Ok(progress) => progress,
            Err(err) => {
                log::debug!(
                    "failed to load runtime lane progress for repo `{}`: {err:#}",
                    cfg.repo.repo_id
                );
                RuntimeLaneProgressState::default()
            }
        };

        let sync_lane = derive_sync_lane(
            &session,
            initial_sync.as_ref(),
            follow_up_sync.as_ref(),
            stats.current_state,
        );
        let ingest_lane = derive_ingest_lane(&session, initial_sync.as_ref(), ingest_task.as_ref());
        let code_embeddings_lane = derive_code_embeddings_lane(
            &session,
            initial_sync.as_ref(),
            follow_up_sync.as_ref(),
            embeddings_task.as_ref(),
            stats.current_state,
            &stats,
            lane_progress.code_embeddings.clone(),
        );
        let summaries_lane = derive_summaries_lane(
            &session,
            initial_sync.as_ref(),
            follow_up_sync.as_ref(),
            summary_run.as_ref(),
            stats.current_state,
            &stats,
            lane_progress.summaries.clone(),
        );
        let summary_embeddings_lane = derive_summary_embeddings_lane(
            &session,
            &stats,
            SummaryEmbeddingsLaneContext {
                initial_sync: initial_sync.as_ref(),
                follow_up_sync: follow_up_sync.as_ref(),
                embeddings_task: embeddings_task.as_ref(),
                summary_run: summary_run.as_ref(),
                current_state: stats.current_state,
                progress: lane_progress.summary_embeddings.clone(),
                summaries_progress: lane_progress.summaries.clone(),
            },
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
        let blocked_embedding = stats
            .blocked_code_embedding_reason
            .clone()
            .or(stats.blocked_summary_embedding_reason.clone());
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
            embeddings_selected: session.selections.run_code_embeddings,
            summaries_selected: session.selections.run_summaries,
            summary_embeddings_selected: session.selections.run_summary_embeddings,
            initial_sync_task_id: session.initial_sync_task_id,
            ingest_task_id: session.ingest_task_id,
            follow_up_sync_task_id: session.follow_up_sync_task_id,
            embeddings_bootstrap_task_id: session.embeddings_bootstrap_task_id,
            summary_bootstrap_task_id: session.summary_bootstrap_task_id,
            terminal_error: if status == "failed" {
                fatal_failure_detail.or(session.terminal_error)
            } else {
                None
            },
            sync_lane,
            ingest_lane,
            code_embeddings_lane,
            summaries_lane,
            summary_embeddings_lane,
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
        let summary_task = load_summary_task_by_id(session.summary_bootstrap_task_id.as_deref())?;
        let summary_run = summary_task.as_ref().and_then(summary_run_from_task_ref);
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
        let queued = crate::daemon::shared_devql_task_coordinator().enqueue_with_init_session(
            &cfg,
            crate::daemon::DevqlTaskSource::Init,
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
        let summary_task = load_summary_task_by_id(session.summary_bootstrap_task_id.as_deref())?;
        let summary_run = summary_task.as_ref().and_then(summary_run_from_task_ref);
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

    pub(crate) fn record_summary_in_memory_artefact(
        &self,
        repo_id: &str,
        lease_token: &str,
        artefact_id: &str,
        init_session_ids: &BTreeSet<String>,
    ) {
        if init_session_ids.is_empty() {
            return;
        }

        let mut updated_sessions = Vec::new();
        if let Ok(mut batches) = self.summary_in_memory_batches.lock() {
            let batch = batches.entry(lease_token.to_string()).or_insert_with(|| {
                SummaryInMemoryBatchProgress {
                    repo_id: repo_id.to_string(),
                    artefact_ids_by_session: BTreeMap::new(),
                }
            });
            for init_session_id in init_session_ids {
                let artefact_ids = batch
                    .artefact_ids_by_session
                    .entry(init_session_id.clone())
                    .or_default();
                if artefact_ids.insert(artefact_id.to_string()) {
                    updated_sessions.push(init_session_id.clone());
                }
            }
        }

        self.publish_summary_progress_events(repo_id, updated_sessions);
    }

    pub(crate) fn clear_summary_in_memory_batch(&self, lease_token: &str) {
        let mut cleared = None;
        if let Ok(mut batches) = self.summary_in_memory_batches.lock() {
            cleared = batches.remove(lease_token);
        }

        let Some(batch) = cleared else {
            return;
        };
        self.publish_summary_progress_events(
            &batch.repo_id,
            batch
                .artefact_ids_by_session
                .into_keys()
                .collect::<Vec<_>>(),
        );
    }

    pub(crate) fn summary_in_memory_completed(&self, repo_id: &str, init_session_id: &str) -> u64 {
        let Ok(batches) = self.summary_in_memory_batches.lock() else {
            return 0;
        };

        let mut artefact_ids = BTreeSet::new();
        for batch in batches.values() {
            if batch.repo_id != repo_id {
                continue;
            }
            if let Some(batch_artefacts) = batch.artefact_ids_by_session.get(init_session_id) {
                artefact_ids.extend(batch_artefacts.iter().cloned());
            }
        }
        artefact_ids.len() as u64
    }

    fn publish_summary_progress_events<I>(&self, repo_id: &str, init_session_ids: I)
    where
        I: IntoIterator<Item = String>,
    {
        let updated_at_unix = unix_timestamp_now();
        for init_session_id in init_session_ids.into_iter().collect::<BTreeSet<_>>() {
            self.publish_event(RuntimeEventRecord {
                domain: "workplane".to_string(),
                repo_id: repo_id.to_string(),
                init_session_id: Some(init_session_id),
                updated_at_unix,
                task_id: None,
                run_id: None,
                mailbox_name: Some(SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX.to_string()),
            });
        }
    }
}

use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use anyhow::{Context, Result, anyhow};
use tokio::sync::{mpsc, oneshot};
use tokio::time::{Duration, sleep};

use crate::config::resolve_store_backend_config_for_repo;
use crate::graphql::SubscriptionHub;
use crate::host::devql::{
    DevqlConfig, RelationalStorage, RepoIdentity, SyncProgressPhase, SyncProgressUpdate,
};

use super::super::super::types::{
    DevqlTaskKind, DevqlTaskProgress, DevqlTaskRecord, DevqlTaskSource, DevqlTaskSpec,
    DevqlTaskStatus, SyncTaskMode, SyncTaskSpec,
};
use super::super::queue::{
    next_runnable_task_indexes, sync_task_mode_from_host as queue_sync_task_mode_from_host,
    sync_task_mode_to_host as queue_sync_task_mode_to_host,
};
use super::DevqlTaskCoordinator;
use super::helpers::{enqueue_sync_completed_runs, receive_embeddings_bootstrap_outcome};
use super::observers::{IngestCoordinatorObserver, ProgressPersistState, SyncCoordinatorObserver};

const WORKER_POLL_INTERVAL: Duration = Duration::from_secs(2);

struct WorkerStartedGuard {
    coordinator: Arc<DevqlTaskCoordinator>,
}

impl Drop for WorkerStartedGuard {
    fn drop(&mut self) {
        self.coordinator
            .worker_started
            .store(false, Ordering::SeqCst);
    }
}

impl DevqlTaskCoordinator {
    pub(crate) fn activate_worker(
        self: &Arc<Self>,
        config_root: &Path,
        repo_registry_path: Option<&Path>,
        hub: Option<Arc<SubscriptionHub>>,
    ) {
        if let Some(hub) = hub {
            self.register_subscription_hub(hub);
        }
        if self.worker_started.swap(true, Ordering::SeqCst) {
            return;
        }
        if let Err(err) = self.recover_running_tasks() {
            log::warn!("failed to recover queued DevQL tasks: {err:#}");
        }
        if let Err(err) = crate::host::devql::recover_running_producer_spool_jobs(config_root) {
            log::warn!("failed to recover DevQL producer spool jobs: {err:#}");
        }
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            self.worker_started.store(false, Ordering::SeqCst);
            log::warn!("DevQL task worker activation requested without an active tokio runtime");
            return;
        };
        let coordinator = Arc::clone(self);
        let producer_spool_config_root = config_root.to_path_buf();
        let producer_spool_repo_registry_path = repo_registry_path.map(Path::to_path_buf);
        handle.spawn(async move {
            let _guard = WorkerStartedGuard {
                coordinator: Arc::clone(&coordinator),
            };
            coordinator
                .run_loop(
                    producer_spool_config_root,
                    producer_spool_repo_registry_path,
                )
                .await;
        });
    }

    pub(crate) fn register_subscription_hub(&self, hub: Arc<SubscriptionHub>) {
        if let Ok(mut slot) = self.subscription_hub.lock() {
            *slot = Some(hub);
        }
    }

    async fn run_loop(
        self: Arc<Self>,
        producer_spool_config_root: std::path::PathBuf,
        repo_registry_path: Option<PathBuf>,
    ) {
        loop {
            let mut made_progress = false;
            let reconcile_blocked = match self
                .ensure_scope_exclusion_reconciles(
                    &producer_spool_config_root,
                    repo_registry_path.as_deref(),
                )
                .await
            {
                Ok(blocked) => blocked,
                Err(err) => {
                    log::warn!("daemon DevQL exclusion reconcile error: {err:#}");
                    false
                }
            };

            if !reconcile_blocked {
                match self.schedule_pending_producer_spool_jobs(&producer_spool_config_root) {
                    Ok(progressed) => made_progress |= progressed,
                    Err(err) => log::warn!("daemon DevQL producer spool worker error: {err:#}"),
                }
            }
            match self.schedule_pending_tasks() {
                Ok(progressed) => made_progress |= progressed,
                Err(err) => log::warn!("daemon DevQL task worker error: {err:#}"),
            }

            if made_progress {
                continue;
            }
            tokio::select! {
                _ = self.notify.notified() => {},
                _ = sleep(WORKER_POLL_INTERVAL) => {},
            }
        }
    }

    fn schedule_pending_producer_spool_jobs(self: &Arc<Self>, config_root: &Path) -> Result<bool> {
        let jobs = crate::host::devql::claim_next_producer_spool_jobs(config_root)?;
        if jobs.is_empty() {
            return Ok(false);
        }

        for job in jobs {
            let coordinator = Arc::clone(self);
            tokio::spawn(async move {
                if let Err(err) = coordinator.run_producer_spool_job(job).await {
                    log::warn!("DevQL producer spool execution failed: {err:#}");
                }
            });
        }

        Ok(true)
    }

    async fn ensure_scope_exclusion_reconciles(
        self: &Arc<Self>,
        config_root: &Path,
        repo_registry_path: Option<&Path>,
    ) -> Result<bool> {
        let mut blocked = false;
        for repo_root in
            self.scope_exclusion_reconcile_repo_roots(config_root, repo_registry_path)?
        {
            match self
                .ensure_scope_exclusion_reconcile_for_repo(&repo_root)
                .await
            {
                Ok(repo_blocked) => blocked |= repo_blocked,
                Err(err) => log::warn!(
                    "daemon DevQL exclusion reconcile error for {}: {err:#}",
                    repo_root.display()
                ),
            }
        }
        Ok(blocked)
    }

    fn scope_exclusion_reconcile_repo_roots(
        &self,
        config_root: &Path,
        repo_registry_path: Option<&Path>,
    ) -> Result<Vec<PathBuf>> {
        let current_repo_root = config_root
            .canonicalize()
            .unwrap_or_else(|_| config_root.to_path_buf());
        let current_daemon_config_root =
            crate::config::resolve_bound_daemon_config_root_for_repo(&current_repo_root)
                .unwrap_or_else(|_| current_repo_root.clone());
        let mut repo_roots = vec![current_repo_root.clone()];
        let mut seen = HashSet::from([current_repo_root]);

        let Some(repo_registry_path) = repo_registry_path else {
            return Ok(repo_roots);
        };
        let registry = crate::devql_transport::load_repo_path_registry(repo_registry_path)?;
        for entry in registry.entries {
            let repo_root = entry
                .repo_root
                .canonicalize()
                .unwrap_or(entry.repo_root.clone());
            let Ok(bound_daemon_config_root) =
                crate::config::resolve_bound_daemon_config_root_for_repo(&repo_root)
            else {
                continue;
            };
            if bound_daemon_config_root != current_daemon_config_root {
                continue;
            }
            if seen.insert(repo_root.clone()) {
                repo_roots.push(repo_root);
            }
        }

        Ok(repo_roots)
    }

    async fn ensure_scope_exclusion_reconcile_for_repo(
        self: &Arc<Self>,
        repo_root: &Path,
    ) -> Result<bool> {
        let repo = crate::host::devql::resolve_repo_identity(repo_root)
            .context("resolving repo identity for exclusion reconciliation")?;
        let cfg = DevqlConfig::from_env(repo_root.to_path_buf(), repo)
            .context("building DevQL config for exclusion reconciliation")?;
        let backends = resolve_store_backend_config_for_repo(&cfg.daemon_config_root)
            .context("resolving backend config for exclusion reconciliation")?;
        let relational = RelationalStorage::connect(
            &cfg,
            &backends.relational,
            "daemon exclusion reconciliation",
        )
        .await?;
        let blocking = self.has_blocking_scope_exclusion_reconcile(&cfg.repo.repo_id)?;
        let needs_reconcile =
            crate::host::devql::scope_exclusion_reconcile_needed(&cfg, &relational)
                .await?
                .is_some();
        if blocking || needs_reconcile {
            self.prune_excluded_path_sync_tasks_for_repo(&cfg)?;
        }
        if blocking {
            return Ok(true);
        }
        if needs_reconcile {
            self.enqueue(
                &cfg,
                DevqlTaskSource::RepoPolicyChange,
                DevqlTaskSpec::Sync(SyncTaskSpec {
                    mode: SyncTaskMode::Full,
                }),
            )?;
            return Ok(true);
        }
        Ok(blocking)
    }

    fn schedule_pending_tasks(self: &Arc<Self>) -> Result<bool> {
        let tasks = self.mutate_state(|state| {
            let indexes = next_runnable_task_indexes(state);
            if indexes.is_empty() {
                return Ok(Vec::new());
            }

            let now = super::super::super::types::unix_timestamp_now();
            let mut scheduled = Vec::with_capacity(indexes.len());
            for index in indexes {
                let mut task = state.tasks[index].clone();
                task.status = DevqlTaskStatus::Running;
                task.started_at_unix = Some(task.started_at_unix.unwrap_or(now));
                task.updated_at_unix = now;
                task.error = None;
                task.completed_at_unix = None;
                task.result = None;
                state.tasks[index] = task.clone();
                scheduled.push(task);
            }
            state.last_action = Some("running".to_string());
            Ok(scheduled)
        })?;

        if tasks.is_empty() {
            return Ok(false);
        }

        for task in tasks {
            let coordinator = Arc::clone(self);
            tokio::spawn(async move {
                if let Err(err) = coordinator.run_task(task).await {
                    log::warn!("DevQL task execution failed: {err:#}");
                }
            });
        }

        Ok(true)
    }

    async fn run_task(self: Arc<Self>, task: DevqlTaskRecord) -> Result<()> {
        match task.kind {
            DevqlTaskKind::Sync => self.run_sync_task(task).await,
            DevqlTaskKind::Ingest => self.run_ingest_task(task).await,
            DevqlTaskKind::EmbeddingsBootstrap => self.run_embeddings_bootstrap_task(task).await,
        }
    }

    pub(super) async fn run_producer_spool_job(
        self: Arc<Self>,
        job: crate::host::devql::ProducerSpoolJobRecord,
    ) -> Result<()> {
        let outcome = self.process_producer_spool_job(&job).await;
        match outcome {
            Ok(()) => {
                if let Err(err) =
                    crate::host::devql::delete_producer_spool_job(&job.config_root, &job.job_id)
                {
                    log::warn!(
                        "failed to delete completed DevQL producer spool job `{}`: {err:#}",
                        job.job_id
                    );
                    if let Err(requeue_err) = crate::host::devql::requeue_producer_spool_job(
                        &job.config_root,
                        &job.job_id,
                        &err,
                    ) {
                        log::warn!(
                            "failed to requeue DevQL producer spool job `{}` after delete failure: {requeue_err:#}",
                            job.job_id
                        );
                    }
                }
            }
            Err(err) => {
                log::warn!("DevQL producer spool job `{}` failed: {err:#}", job.job_id);
                if let Err(requeue_err) = crate::host::devql::requeue_producer_spool_job(
                    &job.config_root,
                    &job.job_id,
                    &err,
                ) {
                    log::warn!(
                        "failed to requeue DevQL producer spool job `{}`: {requeue_err:#}",
                        job.job_id
                    );
                }
            }
        }

        self.notify.notify_waiters();
        Ok(())
    }

    async fn process_producer_spool_job(
        &self,
        job: &crate::host::devql::ProducerSpoolJobRecord,
    ) -> Result<()> {
        match &job.payload {
            crate::host::devql::ProducerSpoolJobPayload::Task { source, spec } => {
                let cfg = self.devql_config_from_producer_spool_job(job)?;
                self.enqueue(&cfg, *source, spec.clone())?;
                Ok(())
            }
            crate::host::devql::ProducerSpoolJobPayload::PostCommitRefresh {
                commit_sha,
                changed_files,
            } => {
                let cfg = self.devql_config_from_producer_spool_job(job)?;
                crate::host::checkpoints::strategy::manual_commit::execute_devql_post_commit_refresh(
                    &cfg,
                    commit_sha,
                    changed_files,
                )
                .await
            }
            crate::host::devql::ProducerSpoolJobPayload::PostMergeRefresh {
                head_sha,
                changed_files,
            } => {
                let cfg = self.devql_config_from_producer_spool_job(job)?;
                crate::host::checkpoints::strategy::manual_commit::execute_devql_post_merge_refresh(
                    &cfg,
                    head_sha,
                    changed_files,
                )
                .await
            }
            crate::host::devql::ProducerSpoolJobPayload::PrePushSync {
                remote,
                stdin_lines,
            } => {
                crate::host::checkpoints::strategy::manual_commit::execute_devql_pre_push_sync(
                    &job.repo_root,
                    remote,
                    stdin_lines,
                )
                .await
            }
        }
    }

    fn devql_config_from_producer_spool_job(
        &self,
        job: &crate::host::devql::ProducerSpoolJobRecord,
    ) -> Result<DevqlConfig> {
        let repo = RepoIdentity {
            repo_id: job.repo_id.clone(),
            name: job.repo_name.clone(),
            provider: job.repo_provider.clone(),
            organization: job.repo_organisation.clone(),
            identity: job.repo_identity.clone(),
        };
        DevqlConfig::from_roots(job.config_root.clone(), job.repo_root.clone(), repo)
    }

    async fn run_sync_task(self: Arc<Self>, task: DevqlTaskRecord) -> Result<()> {
        self.update_task_progress(
            &task.task_id,
            DevqlTaskProgress::Sync(SyncProgressUpdate {
                phase: SyncProgressPhase::EnsuringSchema,
                ..task
                    .sync_progress()
                    .cloned()
                    .unwrap_or_else(SyncProgressUpdate::default)
            }),
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
        let requested_mode = queue_sync_task_mode_to_host(
            &task
                .sync_spec()
                .map(|spec| &spec.mode)
                .cloned()
                .ok_or_else(|| anyhow!("sync task missing sync spec"))?,
        );

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
                return Ok(());
            }
        };
        let effective_mode = crate::host::devql::effective_sync_mode_after_schema_preparation(
            requested_mode,
            schema_outcome,
        );
        let effective_spec = queue_sync_task_mode_from_host(&effective_mode);
        if task
            .sync_spec()
            .is_none_or(|spec| spec.mode != effective_spec)
        {
            self.update_sync_mode(&task.task_id, effective_spec)?;
        }
        let reconcile_relational = if task.source == DevqlTaskSource::RepoPolicyChange {
            let backends = match resolve_store_backend_config_for_repo(&cfg.daemon_config_root)
                .context("resolving backend config for queued exclusion reconciliation")
            {
                Ok(backends) => backends,
                Err(err) => {
                    self.finish_task_failed(&task.task_id, err)?;
                    return Ok(());
                }
            };
            match RelationalStorage::connect(
                &cfg,
                &backends.relational,
                "queued DevQL exclusion reconciliation",
            )
            .await
            {
                Ok(relational) => Some(relational),
                Err(err) => {
                    self.finish_task_failed(&task.task_id, err)?;
                    return Ok(());
                }
            }
        } else {
            None
        };
        let reconcile_fingerprint = if let Some(relational) = reconcile_relational.as_ref() {
            if let Err(err) = crate::daemon::shared_current_state_consumer_coordinator()
                .clear_queued_runs_for_repo(&task.repo_id)
            {
                self.finish_task_failed(&task.task_id, err)?;
                return Ok(());
            }
            let enrichment = crate::daemon::shared_enrichment_coordinator();
            let fingerprint =
                match crate::host::devql::purge_scope_excluded_repo_data(&cfg, relational).await {
                    Ok(fingerprint) => fingerprint,
                    Err(err) => {
                        self.finish_task_failed(&task.task_id, err)?;
                        return Ok(());
                    }
                };
            if let Err(err) = enrichment
                .prune_pending_single_artefact_jobs_after_reconcile(&task.repo_id, relational)
                .await
            {
                self.finish_task_failed(&task.task_id, err)?;
                return Ok(());
            }
            Some(fingerprint)
        } else {
            None
        };

        let observer = SyncCoordinatorObserver {
            coordinator: Arc::clone(&self),
            task_id: task.task_id.clone(),
            progress_state: std::sync::Mutex::new(ProgressPersistState::default()),
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
                if let (Some(relational), Some(fingerprint)) = (
                    reconcile_relational.as_ref(),
                    reconcile_fingerprint.as_ref(),
                ) && let Err(err) =
                    crate::host::devql::sync::state::write_scope_exclusions_fingerprint(
                        relational,
                        &cfg.repo.repo_id,
                        fingerprint,
                    )
                    .await
                {
                    self.finish_task_failed(&task.task_id, err)?;
                    return Ok(());
                }
                if let Some(host) = host.as_ref() {
                    let capability_event_coordinator =
                        crate::daemon::shared_capability_event_coordinator();
                    capability_event_coordinator.activate_worker();
                    if let Err(err) = enqueue_sync_completed_runs(
                        capability_event_coordinator.as_ref(),
                        host,
                        &cfg,
                        &task.task_id,
                        &summary,
                        file_diff,
                        artefact_diff,
                    ) {
                        log::warn!(
                            "failed to enqueue sync current-state consumer runs (task_id={}): {err:#}",
                            task.task_id
                        );
                    }
                }
                self.finish_sync_task_completed(&task.task_id, summary)?
            }
            Err(err) => self.finish_task_failed(&task.task_id, err)?,
        }

        Ok(())
    }

    async fn run_ingest_task(self: Arc<Self>, task: DevqlTaskRecord) -> Result<()> {
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
        let observer = IngestCoordinatorObserver {
            coordinator: Arc::clone(&self),
            task_id: task.task_id.clone(),
            repo_name: task.repo_name.clone(),
            progress_state: std::sync::Mutex::new(ProgressPersistState::default()),
        };
        let backfill = task.ingest_spec().and_then(|spec| spec.backfill);

        let result = match backfill {
            Some(backfill) => {
                crate::host::devql::execute_ingest_with_backfill_window(
                    &cfg,
                    false,
                    backfill,
                    Some(&observer),
                    Some(crate::daemon::shared_enrichment_coordinator()),
                )
                .await
            }
            None => {
                crate::host::devql::execute_ingest_with_observer(
                    &cfg,
                    false,
                    0,
                    Some(&observer),
                    Some(crate::daemon::shared_enrichment_coordinator()),
                )
                .await
            }
        };

        match result {
            Ok(summary) => self.finish_ingest_task_completed(&task.task_id, summary)?,
            Err(err) => self.finish_task_failed(&task.task_id, err)?,
        }
        Ok(())
    }

    async fn run_embeddings_bootstrap_task(self: Arc<Self>, task: DevqlTaskRecord) -> Result<()> {
        let spec = task
            .embeddings_bootstrap_spec()
            .cloned()
            .ok_or_else(|| anyhow!("embeddings bootstrap task missing spec"))?;
        let task_id = task.task_id.clone();
        let runtime_store = self.runtime_store.clone();
        let repo_root = task.repo_root.clone();
        let (progress_tx, progress_rx) = mpsc::unbounded_channel();
        let execution = tokio::task::spawn_blocking(move || {
            crate::daemon::embeddings_bootstrap::execute_task_with_progress(
                &runtime_store,
                &repo_root,
                &task_id,
                &spec,
                |progress| {
                    progress_tx
                        .send(progress)
                        .map_err(|_| anyhow!("embeddings bootstrap progress receiver dropped"))?;
                    Ok(())
                },
            )
        });
        let (result_tx, result_rx) = oneshot::channel();
        tokio::spawn(async move {
            let result = execution
                .await
                .map_err(|err| anyhow!("embeddings bootstrap worker join failed: {err:#}"))
                .and_then(|result| result);
            let _ = result_tx.send(result);
        });

        let final_result =
            receive_embeddings_bootstrap_outcome(progress_rx, result_rx, |progress| {
                self.update_task_progress(
                    &task.task_id,
                    DevqlTaskProgress::EmbeddingsBootstrap(progress),
                )
            })
            .await?;

        match final_result {
            Ok(result) => self.finish_embeddings_bootstrap_task_completed(&task.task_id, result)?,
            Err(err) => self.finish_task_failed(&task.task_id, err)?,
        }

        Ok(())
    }
}

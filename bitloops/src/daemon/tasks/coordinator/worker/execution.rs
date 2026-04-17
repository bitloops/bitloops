use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result, anyhow};
use tokio::sync::{mpsc, oneshot};

use crate::config::resolve_store_backend_config_for_repo;
use crate::daemon::tasks::queue::{
    sync_task_mode_from_host as queue_sync_task_mode_from_host,
    sync_task_mode_to_host as queue_sync_task_mode_to_host,
};
use crate::daemon::types::{DevqlTaskProgress, DevqlTaskRecord, DevqlTaskSource};
use crate::host::devql::{
    DevqlConfig, RelationalStorage, RepoIdentity, SyncProgressPhase, SyncProgressUpdate,
};

use super::super::DevqlTaskCoordinator;
use super::super::helpers::{enqueue_sync_completed_runs, receive_embeddings_bootstrap_outcome};
use super::super::observers::{
    IngestCoordinatorObserver, ProgressPersistState, SyncCoordinatorObserver,
};
impl DevqlTaskCoordinator {
    pub(super) async fn run_sync_task(self: Arc<Self>, task: DevqlTaskRecord) -> Result<()> {
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

        let cfg = DevqlConfig::from_roots(
            task.daemon_config_root.clone(),
            task.repo_root.clone(),
            repo_identity_from_task(&task),
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
            self.update_sync_mode(&task.task_id, effective_spec.clone())?;
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

        match crate::host::devql::run_sync_with_summary_and_stats_and_observer_and_diffs(
            &cfg,
            effective_mode,
            Some(&observer),
            reconcile_fingerprint.as_deref(),
        )
        .await
        {
            Ok((summary, mut stats, file_diff, artefact_diff)) => {
                if let Some(snapshot) = task
                    .sync_spec()
                    .and_then(|spec| spec.post_commit_snapshot.as_ref())
                    && let Err(err) =
                        crate::host::devql::snapshot_committed_current_rows_for_commit_for_config(
                            &cfg, snapshot,
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
                    let enqueue_started = Instant::now();
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
                    stats.capability_event_enqueue_total = enqueue_started.elapsed();
                }
                stats.log(&cfg.repo.repo_id, &summary.mode);
                self.finish_sync_task_completed(&task.task_id, summary)?
            }
            Err(err) => self.finish_task_failed(&task.task_id, err)?,
        }

        Ok(())
    }

    pub(super) async fn run_ingest_task(self: Arc<Self>, task: DevqlTaskRecord) -> Result<()> {
        let cfg = DevqlConfig::from_roots(
            task.daemon_config_root.clone(),
            task.repo_root.clone(),
            repo_identity_from_task(&task),
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

    pub(super) async fn run_embeddings_bootstrap_task(
        self: Arc<Self>,
        task: DevqlTaskRecord,
    ) -> Result<()> {
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

fn repo_identity_from_task(task: &DevqlTaskRecord) -> RepoIdentity {
    RepoIdentity {
        repo_id: task.repo_id.clone(),
        name: task.repo_name.clone(),
        provider: task.repo_provider.clone(),
        organization: task.repo_organisation.clone(),
        identity: task.repo_identity.clone(),
    }
}

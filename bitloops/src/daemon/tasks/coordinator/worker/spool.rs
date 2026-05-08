use std::sync::Arc;

use anyhow::{Context, Result};

use crate::host::devql::{DevqlConfig, RepoIdentity};

use super::super::DevqlTaskCoordinator;

impl DevqlTaskCoordinator {
    pub(in super::super) async fn run_producer_spool_job(
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
                match spec {
                    crate::daemon::DevqlTaskSpec::Sync(_)
                        if !crate::config::settings::devql_sync_enabled(&job.repo_root).context(
                            "loading DevQL sync producer policy for spooled sync task",
                        )? =>
                    {
                        return Ok(());
                    }
                    crate::daemon::DevqlTaskSpec::Ingest(_)
                        if !crate::config::settings::devql_ingest_enabled(&job.repo_root)
                            .context(
                                "loading DevQL ingest producer policy for spooled ingest task",
                            )? =>
                    {
                        return Ok(());
                    }
                    _ => {}
                }
                let cfg = self.devql_config_from_producer_spool_job(job)?;
                self.enqueue(&cfg, *source, spec.clone())?;
                Ok(())
            }
            crate::host::devql::ProducerSpoolJobPayload::PostCommitRefresh {
                commit_sha,
                changed_files,
            } => {
                if !crate::config::settings::devql_sync_enabled(&job.repo_root)
                    .context("loading DevQL sync producer policy for post-commit refresh")?
                {
                    return Ok(());
                }
                let cfg = self.devql_config_from_producer_spool_job(job)?;
                crate::host::checkpoints::strategy::manual_commit::execute_devql_post_commit_refresh(
                    &cfg,
                    commit_sha,
                    changed_files,
                )
                .await
            }
            crate::host::devql::ProducerSpoolJobPayload::PostCommitDerivation {
                commit_sha,
                committed_files,
                is_rebase_in_progress,
            } => {
                crate::host::checkpoints::strategy::manual_commit::execute_devql_post_commit_derivation(
                    &job.repo_root,
                    commit_sha,
                    committed_files,
                    *is_rebase_in_progress,
                )
            }
            crate::host::devql::ProducerSpoolJobPayload::PostMergeRefresh {
                changed_files,
                ..
            } => {
                let cfg = self.devql_config_from_producer_spool_job(job)?;
                let ingest_enabled = crate::config::settings::devql_ingest_enabled(&job.repo_root)
                    .context("loading DevQL ingest producer policy for post-merge refresh")?;
                if ingest_enabled {
                    let backfill =
                        crate::host::checkpoints::strategy::manual_commit::default_post_merge_history_backfill();
                    self.enqueue(
                        &cfg,
                        crate::daemon::DevqlTaskSource::PostMerge,
                        crate::daemon::DevqlTaskSpec::Ingest(crate::daemon::IngestTaskSpec {
                            commits: Vec::new(),
                            backfill: Some(backfill),
                        }),
                    )?;
                }
                let sync_enabled = crate::config::settings::devql_sync_enabled(&job.repo_root)
                    .context("loading DevQL sync producer policy for post-merge refresh")?;
                if sync_enabled {
                    let paths = crate::host::devql::refresh_paths_for_sync(
                        &cfg,
                        changed_files,
                        "post-merge",
                    )?;
                    if !paths.is_empty() {
                        self.enqueue(
                            &cfg,
                            crate::daemon::DevqlTaskSource::PostMerge,
                            crate::daemon::DevqlTaskSpec::Sync(crate::daemon::SyncTaskSpec {
                                mode: crate::daemon::SyncTaskMode::Paths { paths },
                                post_commit_snapshot: None,
                            }),
                        )?;
                    }
                }
                Ok(())
            }
            crate::host::devql::ProducerSpoolJobPayload::PrePushSync {
                remote,
                stdin_lines,
            } => {
                if !crate::config::settings::devql_sync_enabled(&job.repo_root)
                    .context("loading DevQL sync producer policy for pre-push sync")?
                {
                    return Ok(());
                }
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
}

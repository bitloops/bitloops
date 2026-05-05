use std::sync::Arc;

use anyhow::Result;

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
}

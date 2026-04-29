use anyhow::Result;
use serde_json::json;

use crate::host::capability_host::{
    CurrentStateConsumer, CurrentStateConsumerContext, CurrentStateConsumerFuture,
    CurrentStateConsumerRequest, CurrentStateConsumerResult,
};

use super::services::config::CodeCityConfig;
use super::services::snapshot::build_codecity_snapshot_from_current_rows;
use super::storage::SqliteCodeCityRepository;
use super::types::{CODECITY_CAPABILITY_ID, CODECITY_SNAPSHOT_CONSUMER_ID};

pub struct CodeCitySnapshotConsumer;

impl CurrentStateConsumer for CodeCitySnapshotConsumer {
    fn capability_id(&self) -> &str {
        CODECITY_CAPABILITY_ID
    }

    fn consumer_id(&self) -> &str {
        CODECITY_SNAPSHOT_CONSUMER_ID
    }

    fn reconcile<'a>(
        &'a self,
        request: &'a CurrentStateConsumerRequest,
        context: &'a CurrentStateConsumerContext,
    ) -> CurrentStateConsumerFuture<'a> {
        Box::pin(async move {
            let repo = SqliteCodeCityRepository::open_for_repo_root(&request.repo_root)?;
            repo.initialise_schema()?;

            let config = CodeCityConfig::default();
            let config_fingerprint = config.fingerprint()?;
            if repo.load_snapshot_requests(&request.repo_id)?.is_empty() {
                repo.upsert_snapshot_request(
                    &request.repo_id,
                    None,
                    &config_fingerprint,
                    Some(request.to_generation_seq_inclusive),
                )?;
            }

            let requests = repo.load_snapshot_requests(&request.repo_id)?;
            let mut warnings = Vec::new();
            let mut refreshed = 0_u64;
            let mut failed = 0_u64;

            for snapshot_request in requests {
                repo.mark_snapshot_running(
                    &request.repo_id,
                    &snapshot_request.snapshot_key,
                    &request.to_generation_seq_inclusive.to_string(),
                    request.to_generation_seq_inclusive,
                )?;
                match build_codecity_snapshot_from_current_rows(
                    context.relational.as_ref(),
                    &request.repo_id,
                    &request.repo_root,
                    snapshot_request.project_path.as_deref(),
                    &config,
                    request.to_generation_seq_inclusive,
                    context.git_history.as_ref(),
                    context.test_harness.as_deref(),
                ) {
                    Ok(snapshot) => {
                        repo.replace_codecity_snapshot(
                            &snapshot.snapshot_key,
                            snapshot.project_path.as_deref(),
                            request.to_generation_seq_inclusive,
                            &snapshot.world,
                            &snapshot.phase4,
                        )?;
                        refreshed += 1;
                    }
                    Err(err) => {
                        let error = format!("{err:#}");
                        repo.mark_snapshot_failed(
                            &request.repo_id,
                            &snapshot_request.snapshot_key,
                            request.to_generation_seq_inclusive,
                            &error,
                        )?;
                        warnings.push(format!(
                            "CodeCity snapshot `{}` failed: {error}",
                            snapshot_request.snapshot_key
                        ));
                        failed += 1;
                    }
                }
            }

            Ok(CurrentStateConsumerResult {
                applied_to_generation_seq: request.to_generation_seq_inclusive,
                warnings,
                metrics: Some(json!({
                    "refreshed_snapshots": refreshed,
                    "failed_snapshots": failed,
                    "reconcile_mode": format!("{:?}", request.reconcile_mode),
                })),
            })
        })
    }
}

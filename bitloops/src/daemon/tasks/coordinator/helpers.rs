use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use tokio::sync::{mpsc, oneshot};

use crate::daemon::CapabilityEventCoordinator;
use crate::host::capability_host::{DevqlCapabilityHost, SyncArtefactDiff, SyncFileDiff};
use crate::host::devql::{
    DevqlConfig, IngestionProgressPhase, SyncCurrentStateBatchUpdate, SyncSummary,
};

use super::super::super::types::{
    DevqlTaskKind, DevqlTaskProgress, DevqlTaskSpec, EmbeddingsBootstrapProgress,
    EmbeddingsBootstrapResult, SyncTaskSpec,
};

pub(super) const PROGRESS_PERSIST_INTERVAL: Duration = Duration::from_secs(1);

pub(super) async fn receive_embeddings_bootstrap_outcome<R>(
    mut progress_rx: mpsc::UnboundedReceiver<EmbeddingsBootstrapProgress>,
    mut result_rx: oneshot::Receiver<Result<EmbeddingsBootstrapResult>>,
    mut on_progress: R,
) -> Result<Result<EmbeddingsBootstrapResult>>
where
    R: FnMut(EmbeddingsBootstrapProgress) -> Result<()>,
{
    let mut progress_closed = false;
    let mut final_result = None;

    while final_result.is_none() || !progress_closed {
        tokio::select! {
            maybe_progress = progress_rx.recv(), if !progress_closed => {
                match maybe_progress {
                    Some(progress) => on_progress(progress)?,
                    None => progress_closed = true,
                }
            }
            result = &mut result_rx, if final_result.is_none() => {
                let received: Result<EmbeddingsBootstrapResult> =
                    result.map_err(|_| anyhow!("embeddings bootstrap worker result channel dropped"))?;
                final_result = Some(received);
            }
        }
    }

    final_result.ok_or_else(|| anyhow!("embeddings bootstrap task exited without a result"))
}

pub(super) fn task_kind_from_spec(spec: &DevqlTaskSpec) -> DevqlTaskKind {
    match spec {
        DevqlTaskSpec::Sync(_) => DevqlTaskKind::Sync,
        DevqlTaskSpec::Ingest(_) => DevqlTaskKind::Ingest,
        DevqlTaskSpec::EmbeddingsBootstrap(_) => DevqlTaskKind::EmbeddingsBootstrap,
    }
}

pub(super) fn progress_action(update: &DevqlTaskProgress) -> String {
    match update {
        DevqlTaskProgress::Sync(update) => update.phase.as_str().to_string(),
        DevqlTaskProgress::Ingest(update) => match update.phase {
            IngestionProgressPhase::Initializing => "initializing".to_string(),
            IngestionProgressPhase::Extracting => "extracting".to_string(),
            IngestionProgressPhase::Persisting => "persisting".to_string(),
            IngestionProgressPhase::Complete => "complete".to_string(),
            IngestionProgressPhase::Failed => "failed".to_string(),
        },
        DevqlTaskProgress::EmbeddingsBootstrap(update) => update.phase.as_str().to_string(),
    }
}

pub(super) fn enqueue_sync_completed_runs(
    coordinator: &CapabilityEventCoordinator,
    host: &DevqlCapabilityHost,
    cfg: &DevqlConfig,
    source_task_id: &str,
    summary: &SyncSummary,
    file_diff: SyncFileDiff,
    artefact_diff: SyncArtefactDiff,
) -> Result<usize> {
    let runs = coordinator.record_sync_generation(
        host,
        cfg,
        summary,
        file_diff,
        artefact_diff,
        Some(source_task_id),
    )?;
    if runs.runs.is_empty() {
        return Ok(0);
    }
    Ok(runs.runs.len())
}

pub(super) fn enqueue_sync_current_state_batch_runs(
    coordinator: &CapabilityEventCoordinator,
    host: &DevqlCapabilityHost,
    cfg: &DevqlConfig,
    source_task_id: &str,
    sync_mode: &str,
    batch: SyncCurrentStateBatchUpdate,
) -> Result<()> {
    if batch.is_empty() {
        return Ok(());
    }

    let _ = coordinator.record_sync_generation_with_options(
        host,
        cfg,
        &SyncSummary {
            success: true,
            mode: sync_mode.to_string(),
            active_branch: batch.active_branch,
            head_commit_sha: batch.head_commit_sha,
            ..SyncSummary::default()
        },
        SyncFileDiff {
            added: Vec::new(),
            changed: batch.file_upserts,
            removed: batch.file_removals,
        },
        SyncArtefactDiff {
            added: Vec::new(),
            changed: batch.artefact_upserts,
            removed: batch.artefact_removals,
        },
        Some(source_task_id),
        Some(false),
    )?;
    Ok(())
}

pub(super) fn should_persist_progress<T: PartialEq>(
    previous: Option<&T>,
    update: &T,
    last_persisted_at: Option<Instant>,
    now: Instant,
) -> bool {
    let Some(previous) = previous else {
        return true;
    };

    let interval_elapsed = last_persisted_at
        .is_none_or(|timestamp| now.duration_since(timestamp) >= PROGRESS_PERSIST_INTERVAL);
    interval_elapsed && previous != update
}

pub(super) fn sync_spec_from_task_spec_mut(spec: &mut DevqlTaskSpec) -> Option<&mut SyncTaskSpec> {
    match spec {
        DevqlTaskSpec::Sync(spec) => Some(spec),
        DevqlTaskSpec::Ingest(_) | DevqlTaskSpec::EmbeddingsBootstrap(_) => None,
    }
}

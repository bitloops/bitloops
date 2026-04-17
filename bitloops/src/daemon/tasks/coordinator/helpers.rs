use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use tokio::sync::{mpsc, oneshot};

use crate::daemon::{CapabilityEventCoordinator, SyncGenerationInput};
use crate::host::capability_host::{DevqlCapabilityHost, SyncArtefactDiff, SyncFileDiff};
use crate::host::devql::{DevqlConfig, IngestionProgressPhase, SyncSummary};

use super::super::super::types::{
    DevqlTaskKind, DevqlTaskProgress, DevqlTaskRecord, DevqlTaskSpec, EmbeddingsBootstrapProgress,
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
        DevqlTaskSpec::SummaryBootstrap(_) => DevqlTaskKind::SummaryBootstrap,
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
        DevqlTaskProgress::SummaryBootstrap(update) => update.phase.to_string(),
    }
}

pub(super) fn enqueue_sync_completed_runs(
    coordinator: &CapabilityEventCoordinator,
    host: &DevqlCapabilityHost,
    cfg: &DevqlConfig,
    task: &DevqlTaskRecord,
    summary: &SyncSummary,
    file_diff: SyncFileDiff,
    artefact_diff: SyncArtefactDiff,
) -> Result<usize> {
    let runs = coordinator.record_sync_generation(
        host,
        cfg,
        summary,
        SyncGenerationInput {
            file_diff,
            artefact_diff,
            source_task_id: Some(task.task_id.as_str()),
            init_session_id: task.init_session_id.as_deref(),
        },
    )?;
    if runs.runs.is_empty() {
        return Ok(0);
    }
    Ok(runs.runs.len())
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

pub(super) fn should_persist_embeddings_bootstrap_progress(
    previous: Option<&EmbeddingsBootstrapProgress>,
    update: &EmbeddingsBootstrapProgress,
    last_persisted_at: Option<Instant>,
    now: Instant,
) -> bool {
    let Some(previous) = previous else {
        return true;
    };

    if previous.phase != update.phase
        || previous.asset_name != update.asset_name
        || previous.bytes_total != update.bytes_total
        || previous.version != update.version
        || previous.message != update.message
    {
        return true;
    }

    let interval_elapsed = last_persisted_at
        .is_none_or(|timestamp| now.duration_since(timestamp) >= PROGRESS_PERSIST_INTERVAL);
    interval_elapsed && previous.bytes_downloaded != update.bytes_downloaded
}

pub(super) fn sync_spec_from_task_spec_mut(spec: &mut DevqlTaskSpec) -> Option<&mut SyncTaskSpec> {
    match spec {
        DevqlTaskSpec::Sync(spec) => Some(spec),
        DevqlTaskSpec::Ingest(_)
        | DevqlTaskSpec::EmbeddingsBootstrap(_)
        | DevqlTaskSpec::SummaryBootstrap(_) => None,
    }
}

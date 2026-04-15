use super::*;

pub(super) fn emit_progress(
    observer: Option<&dyn IngestionObserver>,
    phase: IngestionProgressPhase,
    commits_total: usize,
    commits_processed: usize,
    current_checkpoint_id: Option<String>,
    current_commit_sha: Option<String>,
    counters: &IngestionCounters,
) {
    let Some(observer) = observer else {
        return;
    };
    observer.on_progress(IngestionProgressUpdate {
        phase,
        commits_total,
        commits_processed,
        current_checkpoint_id,
        current_commit_sha,
        counters: counters.clone(),
    });
}

pub(super) fn emit_checkpoint_ingested(
    observer: Option<&dyn IngestionObserver>,
    checkpoint: crate::host::checkpoints::strategy::manual_commit::CommittedInfo,
    commit_sha: Option<String>,
) {
    let Some(observer) = observer else {
        return;
    };
    observer.on_checkpoint_ingested(IngestedCheckpointNotification {
        checkpoint,
        commit_sha,
    });
}

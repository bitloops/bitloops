use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::graphql::Checkpoint;
use crate::host::devql::{
    IngestedCheckpointNotification, IngestionObserver, IngestionProgressUpdate, SyncObserver,
    SyncProgressUpdate,
};

use super::super::super::types::DevqlTaskProgress;
use super::DevqlTaskCoordinator;
use super::helpers::should_persist_progress;

pub(super) struct SyncCoordinatorObserver {
    pub(super) coordinator: Arc<DevqlTaskCoordinator>,
    pub(super) task_id: String,
    pub(super) progress_state: Mutex<ProgressPersistState<SyncProgressUpdate>>,
}

pub(super) struct IngestCoordinatorObserver {
    pub(super) coordinator: Arc<DevqlTaskCoordinator>,
    pub(super) task_id: String,
    pub(super) repo_name: String,
    pub(super) progress_state: Mutex<ProgressPersistState<IngestionProgressUpdate>>,
}

#[derive(Debug)]
pub(super) struct ProgressPersistState<T> {
    pub(super) last_persisted: Option<T>,
    pub(super) last_persisted_at: Option<Instant>,
}

impl<T> Default for ProgressPersistState<T> {
    fn default() -> Self {
        Self {
            last_persisted: None,
            last_persisted_at: None,
        }
    }
}

impl SyncObserver for SyncCoordinatorObserver {
    fn on_progress(&self, update: SyncProgressUpdate) {
        match self.progress_state.lock() {
            Ok(mut state) => {
                let now = Instant::now();
                if !should_persist_progress(
                    state.last_persisted.as_ref(),
                    &update,
                    state.last_persisted_at,
                    now,
                ) {
                    return;
                }
                state.last_persisted = Some(update.clone());
                state.last_persisted_at = Some(now);
            }
            Err(_) => {
                log::warn!(
                    "failed to acquire sync progress throttle state for task `{}`",
                    self.task_id
                );
            }
        }

        if let Err(err) = self
            .coordinator
            .update_task_progress(&self.task_id, DevqlTaskProgress::Sync(update))
        {
            log::warn!(
                "failed to persist sync progress for task `{}`: {err:#}",
                self.task_id
            );
        }
    }
}

impl IngestionObserver for IngestCoordinatorObserver {
    fn on_progress(&self, update: IngestionProgressUpdate) {
        match self.progress_state.lock() {
            Ok(mut state) => {
                let now = Instant::now();
                if !should_persist_progress(
                    state.last_persisted.as_ref(),
                    &update,
                    state.last_persisted_at,
                    now,
                ) {
                    return;
                }
                state.last_persisted = Some(update.clone());
                state.last_persisted_at = Some(now);
            }
            Err(_) => {
                log::warn!(
                    "failed to acquire ingest progress throttle state for task `{}`",
                    self.task_id
                );
            }
        }

        if let Err(err) = self
            .coordinator
            .update_task_progress(&self.task_id, DevqlTaskProgress::Ingest(update))
        {
            log::warn!(
                "failed to persist ingest progress for task `{}`: {err:#}",
                self.task_id
            );
        }
    }

    fn on_checkpoint_ingested(&self, checkpoint: IngestedCheckpointNotification) {
        self.coordinator.publish_checkpoint(
            self.repo_name.clone(),
            Checkpoint::from_ingested(&checkpoint.checkpoint, checkpoint.commit_sha.as_deref()),
        );
    }
}

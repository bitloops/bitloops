use std::sync::Arc;

use tokio::sync::broadcast;

use super::types::{Checkpoint, IngestionProgressEvent};

const SUBSCRIPTION_CHANNEL_CAPACITY: usize = 128;

#[derive(Debug, Clone)]
pub(crate) struct CheckpointIngestedEvent {
    pub(crate) checkpoint: Checkpoint,
}

#[derive(Debug, Clone)]
pub(crate) struct IngestionProgressMessage {
    pub(crate) event: IngestionProgressEvent,
}

#[derive(Debug, Clone)]
pub(crate) struct SubscriptionHub {
    checkpoint_ingested: broadcast::Sender<CheckpointIngestedEvent>,
    ingestion_progress: broadcast::Sender<IngestionProgressMessage>,
}

impl Default for SubscriptionHub {
    fn default() -> Self {
        let (checkpoint_ingested, _) = broadcast::channel(SUBSCRIPTION_CHANNEL_CAPACITY);
        let (ingestion_progress, _) = broadcast::channel(SUBSCRIPTION_CHANNEL_CAPACITY);
        Self {
            checkpoint_ingested,
            ingestion_progress,
        }
    }
}

impl SubscriptionHub {
    pub(crate) fn shared() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub(crate) fn subscribe_checkpoints(&self) -> broadcast::Receiver<CheckpointIngestedEvent> {
        self.checkpoint_ingested.subscribe()
    }

    pub(crate) fn subscribe_progress(&self) -> broadcast::Receiver<IngestionProgressMessage> {
        self.ingestion_progress.subscribe()
    }

    pub(crate) fn publish_checkpoint(&self, repo_name: impl Into<String>, checkpoint: Checkpoint) {
        let _ = repo_name.into();
        let _ = self
            .checkpoint_ingested
            .send(CheckpointIngestedEvent { checkpoint });
    }

    pub(crate) fn publish_progress(
        &self,
        repo_name: impl Into<String>,
        event: IngestionProgressEvent,
    ) {
        let _ = repo_name.into();
        let _ = self
            .ingestion_progress
            .send(IngestionProgressMessage { event });
    }
}

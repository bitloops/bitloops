use std::sync::Arc;

use tokio::sync::broadcast;

use super::types::Checkpoint;
use crate::daemon::{DevqlTaskRecord, RuntimeEventRecord};

const SUBSCRIPTION_CHANNEL_CAPACITY: usize = 128;

#[derive(Debug, Clone)]
pub(crate) struct CheckpointIngestedEvent {
    pub(crate) repo_name: String,
    pub(crate) checkpoint: Checkpoint,
}

#[derive(Debug, Clone)]
pub(crate) struct TaskProgressMessage {
    pub(crate) task_id: String,
    pub(crate) task: DevqlTaskRecord,
}

#[derive(Debug, Clone)]
pub(crate) struct SubscriptionHub {
    checkpoint_ingested: broadcast::Sender<CheckpointIngestedEvent>,
    task_progress: broadcast::Sender<TaskProgressMessage>,
    runtime_events: broadcast::Sender<RuntimeEventRecord>,
}

impl Default for SubscriptionHub {
    fn default() -> Self {
        let (checkpoint_ingested, _) = broadcast::channel(SUBSCRIPTION_CHANNEL_CAPACITY);
        let (task_progress, _) = broadcast::channel(SUBSCRIPTION_CHANNEL_CAPACITY);
        let (runtime_events, _) = broadcast::channel(SUBSCRIPTION_CHANNEL_CAPACITY);
        Self {
            checkpoint_ingested,
            task_progress,
            runtime_events,
        }
    }
}

impl SubscriptionHub {
    pub(crate) fn new_arc() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub(crate) fn subscribe_checkpoints(&self) -> broadcast::Receiver<CheckpointIngestedEvent> {
        self.checkpoint_ingested.subscribe()
    }

    pub(crate) fn subscribe_task_progress(&self) -> broadcast::Receiver<TaskProgressMessage> {
        self.task_progress.subscribe()
    }

    pub(crate) fn subscribe_runtime_events(&self) -> broadcast::Receiver<RuntimeEventRecord> {
        self.runtime_events.subscribe()
    }

    pub(crate) fn publish_checkpoint(&self, repo_name: impl Into<String>, checkpoint: Checkpoint) {
        let repo_name = repo_name.into();
        let _ = self.checkpoint_ingested.send(CheckpointIngestedEvent {
            repo_name,
            checkpoint,
        });
    }

    pub(crate) fn publish_task(&self, task: DevqlTaskRecord) {
        let task_id = task.task_id.clone();
        let runtime_event = RuntimeEventRecord {
            domain: "task_queue".to_string(),
            repo_id: task.repo_id.clone(),
            init_session_id: task.init_session_id.clone(),
            updated_at_unix: task.updated_at_unix,
            task_id: Some(task_id.clone()),
            run_id: None,
            mailbox_name: None,
        };
        let _ = self
            .task_progress
            .send(TaskProgressMessage { task_id, task });
        let _ = self.runtime_events.send(runtime_event);
    }

    pub(crate) fn publish_runtime_event(&self, event: RuntimeEventRecord) {
        let _ = self.runtime_events.send(event);
    }
}

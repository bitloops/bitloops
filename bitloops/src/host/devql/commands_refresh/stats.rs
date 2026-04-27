use crate::host::devql::SyncSummary;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedSyncTaskMetadata {
    pub task_id: String,
    pub merged: bool,
    pub queue_position: Option<u64>,
    pub tasks_ahead: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PostCommitArtefactRefreshStats {
    pub files_seen: usize,
    pub files_indexed: usize,
    pub files_deleted: usize,
    pub files_failed: usize,
    pub queued_task: Option<QueuedSyncTaskMetadata>,
}

impl PostCommitArtefactRefreshStats {
    pub(crate) fn completed_with_failures(&self) -> bool {
        self.queued_task.is_none() && self.files_failed > 0
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn inline_from_summary(files_seen: usize, summary: &SyncSummary) -> Self {
        Self {
            files_seen,
            files_indexed: summary.paths_added + summary.paths_changed,
            files_deleted: summary.paths_removed,
            files_failed: summary.parse_errors,
            queued_task: None,
        }
    }

    pub(super) fn queued(files_seen: usize, queued: crate::daemon::DevqlTaskEnqueueResult) -> Self {
        Self {
            files_seen,
            files_indexed: 0,
            files_deleted: 0,
            files_failed: 0,
            queued_task: Some(QueuedSyncTaskMetadata {
                task_id: queued.task.task_id,
                merged: queued.merged,
                queue_position: queued.task.queue_position,
                tasks_ahead: queued.task.tasks_ahead,
            }),
        }
    }
}

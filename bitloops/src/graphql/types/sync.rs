use async_graphql::{Enum, SimpleObject};

use crate::daemon::{
    DevqlTaskControlResult, DevqlTaskKindCounts, DevqlTaskQueueStatus, DevqlTaskRecord,
    DevqlTaskStatus,
};
use crate::graphql::mutation_root::{IngestResult, SyncResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum TaskKind {
    Sync,
    Ingest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum TaskStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, SimpleObject)]
#[graphql(name = "SyncTaskSpec")]
pub struct SyncTaskSpecObject {
    pub mode: String,
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, SimpleObject)]
#[graphql(name = "IngestTaskSpec")]
pub struct IngestTaskSpecObject {
    pub backfill: Option<i32>,
}

#[derive(Debug, Clone, SimpleObject)]
#[graphql(name = "SyncTaskProgress")]
pub struct SyncTaskProgressObject {
    pub phase: String,
    pub current_path: Option<String>,
    pub paths_total: i32,
    pub paths_completed: i32,
    pub paths_remaining: i32,
    pub paths_unchanged: i32,
    pub paths_added: i32,
    pub paths_changed: i32,
    pub paths_removed: i32,
    pub cache_hits: i32,
    pub cache_misses: i32,
    pub parse_errors: i32,
}

#[derive(Debug, Clone, SimpleObject)]
#[graphql(name = "Task")]
pub struct TaskObject {
    pub task_id: String,
    pub repo_id: String,
    pub repo_name: String,
    pub repo_identity: String,
    pub kind: TaskKind,
    pub source: String,
    pub status: TaskStatus,
    pub submitted_at_unix: i64,
    pub started_at_unix: Option<i64>,
    pub updated_at_unix: i64,
    pub completed_at_unix: Option<i64>,
    pub queue_position: Option<i32>,
    pub tasks_ahead: Option<i32>,
    pub error: Option<String>,
    pub sync_spec: Option<SyncTaskSpecObject>,
    pub ingest_spec: Option<IngestTaskSpecObject>,
    pub sync_progress: Option<SyncTaskProgressObject>,
    pub ingest_progress: Option<super::IngestionProgressEvent>,
    pub sync_result: Option<SyncResult>,
    pub ingest_result: Option<IngestResult>,
}

#[derive(Debug, Clone, SimpleObject)]
#[graphql(name = "TaskProgressEvent")]
pub struct TaskProgressEvent {
    pub task: TaskObject,
}

#[derive(Debug, Clone, SimpleObject)]
#[graphql(name = "TaskKindCounts")]
pub struct TaskKindCountsObject {
    pub kind: TaskKind,
    pub queued_tasks: i32,
    pub running_tasks: i32,
    pub failed_tasks: i32,
    pub completed_recent_tasks: i32,
}

#[derive(Debug, Clone, SimpleObject)]
#[graphql(name = "TaskQueueStatus")]
pub struct TaskQueueStatusObject {
    pub persisted: bool,
    pub queued_tasks: i32,
    pub running_tasks: i32,
    pub failed_tasks: i32,
    pub completed_recent_tasks: i32,
    pub by_kind: Vec<TaskKindCountsObject>,
    pub paused: bool,
    pub paused_reason: Option<String>,
    pub last_action: Option<String>,
    pub last_updated_unix: i64,
    pub current_repo_tasks: Vec<TaskObject>,
}

#[derive(Debug, Clone, SimpleObject)]
#[graphql(name = "TaskQueueControlResult")]
pub struct TaskQueueControlResultObject {
    pub message: String,
    pub repo_id: String,
    pub paused: bool,
    pub paused_reason: Option<String>,
    pub updated_at_unix: i64,
}

impl From<DevqlTaskRecord> for TaskObject {
    fn from(value: DevqlTaskRecord) -> Self {
        let sync_spec = value.sync_spec().map(|spec| match &spec.mode {
            crate::daemon::SyncTaskMode::Auto => SyncTaskSpecObject {
                mode: "auto".to_string(),
                paths: Vec::new(),
            },
            crate::daemon::SyncTaskMode::Full => SyncTaskSpecObject {
                mode: "full".to_string(),
                paths: Vec::new(),
            },
            crate::daemon::SyncTaskMode::Paths { paths } => SyncTaskSpecObject {
                mode: "paths".to_string(),
                paths: paths.clone(),
            },
            crate::daemon::SyncTaskMode::Repair => SyncTaskSpecObject {
                mode: "repair".to_string(),
                paths: Vec::new(),
            },
            crate::daemon::SyncTaskMode::Validate => SyncTaskSpecObject {
                mode: "validate".to_string(),
                paths: Vec::new(),
            },
        });
        let ingest_spec = value.ingest_spec().map(|spec| IngestTaskSpecObject {
            backfill: spec.backfill.map(to_graphql_count),
        });
        let sync_progress = value.sync_progress().map(|progress| SyncTaskProgressObject {
            phase: progress.phase.as_str().to_string(),
            current_path: progress.current_path.clone(),
            paths_total: to_graphql_count(progress.paths_total),
            paths_completed: to_graphql_count(progress.paths_completed),
            paths_remaining: to_graphql_count(progress.paths_remaining),
            paths_unchanged: to_graphql_count(progress.paths_unchanged),
            paths_added: to_graphql_count(progress.paths_added),
            paths_changed: to_graphql_count(progress.paths_changed),
            paths_removed: to_graphql_count(progress.paths_removed),
            cache_hits: to_graphql_count(progress.cache_hits),
            cache_misses: to_graphql_count(progress.cache_misses),
            parse_errors: to_graphql_count(progress.parse_errors),
        });
        let ingest_progress = value.ingest_progress().cloned().map(Into::into);
        let sync_result = value.sync_result().cloned().map(Into::into);
        let ingest_result = value.ingest_result().cloned().map(Into::into);

        Self {
            task_id: value.task_id,
            repo_id: value.repo_id,
            repo_name: value.repo_name,
            repo_identity: value.repo_identity,
            kind: match value.kind {
                crate::daemon::DevqlTaskKind::Sync => TaskKind::Sync,
                crate::daemon::DevqlTaskKind::Ingest => TaskKind::Ingest,
            },
            source: value.source.to_string(),
            status: match value.status {
                DevqlTaskStatus::Queued => TaskStatus::Queued,
                DevqlTaskStatus::Running => TaskStatus::Running,
                DevqlTaskStatus::Completed => TaskStatus::Completed,
                DevqlTaskStatus::Failed => TaskStatus::Failed,
                DevqlTaskStatus::Cancelled => TaskStatus::Cancelled,
            },
            submitted_at_unix: value.submitted_at_unix as i64,
            started_at_unix: value.started_at_unix.map(|value| value as i64),
            updated_at_unix: value.updated_at_unix as i64,
            completed_at_unix: value.completed_at_unix.map(|value| value as i64),
            queue_position: value.queue_position.map(to_graphql_count),
            tasks_ahead: value.tasks_ahead.map(to_graphql_count),
            error: value.error,
            sync_spec,
            ingest_spec,
            sync_progress,
            ingest_progress,
            sync_result,
            ingest_result,
        }
    }
}

impl From<DevqlTaskRecord> for TaskProgressEvent {
    fn from(value: DevqlTaskRecord) -> Self {
        Self { task: value.into() }
    }
}

impl From<TaskKind> for crate::daemon::DevqlTaskKind {
    fn from(value: TaskKind) -> Self {
        match value {
            TaskKind::Sync => Self::Sync,
            TaskKind::Ingest => Self::Ingest,
        }
    }
}

impl From<TaskStatus> for crate::daemon::DevqlTaskStatus {
    fn from(value: TaskStatus) -> Self {
        match value {
            TaskStatus::Queued => Self::Queued,
            TaskStatus::Running => Self::Running,
            TaskStatus::Completed => Self::Completed,
            TaskStatus::Failed => Self::Failed,
            TaskStatus::Cancelled => Self::Cancelled,
        }
    }
}

impl From<DevqlTaskKindCounts> for TaskKindCountsObject {
    fn from(value: DevqlTaskKindCounts) -> Self {
        Self {
            kind: match value.kind {
                crate::daemon::DevqlTaskKind::Sync => TaskKind::Sync,
                crate::daemon::DevqlTaskKind::Ingest => TaskKind::Ingest,
            },
            queued_tasks: to_graphql_count(value.queued_tasks),
            running_tasks: to_graphql_count(value.running_tasks),
            failed_tasks: to_graphql_count(value.failed_tasks),
            completed_recent_tasks: to_graphql_count(value.completed_recent_tasks),
        }
    }
}

impl From<DevqlTaskQueueStatus> for TaskQueueStatusObject {
    fn from(value: DevqlTaskQueueStatus) -> Self {
        Self {
            persisted: value.persisted,
            queued_tasks: to_graphql_count(value.state.queued_tasks),
            running_tasks: to_graphql_count(value.state.running_tasks),
            failed_tasks: to_graphql_count(value.state.failed_tasks),
            completed_recent_tasks: to_graphql_count(value.state.completed_recent_tasks),
            by_kind: value.state.by_kind.into_iter().map(Into::into).collect(),
            paused: value
                .current_repo_control
                .as_ref()
                .map(|control| control.paused)
                .unwrap_or(false),
            paused_reason: value
                .current_repo_control
                .as_ref()
                .and_then(|control| control.paused_reason.clone()),
            last_action: value.state.last_action,
            last_updated_unix: value.state.last_updated_unix as i64,
            current_repo_tasks: value.current_repo_tasks.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<DevqlTaskControlResult> for TaskQueueControlResultObject {
    fn from(value: DevqlTaskControlResult) -> Self {
        Self {
            message: value.message,
            repo_id: value.control.repo_id,
            paused: value.control.paused,
            paused_reason: value.control.paused_reason,
            updated_at_unix: value.control.updated_at_unix as i64,
        }
    }
}

fn to_graphql_count(value: impl TryInto<i32>) -> i32 {
    value.try_into().unwrap_or(i32::MAX)
}

use async_graphql::SimpleObject;

use crate::daemon::SyncTaskRecord;
use crate::graphql::mutation_root::SyncResult;

#[derive(Debug, Clone, SimpleObject)]
#[graphql(name = "SyncTask")]
pub struct SyncTaskObject {
    pub task_id: String,
    pub repo_id: String,
    pub repo_name: String,
    pub repo_identity: String,
    pub source: String,
    pub mode: String,
    pub status: String,
    pub submitted_at_unix: i64,
    pub started_at_unix: Option<i64>,
    pub updated_at_unix: i64,
    pub completed_at_unix: Option<i64>,
    pub queue_position: Option<i32>,
    pub tasks_ahead: Option<i32>,
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
    pub error: Option<String>,
    pub summary: Option<SyncResult>,
}

#[derive(Debug, Clone, SimpleObject)]
#[graphql(name = "SyncProgressEvent")]
pub struct SyncProgressEvent {
    pub task_id: String,
    pub repo_id: String,
    pub repo_name: String,
    pub repo_identity: String,
    pub source: String,
    pub mode: String,
    pub status: String,
    pub submitted_at_unix: i64,
    pub started_at_unix: Option<i64>,
    pub updated_at_unix: i64,
    pub completed_at_unix: Option<i64>,
    pub queue_position: Option<i32>,
    pub tasks_ahead: Option<i32>,
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
    pub error: Option<String>,
    pub summary: Option<SyncResult>,
}

impl From<SyncTaskRecord> for SyncTaskObject {
    fn from(value: SyncTaskRecord) -> Self {
        Self {
            task_id: value.task_id,
            repo_id: value.repo_id,
            repo_name: value.repo_name,
            repo_identity: value.repo_identity,
            source: value.source.to_string(),
            mode: value.mode.to_string(),
            status: value.status.to_string(),
            submitted_at_unix: value.submitted_at_unix as i64,
            started_at_unix: value.started_at_unix.map(|value| value as i64),
            updated_at_unix: value.updated_at_unix as i64,
            completed_at_unix: value.completed_at_unix.map(|value| value as i64),
            queue_position: value.queue_position.map(to_graphql_count),
            tasks_ahead: value.tasks_ahead.map(to_graphql_count),
            phase: value.progress.phase.as_str().to_string(),
            current_path: value.progress.current_path,
            paths_total: to_graphql_count(value.progress.paths_total),
            paths_completed: to_graphql_count(value.progress.paths_completed),
            paths_remaining: to_graphql_count(value.progress.paths_remaining),
            paths_unchanged: to_graphql_count(value.progress.paths_unchanged),
            paths_added: to_graphql_count(value.progress.paths_added),
            paths_changed: to_graphql_count(value.progress.paths_changed),
            paths_removed: to_graphql_count(value.progress.paths_removed),
            cache_hits: to_graphql_count(value.progress.cache_hits),
            cache_misses: to_graphql_count(value.progress.cache_misses),
            parse_errors: to_graphql_count(value.progress.parse_errors),
            error: value.error,
            summary: value.summary.map(Into::into),
        }
    }
}

impl From<SyncTaskRecord> for SyncProgressEvent {
    fn from(value: SyncTaskRecord) -> Self {
        let task: SyncTaskObject = value.into();
        Self {
            task_id: task.task_id,
            repo_id: task.repo_id,
            repo_name: task.repo_name,
            repo_identity: task.repo_identity,
            source: task.source,
            mode: task.mode,
            status: task.status,
            submitted_at_unix: task.submitted_at_unix,
            started_at_unix: task.started_at_unix,
            updated_at_unix: task.updated_at_unix,
            completed_at_unix: task.completed_at_unix,
            queue_position: task.queue_position,
            tasks_ahead: task.tasks_ahead,
            phase: task.phase,
            current_path: task.current_path,
            paths_total: task.paths_total,
            paths_completed: task.paths_completed,
            paths_remaining: task.paths_remaining,
            paths_unchanged: task.paths_unchanged,
            paths_added: task.paths_added,
            paths_changed: task.paths_changed,
            paths_removed: task.paths_removed,
            cache_hits: task.cache_hits,
            cache_misses: task.cache_misses,
            parse_errors: task.parse_errors,
            error: task.error,
            summary: task.summary,
        }
    }
}

fn to_graphql_count(value: impl TryInto<i32>) -> i32 {
    value.try_into().unwrap_or(i32::MAX)
}

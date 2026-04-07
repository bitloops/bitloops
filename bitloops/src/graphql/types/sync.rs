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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_sync_record() -> SyncTaskRecord {
        SyncTaskRecord {
            task_id: "sync-task-1".to_string(),
            repo_id: "repo-1".to_string(),
            repo_name: "bitloops".to_string(),
            repo_provider: "local".to_string(),
            repo_organisation: "local".to_string(),
            repo_identity: "local/bitloops".to_string(),
            daemon_config_root: std::path::PathBuf::from("/tmp/config"),
            repo_root: std::path::PathBuf::from("/tmp/repo"),
            source: crate::daemon::SyncTaskSource::ManualCli,
            mode: crate::daemon::SyncTaskMode::Validate,
            status: crate::daemon::SyncTaskStatus::Running,
            submitted_at_unix: 1,
            started_at_unix: Some(2),
            updated_at_unix: 3,
            completed_at_unix: None,
            queue_position: Some((i32::MAX as u64) + 1),
            tasks_ahead: Some((i32::MAX as u64) + 2),
            progress: crate::host::devql::SyncProgressUpdate {
                phase: crate::host::devql::SyncProgressPhase::MaterialisingPaths,
                current_path: Some("src/lib.rs".to_string()),
                paths_total: (i32::MAX as usize) + 3,
                paths_completed: (i32::MAX as usize) + 4,
                paths_remaining: (i32::MAX as usize) + 5,
                paths_unchanged: (i32::MAX as usize) + 6,
                paths_added: (i32::MAX as usize) + 7,
                paths_changed: (i32::MAX as usize) + 8,
                paths_removed: (i32::MAX as usize) + 9,
                cache_hits: (i32::MAX as usize) + 10,
                cache_misses: (i32::MAX as usize) + 11,
                parse_errors: (i32::MAX as usize) + 12,
            },
            error: Some("boom".to_string()),
            summary: Some(crate::host::devql::SyncSummary {
                success: false,
                mode: "validate".to_string(),
                parser_version: "parser@1".to_string(),
                extractor_version: "extractor@1".to_string(),
                active_branch: Some("main".to_string()),
                head_commit_sha: Some("abc".to_string()),
                head_tree_sha: Some("def".to_string()),
                paths_unchanged: 1,
                paths_added: 2,
                paths_changed: 3,
                paths_removed: 4,
                cache_hits: 5,
                cache_misses: 6,
                parse_errors: 7,
                validation: Some(crate::host::devql::SyncValidationSummary {
                    valid: false,
                    expected_artefacts: 10,
                    actual_artefacts: 9,
                    expected_edges: 8,
                    actual_edges: 7,
                    missing_artefacts: 1,
                    stale_artefacts: 2,
                    mismatched_artefacts: 3,
                    missing_edges: 4,
                    stale_edges: 5,
                    mismatched_edges: 6,
                    files_with_drift: vec![crate::host::devql::SyncValidationFileDrift {
                        path: "src/lib.rs".to_string(),
                        missing_artefacts: 1,
                        stale_artefacts: 2,
                        mismatched_artefacts: 3,
                        missing_edges: 4,
                        stale_edges: 5,
                        mismatched_edges: 6,
                    }],
                }),
            }),
        }
    }

    #[test]
    fn sync_task_object_from_record_maps_fields_and_clamps_counts() {
        let object = SyncTaskObject::from(sample_sync_record());
        assert_eq!(object.task_id, "sync-task-1");
        assert_eq!(object.source, "manual_cli");
        assert_eq!(object.mode, "validate");
        assert_eq!(object.status, "running");
        assert_eq!(object.phase, "materialising_paths");
        assert_eq!(object.queue_position, Some(i32::MAX));
        assert_eq!(object.tasks_ahead, Some(i32::MAX));
        assert_eq!(object.paths_total, i32::MAX);
        assert_eq!(object.cache_misses, i32::MAX);
        assert_eq!(object.parse_errors, i32::MAX);
        let summary = object.summary.expect("summary");
        assert_eq!(summary.mode, "validate");
        assert_eq!(summary.paths_added, 2);
        assert!(summary.validation.is_some());
    }

    #[test]
    fn sync_progress_event_reuses_sync_task_conversion() {
        let event = SyncProgressEvent::from(sample_sync_record());
        assert_eq!(event.task_id, "sync-task-1");
        assert_eq!(event.phase, "materialising_paths");
        assert_eq!(event.queue_position, Some(i32::MAX));
        assert_eq!(event.paths_changed, i32::MAX);
        assert_eq!(event.error.as_deref(), Some("boom"));
        assert!(event.summary.is_some());
    }

    #[test]
    fn to_graphql_count_clamps_large_inputs() {
        assert_eq!(to_graphql_count(123u32), 123);
        assert_eq!(to_graphql_count((i32::MAX as i64) + 1), i32::MAX);
    }
}

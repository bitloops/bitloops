use std::collections::{HashMap, HashSet};

use crate::host::devql::{
    DevqlConfig, SyncMode, SyncProgressPhase, SyncProgressUpdate, SyncSummary,
};

use super::super::types::{
    SyncQueueState, SyncQueueStatus, SyncTaskMode, SyncTaskRecord, SyncTaskSource, SyncTaskStatus,
    unix_timestamp_now,
};
use super::state::PersistedSyncQueueState;

const MAX_TERMINAL_TASKS: usize = 64;

pub(super) fn changed_tasks(
    previous: &[SyncTaskRecord],
    current: &[SyncTaskRecord],
) -> Vec<SyncTaskRecord> {
    let previous_by_id = previous
        .iter()
        .map(|task| (task.task_id.as_str(), task))
        .collect::<HashMap<_, _>>();
    current
        .iter()
        .filter(|task| {
            previous_by_id
                .get(task.task_id.as_str())
                .is_none_or(|previous| *previous != *task)
        })
        .cloned()
        .collect()
}

pub(super) fn merge_existing_task(
    state: &mut PersistedSyncQueueState,
    cfg: &DevqlConfig,
    _source: SyncTaskSource,
    mode: &SyncTaskMode,
) -> Option<SyncTaskRecord> {
    if *mode != SyncTaskMode::Validate
        && let Some(existing) = state.tasks.iter_mut().find(|task| {
            task.repo_id == cfg.repo.repo_id
                && matches!(
                    task.status,
                    SyncTaskStatus::Queued | SyncTaskStatus::Running
                )
                && match (&task.mode, mode) {
                    (SyncTaskMode::Repair, _) => true,
                    (existing_mode, incoming_mode)
                        if is_full_like(existing_mode) && is_weaker_than_repair(incoming_mode) =>
                    {
                        true
                    }
                    _ => false,
                }
        })
    {
        existing.updated_at_unix = unix_timestamp_now();
        existing.error = None;
        return Some(existing.clone());
    }

    if let SyncTaskMode::Paths { paths } = mode
        && let Some(existing) = state.tasks.iter_mut().find(|task| {
            task.repo_id == cfg.repo.repo_id
                && task.status == SyncTaskStatus::Queued
                && matches!(task.mode, SyncTaskMode::Paths { .. })
        })
    {
        if let SyncTaskMode::Paths {
            paths: existing_paths,
        } = &mut existing.mode
        {
            existing_paths.extend(paths.iter().cloned());
            existing_paths.sort();
            existing_paths.dedup();
        }
        existing.updated_at_unix = unix_timestamp_now();
        existing.error = None;
        return Some(existing.clone());
    }

    None
}

pub(super) fn next_pending_task_index(state: &PersistedSyncQueueState) -> Option<usize> {
    state
        .tasks
        .iter()
        .enumerate()
        .filter(|(_, task)| task.status == SyncTaskStatus::Queued)
        .min_by_key(|(_, task)| pending_sort_key(task))
        .map(|(index, _)| index)
}

fn pending_sort_key(task: &SyncTaskRecord) -> (u8, u64, String) {
    (
        if matches!(task.mode, SyncTaskMode::Validate) {
            1
        } else {
            0
        },
        task.submitted_at_unix,
        task.task_id.clone(),
    )
}

pub(super) fn recompute_queue_positions(tasks: &mut [SyncTaskRecord]) {
    for task in tasks.iter_mut() {
        task.queue_position = None;
        task.tasks_ahead = None;
    }

    let mut order = tasks
        .iter()
        .enumerate()
        .filter(|(_, task)| {
            matches!(
                task.status,
                SyncTaskStatus::Running | SyncTaskStatus::Queued
            )
        })
        .map(|(index, task)| {
            (
                index,
                task.status == SyncTaskStatus::Running,
                pending_sort_key(task),
            )
        })
        .collect::<Vec<_>>();
    order.sort_by(
        |(_, left_running, left_key), (_, right_running, right_key)| {
            let left_running = *left_running;
            let right_running = *right_running;
            left_running
                .cmp(&right_running)
                .reverse()
                .then_with(|| left_key.cmp(right_key))
        },
    );

    for (index, (task_index, _, _)) in order.into_iter().enumerate() {
        let position = (index as u64) + 1;
        tasks[task_index].queue_position = Some(position);
        tasks[task_index].tasks_ahead = Some(position.saturating_sub(1));
    }
}

pub(super) fn prune_terminal_tasks(tasks: &mut Vec<SyncTaskRecord>) {
    let mut terminal = tasks
        .iter()
        .filter(|task| {
            matches!(
                task.status,
                SyncTaskStatus::Completed | SyncTaskStatus::Failed | SyncTaskStatus::Cancelled
            )
        })
        .cloned()
        .collect::<Vec<_>>();
    terminal.sort_by(|left, right| right.updated_at_unix.cmp(&left.updated_at_unix));
    terminal.truncate(MAX_TERMINAL_TASKS);

    let terminal_ids = terminal
        .into_iter()
        .map(|task| task.task_id)
        .collect::<HashSet<_>>();
    tasks.retain(|task| {
        !matches!(
            task.status,
            SyncTaskStatus::Completed | SyncTaskStatus::Failed | SyncTaskStatus::Cancelled
        ) || terminal_ids.contains(&task.task_id)
    });
}

pub(super) fn project_status(
    state: &PersistedSyncQueueState,
    repo_id: Option<&str>,
    persisted: bool,
) -> SyncQueueStatus {
    let pending_tasks = state
        .tasks
        .iter()
        .filter(|task| task.status == SyncTaskStatus::Queued)
        .count() as u64;
    let running_tasks = state
        .tasks
        .iter()
        .filter(|task| task.status == SyncTaskStatus::Running)
        .count() as u64;
    let failed_tasks = state
        .tasks
        .iter()
        .filter(|task| task.status == SyncTaskStatus::Failed)
        .count() as u64;
    let completed_recent_tasks = state
        .tasks
        .iter()
        .filter(|task| task.status == SyncTaskStatus::Completed)
        .count() as u64;
    let current_repo_task = repo_id.and_then(|repo_id| select_repo_task(&state.tasks, repo_id));

    SyncQueueStatus {
        state: SyncQueueState {
            version: state.version,
            pending_tasks,
            running_tasks,
            failed_tasks,
            completed_recent_tasks,
            last_action: state.last_action.clone(),
            last_updated_unix: state.updated_at_unix,
        },
        persisted,
        current_repo_task,
    }
}

fn select_repo_task(tasks: &[SyncTaskRecord], repo_id: &str) -> Option<SyncTaskRecord> {
    tasks
        .iter()
        .filter(|task| task.repo_id == repo_id && task.status == SyncTaskStatus::Running)
        .max_by_key(|task| task.updated_at_unix)
        .cloned()
        .or_else(|| {
            tasks
                .iter()
                .filter(|task| task.repo_id == repo_id && task.status == SyncTaskStatus::Queued)
                .min_by_key(|task| task.queue_position.unwrap_or(u64::MAX))
                .cloned()
        })
        .or_else(|| {
            tasks
                .iter()
                .filter(|task| task.repo_id == repo_id)
                .max_by_key(|task| task.updated_at_unix)
                .cloned()
        })
}

pub(super) fn sync_task_mode_from_host(mode: &SyncMode) -> SyncTaskMode {
    match mode {
        SyncMode::Auto => SyncTaskMode::Auto,
        SyncMode::Full => SyncTaskMode::Full,
        SyncMode::Paths(paths) => SyncTaskMode::Paths {
            paths: normalize_paths(paths),
        },
        SyncMode::Repair => SyncTaskMode::Repair,
        SyncMode::Validate => SyncTaskMode::Validate,
    }
}

pub(super) fn sync_task_mode_to_host(mode: &SyncTaskMode) -> SyncMode {
    match mode {
        SyncTaskMode::Auto => SyncMode::Auto,
        SyncTaskMode::Full => SyncMode::Full,
        SyncTaskMode::Paths { paths } => SyncMode::Paths(paths.clone()),
        SyncTaskMode::Repair => SyncMode::Repair,
        SyncTaskMode::Validate => SyncMode::Validate,
    }
}

fn normalize_paths(paths: &[String]) -> Vec<String> {
    let mut paths = paths
        .iter()
        .map(|path| normalize_repo_path(path))
        .filter(|path| !path.is_empty())
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    paths
}

fn normalize_repo_path(path: &str) -> String {
    let mut normalized = path.trim().replace('\\', "/");
    while normalized.starts_with("./") {
        normalized = normalized[2..].to_string();
    }
    normalized.trim_start_matches('/').to_string()
}

fn is_full_like(mode: &SyncTaskMode) -> bool {
    matches!(mode, SyncTaskMode::Auto | SyncTaskMode::Full)
}

fn is_weaker_than_repair(mode: &SyncTaskMode) -> bool {
    matches!(
        mode,
        SyncTaskMode::Auto | SyncTaskMode::Full | SyncTaskMode::Paths { .. }
    )
}

pub(super) fn progress_from_summary(summary: &SyncSummary) -> SyncProgressUpdate {
    let total = summary.paths_unchanged
        + summary.paths_added
        + summary.paths_changed
        + summary.paths_removed;
    SyncProgressUpdate {
        phase: SyncProgressPhase::Complete,
        current_path: None,
        paths_total: total,
        paths_completed: total,
        paths_remaining: 0,
        paths_unchanged: summary.paths_unchanged,
        paths_added: summary.paths_added,
        paths_changed: summary.paths_changed,
        paths_removed: summary.paths_removed,
        cache_hits: summary.cache_hits,
        cache_misses: summary.cache_misses,
        parse_errors: summary.parse_errors,
    }
}

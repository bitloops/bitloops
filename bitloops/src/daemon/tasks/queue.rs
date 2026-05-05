use std::collections::{HashMap, HashSet};

use crate::host::devql::{
    DevqlConfig, IngestionCounters, IngestionProgressPhase, IngestionProgressUpdate, SyncMode,
    SyncProgressPhase, SyncProgressUpdate, SyncSummary,
};

use super::super::types::{
    DevqlTaskKind, DevqlTaskKindCounts, DevqlTaskProgress, DevqlTaskQueueState,
    DevqlTaskQueueStatus, DevqlTaskRecord, DevqlTaskSource, DevqlTaskSpec, DevqlTaskStatus,
    SyncTaskMode, SyncTaskSpec, unix_timestamp_now,
};
use super::state::PersistedDevqlTaskQueueState;

const MAX_TERMINAL_TASKS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TaskLaneKey {
    repo_id: String,
    kind: DevqlTaskKind,
}

pub(super) fn changed_tasks(
    previous: &[DevqlTaskRecord],
    current: &[DevqlTaskRecord],
) -> Vec<DevqlTaskRecord> {
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
    state: &mut PersistedDevqlTaskQueueState,
    cfg: &DevqlConfig,
    source: DevqlTaskSource,
    kind: DevqlTaskKind,
    spec: &DevqlTaskSpec,
    init_session_id: Option<&str>,
) -> Option<DevqlTaskRecord> {
    if kind == DevqlTaskKind::EmbeddingsBootstrap {
        return None;
    }
    if kind == DevqlTaskKind::SummaryBootstrap {
        return None;
    }

    if kind == DevqlTaskKind::Ingest {
        if let Some(existing) = state.tasks.iter_mut().find(|task| {
            task.repo_id == cfg.repo.repo_id
                && task.kind == DevqlTaskKind::Ingest
                && matches!(
                    task.status,
                    DevqlTaskStatus::Queued | DevqlTaskStatus::Running
                )
                && task.init_session_id.as_deref() == init_session_id
                && task.spec == *spec
        }) {
            existing.updated_at_unix = unix_timestamp_now();
            existing.error = None;
            return Some(existing.clone());
        }
        return None;
    }

    let mode = sync_spec_from_task_spec(spec).map(|spec| &spec.mode)?;
    if *mode != SyncTaskMode::Validate
        && let Some(existing) = state.tasks.iter_mut().find(|task| {
            task.repo_id == cfg.repo.repo_id
                && task.kind == DevqlTaskKind::Sync
                && matches!(
                    task.status,
                    DevqlTaskStatus::Queued | DevqlTaskStatus::Running
                )
                && task.init_session_id.as_deref() == init_session_id
                && (source != DevqlTaskSource::RepoPolicyChange
                    || task.status == DevqlTaskStatus::Queued)
                && sync_spec_from_task_spec(&task.spec).is_some_and(|existing_spec| {
                    !should_keep_distinct_producer_sources(
                        task.source,
                        source,
                        &existing_spec.mode,
                        mode,
                    ) && sync_specs_have_compatible_snapshots(
                        existing_spec,
                        sync_spec_from_task_spec(spec).expect("sync spec"),
                    ) && match (&existing_spec.mode, mode) {
                        (SyncTaskMode::Repair, _) => true,
                        (existing_mode, incoming_mode)
                            if is_full_like(existing_mode)
                                && is_weaker_than_repair(incoming_mode) =>
                        {
                            true
                        }
                        (SyncTaskMode::Paths { .. }, incoming_mode)
                            if task.status == DevqlTaskStatus::Queued
                                && is_stronger_than_paths(incoming_mode) =>
                        {
                            true
                        }
                        _ => false,
                    }
                })
        })
    {
        let mut use_incoming_source = source == DevqlTaskSource::RepoPolicyChange;
        if let Some(existing_spec) = sync_spec_from_task_spec_mut(&mut existing.spec) {
            use_incoming_source |= existing.source != DevqlTaskSource::RepoPolicyChange
                && matches!(existing_spec.mode, SyncTaskMode::Paths { .. })
                && is_stronger_than_paths(mode);
            existing_spec.mode = merge_sync_modes(&existing_spec.mode, mode);
            merge_sync_snapshot_metadata(
                existing_spec,
                sync_spec_from_task_spec(spec).expect("sync spec"),
            );
        }
        if use_incoming_source {
            existing.source = source;
        }
        existing.updated_at_unix = unix_timestamp_now();
        existing.error = None;
        return Some(existing.clone());
    }

    if let SyncTaskMode::Paths { paths } = mode
        && let Some(existing) = state.tasks.iter_mut().find(|task| {
            task.repo_id == cfg.repo.repo_id
                && task.kind == DevqlTaskKind::Sync
                && task.status == DevqlTaskStatus::Queued
                && task.init_session_id.as_deref() == init_session_id
                && sync_spec_from_task_spec(&task.spec).is_some_and(|existing| {
                    !should_keep_distinct_producer_sources(
                        task.source,
                        source,
                        &existing.mode,
                        mode,
                    ) && matches!(existing.mode, SyncTaskMode::Paths { .. })
                        && sync_specs_have_compatible_snapshots(
                            existing,
                            sync_spec_from_task_spec(spec).expect("sync spec"),
                        )
                })
        })
    {
        if let Some(SyncTaskSpec {
            mode: SyncTaskMode::Paths {
                paths: existing_paths,
            },
            post_commit_snapshot,
        }) = sync_spec_from_task_spec_mut(&mut existing.spec)
        {
            existing_paths.extend(paths.iter().cloned());
            existing_paths.sort();
            existing_paths.dedup();
            if let (Some(existing_snapshot), Some(incoming_snapshot)) = (
                post_commit_snapshot.as_mut(),
                sync_spec_from_task_spec(spec)
                    .and_then(|sync_spec| sync_spec.post_commit_snapshot.as_ref()),
            ) {
                existing_snapshot
                    .changed_paths
                    .extend(incoming_snapshot.changed_paths.iter().cloned());
                existing_snapshot.changed_paths.sort();
                existing_snapshot.changed_paths.dedup();
            }
        }
        existing.updated_at_unix = unix_timestamp_now();
        existing.error = None;
        return Some(existing.clone());
    }

    None
}

#[cfg(test)]
pub(super) fn next_runnable_task_indexes(state: &PersistedDevqlTaskQueueState) -> Vec<usize> {
    next_runnable_task_indexes_blocking_repo_ids(state, &HashSet::new())
}

pub(super) fn next_runnable_task_indexes_blocking_repo_ids(
    state: &PersistedDevqlTaskQueueState,
    blocked_repo_ids: &HashSet<String>,
) -> Vec<usize> {
    let paused_repo_ids = state
        .repo_controls
        .iter()
        .filter(|(_, control)| control.paused)
        .map(|(repo_id, _)| repo_id.as_str())
        .collect::<HashSet<_>>();
    let repo_policy_change_blocked_repo_ids = state
        .tasks
        .iter()
        .filter(|task| {
            task.kind == DevqlTaskKind::Sync
                && task.source == DevqlTaskSource::RepoPolicyChange
                && matches!(
                    task.status,
                    DevqlTaskStatus::Queued | DevqlTaskStatus::Running
                )
        })
        .map(|task| task.repo_id.as_str())
        .collect::<HashSet<_>>();
    let running_lanes = state
        .tasks
        .iter()
        .filter(|task| task.status == DevqlTaskStatus::Running)
        .map(task_lane)
        .collect::<HashSet<_>>();
    let running_repo_ids = state
        .tasks
        .iter()
        .filter(|task| task.status == DevqlTaskStatus::Running)
        .map(|task| task.repo_id.as_str())
        .collect::<HashSet<_>>();

    let mut selected = HashMap::<TaskLaneKey, (usize, (u8, u64, usize))>::new();
    for (index, task) in state.tasks.iter().enumerate() {
        if task.status != DevqlTaskStatus::Queued || paused_repo_ids.contains(task.repo_id.as_str())
        {
            continue;
        }
        if blocked_repo_ids.contains(&task.repo_id) {
            continue;
        }
        if repo_policy_change_blocked_repo_ids.contains(task.repo_id.as_str())
            && !(task.kind == DevqlTaskKind::Sync
                && task.source == DevqlTaskSource::RepoPolicyChange)
        {
            continue;
        }
        if task.kind == DevqlTaskKind::Sync
            && task.source == DevqlTaskSource::RepoPolicyChange
            && running_repo_ids.contains(task.repo_id.as_str())
        {
            continue;
        }
        let lane = task_lane(task);
        if running_lanes.contains(&lane) {
            continue;
        }
        let key = pending_sort_key(index, task);
        selected
            .entry(lane)
            .and_modify(|(existing_index, existing_key)| {
                if key < *existing_key {
                    *existing_index = index;
                    *existing_key = key;
                }
            })
            .or_insert((index, key));
    }

    let mut selected = selected.into_values().collect::<Vec<_>>();
    selected.sort_by_key(|(_, key)| *key);
    selected.into_iter().map(|(index, _)| index).collect()
}

pub(super) fn post_commit_derivation_claim_guards(
    state: &PersistedDevqlTaskQueueState,
) -> crate::host::devql::PostCommitDerivationClaimGuards {
    let mut guards = crate::host::devql::PostCommitDerivationClaimGuards::default();
    for task in state.tasks.iter().filter(|task| {
        task.kind == DevqlTaskKind::Sync && task.source == DevqlTaskSource::PostCommit
    }) {
        let Some(snapshot) = task
            .sync_spec()
            .and_then(|spec| spec.post_commit_snapshot.as_ref())
        else {
            continue;
        };
        let key = (task.repo_id.clone(), snapshot.commit_sha.clone());
        match task.status {
            DevqlTaskStatus::Queued | DevqlTaskStatus::Running => {
                guards.blocked.insert(key);
            }
            DevqlTaskStatus::Failed | DevqlTaskStatus::Cancelled => {
                guards.abandoned.insert(key);
            }
            DevqlTaskStatus::Completed => {}
        }
    }
    guards
}

fn pending_sort_key(index: usize, task: &DevqlTaskRecord) -> (u8, u64, usize) {
    (
        if task.kind == DevqlTaskKind::Sync && task.source == DevqlTaskSource::RepoPolicyChange {
            0
        } else if is_git_hook_sync_task(task) {
            1
        } else if matches!(
            task.kind,
            DevqlTaskKind::EmbeddingsBootstrap | DevqlTaskKind::SummaryBootstrap
        ) {
            3
        } else if task.kind == DevqlTaskKind::Sync
            && task
                .sync_spec()
                .is_some_and(|spec| matches!(spec.mode, SyncTaskMode::Validate))
        {
            4
        } else {
            2
        },
        task.submitted_at_unix,
        index,
    )
}

fn is_git_hook_sync_task(task: &DevqlTaskRecord) -> bool {
    task.kind == DevqlTaskKind::Sync
        && matches!(
            task.source,
            DevqlTaskSource::PostCheckout
                | DevqlTaskSource::PostCommit
                | DevqlTaskSource::PostMerge
        )
        && task
            .sync_spec()
            .is_none_or(|spec| !matches!(spec.mode, SyncTaskMode::Validate))
}

fn should_keep_distinct_producer_sources(
    existing: DevqlTaskSource,
    incoming: DevqlTaskSource,
    existing_mode: &SyncTaskMode,
    incoming_mode: &SyncTaskMode,
) -> bool {
    if existing == incoming {
        return false;
    }
    if existing == DevqlTaskSource::RepoPolicyChange
        || incoming == DevqlTaskSource::RepoPolicyChange
    {
        return false;
    }
    if !is_background_producer_source(existing) || !is_background_producer_source(incoming) {
        return false;
    }
    if existing == DevqlTaskSource::PostCheckout
        && incoming == DevqlTaskSource::Watcher
        && is_full_like(existing_mode)
    {
        return false;
    }
    if existing == DevqlTaskSource::Watcher
        && incoming == DevqlTaskSource::PostCheckout
        && matches!(existing_mode, SyncTaskMode::Paths { .. })
        && is_stronger_than_paths(incoming_mode)
    {
        return false;
    }
    true
}

fn is_background_producer_source(source: DevqlTaskSource) -> bool {
    matches!(
        source,
        DevqlTaskSource::Watcher
            | DevqlTaskSource::PostCheckout
            | DevqlTaskSource::PostCommit
            | DevqlTaskSource::PostMerge
    )
}

pub(super) fn recompute_queue_positions(tasks: &mut [DevqlTaskRecord]) {
    for task in tasks.iter_mut() {
        task.queue_position = None;
        task.tasks_ahead = None;
    }

    let lanes = tasks
        .iter()
        .filter(|task| {
            matches!(
                task.status,
                DevqlTaskStatus::Running | DevqlTaskStatus::Queued
            )
        })
        .map(task_lane)
        .collect::<HashSet<_>>();

    for lane in lanes {
        let mut order = tasks
            .iter()
            .enumerate()
            .filter(|(_, task)| {
                task.repo_id == lane.repo_id
                    && task.kind == lane.kind
                    && matches!(
                        task.status,
                        DevqlTaskStatus::Running | DevqlTaskStatus::Queued
                    )
            })
            .map(|(index, task)| {
                (
                    index,
                    task.status == DevqlTaskStatus::Running,
                    pending_sort_key(index, task),
                )
            })
            .collect::<Vec<_>>();
        order.sort_by(
            |(_, left_running, left_key), (_, right_running, right_key)| {
                left_running
                    .cmp(right_running)
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
}

pub(super) fn prune_terminal_tasks(
    tasks: &mut Vec<DevqlTaskRecord>,
    protected_task_ids: &HashSet<String>,
) {
    let mut terminal = tasks
        .iter()
        .filter(|task| {
            matches!(
                task.status,
                DevqlTaskStatus::Completed | DevqlTaskStatus::Failed | DevqlTaskStatus::Cancelled
            ) && !protected_task_ids.contains(&task.task_id)
        })
        .cloned()
        .collect::<Vec<_>>();
    terminal.sort_by_key(|task| std::cmp::Reverse(task.updated_at_unix));
    terminal.truncate(MAX_TERMINAL_TASKS);

    let terminal_ids = terminal
        .into_iter()
        .map(|task| task.task_id)
        .collect::<HashSet<_>>();
    tasks.retain(|task| {
        !matches!(
            task.status,
            DevqlTaskStatus::Completed | DevqlTaskStatus::Failed | DevqlTaskStatus::Cancelled
        ) || terminal_ids.contains(&task.task_id)
            || protected_task_ids.contains(&task.task_id)
    });
}

pub(super) fn project_status(
    state: &PersistedDevqlTaskQueueState,
    repo_id: Option<&str>,
    persisted: bool,
) -> DevqlTaskQueueStatus {
    let queued_tasks = state
        .tasks
        .iter()
        .filter(|task| task.status == DevqlTaskStatus::Queued)
        .count() as u64;
    let running_tasks = state
        .tasks
        .iter()
        .filter(|task| task.status == DevqlTaskStatus::Running)
        .count() as u64;
    let failed_tasks = state
        .tasks
        .iter()
        .filter(|task| task.status == DevqlTaskStatus::Failed)
        .count() as u64;
    let completed_recent_tasks = state
        .tasks
        .iter()
        .filter(|task| task.status == DevqlTaskStatus::Completed)
        .count() as u64;

    DevqlTaskQueueStatus {
        state: DevqlTaskQueueState {
            version: state.version,
            queued_tasks,
            running_tasks,
            failed_tasks,
            completed_recent_tasks,
            by_kind: counts_by_kind(&state.tasks),
            last_action: state.last_action.clone(),
            last_updated_unix: state.updated_at_unix,
        },
        persisted,
        current_repo_tasks: repo_id
            .map(|repo_id| select_repo_tasks(&state.tasks, repo_id))
            .unwrap_or_default(),
        current_repo_control: repo_id.and_then(|repo_id| state.repo_controls.get(repo_id).cloned()),
    }
}

fn counts_by_kind(tasks: &[DevqlTaskRecord]) -> Vec<DevqlTaskKindCounts> {
    let mut counts = HashMap::<DevqlTaskKind, DevqlTaskKindCounts>::new();
    for kind in [
        DevqlTaskKind::Sync,
        DevqlTaskKind::Ingest,
        DevqlTaskKind::EmbeddingsBootstrap,
        DevqlTaskKind::SummaryBootstrap,
    ] {
        counts.insert(
            kind,
            DevqlTaskKindCounts {
                kind,
                queued_tasks: 0,
                running_tasks: 0,
                failed_tasks: 0,
                completed_recent_tasks: 0,
            },
        );
    }

    for task in tasks {
        let entry = counts.get_mut(&task.kind).expect("kind counts initialised");
        match task.status {
            DevqlTaskStatus::Queued => entry.queued_tasks += 1,
            DevqlTaskStatus::Running => entry.running_tasks += 1,
            DevqlTaskStatus::Failed => entry.failed_tasks += 1,
            DevqlTaskStatus::Completed => entry.completed_recent_tasks += 1,
            DevqlTaskStatus::Cancelled => {}
        }
    }

    let mut counts = counts.into_values().collect::<Vec<_>>();
    counts.sort_by_key(|entry| entry.kind);
    counts
}

fn select_repo_tasks(tasks: &[DevqlTaskRecord], repo_id: &str) -> Vec<DevqlTaskRecord> {
    [
        DevqlTaskKind::Sync,
        DevqlTaskKind::Ingest,
        DevqlTaskKind::EmbeddingsBootstrap,
        DevqlTaskKind::SummaryBootstrap,
    ]
    .into_iter()
    .filter_map(|kind| select_repo_task(tasks, repo_id, kind))
    .collect()
}

fn select_repo_task(
    tasks: &[DevqlTaskRecord],
    repo_id: &str,
    kind: DevqlTaskKind,
) -> Option<DevqlTaskRecord> {
    tasks
        .iter()
        .filter(|task| {
            task.repo_id == repo_id && task.kind == kind && task.status == DevqlTaskStatus::Running
        })
        .max_by_key(|task| task.updated_at_unix)
        .cloned()
        .or_else(|| {
            tasks
                .iter()
                .filter(|task| {
                    task.repo_id == repo_id
                        && task.kind == kind
                        && task.status == DevqlTaskStatus::Queued
                })
                .min_by_key(|task| task.queue_position.unwrap_or(u64::MAX))
                .cloned()
        })
        .or_else(|| {
            tasks
                .iter()
                .filter(|task| task.repo_id == repo_id && task.kind == kind)
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

pub(super) fn sync_progress_from_summary(summary: &SyncSummary) -> SyncProgressUpdate {
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

pub(super) fn ingest_progress_from_summary(summary: &IngestionCounters) -> IngestionProgressUpdate {
    IngestionProgressUpdate {
        phase: IngestionProgressPhase::Complete,
        commits_total: summary.commits_processed,
        commits_processed: summary.commits_processed,
        current_checkpoint_id: None,
        current_commit_sha: None,
        counters: summary.clone(),
    }
}

pub(super) fn default_progress_for_spec(spec: &DevqlTaskSpec) -> DevqlTaskProgress {
    match spec {
        DevqlTaskSpec::Sync(_) => DevqlTaskProgress::Sync(SyncProgressUpdate::default()),
        DevqlTaskSpec::Ingest(_) => DevqlTaskProgress::Ingest(IngestionProgressUpdate {
            phase: IngestionProgressPhase::Initializing,
            commits_total: 0,
            commits_processed: 0,
            current_checkpoint_id: None,
            current_commit_sha: None,
            counters: IngestionCounters::default(),
        }),
        DevqlTaskSpec::EmbeddingsBootstrap(_) => DevqlTaskProgress::EmbeddingsBootstrap(
            crate::daemon::EmbeddingsBootstrapProgress::default(),
        ),
        DevqlTaskSpec::SummaryBootstrap(_) => {
            DevqlTaskProgress::SummaryBootstrap(crate::daemon::SummaryBootstrapProgress::default())
        }
    }
}

pub(super) fn failed_progress(progress: &DevqlTaskProgress) -> DevqlTaskProgress {
    match progress {
        DevqlTaskProgress::Sync(progress) => {
            let mut progress = progress.clone();
            progress.phase = SyncProgressPhase::Failed;
            DevqlTaskProgress::Sync(progress)
        }
        DevqlTaskProgress::Ingest(progress) => {
            let mut progress = progress.clone();
            progress.phase = IngestionProgressPhase::Failed;
            DevqlTaskProgress::Ingest(progress)
        }
        DevqlTaskProgress::EmbeddingsBootstrap(progress) => {
            let mut progress = progress.clone();
            progress.phase = crate::daemon::EmbeddingsBootstrapPhase::Failed;
            DevqlTaskProgress::EmbeddingsBootstrap(progress)
        }
        DevqlTaskProgress::SummaryBootstrap(progress) => {
            let mut progress = progress.clone();
            progress.phase = crate::daemon::SummaryBootstrapPhase::Failed;
            DevqlTaskProgress::SummaryBootstrap(progress)
        }
    }
}

pub(super) fn sync_spec_from_task_spec(spec: &DevqlTaskSpec) -> Option<&SyncTaskSpec> {
    match spec {
        DevqlTaskSpec::Sync(spec) => Some(spec),
        DevqlTaskSpec::Ingest(_)
        | DevqlTaskSpec::EmbeddingsBootstrap(_)
        | DevqlTaskSpec::SummaryBootstrap(_) => None,
    }
}

fn sync_spec_from_task_spec_mut(spec: &mut DevqlTaskSpec) -> Option<&mut SyncTaskSpec> {
    match spec {
        DevqlTaskSpec::Sync(spec) => Some(spec),
        DevqlTaskSpec::Ingest(_)
        | DevqlTaskSpec::EmbeddingsBootstrap(_)
        | DevqlTaskSpec::SummaryBootstrap(_) => None,
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

fn is_stronger_than_paths(mode: &SyncTaskMode) -> bool {
    matches!(
        mode,
        SyncTaskMode::Auto | SyncTaskMode::Full | SyncTaskMode::Repair
    )
}

fn merge_sync_modes(existing: &SyncTaskMode, incoming: &SyncTaskMode) -> SyncTaskMode {
    match (existing, incoming) {
        (SyncTaskMode::Repair, _) | (_, SyncTaskMode::Repair) => SyncTaskMode::Repair,
        (SyncTaskMode::Full, _) | (_, SyncTaskMode::Full) => SyncTaskMode::Full,
        (SyncTaskMode::Auto, _) => SyncTaskMode::Auto,
        (_, mode) => mode.clone(),
    }
}

fn sync_specs_have_compatible_snapshots(existing: &SyncTaskSpec, incoming: &SyncTaskSpec) -> bool {
    match (
        &existing.post_commit_snapshot,
        &incoming.post_commit_snapshot,
    ) {
        (None, None) => true,
        (Some(existing_snapshot), Some(incoming_snapshot)) => {
            existing_snapshot.commit_sha == incoming_snapshot.commit_sha
        }
        _ => false,
    }
}

fn merge_sync_snapshot_metadata(existing: &mut SyncTaskSpec, incoming: &SyncTaskSpec) {
    match (
        existing.post_commit_snapshot.as_mut(),
        incoming.post_commit_snapshot.as_ref(),
    ) {
        (None, Some(incoming_snapshot)) => {
            existing.post_commit_snapshot = Some(incoming_snapshot.clone());
        }
        (Some(existing_snapshot), Some(incoming_snapshot))
            if existing_snapshot.commit_sha == incoming_snapshot.commit_sha =>
        {
            existing_snapshot
                .changed_paths
                .extend(incoming_snapshot.changed_paths.iter().cloned());
            existing_snapshot.changed_paths.sort();
            existing_snapshot.changed_paths.dedup();
        }
        _ => {}
    }
}

fn task_lane(task: &DevqlTaskRecord) -> TaskLaneKey {
    TaskLaneKey {
        repo_id: task.repo_id.clone(),
        kind: task.kind,
    }
}

#[cfg(test)]
#[path = "queue_tests.rs"]
mod tests;

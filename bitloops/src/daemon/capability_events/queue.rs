use std::collections::HashSet;

use crate::daemon::{
    CapabilityEventQueueState, CapabilityEventQueueStatus, CapabilityEventRunRecord,
    CapabilityEventRunStatus,
};
use crate::host::runtime_store::PersistedCapabilityEventQueueState;

const MAX_TERMINAL_RUNS: usize = 64;

pub(super) fn next_pending_run_index(state: &PersistedCapabilityEventQueueState) -> Option<usize> {
    let running_lanes = running_lane_keys(&state.runs);
    state
        .runs
        .iter()
        .enumerate()
        .filter(|(_, run)| run.status == CapabilityEventRunStatus::Queued)
        .filter(|(_, run)| !running_lanes.contains(run.lane_key.as_str()))
        .min_by_key(|(index, run)| (run.submitted_at_unix, *index))
        .map(|(index, _)| index)
}

pub(super) fn running_lane_keys(runs: &[CapabilityEventRunRecord]) -> HashSet<&str> {
    runs.iter()
        .filter(|run| run.status == CapabilityEventRunStatus::Running)
        .map(|run| run.lane_key.as_str())
        .collect()
}

pub(super) fn prune_terminal_runs(runs: &mut Vec<CapabilityEventRunRecord>) {
    let mut terminal = runs
        .iter()
        .filter(|run| {
            matches!(
                run.status,
                CapabilityEventRunStatus::Completed
                    | CapabilityEventRunStatus::Failed
                    | CapabilityEventRunStatus::Cancelled
            )
        })
        .cloned()
        .collect::<Vec<_>>();
    terminal.sort_by(|left, right| right.updated_at_unix.cmp(&left.updated_at_unix));
    terminal.truncate(MAX_TERMINAL_RUNS);

    let terminal_ids = terminal
        .into_iter()
        .map(|run| run.run_id)
        .collect::<HashSet<_>>();
    runs.retain(|run| {
        !matches!(
            run.status,
            CapabilityEventRunStatus::Completed
                | CapabilityEventRunStatus::Failed
                | CapabilityEventRunStatus::Cancelled
        ) || terminal_ids.contains(&run.run_id)
    });
}

pub(super) fn project_status(
    state: &PersistedCapabilityEventQueueState,
    repo_id: Option<&str>,
    persisted: bool,
) -> CapabilityEventQueueStatus {
    let pending_runs = state
        .runs
        .iter()
        .filter(|run| run.status == CapabilityEventRunStatus::Queued)
        .count() as u64;
    let running_runs = state
        .runs
        .iter()
        .filter(|run| run.status == CapabilityEventRunStatus::Running)
        .count() as u64;
    let failed_runs = state
        .runs
        .iter()
        .filter(|run| run.status == CapabilityEventRunStatus::Failed)
        .count() as u64;
    let completed_recent_runs = state
        .runs
        .iter()
        .filter(|run| run.status == CapabilityEventRunStatus::Completed)
        .count() as u64;
    let current_repo_run = repo_id.and_then(|repo_id| select_repo_run(&state.runs, repo_id));

    CapabilityEventQueueStatus {
        state: CapabilityEventQueueState {
            version: state.version,
            pending_runs,
            running_runs,
            failed_runs,
            completed_recent_runs,
            last_action: state.last_action.clone(),
            last_updated_unix: state.updated_at_unix,
        },
        persisted,
        current_repo_run,
    }
}

fn select_repo_run(
    runs: &[CapabilityEventRunRecord],
    repo_id: &str,
) -> Option<CapabilityEventRunRecord> {
    runs.iter()
        .filter(|run| run.repo_id == repo_id && run.status == CapabilityEventRunStatus::Running)
        .max_by_key(|run| run.updated_at_unix)
        .cloned()
        .or_else(|| {
            runs.iter()
                .enumerate()
                .filter(|run| {
                    run.1.repo_id == repo_id && run.1.status == CapabilityEventRunStatus::Queued
                })
                .min_by_key(|(index, run)| (run.submitted_at_unix, *index))
                .map(|(_, run)| run)
                .cloned()
        })
        .or_else(|| {
            runs.iter()
                .filter(|run| run.repo_id == repo_id)
                .max_by_key(|run| run.updated_at_unix)
                .cloned()
        })
}

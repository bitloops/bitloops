pub(crate) fn task_for_lane<'a>(
    snapshot: &'a crate::cli::devql::graphql::RuntimeSnapshotGraphqlRecord,
    lane: &crate::cli::devql::graphql::RuntimeInitLaneGraphqlRecord,
) -> Option<&'a crate::cli::devql::graphql::TaskGraphqlRecord> {
    lane.task_id
        .as_deref()
        .and_then(|task_id| task_by_id(snapshot, task_id))
}

pub(crate) fn task_by_id<'a>(
    snapshot: &'a crate::cli::devql::graphql::RuntimeSnapshotGraphqlRecord,
    task_id: &str,
) -> Option<&'a crate::cli::devql::graphql::TaskGraphqlRecord> {
    snapshot
        .task_queue
        .current_repo_tasks
        .iter()
        .find(|task| task.task_id == task_id)
}

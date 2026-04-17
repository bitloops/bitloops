use anyhow::Result;

use crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CAPABILITY_ID;
use crate::capability_packs::semantic_clones::types::{
    SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX, SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
};
use crate::devql_transport::SlimCliRepoScope;
use crate::host::capability_host::gateways::CapabilityMailboxStatus;
use crate::host::runtime_store::RepoSqliteRuntimeStore;

use super::EmbeddingQueueSnapshot;

pub(super) async fn current_embedding_queue_snapshot(
    repo_root: &std::path::Path,
) -> Result<Option<EmbeddingQueueSnapshot>> {
    let daemon_status = crate::daemon::status().await?;
    let Some(enrichment) = daemon_status.enrichment else {
        return Ok(None);
    };

    let repo_snapshot = RepoSqliteRuntimeStore::open(repo_root)
        .ok()
        .and_then(|store| {
            store
                .load_capability_workplane_mailbox_status(
                    SEMANTIC_CLONES_CAPABILITY_ID,
                    [
                        SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
                        SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
                    ],
                )
                .ok()
        })
        .map(|status_by_mailbox| snapshot_from_mailbox_statuses(status_by_mailbox.into_values()));

    if let Some(snapshot) = repo_snapshot {
        return Ok(Some(snapshot));
    }

    Ok(Some(EmbeddingQueueSnapshot {
        pending: enrichment.state.pending_embedding_jobs,
        running: enrichment.state.running_embedding_jobs,
        failed: enrichment.state.failed_embedding_jobs,
        completed: 0,
    }))
}

pub(super) async fn current_summary_queue_snapshot(
    repo_root: &std::path::Path,
) -> Result<Option<EmbeddingQueueSnapshot>> {
    let daemon_status = crate::daemon::status().await?;
    let Some(enrichment) = daemon_status.enrichment else {
        return Ok(None);
    };

    let repo_snapshot = RepoSqliteRuntimeStore::open(repo_root)
        .ok()
        .and_then(|store| {
            store
                .load_capability_workplane_mailbox_status(
                    SEMANTIC_CLONES_CAPABILITY_ID,
                    [SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX],
                )
                .ok()
        })
        .map(|status_by_mailbox| snapshot_from_mailbox_statuses(status_by_mailbox.into_values()));

    if let Some(snapshot) = repo_snapshot {
        return Ok(Some(snapshot));
    }

    Ok(Some(EmbeddingQueueSnapshot {
        pending: enrichment.state.pending_semantic_jobs,
        running: enrichment.state.running_semantic_jobs,
        failed: enrichment.state.failed_semantic_jobs,
        completed: 0,
    }))
}

fn snapshot_from_mailbox_statuses(
    status_by_mailbox: impl IntoIterator<Item = CapabilityMailboxStatus>,
) -> EmbeddingQueueSnapshot {
    status_by_mailbox.into_iter().fold(
        EmbeddingQueueSnapshot {
            pending: 0,
            running: 0,
            failed: 0,
            completed: 0,
        },
        |mut snapshot, status| {
            snapshot.pending += status.pending_jobs + status.pending_cursor_runs;
            snapshot.running += status.running_jobs + status.running_cursor_runs;
            snapshot.failed += status.failed_jobs + status.failed_cursor_runs;
            snapshot.completed +=
                status.completed_recent_jobs + status.completed_recent_cursor_runs;
            snapshot
        },
    )
}

pub(super) async fn refresh_init_progress_task(
    scope: &SlimCliRepoScope,
    current: &crate::cli::devql::graphql::TaskGraphqlRecord,
) -> Result<Option<crate::cli::devql::graphql::TaskGraphqlRecord>> {
    if let Some(task) =
        crate::cli::devql::graphql::query_task_via_graphql(scope, current.task_id.as_str()).await?
    {
        return Ok(Some(task));
    }

    let kind = if current.is_sync() {
        Some("sync")
    } else if current.is_ingest() {
        Some("ingest")
    } else if current.is_embeddings_bootstrap() {
        Some("embeddings_bootstrap")
    } else {
        None
    };
    let Some(kind) = kind else {
        return Ok(None);
    };

    let tasks =
        crate::cli::devql::graphql::list_tasks_via_graphql(scope, Some(kind), None, Some(16))
            .await?;
    Ok(tasks
        .into_iter()
        .find(|task| task.task_id == current.task_id))
}

#[cfg(test)]
mod tests {
    use super::snapshot_from_mailbox_statuses;
    use crate::host::capability_host::gateways::CapabilityMailboxStatus;

    #[test]
    fn mailbox_status_snapshot_uses_repo_scoped_job_and_cursor_counts() {
        let snapshot = snapshot_from_mailbox_statuses([
            CapabilityMailboxStatus {
                pending_jobs: 10,
                running_jobs: 2,
                failed_jobs: 1,
                completed_recent_jobs: 30,
                pending_cursor_runs: 3,
                running_cursor_runs: 4,
                failed_cursor_runs: 0,
                completed_recent_cursor_runs: 5,
                intent_active: true,
                blocked_reason: None,
            },
            CapabilityMailboxStatus {
                pending_jobs: 7,
                running_jobs: 1,
                failed_jobs: 2,
                completed_recent_jobs: 11,
                pending_cursor_runs: 0,
                running_cursor_runs: 1,
                failed_cursor_runs: 1,
                completed_recent_cursor_runs: 2,
                intent_active: true,
                blocked_reason: None,
            },
        ]);

        assert_eq!(snapshot.pending, 20);
        assert_eq!(snapshot.running, 8);
        assert_eq!(snapshot.failed, 4);
        assert_eq!(snapshot.completed, 48);
    }
}

use anyhow::Result;

use crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CAPABILITY_ID;
use crate::capability_packs::semantic_clones::types::{
    SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX, SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
};
use crate::devql_transport::SlimCliRepoScope;
use crate::host::runtime_store::RepoSqliteRuntimeStore;

use super::EmbeddingQueueSnapshot;

pub(super) async fn current_embedding_queue_snapshot(
    repo_root: &std::path::Path,
) -> Result<Option<EmbeddingQueueSnapshot>> {
    let daemon_status = crate::daemon::status().await?;
    let Some(enrichment) = daemon_status.enrichment else {
        return Ok(None);
    };

    let completed = RepoSqliteRuntimeStore::open(repo_root)
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
        .map(|status_by_mailbox| {
            status_by_mailbox
                .into_values()
                .map(|status| status.completed_recent_jobs)
                .sum()
        })
        .unwrap_or_default();

    Ok(Some(EmbeddingQueueSnapshot {
        pending: enrichment.state.pending_embedding_jobs,
        running: enrichment.state.running_embedding_jobs,
        failed: enrichment.state.failed_embedding_jobs,
        completed,
    }))
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

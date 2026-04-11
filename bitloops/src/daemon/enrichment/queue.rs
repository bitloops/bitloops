use super::super::types::{
    EnrichmentQueueMode, EnrichmentQueueState as ProjectedEnrichmentQueueState,
};
use super::{EnrichmentJobKind, EnrichmentJobStatus, EnrichmentQueueState};
use crate::host::runtime_store::DaemonSqliteRuntimeStore;

pub(super) fn next_pending_job_index(
    state: &EnrichmentQueueState,
    runtime_store: &DaemonSqliteRuntimeStore,
) -> anyhow::Result<Option<usize>> {
    Ok(state
        .jobs
        .iter()
        .enumerate()
        .filter(|(_, job)| job.status == EnrichmentJobStatus::Pending)
        .filter(|(_, job)| !job_is_paused(state, &job.job))
        .filter(|(_, job)| {
            !matches!(
                job.job,
                EnrichmentJobKind::SymbolEmbeddings { .. }
                    | EnrichmentJobKind::CloneEdgesRebuild { .. }
            ) || !crate::daemon::embeddings_bootstrap::embeddings_blocked_for_config_root(
                runtime_store,
                &job.config_root,
            )
            .unwrap_or(true)
        })
        .min_by_key(|(_, job)| {
            let active_branch = state.active_branch_by_repo.get(&job.repo_id);
            let branch_rank = match active_branch {
                Some(active_branch) if active_branch == &job.branch => 0usize,
                Some(_) => 1usize,
                None => 0usize,
            };
            (
                branch_rank,
                job_kind_priority(&job.job),
                job.created_at_unix,
            )
        })
        .map(|(index, _)| index))
}

pub(super) fn job_is_paused(state: &EnrichmentQueueState, job: &EnrichmentJobKind) -> bool {
    match job {
        EnrichmentJobKind::SemanticSummaries { .. } => state.paused_semantic,
        EnrichmentJobKind::SymbolEmbeddings { .. }
        | EnrichmentJobKind::CloneEdgesRebuild { .. } => state.paused_embeddings,
    }
}

fn job_kind_priority(job: &EnrichmentJobKind) -> usize {
    match job {
        EnrichmentJobKind::SemanticSummaries { .. } => 0,
        EnrichmentJobKind::SymbolEmbeddings { .. } => 1,
        EnrichmentJobKind::CloneEdgesRebuild { .. } => 2,
    }
}

pub(super) fn project_status(state: &EnrichmentQueueState) -> ProjectedEnrichmentQueueState {
    let pending_semantic_jobs = count_jobs(state, EnrichmentJobStatus::Pending, |job| {
        matches!(job, EnrichmentJobKind::SemanticSummaries { .. })
    });
    let pending_embedding_jobs = count_jobs(state, EnrichmentJobStatus::Pending, |job| {
        matches!(job, EnrichmentJobKind::SymbolEmbeddings { .. })
    });
    let pending_clone_edges_rebuild_jobs = count_jobs(state, EnrichmentJobStatus::Pending, |job| {
        matches!(job, EnrichmentJobKind::CloneEdgesRebuild { .. })
    });
    let running_semantic_jobs = count_jobs(state, EnrichmentJobStatus::Running, |job| {
        matches!(job, EnrichmentJobKind::SemanticSummaries { .. })
    });
    let running_embedding_jobs = count_jobs(state, EnrichmentJobStatus::Running, |job| {
        matches!(job, EnrichmentJobKind::SymbolEmbeddings { .. })
    });
    let running_clone_edges_rebuild_jobs = count_jobs(state, EnrichmentJobStatus::Running, |job| {
        matches!(job, EnrichmentJobKind::CloneEdgesRebuild { .. })
    });
    let failed_semantic_jobs = count_jobs(state, EnrichmentJobStatus::Failed, |job| {
        matches!(job, EnrichmentJobKind::SemanticSummaries { .. })
    });
    let failed_embedding_jobs = count_jobs(state, EnrichmentJobStatus::Failed, |job| {
        matches!(job, EnrichmentJobKind::SymbolEmbeddings { .. })
    });
    let failed_clone_edges_rebuild_jobs = count_jobs(state, EnrichmentJobStatus::Failed, |job| {
        matches!(job, EnrichmentJobKind::CloneEdgesRebuild { .. })
    });

    ProjectedEnrichmentQueueState {
        version: state.version,
        mode: if state.paused_embeddings || state.paused_semantic {
            EnrichmentQueueMode::Paused
        } else {
            EnrichmentQueueMode::Running
        },
        pending_jobs: pending_semantic_jobs
            + pending_embedding_jobs
            + pending_clone_edges_rebuild_jobs,
        pending_semantic_jobs,
        pending_embedding_jobs,
        pending_clone_edges_rebuild_jobs,
        running_jobs: running_semantic_jobs
            + running_embedding_jobs
            + running_clone_edges_rebuild_jobs,
        running_semantic_jobs,
        running_embedding_jobs,
        running_clone_edges_rebuild_jobs,
        failed_jobs: failed_semantic_jobs + failed_embedding_jobs + failed_clone_edges_rebuild_jobs,
        failed_semantic_jobs,
        failed_embedding_jobs,
        failed_clone_edges_rebuild_jobs,
        retried_failed_jobs: state.retried_failed_jobs,
        last_action: state.last_action.clone(),
        last_updated_unix: state.updated_at_unix,
        paused_reason: state.paused_reason.clone(),
    }
}

fn count_jobs(
    state: &EnrichmentQueueState,
    status: EnrichmentJobStatus,
    predicate: impl Fn(&EnrichmentJobKind) -> bool,
) -> u64 {
    state
        .jobs
        .iter()
        .filter(|job| job.status == status && predicate(&job.job))
        .count() as u64
}

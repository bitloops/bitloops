use anyhow::Result;

use crate::config::resolve_repo_runtime_db_path_for_config_root;
use crate::daemon::types::{BlockedMailboxStatus, EnrichmentQueueMode, EnrichmentQueueStatus};
use crate::host::runtime_store::DaemonSqliteRuntimeStore;

use super::EnrichmentControlResult;
use super::worker_count::{EnrichmentWorkerBudgets, configured_enrichment_worker_budgets_for_repo};
use super::workplane::{
    compact_and_prune_workplane_jobs, current_workplane_mailbox_blocked_statuses,
    current_workplane_mailbox_blocked_statuses_for_repo, default_state,
    iter_workplane_job_config_roots, last_failed_embedding_job_from_workplane,
    migrate_legacy_semantic_workplane_rows, project_workplane_status,
    prune_failed_semantic_inbox_items, retry_failed_semantic_inbox_items,
    retry_failed_workplane_jobs,
};

pub(crate) fn snapshot() -> Result<EnrichmentQueueStatus> {
    let runtime_store = DaemonSqliteRuntimeStore::open()?;
    let daemon_config = crate::daemon::resolve_daemon_config(None)?;
    let workplane_store = DaemonSqliteRuntimeStore::open_at(
        resolve_repo_runtime_db_path_for_config_root(&daemon_config.config_root),
    )?;
    let state = runtime_store
        .load_enrichment_queue_state()?
        .unwrap_or_else(default_state);
    let projected = project_workplane_status(
        &workplane_store,
        &state,
        effective_worker_budgets(&workplane_store, &daemon_config.config_root)?,
    )?;
    let gate = crate::daemon::embeddings_bootstrap::gate_status_for_enrichment_queue(
        &runtime_store,
        iter_workplane_job_config_roots(&workplane_store)?,
    )?;
    Ok(EnrichmentQueueStatus {
        state: projected,
        persisted: runtime_store.enrichment_state_exists()?,
        embeddings_gate: gate,
        blocked_mailboxes: current_workplane_mailbox_blocked_statuses(
            &workplane_store,
            &runtime_store,
        )?,
        last_failed_embedding: last_failed_embedding_job_from_workplane(&workplane_store)?,
    })
}

pub(crate) fn blocked_mailboxes_for_repo(
    workplane_store: &DaemonSqliteRuntimeStore,
    runtime_store: &DaemonSqliteRuntimeStore,
    repo_id: &str,
) -> Result<Vec<BlockedMailboxStatus>> {
    current_workplane_mailbox_blocked_statuses_for_repo(workplane_store, runtime_store, repo_id)
}

pub(crate) fn retry_failed_jobs_in_store(
    workplane_store: &DaemonSqliteRuntimeStore,
) -> Result<u64> {
    let retried = retry_failed_workplane_jobs(workplane_store)?
        .saturating_add(retry_failed_semantic_inbox_items(workplane_store)?);
    migrate_legacy_semantic_workplane_rows(workplane_store)?;
    prune_failed_semantic_inbox_items(workplane_store)?;
    compact_and_prune_workplane_jobs(workplane_store)?;
    Ok(retried)
}

pub(crate) fn pause_enrichments(reason: Option<String>) -> Result<EnrichmentControlResult> {
    let runtime_store = DaemonSqliteRuntimeStore::open()?;
    let daemon_config = crate::daemon::resolve_daemon_config(None)?;
    let workplane_store = DaemonSqliteRuntimeStore::open_at(
        resolve_repo_runtime_db_path_for_config_root(&daemon_config.config_root),
    )?;
    let mut state = runtime_store
        .load_enrichment_queue_state()?
        .unwrap_or_else(default_state);
    state.paused_embeddings = true;
    state.paused_semantic = true;
    state.paused_reason = reason.clone();
    state.last_action = Some("paused".to_string());
    runtime_store.save_enrichment_queue_state(&state)?;
    let mut projected = project_workplane_status(
        &workplane_store,
        &state,
        effective_worker_budgets(&workplane_store, &daemon_config.config_root)?,
    )?;
    projected.mode = EnrichmentQueueMode::Paused;
    projected.last_action = Some("paused".to_string());
    projected.paused_reason = reason.clone();
    Ok(EnrichmentControlResult {
        message: reason
            .map(|reason| format!("Enrichment queue paused: {reason}"))
            .unwrap_or_else(|| "Enrichment queue paused.".to_string()),
        state: projected,
    })
}

pub(crate) fn resume_enrichments() -> Result<EnrichmentControlResult> {
    let runtime_store = DaemonSqliteRuntimeStore::open()?;
    let mut state = runtime_store
        .load_enrichment_queue_state()?
        .unwrap_or_else(default_state);
    state.paused_embeddings = false;
    state.paused_semantic = false;
    state.paused_reason = None;
    state.last_action = Some("resumed".to_string());
    runtime_store.save_enrichment_queue_state(&state)?;
    let daemon_config = crate::daemon::resolve_daemon_config(None)?;
    let workplane_store = DaemonSqliteRuntimeStore::open_at(
        resolve_repo_runtime_db_path_for_config_root(&daemon_config.config_root),
    )?;
    Ok(EnrichmentControlResult {
        message: "Enrichment queue resumed.".to_string(),
        state: project_workplane_status(
            &workplane_store,
            &state,
            effective_worker_budgets(&workplane_store, &daemon_config.config_root)?,
        )?,
    })
}

pub(crate) fn retry_failed_enrichments() -> Result<EnrichmentControlResult> {
    let runtime_store = DaemonSqliteRuntimeStore::open()?;
    let daemon_config = crate::daemon::resolve_daemon_config(None)?;
    let workplane_store = DaemonSqliteRuntimeStore::open_at(
        resolve_repo_runtime_db_path_for_config_root(&daemon_config.config_root),
    )?;
    let mut state = runtime_store
        .load_enrichment_queue_state()?
        .unwrap_or_else(default_state);
    let retried = retry_failed_jobs_in_store(&workplane_store)?;
    state.retried_failed_jobs += retried;
    state.last_action = Some("retry_failed".to_string());
    runtime_store.save_enrichment_queue_state(&state)?;
    let mut projected = project_workplane_status(
        &workplane_store,
        &state,
        effective_worker_budgets(&workplane_store, &daemon_config.config_root)?,
    )?;
    projected.retried_failed_jobs = state.retried_failed_jobs;
    projected.last_action = Some("retry_failed".to_string());
    Ok(EnrichmentControlResult {
        message: format!("Requeued {retried} failed enrichment jobs."),
        state: projected,
    })
}

pub(crate) fn effective_worker_budgets(
    workplane_store: &DaemonSqliteRuntimeStore,
    fallback_config_root: &std::path::Path,
) -> Result<EnrichmentWorkerBudgets> {
    let mut budgets = configured_enrichment_worker_budgets_for_repo(fallback_config_root);
    for config_root in iter_workplane_job_config_roots(workplane_store)? {
        let next = configured_enrichment_worker_budgets_for_repo(&config_root);
        budgets.summary_refresh = budgets.summary_refresh.max(next.summary_refresh);
        budgets.embeddings = budgets.embeddings.max(next.embeddings);
        budgets.clone_rebuild = budgets.clone_rebuild.max(next.clone_rebuild);
    }
    Ok(budgets)
}

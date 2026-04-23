use crate::capability_packs::semantic_clones::types::{
    SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX, SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_IDENTITY_EMBEDDING_MAILBOX, SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
};

pub(crate) const INIT_SYNC_SECTION_TITLE: &str = "Sync";
pub(crate) const INIT_SYNC_LANE_LABEL: &str = "Syncing repository";
pub(crate) const INIT_INGEST_SECTION_TITLE: &str = "Ingest";
pub(crate) const INIT_INGEST_LANE_LABEL: &str = "Ingesting commit history";
pub(crate) const INIT_CODE_EMBEDDINGS_SECTION_TITLE: &str = "Code Embeddings";
pub(crate) const INIT_CODE_EMBEDDINGS_LANE_LABEL: &str = "Creating code embeddings";
pub(crate) const INIT_SUMMARIES_SECTION_TITLE: &str = "Summaries";
pub(crate) const INIT_SUMMARIES_LANE_LABEL: &str = "Generating summaries";
pub(crate) const INIT_SUMMARY_EMBEDDINGS_SECTION_TITLE: &str = "Summary Embeddings";
pub(crate) const INIT_SUMMARY_EMBEDDINGS_LANE_LABEL: &str = "Creating summary embeddings";
pub(crate) const RETRY_FAILED_ENRICHMENTS_COMMAND: &str =
    "bitloops daemon enrichments retry-failed";

pub(crate) fn workplane_pool_label(pool_name: &str) -> &'static str {
    match pool_name {
        "summary_refresh" => "Code summaries",
        "embeddings" => "Semantic search indexing",
        "clone_rebuild" => "Clone matching",
        _ => "Background work",
    }
}

pub(crate) fn mailbox_label(mailbox_name: &str) -> &'static str {
    match mailbox_name {
        SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX => "Indexing source code",
        SEMANTIC_CLONES_IDENTITY_EMBEDDING_MAILBOX => "Indexing symbol identity",
        SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX => "Indexing generated summaries",
        SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX => "Generating summaries",
        SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX => "Refreshing clone matches",
        _ => "Background work",
    }
}

pub(crate) fn task_kind_label(kind: &str) -> &'static str {
    match kind.to_ascii_lowercase().as_str() {
        "sync" => "Syncing repository",
        "ingest" => "Ingesting commit history",
        "embeddings_bootstrap" => "Preparing the embeddings runtime",
        "summary_bootstrap" => "Preparing summary generation",
        _ => "Background task",
    }
}

pub(crate) fn lane_activity_label(detail: &str) -> &'static str {
    match detail {
        "sync" => "Syncing repository",
        "ingest" => "Ingesting commit history",
        "follow_up_sync" => "Running a follow-up sync",
        "code_embeddings" => "Creating code embeddings",
        "identity_embeddings" | "locator_embeddings" => "Creating identity embeddings",
        "summary_embeddings" => "Creating summary embeddings",
        "summaries" => "Generating summaries",
        "embeddings_bootstrap" => "Preparing the embeddings runtime",
        "summary_bootstrap" => "Preparing summary generation",
        "current_state_consumer" => "Applying codebase updates",
        _ => "Working",
    }
}

pub(crate) fn session_status_label(status: &str) -> &'static str {
    match status.to_ascii_lowercase().as_str() {
        "completed" => "Finished",
        "completed_with_warnings" => "Finished with warnings",
        "failing" => "Finishing remaining work after a failure",
        "failed" => "Failed",
        "waiting" => "Waiting",
        "running" => "Running",
        "queued" => "Queued",
        _ => "Running",
    }
}

pub(crate) fn waiting_reason_label(reason: &str) -> &'static str {
    match reason {
        "waiting_for_sync" => "Waiting for sync to finish",
        "waiting_for_embeddings_bootstrap" => "Waiting for the embeddings runtime to warm up",
        "waiting_for_summary_bootstrap" => "Waiting for summary generation to be ready",
        "waiting_for_summaries" => "Waiting for summaries to be ready",
        "waiting_for_follow_up_sync" => "Waiting for the follow-up sync",
        "waiting_for_top_level_work" => "Waiting for the codebase work to finish",
        "waiting_for_bootstrap" => "Waiting for bootstrap work to finish",
        "waiting_for_current_state_consumer" => "Waiting for the codebase update queue",
        "waiting_on_blocked_mailbox" => "Waiting for a blocked worker pool",
        "waiting_for_workplane" => "Waiting for queued enrichment work to finish",
        "blocked_mailbox" => "Blocked by a worker pool",
        "failed" => "Failed",
        "running" => "Running",
        "queued" => "Queued",
        _ => "Working",
    }
}

pub(crate) fn sync_phase_label(phase: &str) -> &'static str {
    match phase {
        "queued" => "Waiting in the queue",
        "ensuring_schema" => "Preparing the schema",
        "inspecting_workspace" => "Inspecting the workspace",
        "building_manifest" => "Building the manifest",
        "loading_stored_state" => "Loading stored state",
        "classifying_paths" => "Classifying files",
        "removing_paths" => "Removing stale files",
        "extracting_paths" => "Extracting symbols",
        "materialising_paths" => "Writing the latest code graph",
        "running_gc" => "Cleaning caches",
        "complete" => "Complete",
        "failed" => "Failed",
        _ => "Working",
    }
}

pub(crate) fn ingest_phase_label(phase: &str) -> &'static str {
    match phase.to_ascii_lowercase().as_str() {
        "initializing" => "Initialising",
        "extracting" => "Extracting checkpoints",
        "persisting" => "Persisting history",
        "complete" => "Complete",
        "failed" => "Failed",
        _ => "Working",
    }
}

pub(crate) fn embeddings_bootstrap_phase_label(phase: &str) -> &'static str {
    match phase {
        "queued" => "Waiting in the queue",
        "preparing_config" => "Preparing the embeddings runtime",
        "resolving_release" => "Preparing the embeddings runtime",
        "downloading_runtime" => "Downloading the embeddings runtime",
        "extracting_runtime" => "Preparing the embeddings runtime",
        "rewriting_runtime" => "Preparing the embeddings runtime",
        "warming_profile" => "Warming the embeddings runtime",
        "complete" => "Complete",
        "failed" => "Failed",
        _ => "Working",
    }
}

pub(crate) fn summary_bootstrap_phase_label(phase: &str) -> &'static str {
    match phase {
        "queued" => "Waiting in the queue",
        "resolving_release" => "Preparing summary generation",
        "downloading_runtime" => "Preparing summary generation",
        "extracting_runtime" => "Preparing summary generation",
        "rewriting_runtime" => "Preparing summary generation",
        "writing_profile" => "Preparing summary generation",
        "complete" => "Complete",
        "failed" => "Failed",
        _ => "Working",
    }
}

pub(crate) fn queue_state_summary(queued: u64, running: u64, failed: u64) -> String {
    format!("Work items: {queued} waiting · {running} in flight · {failed} failed")
}

pub(crate) fn workplane_warning_message(failed_jobs: u64, latest_error: Option<&str>) -> String {
    let task_label = if failed_jobs == 1 { "task" } else { "tasks" };
    match latest_error
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(error) => format!("{failed_jobs} {task_label} failed: {error}"),
        None => format!("{failed_jobs} {task_label} failed"),
    }
}

pub(crate) fn warning_summary(failed_jobs: u64) -> String {
    let task_label = if failed_jobs == 1 { "task" } else { "tasks" };
    format!(
        "{failed_jobs} enrichment {task_label} failed. Retry with: {RETRY_FAILED_ENRICHMENTS_COMMAND}"
    )
}

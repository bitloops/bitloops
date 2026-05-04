// Workplane orchestration for the daemon enrichment loop.
//
// The implementation lives in cohesive submodules; this facade declares the
// module tree and re-exports the surface that the parent `enrichment` module
// (and a small number of sibling test modules) depends on. No production logic
// should live here.

#[path = "workplane/enqueue.rs"]
mod enqueue;
#[path = "workplane/job_claim.rs"]
mod job_claim;
#[path = "workplane/job_completion.rs"]
mod job_completion;
#[path = "workplane/jobs.rs"]
mod jobs;
#[path = "workplane/mailbox_claim.rs"]
mod mailbox_claim;
#[path = "workplane/mailbox_persistence.rs"]
mod mailbox_persistence;
#[path = "workplane/maintenance.rs"]
mod maintenance;
#[path = "workplane/readiness.rs"]
mod readiness;
#[path = "workplane/setup.rs"]
mod setup;
#[path = "workplane/sql.rs"]
mod sql;
#[path = "workplane/status.rs"]
mod status;

pub(crate) use enqueue::{
    enqueue_workplane_clone_rebuild, enqueue_workplane_embedding_jobs,
    enqueue_workplane_embedding_repo_backfill_job, enqueue_workplane_summary_jobs,
};
pub(crate) use job_claim::claim_next_workplane_job;
#[cfg(test)]
pub(crate) use job_completion::format_workplane_job_completion_log;
pub(crate) use job_completion::{
    WorkplaneJobCompletionDisposition, persist_workplane_job_completion,
};
#[cfg(test)]
pub(crate) use jobs::load_workplane_jobs_by_status;
pub(crate) use mailbox_claim::{
    ClaimedEmbeddingMailboxBatch, ClaimedSummaryMailboxBatch,
    SEMANTIC_EMBEDDING_MAILBOX_BATCH_SIZE, SEMANTIC_SUMMARY_MAILBOX_BATCH_SIZE,
    claim_embedding_mailbox_batch, claim_summary_mailbox_batch,
};
pub(crate) use mailbox_persistence::{
    fail_summary_mailbox_batch, persist_embedding_mailbox_batch_failure,
    requeue_embedding_mailbox_batch, requeue_summary_mailbox_batch,
};
pub(crate) use maintenance::{
    compact_and_prune_workplane_jobs, prune_failed_semantic_inbox_items,
    recover_expired_semantic_inbox_leases, requeue_leased_semantic_inbox_items,
    retry_failed_semantic_inbox_items, retry_failed_workplane_jobs,
};
pub(crate) use readiness::current_workplane_mailbox_blocked_statuses;
pub(crate) use readiness::current_workplane_mailbox_blocked_statuses_for_repo;
pub(crate) use setup::{default_state, migrate_legacy_semantic_workplane_rows};
pub(crate) use sql::{repo_identity_from_runtime_metadata, sql_i64};
pub(crate) use status::{
    iter_workplane_job_config_roots, last_failed_embedding_job_from_workplane,
    project_workplane_status,
};

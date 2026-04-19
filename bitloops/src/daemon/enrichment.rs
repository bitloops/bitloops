#[path = "enrichment/controls.rs"]
mod controls;
#[path = "enrichment/coordinator.rs"]
mod coordinator;
#[path = "enrichment/enqueue.rs"]
mod enqueue;
#[path = "enrichment/execution.rs"]
mod execution;
#[path = "enrichment/model.rs"]
mod model;
#[path = "enrichment/runtime_events.rs"]
mod runtime_events;
#[path = "enrichment/semantic_writer.rs"]
mod semantic_writer;
#[path = "enrichment/worker_count.rs"]
pub(crate) mod worker_count;
#[path = "enrichment/worker_loop.rs"]
mod worker_loop;
#[path = "enrichment/workplane.rs"]
mod workplane;

pub use self::coordinator::EnrichmentCoordinator;
pub use self::model::{EnrichmentControlResult, EnrichmentControlState, EnrichmentJobTarget};

pub(crate) use self::controls::{
    blocked_mailboxes_for_repo, pause_enrichments, resume_enrichments, retry_failed_enrichments,
    snapshot,
};

pub(super) use self::controls::effective_worker_budgets;
pub(super) use self::model::{
    WORKPLANE_TERMINAL_RETENTION_SECS, WORKPLANE_TERMINAL_ROW_LIMIT, WorkplaneMailboxReadiness,
};
// `FollowUpJob` and `JobExecutionOutcome` need `pub(crate)` so that
// `enrichment::execution` can re-export them for sibling test modules. The
// items themselves are still gated by the parent `enrichment` module's
// `pub(crate)` boundary.
pub(crate) use self::model::{FollowUpJob, JobExecutionOutcome};
// Tests in sibling modules read these via `use super::*;`; the `unused_imports`
// lint cannot see those transitive uses.
#[allow(unused_imports)]
pub(crate) use self::model::{EnrichmentJob, EnrichmentJobKind, EnrichmentJobStatus};

#[cfg(test)]
pub(super) use self::controls::retry_failed_jobs_in_store;
#[cfg(test)]
#[allow(unused_imports)]
pub(super) use self::execution::load_repo_backfill_inputs;
#[cfg(test)]
pub(super) use self::model::{
    MAX_SEMANTIC_ENRICHMENT_JOB_ARTEFACTS, WORKPLANE_PENDING_COMPACTION_MIN_COUNT,
};
#[cfg(test)]
pub(super) use self::workplane::{
    WorkplaneJobCompletionDisposition, claim_embedding_mailbox_batch, claim_next_workplane_job,
    claim_summary_mailbox_batch, compact_and_prune_workplane_jobs, default_state,
    last_failed_embedding_job_from_workplane, load_workplane_jobs_by_status,
    persist_workplane_job_completion, project_workplane_status, sql_i64,
};

#[cfg(test)]
pub(super) use crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CAPABILITY_ID;
#[cfg(test)]
pub(super) use crate::capability_packs::semantic_clones::embeddings::EmbeddingRepresentationKind;
#[cfg(test)]
pub(super) use crate::capability_packs::semantic_clones::features as semantic_features;
#[cfg(test)]
pub(super) use crate::capability_packs::semantic_clones::types::{
    SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX, SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX, SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
};
#[cfg(test)]
pub(super) use crate::daemon::types::unix_timestamp_now;
#[cfg(test)]
pub(super) use std::collections::BTreeMap;

#[cfg(test)]
#[path = "enrichment_tests.rs"]
mod tests;

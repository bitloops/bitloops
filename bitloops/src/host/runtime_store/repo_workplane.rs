//! Repo workplane store: capability mailboxes, jobs, cursor runs, and semantic
//! summary/embedding queues. Implementation is split across submodules; this
//! file is a slim facade that pins the public surface.

mod dedupe;
mod intents;
mod jobs;
mod read_only_status;
mod schema;
mod semantic_mailboxes;
mod status;
mod types;
mod util;

pub use read_only_status::RepoCapabilityWorkplaneStatusReader;
pub(crate) use schema::{REPO_WORKPLANE_SCHEMA, ensure_repo_workplane_schema_upgrades};
pub use types::{
    CapabilityWorkplaneEnqueueResult, CapabilityWorkplaneJobInsert,
    CapabilityWorkplaneMailboxStatus, SemanticEmbeddingMailboxItemInsert,
    SemanticEmbeddingMailboxItemRecord, SemanticMailboxItemKind, SemanticMailboxItemStatus,
    SemanticSummaryMailboxItemInsert, SemanticSummaryMailboxItemRecord, WorkplaneCursorRunRecord,
    WorkplaneCursorRunStatus, WorkplaneJobQuery, WorkplaneJobRecord, WorkplaneJobStatus,
};

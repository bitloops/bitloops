//! Local-only SQLite runtime boundary for repo workflow state, interaction spool, session
//! metadata blobs, task checkpoint artefacts, and daemon runtime documents.

mod blob_keys;
mod daemon_documents;
mod repo_blob;
mod repo_lifecycle_spool;
mod repo_open;
mod repo_session_metadata;
mod repo_task_checkpoints;
mod repo_watcher;
mod repo_workplane;
mod sqlite_migrate;
mod types;
mod util;

#[cfg(test)]
mod tests;

pub(crate) use repo_lifecycle_spool::{
    LifecycleStopHookEnqueueResult, LifecycleStopJobInsert, LifecycleStopJobRecord,
    claim_next_lifecycle_stop_jobs, delete_lifecycle_stop_job,
    enqueue_lifecycle_stop_job_hook_safe_at, recover_running_lifecycle_stop_jobs,
    requeue_lifecycle_stop_job, unix_timestamp_now,
};
#[cfg(test)]
pub(crate) use repo_lifecycle_spool::{
    enqueue_lifecycle_stop_job_sqlite, list_lifecycle_stop_jobs_for_tests,
};
pub(crate) use repo_open::open_runtime_sqlite_for_config_root;
pub use repo_workplane::{
    CapabilityWorkplaneEnqueueResult, CapabilityWorkplaneJobInsert,
    CapabilityWorkplaneMailboxStatus, RepoCapabilityWorkplaneStatusReader,
    SemanticEmbeddingMailboxItemInsert, SemanticEmbeddingMailboxItemRecord,
    SemanticMailboxItemKind, SemanticMailboxItemStatus, SemanticSummaryMailboxItemInsert,
    SemanticSummaryMailboxItemRecord, WorkplaneCursorRunRecord, WorkplaneCursorRunStatus,
    WorkplaneJobQuery, WorkplaneJobRecord, WorkplaneJobStatus,
};
pub use types::{
    DaemonSqliteRuntimeStore, LegacySyncTaskRecord, PersistedCapabilityEventQueueState,
    PersistedDevqlTaskQueueState, PersistedSyncQueueState, RepoSqliteRuntimeStore,
    RepoWatcherRegistration, RepoWatcherRegistrationState, RuntimeMetadataBlobType, RuntimeStore,
    SessionMetadataSnapshot, SqliteRuntimeStore, TaskCheckpointArtefact,
};

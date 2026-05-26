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
    LifecycleBoundarySnapshot, LifecycleHookEnqueueResult, LifecycleJobInsert, LifecycleJobRecord,
    LifecycleJobStatus, LifecycleWorkspaceSnapshot, MAX_LIFECYCLE_JOB_ATTEMPTS,
    claim_next_lifecycle_job, delete_lifecycle_job, enqueue_lifecycle_job_hook_safe_at,
    fail_or_requeue_lifecycle_job, lifecycle_spool_has_repo_work,
    lifecycle_spool_repo_ids_with_work, mark_lifecycle_job_failed, recover_running_lifecycle_jobs,
    requeue_lifecycle_job, unix_timestamp_now,
};
#[cfg(test)]
pub(crate) use repo_lifecycle_spool::{
    enqueue_lifecycle_job_sqlite, force_pending_job_available_for_tests,
    list_lifecycle_jobs_for_tests,
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

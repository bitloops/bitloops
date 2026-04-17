//! Local-only SQLite runtime boundary for repo workflow state, interaction spool, session
//! metadata blobs, task checkpoint artefacts, and daemon runtime documents.

mod blob_keys;
mod daemon_documents;
mod repo_blob;
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

pub(crate) use repo_open::open_runtime_sqlite_for_config_root;
pub use repo_workplane::{
    CapabilityWorkplaneEnqueueResult, CapabilityWorkplaneJobInsert,
    CapabilityWorkplaneMailboxStatus, WorkplaneCursorRunRecord, WorkplaneCursorRunStatus,
    WorkplaneJobRecord, WorkplaneJobStatus,
};
pub use types::{
    DaemonSqliteRuntimeStore, LegacySyncTaskRecord, PersistedCapabilityEventQueueState,
    PersistedDevqlTaskQueueState, PersistedSyncQueueState, RepoSqliteRuntimeStore,
    RepoWatcherRegistration, RepoWatcherRegistrationState, RuntimeMetadataBlobType, RuntimeStore,
    SessionMetadataSnapshot, SqliteRuntimeStore, TaskCheckpointArtefact,
};

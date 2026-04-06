//! Local-only SQLite runtime boundary for repo workflow state, interaction spool, session
//! metadata blobs, task checkpoint artefacts, and daemon runtime documents.

mod blob_keys;
mod daemon_documents;
mod repo_blob;
mod repo_import_metadata_fs;
mod repo_import_sqlite;
mod repo_open;
mod repo_session_metadata;
mod repo_task_checkpoints;
mod sqlite_migrate;
mod types;
mod util;

#[cfg(test)]
mod tests;

pub use types::{
    DaemonSqliteRuntimeStore, PersistedSyncQueueState, RepoSqliteRuntimeStore,
    RuntimeMetadataBlobType, RuntimeStore, SessionMetadataSnapshot, SqliteRuntimeStore,
    TaskCheckpointArtefact,
};

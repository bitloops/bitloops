use std::path::Path;

use crate::host::interactions::db_store::SqliteInteractionEventStore;

/// Resolves an interaction event store for the given repo root.
/// Returns `None` if the store cannot be created (e.g., no SQLite database).
pub(super) fn resolve_interaction_event_store(
    repo_root: &Path,
) -> Option<SqliteInteractionEventStore> {
    let sqlite_path =
        crate::host::checkpoints::strategy::manual_commit::resolve_temporary_checkpoint_sqlite_path(
            repo_root,
        )
        .ok()?;
    let sqlite = crate::storage::SqliteConnectionPool::connect(sqlite_path).ok()?;
    sqlite.initialise_checkpoint_schema().ok()?;
    let repo_id = crate::host::devql::resolve_repo_identity(repo_root)
        .ok()?
        .repo_id;
    Some(SqliteInteractionEventStore::new(sqlite, repo_id))
}

use std::path::Path;

use crate::host::interactions::db_store::{SqliteInteractionSpool, interaction_spool_db_path};
use crate::host::interactions::event_sink::{
    EventDbInteractionRepository, create_event_repository,
};
use crate::host::interactions::store::InteractionSpool;

pub(super) fn resolve_interaction_spool(repo_root: &Path) -> Option<SqliteInteractionSpool> {
    let sqlite =
        crate::storage::SqliteConnectionPool::connect(interaction_spool_db_path(repo_root)).ok()?;
    let repo_id = crate::host::devql::resolve_repo_identity(repo_root)
        .ok()?
        .repo_id;
    SqliteInteractionSpool::new(sqlite, repo_id).ok()
}

pub(super) fn resolve_event_repository(repo_root: &Path) -> Option<EventDbInteractionRepository> {
    let backends = crate::config::resolve_store_backend_config_for_repo(repo_root).ok()?;
    let repo_id = crate::host::devql::resolve_repo_identity(repo_root)
        .ok()?
        .repo_id;
    create_event_repository(&backends.events, repo_root, repo_id).ok()
}

pub(super) fn flush_interaction_spool_best_effort(repo_root: &Path) {
    let Some(spool) = resolve_interaction_spool(repo_root) else {
        return;
    };
    let Some(repository) = resolve_event_repository(repo_root) else {
        return;
    };
    if let Err(err) = spool.flush(&repository) {
        eprintln!("[bitloops] Warning: failed to flush interaction spool: {err:#}");
    }
}

use std::path::Path;

use crate::host::interactions::db_store::SqliteInteractionSpool;
use crate::host::interactions::interaction_repository::create_interaction_repository;
use crate::host::interactions::store::{InteractionEventRepository, InteractionSpool};

pub(crate) fn resolve_interaction_spool(repo_root: &Path) -> Option<SqliteInteractionSpool> {
    crate::host::runtime_store::RepoSqliteRuntimeStore::open(repo_root)
        .ok()?
        .interaction_spool()
        .ok()
}

pub(crate) fn resolve_interaction_repository(
    repo_root: &Path,
) -> Option<impl InteractionEventRepository + use<>> {
    let backends = crate::config::resolve_store_backend_config_for_repo(repo_root).ok()?;
    let repo_id = crate::host::devql::resolve_repo_identity(repo_root)
        .ok()?
        .repo_id;
    create_interaction_repository(&backends.events, repo_root, repo_id).ok()
}

pub(crate) fn flush_interaction_spool_best_effort(repo_root: &Path) {
    let Some(spool) = resolve_interaction_spool(repo_root) else {
        return;
    };
    let Some(repository) = resolve_interaction_repository(repo_root) else {
        return;
    };
    if let Err(err) = spool.flush(&repository) {
        eprintln!("[bitloops] Warning: failed to flush interaction spool: {err:#}");
    }
}

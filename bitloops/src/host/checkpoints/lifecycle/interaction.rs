use std::path::Path;

use crate::host::interactions::db_store::SqliteInteractionSpool;
use crate::host::interactions::interaction_repository::create_interaction_repository;
use crate::host::interactions::store::{InteractionEventRepository, InteractionSpool};

pub(crate) fn resolve_interaction_spool(repo_root: &Path) -> Option<SqliteInteractionSpool> {
    match crate::host::runtime_store::RepoSqliteRuntimeStore::open(repo_root) {
        Ok(store) => match store.interaction_spool() {
            Ok(spool) => Some(spool),
            Err(err) => {
                eprintln!(
                    "[bitloops] Warning: skipping hook capture because the interaction spool could not be opened: {err:#}"
                );
                None
            }
        },
        Err(err) => {
            eprintln!(
                "[bitloops] Warning: skipping hook capture because this repo is not bound to a valid Bitloops daemon config: {err:#}"
            );
            None
        }
    }
}

pub(crate) fn resolve_interaction_repository(
    repo_root: &Path,
) -> Option<impl InteractionEventRepository + use<>> {
    let backends = match crate::config::resolve_bound_store_backend_config_for_repo(repo_root) {
        Ok(backends) => backends,
        Err(err) => {
            eprintln!(
                "[bitloops] Warning: skipping interaction flush because this repo is not bound to a valid Bitloops daemon config: {err:#}"
            );
            return None;
        }
    };
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

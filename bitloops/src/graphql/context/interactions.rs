use super::DevqlGraphqlContext;
use super::commit_checkpoints::is_missing_sqlite_store_error;
use crate::graphql::ResolverScope;
use crate::graphql::types::interaction::{InteractionSessionObject, InteractionTurnObject};
use crate::host::interactions::db_store::SqliteInteractionEventStore;
use crate::host::interactions::store::InteractionEventStore;
use anyhow::{Context, Result};
use tokio::task;

impl DevqlGraphqlContext {
    pub(crate) async fn list_interaction_sessions(
        &self,
        scope: &ResolverScope,
        agent: Option<&str>,
        limit: Option<i32>,
    ) -> Result<Vec<InteractionSessionObject>> {
        let repo_root = self.repo_root_for_scope(scope)?;
        let repo_id = self.repo_id_for_scope(scope)?;
        let agent = agent.map(str::to_string);
        let limit = limit.unwrap_or(100).clamp(1, 1000) as usize;

        task::spawn_blocking(move || -> Result<Vec<InteractionSessionObject>> {
            let store = match resolve_interaction_store(&repo_root, &repo_id) {
                Some(store) => store,
                None => return Ok(Vec::new()),
            };
            let sessions = store.list_sessions(agent.as_deref(), limit)?;
            Ok(sessions
                .iter()
                .map(InteractionSessionObject::from_domain)
                .collect())
        })
        .await
        .context("joining interaction sessions query task")?
    }

    pub(crate) async fn list_interaction_turns(
        &self,
        scope: &ResolverScope,
        session_id: &str,
    ) -> Result<Vec<InteractionTurnObject>> {
        let repo_root = self.repo_root_for_scope(scope)?;
        let repo_id = self.repo_id_for_scope(scope)?;
        let session_id = session_id.to_string();

        task::spawn_blocking(move || -> Result<Vec<InteractionTurnObject>> {
            let store = match resolve_interaction_store(&repo_root, &repo_id) {
                Some(store) => store,
                None => return Ok(Vec::new()),
            };
            let turns = store.load_turns_for_session(&session_id)?;
            Ok(turns
                .iter()
                .map(InteractionTurnObject::from_domain)
                .collect())
        })
        .await
        .context("joining interaction turns query task")?
    }
}

fn resolve_interaction_store(
    repo_root: &std::path::Path,
    repo_id: &str,
) -> Option<SqliteInteractionEventStore> {
    let sqlite_path =
        crate::host::checkpoints::strategy::manual_commit::resolve_temporary_checkpoint_sqlite_path(
            repo_root,
        )
        .ok()?;
    let sqlite = match crate::storage::SqliteConnectionPool::connect_existing(sqlite_path) {
        Ok(pool) => pool,
        Err(err) if is_missing_sqlite_store_error(&err) => return None,
        Err(err) => {
            log::warn!("interaction store unavailable: {err:#}");
            return None;
        }
    };
    if let Err(err) = sqlite.initialise_checkpoint_schema() {
        log::warn!("interaction store schema init failed: {err:#}");
        return None;
    }
    Some(SqliteInteractionEventStore::new(
        sqlite,
        repo_id.to_string(),
    ))
}

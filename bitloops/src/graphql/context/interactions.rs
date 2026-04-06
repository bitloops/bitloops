use super::DevqlGraphqlContext;
use crate::graphql::ResolverScope;
use crate::graphql::types::interaction::{
    InteractionEventObject, InteractionSessionObject, InteractionTurnObject,
};
use crate::host::interactions::event_sink::create_event_repository;
use crate::host::interactions::store::InteractionEventRepository;
use crate::host::interactions::types::{InteractionEventFilter, InteractionEventType};
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
            let repository = match resolve_interaction_repository(&repo_root, &repo_id) {
                Some(repository) => repository,
                None => return Ok(Vec::new()),
            };
            let sessions = repository.list_sessions(agent.as_deref(), limit)?;
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
        limit: Option<i32>,
    ) -> Result<Vec<InteractionTurnObject>> {
        let repo_root = self.repo_root_for_scope(scope)?;
        let repo_id = self.repo_id_for_scope(scope)?;
        let session_id = session_id.to_string();
        let limit = limit.unwrap_or(100).clamp(1, 1000) as usize;

        task::spawn_blocking(move || -> Result<Vec<InteractionTurnObject>> {
            let repository = match resolve_interaction_repository(&repo_root, &repo_id) {
                Some(repository) => repository,
                None => return Ok(Vec::new()),
            };
            let turns = repository.list_turns_for_session(&session_id, limit)?;
            Ok(turns
                .iter()
                .map(InteractionTurnObject::from_domain)
                .collect())
        })
        .await
        .context("joining interaction turns query task")?
    }

    pub(crate) async fn list_interaction_events(
        &self,
        scope: &ResolverScope,
        session_id: Option<&str>,
        turn_id: Option<&str>,
        event_type: Option<&str>,
        since: Option<&str>,
        limit: Option<i32>,
    ) -> Result<Vec<InteractionEventObject>> {
        let repo_root = self.repo_root_for_scope(scope)?;
        let repo_id = self.repo_id_for_scope(scope)?;
        let parsed_event_type = match event_type.map(str::trim).filter(|value| !value.is_empty()) {
            Some(value) => Some(
                InteractionEventType::parse(value)
                    .with_context(|| format!("invalid interaction event type `{value}`"))?,
            ),
            None => None,
        };
        let filter = InteractionEventFilter {
            session_id: session_id
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
            turn_id: turn_id
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
            event_type: parsed_event_type,
            since: since
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
        };
        let limit = limit.unwrap_or(100).clamp(1, 1000) as usize;

        task::spawn_blocking(move || -> Result<Vec<InteractionEventObject>> {
            let repository = match resolve_interaction_repository(&repo_root, &repo_id) {
                Some(repository) => repository,
                None => return Ok(Vec::new()),
            };
            let events = repository.list_events(&filter, limit)?;
            Ok(events
                .iter()
                .map(InteractionEventObject::from_domain)
                .collect())
        })
        .await
        .context("joining interaction events query task")?
    }
}

fn resolve_interaction_repository(
    repo_root: &std::path::Path,
    repo_id: &str,
) -> Option<impl InteractionEventRepository> {
    let backends = crate::config::resolve_store_backend_config_for_repo(repo_root).ok()?;
    create_event_repository(&backends.events, repo_root, repo_id.to_string()).ok()
}

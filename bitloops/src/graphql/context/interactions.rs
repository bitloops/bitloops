use super::DevqlGraphqlContext;
use crate::graphql::ResolverScope;
use crate::graphql::types::interaction::{
    InteractionEventObject, InteractionFilterInput, InteractionSearchInputObject,
    InteractionSessionObject, InteractionSessionSearchHitObject, InteractionTurnObject,
    InteractionTurnSearchHitObject,
};
use crate::host::interactions::query;
use anyhow::{Context, Result};
use tokio::task;

impl DevqlGraphqlContext {
    pub(crate) async fn list_interaction_sessions(
        &self,
        scope: &ResolverScope,
        filter: Option<&InteractionFilterInput>,
    ) -> Result<Vec<InteractionSessionObject>> {
        let repo_root = self.repo_root_for_scope(scope)?;
        let filter = filter.cloned().unwrap_or_default().to_domain();
        task::spawn_blocking(move || {
            query::list_session_summaries(&repo_root, &filter).map(|sessions| {
                sessions
                    .iter()
                    .map(InteractionSessionObject::from_summary)
                    .collect()
            })
        })
        .await
        .context("joining interaction sessions query task")?
    }

    pub(crate) async fn list_interaction_turns(
        &self,
        scope: &ResolverScope,
        filter: Option<&InteractionFilterInput>,
    ) -> Result<Vec<InteractionTurnObject>> {
        let repo_root = self.repo_root_for_scope(scope)?;
        let filter = filter.cloned().unwrap_or_default().to_domain();
        task::spawn_blocking(move || {
            query::list_turn_summaries(&repo_root, &filter).map(|turns| {
                turns
                    .iter()
                    .map(InteractionTurnObject::from_summary)
                    .collect()
            })
        })
        .await
        .context("joining interaction turns query task")?
    }

    pub(crate) async fn list_interaction_events(
        &self,
        scope: &ResolverScope,
        filter: Option<&InteractionFilterInput>,
    ) -> Result<Vec<InteractionEventObject>> {
        let repo_root = self.repo_root_for_scope(scope)?;
        let filter = filter.cloned().unwrap_or_default().to_domain();
        task::spawn_blocking(move || {
            query::list_events(&repo_root, &filter).map(|events| {
                events
                    .iter()
                    .map(InteractionEventObject::from_domain)
                    .collect()
            })
        })
        .await
        .context("joining interaction events query task")?
    }

    pub(crate) async fn search_interaction_sessions(
        &self,
        scope: &ResolverScope,
        input: &InteractionSearchInputObject,
    ) -> Result<Vec<InteractionSessionSearchHitObject>> {
        let repo_root = self.repo_root_for_scope(scope)?;
        let input = input.clone().to_domain();
        task::spawn_blocking(move || {
            query::search_session_summaries(&repo_root, &input).map(|hits| {
                hits.iter()
                    .map(InteractionSessionSearchHitObject::from_hit)
                    .collect()
            })
        })
        .await
        .context("joining interaction session search task")?
    }

    pub(crate) async fn search_interaction_turns(
        &self,
        scope: &ResolverScope,
        input: &InteractionSearchInputObject,
    ) -> Result<Vec<InteractionTurnSearchHitObject>> {
        let repo_root = self.repo_root_for_scope(scope)?;
        let input = input.clone().to_domain();
        task::spawn_blocking(move || {
            query::search_turn_summaries(&repo_root, &input).map(|hits| {
                hits.iter()
                    .map(InteractionTurnSearchHitObject::from_hit)
                    .collect()
            })
        })
        .await
        .context("joining interaction turn search task")?
    }
}

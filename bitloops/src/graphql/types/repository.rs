use async_graphql::{ComplexObject, Context, ID, Result, SimpleObject};

use crate::graphql::{
    DevqlGraphqlContext, ResolverScope, backend_error, bad_cursor_error, bad_user_input_error,
};

use super::interaction::{
    InteractionFilterInput, InteractionSearchInputObject, InteractionSessionSearchHitObject,
    InteractionTurnSearchHitObject,
};
use super::{
    ArtefactConnection, ArtefactEdge, ArtefactFilterInput, AsOfInput, CheckpointConnection,
    CheckpointEdge, CloneSummary, ClonesFilterInput, CommitConnection, CommitEdge,
    ConnectionPagination, DateTimeScalar, FileContext, InteractionEventConnection,
    InteractionEventEdge, InteractionSessionConnection, InteractionSessionEdge,
    InteractionTurnConnection, InteractionTurnEdge, KnowledgeItemConnection, KnowledgeItemEdge,
    KnowledgeProvider, Project, TelemetryEventConnection, TelemetryEventEdge, TemporalScope,
    paginate_items,
};

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
#[graphql(complex)]
pub struct Repository {
    pub id: ID,
    pub name: String,
    pub provider: String,
    pub organization: String,
    #[graphql(skip)]
    pub(crate) scope: ResolverScope,
}

impl Repository {
    pub fn new(name: &str, provider: &str, organization: &str) -> Self {
        Self {
            id: ID(format!("repo://{provider}/{organization}/{name}")),
            name: name.to_string(),
            provider: provider.to_string(),
            organization: organization.to_string(),
            scope: ResolverScope::default(),
        }
    }

    pub(crate) fn with_scope(mut self, scope: ResolverScope) -> Self {
        self.scope = scope;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
pub struct Branch {
    pub name: String,
    pub checkpoint_count: i32,
    pub latest_checkpoint_at: Option<DateTimeScalar>,
}

#[ComplexObject]
impl Repository {
    async fn project(&self, ctx: &Context<'_>, path: String) -> Result<Project> {
        let project_path = ctx
            .data_unchecked::<DevqlGraphqlContext>()
            .validate_project_path(&self.scope, &path)
            .map_err(bad_user_input_error)?;
        Ok(Project::new(
            project_path.clone(),
            self.scope.with_project_path(project_path),
        ))
    }

    async fn branch(&self, _ctx: &Context<'_>, name: String) -> Result<Repository> {
        let branch_name = name.trim();
        if branch_name.is_empty() {
            return Err(bad_user_input_error("branch name must not be empty"));
        }

        Ok(Self {
            id: self.id.clone(),
            name: self.name.clone(),
            provider: self.provider.clone(),
            organization: self.organization.clone(),
            scope: self.scope.with_branch_name(branch_name.to_string()),
        })
    }

    #[graphql(name = "asOf")]
    async fn as_of(&self, ctx: &Context<'_>, input: AsOfInput) -> Result<TemporalScope> {
        let context = ctx.data_unchecked::<DevqlGraphqlContext>();
        let temporal_scope = context
            .resolve_temporal_scope(&self.scope, &input)
            .await
            .map_err(|err| {
                let message = format!("{err:#}");
                if context.is_unknown_revision_error(&err)
                    || message.contains("asOf(input:")
                    || message.contains("unknown save revision")
                {
                    return bad_user_input_error(message);
                }
                backend_error(format!("failed to resolve temporal scope: {message}"))
            })?;

        Ok(TemporalScope::new(
            &temporal_scope,
            self.scope.with_temporal_scope(temporal_scope.clone()),
        ))
    }

    async fn default_branch(&self, ctx: &Context<'_>) -> String {
        ctx.data_unchecked::<DevqlGraphqlContext>()
            .default_branch_name_for_scope(&self.scope)
            .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn commits(
        &self,
        ctx: &Context<'_>,
        branch: Option<String>,
        author: Option<String>,
        since: Option<DateTimeScalar>,
        until: Option<DateTimeScalar>,
        first: Option<i32>,
        after: Option<String>,
        last: Option<i32>,
        before: Option<String>,
    ) -> Result<CommitConnection> {
        if let (Some(since), Some(until)) = (since.as_ref(), until.as_ref())
            && DateTimeScalar::parse_rfc3339(since.as_str()).expect("validated datetime")
                > DateTimeScalar::parse_rfc3339(until.as_str()).expect("validated datetime")
        {
            return Err(bad_user_input_error(
                "`since` must be earlier than or equal to `until`",
            ));
        }

        let commits = match ctx
            .data_unchecked::<DevqlGraphqlContext>()
            .list_commits(
                &self.scope,
                branch.as_deref(),
                author.as_deref(),
                since.as_ref(),
                until.as_ref(),
            )
            .await
        {
            Ok(commits) => commits,
            Err(err)
                if branch.is_some()
                    && ctx
                        .data_unchecked::<DevqlGraphqlContext>()
                        .is_unknown_revision_error(&err) =>
            {
                return Err(bad_user_input_error(format!(
                    "unknown branch `{}`",
                    branch.as_deref().unwrap_or_default().trim()
                )));
            }
            Err(err) => {
                return Err(backend_error(format!(
                    "failed to query repository commits: {err:#}"
                )));
            }
        };
        let pagination = ConnectionPagination::from_graphql(
            50,
            first,
            after.as_deref(),
            last,
            before.as_deref(),
        )?;
        let page = paginate_items(&commits, &pagination, |commit| commit.cursor())?;
        Ok(CommitConnection::new(
            page.items.into_iter().map(CommitEdge::new).collect(),
            page.page_info,
            page.total_count,
        ))
    }

    async fn branches(
        &self,
        ctx: &Context<'_>,
        since: Option<DateTimeScalar>,
        until: Option<DateTimeScalar>,
    ) -> Result<Vec<Branch>> {
        ctx.data_unchecked::<DevqlGraphqlContext>()
            .list_branches(&self.scope, since.as_ref(), until.as_ref())
            .await
            .map_err(|err| backend_error(format!("failed to query repository branches: {err:#}")))
    }

    #[allow(clippy::too_many_arguments)]
    async fn checkpoints(
        &self,
        ctx: &Context<'_>,
        agent: Option<String>,
        since: Option<DateTimeScalar>,
        first: Option<i32>,
        after: Option<String>,
        last: Option<i32>,
        before: Option<String>,
    ) -> Result<CheckpointConnection> {
        let pagination = ConnectionPagination::from_graphql(
            50,
            first,
            after.as_deref(),
            last,
            before.as_deref(),
        )?;
        let checkpoints = ctx
            .data_unchecked::<DevqlGraphqlContext>()
            .list_checkpoints(&self.scope, agent.as_deref(), since.as_ref())
            .await
            .map_err(|err| {
                backend_error(format!("failed to query repository checkpoints: {err:#}"))
            })?;
        let page = paginate_items(&checkpoints, &pagination, |checkpoint| checkpoint.cursor())?;
        Ok(CheckpointConnection::new(
            page.items.into_iter().map(CheckpointEdge::new).collect(),
            page.page_info,
            page.total_count,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    async fn telemetry(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "eventType")] event_type: Option<String>,
        agent: Option<String>,
        since: Option<DateTimeScalar>,
        first: Option<i32>,
        after: Option<String>,
        last: Option<i32>,
        before: Option<String>,
    ) -> Result<TelemetryEventConnection> {
        let pagination = ConnectionPagination::from_graphql(
            50,
            first,
            after.as_deref(),
            last,
            before.as_deref(),
        )?;
        let telemetry = ctx
            .data_unchecked::<DevqlGraphqlContext>()
            .list_telemetry_events(
                &self.scope,
                event_type.as_deref(),
                agent.as_deref(),
                since.as_ref(),
            )
            .await
            .map_err(|err| {
                backend_error(format!("failed to query repository telemetry: {err:#}"))
            })?;
        let page = paginate_items(&telemetry, &pagination, |event| event.cursor())?;
        Ok(TelemetryEventConnection::new(
            page.items
                .into_iter()
                .map(TelemetryEventEdge::new)
                .collect(),
            page.page_info,
            page.total_count,
        ))
    }

    async fn users(&self, ctx: &Context<'_>) -> Result<Vec<String>> {
        ctx.data_unchecked::<DevqlGraphqlContext>()
            .list_users(&self.scope)
            .await
            .map_err(|err| backend_error(format!("failed to query repository users: {err:#}")))
    }

    async fn agents(&self, ctx: &Context<'_>) -> Result<Vec<String>> {
        ctx.data_unchecked::<DevqlGraphqlContext>()
            .list_agents(&self.scope)
            .await
            .map_err(|err| backend_error(format!("failed to query repository agents: {err:#}")))
    }

    async fn file(&self, ctx: &Context<'_>, path: String) -> Result<FileContext> {
        let normalized = ctx
            .data_unchecked::<DevqlGraphqlContext>()
            .resolve_scope_path(&self.scope, &path, false)
            .map_err(bad_user_input_error)?;
        ctx.data_unchecked::<DevqlGraphqlContext>()
            .resolve_file_context(&normalized, &self.scope)
            .await
            .map_err(|err| {
                backend_error(format!("failed to resolve file `{normalized}`: {err:#}"))
            })?
            .ok_or_else(|| bad_user_input_error(format!("unknown path `{normalized}`")))
    }

    async fn files(&self, ctx: &Context<'_>, path: String) -> Result<Vec<FileContext>> {
        let normalized = ctx
            .data_unchecked::<DevqlGraphqlContext>()
            .resolve_scope_path(&self.scope, &path, true)
            .map_err(bad_user_input_error)?;
        ctx.data_unchecked::<DevqlGraphqlContext>()
            .list_file_contexts(&normalized, &self.scope)
            .await
            .map_err(|err| {
                backend_error(format!("failed to resolve files `{normalized}`: {err:#}"))
            })
    }

    async fn artefacts(
        &self,
        ctx: &Context<'_>,
        filter: Option<ArtefactFilterInput>,
        first: Option<i32>,
        after: Option<String>,
        last: Option<i32>,
        before: Option<String>,
    ) -> Result<ArtefactConnection> {
        if let Some(filter) = filter.as_ref() {
            filter.validate()?;
        }

        let context = ctx.data_unchecked::<DevqlGraphqlContext>();
        let pagination = ConnectionPagination::from_graphql(
            100,
            first,
            after.as_deref(),
            last,
            before.as_deref(),
        )?;

        if let Some(cursor) = pagination.after().or_else(|| pagination.before()) {
            let cursor_exists = context
                .artefact_cursor_exists(None, filter.as_ref(), &self.scope, cursor)
                .await
                .map_err(|err| {
                    backend_error(format!("failed to query repository artefacts: {err:#}"))
                })?;
            if !cursor_exists {
                return Err(bad_cursor_error(format!(
                    "cursor `{cursor}` does not match any result in this connection"
                )));
            }
        }

        let (artefacts, page_info, total_count) = context
            .query_artefact_connection(None, filter.as_ref(), &self.scope, &pagination)
            .await
            .map_err(|err| {
                backend_error(format!("failed to query repository artefacts: {err:#}"))
            })?;

        Ok(ArtefactConnection::new(
            artefacts.into_iter().map(ArtefactEdge::new).collect(),
            page_info,
            total_count,
        ))
    }

    #[graphql(name = "cloneSummary")]
    async fn clone_summary(
        &self,
        ctx: &Context<'_>,
        filter: Option<ArtefactFilterInput>,
        #[graphql(name = "cloneFilter")] clone_filter: Option<ClonesFilterInput>,
    ) -> Result<CloneSummary> {
        if let Some(filter) = filter.as_ref() {
            filter.validate()?;
        }
        if let Some(clone_filter) = clone_filter.as_ref() {
            clone_filter.validate()?;
        }
        if self
            .scope
            .temporal_scope()
            .is_some_and(|scope| scope.use_historical_tables() || scope.save_revision().is_some())
        {
            return Err(bad_user_input_error(
                "`clones` does not support historical or temporary `asOf(...)` scopes yet",
            ));
        }

        super::clone::resolve_clone_summary(
            ctx.data_unchecked::<DevqlGraphqlContext>(),
            None,
            filter.as_ref(),
            clone_filter.as_ref(),
            &self.scope,
        )
        .await
        .map_err(|err| backend_error(format!("failed to query repository clone summary: {err:#}")))
    }

    async fn knowledge(
        &self,
        ctx: &Context<'_>,
        provider: Option<KnowledgeProvider>,
        first: Option<i32>,
        after: Option<String>,
        last: Option<i32>,
        before: Option<String>,
    ) -> Result<KnowledgeItemConnection> {
        let pagination = ConnectionPagination::from_graphql(
            25,
            first,
            after.as_deref(),
            last,
            before.as_deref(),
        )?;
        let items = ctx
            .data_unchecked::<DevqlGraphqlContext>()
            .list_knowledge_items(provider, &self.scope)
            .await
            .map_err(|err| {
                backend_error(format!("failed to query repository knowledge: {err:#}"))
            })?;
        let page = paginate_items(&items, &pagination, |item| item.cursor())?;
        Ok(KnowledgeItemConnection::new(
            page.items.into_iter().map(KnowledgeItemEdge::new).collect(),
            page.page_info,
            page.total_count,
        ))
    }

    #[graphql(name = "interactionSessions")]
    async fn interaction_sessions(
        &self,
        ctx: &Context<'_>,
        filter: Option<InteractionFilterInput>,
        first: Option<i32>,
        after: Option<String>,
        last: Option<i32>,
        before: Option<String>,
    ) -> Result<InteractionSessionConnection> {
        let pagination = ConnectionPagination::from_graphql(
            50,
            first,
            after.as_deref(),
            last,
            before.as_deref(),
        )?;
        let sessions = ctx
            .data_unchecked::<DevqlGraphqlContext>()
            .list_interaction_sessions(&self.scope, filter.as_ref())
            .await
            .map_err(|err| {
                backend_error(format!("failed to query interaction sessions: {err:#}"))
            })?;
        let page = paginate_items(&sessions, &pagination, |session| session.cursor())?;
        Ok(InteractionSessionConnection::new(
            page.items
                .into_iter()
                .map(InteractionSessionEdge::new)
                .collect(),
            page.page_info,
            page.total_count,
        ))
    }

    #[graphql(name = "interactionTurns")]
    async fn interaction_turns(
        &self,
        ctx: &Context<'_>,
        filter: Option<InteractionFilterInput>,
        first: Option<i32>,
        after: Option<String>,
        last: Option<i32>,
        before: Option<String>,
    ) -> Result<InteractionTurnConnection> {
        let pagination = ConnectionPagination::from_graphql(
            50,
            first,
            after.as_deref(),
            last,
            before.as_deref(),
        )?;
        let turns = ctx
            .data_unchecked::<DevqlGraphqlContext>()
            .list_interaction_turns(&self.scope, filter.as_ref())
            .await
            .map_err(|err| backend_error(format!("failed to query interaction turns: {err:#}")))?;
        let page = paginate_items(&turns, &pagination, |turn| turn.cursor())?;
        Ok(InteractionTurnConnection::new(
            page.items
                .into_iter()
                .map(InteractionTurnEdge::new)
                .collect(),
            page.page_info,
            page.total_count,
        ))
    }

    #[graphql(name = "interactionEvents")]
    async fn interaction_events(
        &self,
        ctx: &Context<'_>,
        filter: Option<InteractionFilterInput>,
        first: Option<i32>,
        after: Option<String>,
        last: Option<i32>,
        before: Option<String>,
    ) -> Result<InteractionEventConnection> {
        let pagination = ConnectionPagination::from_graphql(
            50,
            first,
            after.as_deref(),
            last,
            before.as_deref(),
        )?;
        let events = ctx
            .data_unchecked::<DevqlGraphqlContext>()
            .list_interaction_events(&self.scope, filter.as_ref())
            .await
            .map_err(|err| backend_error(format!("failed to query interaction events: {err:#}")))?;
        let page = paginate_items(&events, &pagination, |event| event.cursor())?;
        Ok(InteractionEventConnection::new(
            page.items
                .into_iter()
                .map(InteractionEventEdge::new)
                .collect(),
            page.page_info,
            page.total_count,
        ))
    }

    #[graphql(name = "searchInteractionSessions")]
    async fn search_interaction_sessions(
        &self,
        ctx: &Context<'_>,
        input: InteractionSearchInputObject,
    ) -> Result<Vec<InteractionSessionSearchHitObject>> {
        ctx.data_unchecked::<DevqlGraphqlContext>()
            .search_interaction_sessions(&self.scope, &input)
            .await
            .map_err(|err| backend_error(format!("failed to search interaction sessions: {err:#}")))
    }

    #[graphql(name = "searchInteractionTurns")]
    async fn search_interaction_turns(
        &self,
        ctx: &Context<'_>,
        input: InteractionSearchInputObject,
    ) -> Result<Vec<InteractionTurnSearchHitObject>> {
        ctx.data_unchecked::<DevqlGraphqlContext>()
            .search_interaction_turns(&self.scope, &input)
            .await
            .map_err(|err| backend_error(format!("failed to search interaction turns: {err:#}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_new_builds_expected_id() {
        let repository = Repository::new("demo", "local", "org");
        assert_eq!(repository.id.as_str(), "repo://local/org/demo");
    }
}

use async_graphql::{ComplexObject, Context, ID, Result, SimpleObject};

use crate::graphql::{DevqlGraphqlContext, ResolverScope, backend_error, bad_user_input_error};

use super::{
    ArtefactConnection, ArtefactEdge, ArtefactFilterInput, AsOfInput, CheckpointConnection,
    CheckpointEdge, CommitConnection, CommitEdge, DateTimeScalar, FileContext,
    KnowledgeItemConnection, KnowledgeItemEdge, KnowledgeProvider, Project,
    TelemetryEventConnection, TelemetryEventEdge, TemporalScope, paginate_items,
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
            .validate_project_path(&path)
            .map_err(bad_user_input_error)?;
        Ok(Project::new(
            project_path.clone(),
            self.scope.with_project_path(project_path),
        ))
    }

    #[graphql(name = "asOf")]
    async fn as_of(&self, ctx: &Context<'_>, input: AsOfInput) -> Result<TemporalScope> {
        let context = ctx.data_unchecked::<DevqlGraphqlContext>();
        let temporal_scope = context
            .resolve_temporal_scope(&input)
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
            .default_branch_name()
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
        #[graphql(default = 50)] first: i32,
        after: Option<String>,
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
        let page = paginate_items(&commits, first, after.as_deref(), |commit| commit.cursor())?;
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
            .list_branches(since.as_ref(), until.as_ref())
            .await
            .map_err(|err| backend_error(format!("failed to query repository branches: {err:#}")))
    }

    async fn checkpoints(
        &self,
        ctx: &Context<'_>,
        agent: Option<String>,
        since: Option<DateTimeScalar>,
        #[graphql(default = 50)] first: i32,
        after: Option<String>,
    ) -> Result<CheckpointConnection> {
        let checkpoints = ctx
            .data_unchecked::<DevqlGraphqlContext>()
            .list_checkpoints(&self.scope, agent.as_deref(), since.as_ref())
            .await
            .map_err(|err| {
                backend_error(format!("failed to query repository checkpoints: {err:#}"))
            })?;
        let page = paginate_items(&checkpoints, first, after.as_deref(), |checkpoint| {
            checkpoint.cursor()
        })?;
        Ok(CheckpointConnection::new(
            page.items.into_iter().map(CheckpointEdge::new).collect(),
            page.page_info,
            page.total_count,
        ))
    }

    async fn telemetry(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "eventType")] event_type: Option<String>,
        agent: Option<String>,
        since: Option<DateTimeScalar>,
        #[graphql(default = 50)] first: i32,
        after: Option<String>,
    ) -> Result<TelemetryEventConnection> {
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
        let page = paginate_items(&telemetry, first, after.as_deref(), |event| event.cursor())?;
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
            .list_users()
            .await
            .map_err(|err| backend_error(format!("failed to query repository users: {err:#}")))
    }

    async fn agents(&self, ctx: &Context<'_>) -> Result<Vec<String>> {
        ctx.data_unchecked::<DevqlGraphqlContext>()
            .list_agents()
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
        #[graphql(default = 100)] first: i32,
        after: Option<String>,
    ) -> Result<ArtefactConnection> {
        if let Some(filter) = filter.as_ref() {
            filter.validate()?;
        }
        let artefacts = ctx
            .data_unchecked::<DevqlGraphqlContext>()
            .list_artefacts(None, filter.as_ref(), &self.scope)
            .await
            .map_err(|err| {
                backend_error(format!("failed to query repository artefacts: {err:#}"))
            })?;
        let page = paginate_items(&artefacts, first, after.as_deref(), |artefact| {
            artefact.cursor()
        })?;
        Ok(ArtefactConnection::new(
            page.items.into_iter().map(ArtefactEdge::new).collect(),
            page.page_info,
            page.total_count,
        ))
    }

    async fn knowledge(
        &self,
        ctx: &Context<'_>,
        provider: Option<KnowledgeProvider>,
        #[graphql(default = 25)] first: i32,
        after: Option<String>,
    ) -> Result<KnowledgeItemConnection> {
        let items = ctx
            .data_unchecked::<DevqlGraphqlContext>()
            .list_knowledge_items(provider, &self.scope)
            .await
            .map_err(|err| {
                backend_error(format!("failed to query repository knowledge: {err:#}"))
            })?;
        let page = paginate_items(&items, first, after.as_deref(), |item| item.cursor())?;
        Ok(KnowledgeItemConnection::new(
            page.items.into_iter().map(KnowledgeItemEdge::new).collect(),
            page.page_info,
            page.total_count,
        ))
    }
}

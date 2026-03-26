use async_graphql::{ComplexObject, Context, ID, Result, SimpleObject};

use crate::graphql::{DevqlGraphqlContext, backend_error, bad_user_input_error};

use super::{
    ArtefactConnection, ArtefactEdge, ArtefactFilterInput, CommitConnection, CommitEdge,
    DateTimeScalar, FileContext, paginate_items,
};

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
#[graphql(complex)]
pub struct Repository {
    pub id: ID,
    pub name: String,
    pub provider: String,
    pub organization: String,
}

impl Repository {
    pub fn new(name: &str, provider: &str, organization: &str) -> Self {
        Self {
            id: ID(format!("repo://{provider}/{organization}/{name}")),
            name: name.to_string(),
            provider: provider.to_string(),
            organization: organization.to_string(),
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
            .validate_repo_relative_path(&path, false)
            .map_err(bad_user_input_error)?;
        ctx.data_unchecked::<DevqlGraphqlContext>()
            .resolve_file_context(&normalized)
            .await
            .map_err(|err| {
                backend_error(format!("failed to resolve file `{normalized}`: {err:#}"))
            })?
            .ok_or_else(|| bad_user_input_error(format!("unknown path `{normalized}`")))
    }

    async fn files(&self, ctx: &Context<'_>, path: String) -> Result<Vec<FileContext>> {
        let normalized = ctx
            .data_unchecked::<DevqlGraphqlContext>()
            .validate_repo_relative_path(&path, true)
            .map_err(bad_user_input_error)?;
        ctx.data_unchecked::<DevqlGraphqlContext>()
            .list_file_contexts(&normalized)
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
            .list_artefacts(None, filter.as_ref())
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
}

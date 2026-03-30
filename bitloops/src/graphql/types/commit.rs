use async_graphql::{ComplexObject, Context, Result, SimpleObject};

use crate::graphql::{DevqlGraphqlContext, ResolverScope, backend_error};

use super::{
    CheckpointConnection, CheckpointEdge, ConnectionPagination, DateTimeScalar, paginate_items,
};

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
#[graphql(complex)]
pub struct Commit {
    pub sha: String,
    pub parents: Vec<String>,
    pub author_name: String,
    pub author_email: String,
    pub commit_message: String,
    pub committed_at: DateTimeScalar,
    pub branch: Option<String>,
    #[graphql(skip)]
    pub(crate) scope: ResolverScope,
}

impl Commit {
    pub fn cursor(&self) -> String {
        self.sha.clone()
    }

    pub(crate) fn with_scope(mut self, scope: ResolverScope) -> Self {
        self.scope = scope;
        self
    }
}

#[ComplexObject]
impl Commit {
    async fn files_changed(&self, ctx: &Context<'_>) -> Result<Vec<String>> {
        ctx.data_unchecked::<DevqlGraphqlContext>()
            .list_commit_files_changed(&self.scope, &self.sha)
            .await
            .map_err(|err| {
                backend_error(format!(
                    "failed to read changed files for {}: {err:#}",
                    self.sha
                ))
            })
    }

    async fn checkpoints(
        &self,
        ctx: &Context<'_>,
        first: Option<i32>,
        after: Option<String>,
        last: Option<i32>,
        before: Option<String>,
    ) -> Result<CheckpointConnection> {
        let pagination = ConnectionPagination::from_graphql(
            10,
            first,
            after.as_deref(),
            last,
            before.as_deref(),
        )?;
        let checkpoints = ctx
            .data_unchecked::<DevqlGraphqlContext>()
            .list_commit_checkpoints(&self.scope, &self.sha)
            .await
            .map_err(|err| {
                backend_error(format!(
                    "failed to read checkpoints for commit {}: {err:#}",
                    self.sha
                ))
            })?;
        let page = paginate_items(&checkpoints, &pagination, |checkpoint| checkpoint.cursor())?;
        Ok(CheckpointConnection::new(
            page.items.into_iter().map(CheckpointEdge::new).collect(),
            page.page_info,
            page.total_count,
        ))
    }
}

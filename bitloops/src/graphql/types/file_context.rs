use async_graphql::{ComplexObject, Context, Result, SimpleObject};

use crate::graphql::{DevqlGraphqlContext, ResolverScope, backend_error, bad_cursor_error};

use super::{
    ArtefactConnection, ArtefactEdge, ArtefactFilterInput, CloneSummary, ClonesFilterInput,
    ConnectionPagination, DependencyConnectionEdge, DependencyEdgeConnection, DepsFilterInput,
    paginate_items,
};

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
#[graphql(complex)]
pub struct FileContext {
    pub path: String,
    pub language: Option<String>,
    pub blob_sha: Option<String>,
    #[graphql(skip)]
    pub(crate) scope: ResolverScope,
}

impl FileContext {
    pub(crate) fn with_scope(mut self, scope: ResolverScope) -> Self {
        self.scope = scope;
        self
    }
}

#[ComplexObject]
impl FileContext {
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
                .artefact_cursor_exists(
                    Some(self.path.as_str()),
                    filter.as_ref(),
                    &self.scope,
                    cursor,
                )
                .await
                .map_err(|err| {
                    backend_error(format!(
                        "failed to query artefacts for file {}: {err:#}",
                        self.path
                    ))
                })?;
            if !cursor_exists {
                return Err(bad_cursor_error(format!(
                    "cursor `{cursor}` does not match any result in this connection"
                )));
            }
        }

        let (artefacts, page_info, total_count) = context
            .query_artefact_connection(
                Some(self.path.as_str()),
                filter.as_ref(),
                &self.scope,
                &pagination,
            )
            .await
            .map_err(|err| {
                backend_error(format!(
                    "failed to query artefacts for file {}: {err:#}",
                    self.path
                ))
            })?;

        Ok(ArtefactConnection::new(
            artefacts.into_iter().map(ArtefactEdge::new).collect(),
            page_info,
            total_count,
        ))
    }

    #[graphql(name = "dependencies")]
    async fn deps(
        &self,
        ctx: &Context<'_>,
        filter: Option<DepsFilterInput>,
        first: Option<i32>,
        after: Option<String>,
        last: Option<i32>,
        before: Option<String>,
    ) -> Result<DependencyEdgeConnection> {
        let pagination = ConnectionPagination::from_graphql(
            100,
            first,
            after.as_deref(),
            last,
            before.as_deref(),
        )?;
        let deps = ctx
            .data_unchecked::<DevqlGraphqlContext>()
            .list_file_dependency_edges(self.path.as_str(), filter.as_ref(), &self.scope)
            .await
            .map_err(|err| {
                backend_error(format!(
                    "failed to query dependency edges for file {}: {err:#}",
                    self.path
                ))
            })?;
        let page = paginate_items(&deps, &pagination, |edge| edge.cursor())?;
        Ok(DependencyEdgeConnection::new(
            page.items
                .into_iter()
                .map(DependencyConnectionEdge::new)
                .collect(),
            page.page_info,
            page.total_count,
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
            return Err(crate::graphql::bad_user_input_error(
                "`clones` does not support historical or temporary `asOf(...)` scopes yet",
            ));
        }

        super::clone::resolve_clone_summary(
            ctx.data_unchecked::<DevqlGraphqlContext>(),
            Some(self.path.as_str()),
            filter.as_ref(),
            clone_filter.as_ref(),
            &self.scope,
        )
        .await
        .map_err(|err| backend_error(format!("failed to query file clone summary: {err:#}")))
    }
}

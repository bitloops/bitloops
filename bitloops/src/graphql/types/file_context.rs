use async_graphql::{ComplexObject, Context, Result, SimpleObject};

use crate::graphql::{
    DevqlGraphqlContext, ResolverScope, backend_error, bad_cursor_error, bad_user_input_error,
};

use super::{
    ArtefactConnection, ArtefactEdge, ArtefactFilterInput, DependencyConnectionEdge,
    DependencyEdgeConnection, DepsFilterInput, connection::PageInfo, paginate_items,
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
        #[graphql(default = 100)] first: i32,
        after: Option<String>,
    ) -> Result<ArtefactConnection> {
        if let Some(filter) = filter.as_ref() {
            filter.validate()?;
        }
        if first <= 0 {
            return Err(bad_user_input_error("`first` must be greater than zero"));
        }

        let context = ctx.data_unchecked::<DevqlGraphqlContext>();
        if filter
            .as_ref()
            .is_some_and(|filter| filter.needs_event_backed_filter())
        {
            let artefacts = context
                .list_artefacts(Some(self.path.as_str()), filter.as_ref(), &self.scope)
                .await
                .map_err(|err| {
                    backend_error(format!(
                        "failed to query artefacts for file {}: {err:#}",
                        self.path
                    ))
                })?;
            let page = paginate_items(&artefacts, first, after.as_deref(), |artefact| {
                artefact.cursor()
            })?;
            return Ok(ArtefactConnection::new(
                page.items.into_iter().map(ArtefactEdge::new).collect(),
                page.page_info,
                page.total_count,
            ));
        }

        let total_count = context
            .count_artefacts(Some(self.path.as_str()), filter.as_ref(), &self.scope)
            .await
            .map_err(|err| {
                backend_error(format!(
                    "failed to query artefacts for file {}: {err:#}",
                    self.path
                ))
            })?;

        if let Some(cursor) = after.as_deref() {
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

        let mut artefacts = context
            .list_artefacts_window(
                Some(self.path.as_str()),
                filter.as_ref(),
                &self.scope,
                after.as_deref(),
                first as usize + 1,
            )
            .await
            .map_err(|err| {
                backend_error(format!(
                    "failed to query artefacts for file {}: {err:#}",
                    self.path
                ))
            })?;
        let has_next_page = artefacts.len() > first as usize;
        artefacts.truncate(first as usize);
        let start_cursor = artefacts.first().map(|artefact| artefact.cursor());
        let end_cursor = artefacts.last().map(|artefact| artefact.cursor());

        Ok(ArtefactConnection::new(
            artefacts.into_iter().map(ArtefactEdge::new).collect(),
            PageInfo {
                has_next_page,
                has_previous_page: after.is_some(),
                start_cursor,
                end_cursor,
            },
            total_count,
        ))
    }

    async fn deps(
        &self,
        ctx: &Context<'_>,
        filter: Option<DepsFilterInput>,
        #[graphql(default = 100)] first: i32,
        after: Option<String>,
    ) -> Result<DependencyEdgeConnection> {
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
        let page = paginate_items(&deps, first, after.as_deref(), |edge| edge.cursor())?;
        Ok(DependencyEdgeConnection::new(
            page.items
                .into_iter()
                .map(DependencyConnectionEdge::new)
                .collect(),
            page.page_info,
            page.total_count,
        ))
    }
}

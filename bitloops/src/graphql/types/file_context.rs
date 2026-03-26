use async_graphql::{ComplexObject, Context, Result, SimpleObject};

use crate::graphql::{DevqlGraphqlContext, ResolverScope, backend_error};

use super::{
    ArtefactConnection, ArtefactEdge, ArtefactFilterInput, DependencyConnectionEdge,
    DependencyEdgeConnection, DepsFilterInput, paginate_items,
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
        let artefacts = ctx
            .data_unchecked::<DevqlGraphqlContext>()
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
        Ok(ArtefactConnection::new(
            page.items.into_iter().map(ArtefactEdge::new).collect(),
            page.page_info,
            page.total_count,
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

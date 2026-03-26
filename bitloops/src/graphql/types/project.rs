use async_graphql::{ComplexObject, Context, Result, SimpleObject};

use crate::graphql::{DevqlGraphqlContext, ResolverScope, backend_error, bad_user_input_error};

use super::{
    ArtefactConnection, ArtefactEdge, ArtefactFilterInput, DependencyConnectionEdge,
    DependencyEdgeConnection, DepsFilterInput, FileContext, paginate_items,
};

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
#[graphql(complex)]
pub struct Project {
    pub path: String,
    #[graphql(skip)]
    pub(crate) scope: ResolverScope,
}

impl Project {
    pub fn new(path: String, scope: ResolverScope) -> Self {
        Self { path, scope }
    }
}

#[ComplexObject]
impl Project {
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
            .map_err(|err| backend_error(format!("failed to query project artefacts: {err:#}")))?;
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
            .list_project_dependency_edges(&self.scope, filter.as_ref())
            .await
            .map_err(|err| {
                backend_error(format!("failed to query project dependency edges: {err:#}"))
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

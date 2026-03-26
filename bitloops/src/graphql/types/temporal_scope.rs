use async_graphql::{ComplexObject, Context, Enum, InputObject, Result, SimpleObject};

use crate::graphql::{
    DevqlGraphqlContext, ResolvedTemporalScope, ResolverScope, backend_error, bad_user_input_error,
};

use super::{
    ArtefactConnection, ArtefactEdge, ArtefactFilterInput, FileContext, Project, paginate_items,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum SaveSelector {
    Current,
}

#[derive(Debug, Clone, InputObject)]
pub struct AsOfInput {
    pub r#ref: Option<String>,
    pub commit: Option<String>,
    pub save: Option<SaveSelector>,
    #[graphql(name = "saveRevision")]
    pub save_revision: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AsOfSelector<'a> {
    Ref(&'a str),
    Commit(&'a str),
    SaveCurrent,
    SaveRevision(&'a str),
}

impl AsOfInput {
    pub(crate) fn selector(&self) -> std::result::Result<AsOfSelector<'_>, String> {
        let reference = self
            .r#ref
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let commit = self
            .commit
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let save_revision = self
            .save_revision
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let save = self.save;

        let selected = usize::from(reference.is_some())
            + usize::from(commit.is_some())
            + usize::from(save.is_some())
            + usize::from(save_revision.is_some());

        if selected != 1 {
            return Err(
                "asOf(input: ...) requires exactly one of `ref`, `commit`, `save`, or `saveRevision`"
                    .to_string(),
            );
        }

        if let Some(reference) = reference {
            return Ok(AsOfSelector::Ref(reference));
        }
        if let Some(commit) = commit {
            return Ok(AsOfSelector::Commit(commit));
        }
        if let Some(save_revision) = save_revision {
            return Ok(AsOfSelector::SaveRevision(save_revision));
        }

        match save {
            Some(SaveSelector::Current) => Ok(AsOfSelector::SaveCurrent),
            None => Err(
                "asOf(input: ...) requires exactly one of `ref`, `commit`, `save`, or `saveRevision`"
                    .to_string(),
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
#[graphql(complex)]
pub struct TemporalScope {
    pub resolved_commit: String,
    #[graphql(skip)]
    pub(crate) scope: ResolverScope,
}

impl TemporalScope {
    pub(crate) fn new(resolved: &ResolvedTemporalScope, scope: ResolverScope) -> Self {
        Self {
            resolved_commit: resolved.resolved_commit().to_string(),
            scope,
        }
    }
}

#[ComplexObject]
impl TemporalScope {
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
                backend_error(format!(
                    "failed to query temporally scoped artefacts: {err:#}"
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
}

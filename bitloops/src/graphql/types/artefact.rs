use async_graphql::{ComplexObject, Context, Enum, ID, InputObject, Result, SimpleObject};

use crate::graphql::{
    DevqlGraphqlContext, ResolverScope, backend_error, bad_user_input_error, loaders::DataLoaders,
};

use super::{
    ArtefactConnection, ArtefactEdge, ChatEntryConnection, ChatEntryEdge, CloneConnection,
    CloneEdge, ClonesFilterInput, DateTimeScalar, DependencyConnectionEdge,
    DependencyEdgeConnection, DepsFilterInput, paginate_items,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Enum)]
pub enum CanonicalKind {
    File,
    Namespace,
    Module,
    Import,
    Type,
    Interface,
    Enum,
    Callable,
    Function,
    Method,
    Value,
    Variable,
    Member,
    Parameter,
    TypeParameter,
    Alias,
}

impl CanonicalKind {
    pub(crate) fn as_devql_value(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Namespace => "namespace",
            Self::Module => "module",
            Self::Import => "import",
            Self::Type => "type",
            Self::Interface => "interface",
            Self::Enum => "enum",
            Self::Callable => "callable",
            Self::Function => "function",
            Self::Method => "method",
            Self::Value => "value",
            Self::Variable => "variable",
            Self::Member => "member",
            Self::Parameter => "parameter",
            Self::TypeParameter => "type_parameter",
            Self::Alias => "alias",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, InputObject)]
pub struct LineRangeInput {
    pub start: i32,
    pub end: i32,
}

impl LineRangeInput {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.start <= 0 || self.end <= 0 || self.end < self.start {
            return Err(bad_user_input_error(format!(
                "invalid line range {}..{}",
                self.start, self.end
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, InputObject, Default)]
pub struct ArtefactFilterInput {
    pub kind: Option<CanonicalKind>,
    pub symbol_fqn: Option<String>,
    pub lines: Option<LineRangeInput>,
    pub agent: Option<String>,
    pub since: Option<DateTimeScalar>,
}

impl ArtefactFilterInput {
    pub(crate) fn validate(&self) -> Result<()> {
        if let Some(lines) = self.lines.as_ref() {
            lines.validate()?;
        }
        Ok(())
    }

    pub(crate) fn needs_event_backed_filter(&self) -> bool {
        self.agent.is_some() || self.since.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
#[graphql(complex)]
pub struct Artefact {
    pub id: ID,
    pub symbol_id: String,
    pub path: String,
    pub language: String,
    pub canonical_kind: CanonicalKind,
    pub language_kind: Option<String>,
    pub symbol_fqn: Option<String>,
    pub parent_artefact_id: Option<ID>,
    pub start_line: i32,
    pub end_line: i32,
    pub start_byte: i32,
    pub end_byte: i32,
    pub signature: Option<String>,
    pub modifiers: Vec<String>,
    pub docstring: Option<String>,
    pub content_hash: Option<String>,
    pub blob_sha: String,
    pub created_at: DateTimeScalar,
    #[graphql(skip)]
    pub(crate) scope: ResolverScope,
}

impl Artefact {
    pub fn cursor(&self) -> String {
        self.id.to_string()
    }

    pub(crate) fn with_scope(mut self, scope: ResolverScope) -> Self {
        self.scope = scope;
        self
    }
}

#[ComplexObject]
impl Artefact {
    async fn parent(&self, ctx: &Context<'_>) -> Result<Option<Artefact>> {
        let Some(parent_id) = self.parent_artefact_id.as_ref() else {
            return Ok(None);
        };

        ctx.data_unchecked::<DataLoaders>()
            .load_artefact_by_id(parent_id.as_ref(), &self.scope)
            .await
            .map_err(|err| {
                backend_error(format!(
                    "failed to resolve parent artefact {} for {}: {err:#}",
                    parent_id.as_ref(),
                    self.id.as_ref()
                ))
            })
    }

    async fn children(
        &self,
        ctx: &Context<'_>,
        #[graphql(default = 50)] first: i32,
        after: Option<String>,
    ) -> Result<ArtefactConnection> {
        let artefacts = ctx
            .data_unchecked::<DevqlGraphqlContext>()
            .list_child_artefacts(self.id.as_ref(), &self.scope)
            .await
            .map_err(|err| {
                backend_error(format!(
                    "failed to resolve child artefacts for {}: {err:#}",
                    self.id.as_ref()
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

    async fn outgoing_deps(
        &self,
        ctx: &Context<'_>,
        filter: Option<DepsFilterInput>,
        #[graphql(default = 50)] first: i32,
        after: Option<String>,
    ) -> Result<DependencyEdgeConnection> {
        let deps = ctx
            .data_unchecked::<DataLoaders>()
            .load_outgoing_edges(self.id.as_ref(), filter, &self.scope)
            .await
            .map_err(|err| {
                backend_error(format!(
                    "failed to resolve outgoing dependencies for {}: {err:#}",
                    self.id.as_ref()
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

    async fn incoming_deps(
        &self,
        ctx: &Context<'_>,
        filter: Option<DepsFilterInput>,
        #[graphql(default = 50)] first: i32,
        after: Option<String>,
    ) -> Result<DependencyEdgeConnection> {
        let deps = ctx
            .data_unchecked::<DataLoaders>()
            .load_incoming_edges(self.id.as_ref(), filter, &self.scope)
            .await
            .map_err(|err| {
                backend_error(format!(
                    "failed to resolve incoming dependencies for {}: {err:#}",
                    self.id.as_ref()
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

    async fn clones(
        &self,
        ctx: &Context<'_>,
        filter: Option<ClonesFilterInput>,
        #[graphql(default = 20)] first: i32,
        after: Option<String>,
    ) -> Result<CloneConnection> {
        if let Some(filter) = filter.as_ref() {
            filter.validate()?;
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

        let clones = ctx
            .data_unchecked::<DevqlGraphqlContext>()
            .list_artefact_clones(self.id.as_ref(), filter.as_ref(), &self.scope)
            .await
            .map_err(|err| {
                backend_error(format!(
                    "failed to resolve semantic clones for artefact {}: {err:#}",
                    self.id.as_ref()
                ))
            })?;
        let page = paginate_items(&clones, first, after.as_deref(), |clone| clone.cursor())?;
        Ok(CloneConnection::new(
            page.items.into_iter().map(CloneEdge::new).collect(),
            page.page_info,
            page.total_count,
        ))
    }

    async fn chat_history(
        &self,
        ctx: &Context<'_>,
        #[graphql(default = 10)] first: i32,
        after: Option<String>,
    ) -> Result<ChatEntryConnection> {
        let history = ctx
            .data_unchecked::<DataLoaders>()
            .load_chat_history(self.path.as_str(), self.symbol_fqn.as_deref(), &self.scope)
            .await
            .map_err(|err| {
                backend_error(format!(
                    "failed to resolve chat history for artefact {}: {err:#}",
                    self.id.as_ref()
                ))
            })?;
        let page = paginate_items(&history, first, after.as_deref(), |entry| entry.cursor())?;
        Ok(ChatEntryConnection::new(
            page.items.into_iter().map(ChatEntryEdge::new).collect(),
            page.page_info,
            page.total_count,
        ))
    }
}

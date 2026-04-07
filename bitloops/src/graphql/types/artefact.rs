use async_graphql::{ComplexObject, Context, Enum, ID, InputObject, Result, SimpleObject};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::graphql::pack_adapter::StageResolverAdapter;
use crate::graphql::{
    DevqlGraphqlContext, ResolverScope, backend_error, bad_user_input_error, loaders::DataLoaders,
};

use super::{
    ArtefactConnection, ArtefactEdge, ChatEntryConnection, ChatEntryEdge, CloneConnection,
    CloneEdge, CloneSummary, ClonesFilterInput, ConnectionPagination, DateTimeScalar,
    DependencyConnectionEdge, DependencyEdgeConnection, DepsFilterInput, TestHarnessCoverageResult,
    TestHarnessTestsResult, paginate_items,
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
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
#[graphql(complex)]
pub struct ArtefactCopyLineage {
    pub checkpoint_id: String,
    pub event_time: DateTimeScalar,
    pub commit_sha: String,
    pub source_symbol_id: String,
    pub source_artefact_id: ID,
    pub dest_symbol_id: String,
    pub dest_artefact_id: ID,
    #[graphql(skip)]
    pub(crate) scope: ResolverScope,
}

#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
#[graphql(complex)]
pub struct Artefact {
    pub id: ID,
    pub symbol_id: String,
    pub path: String,
    pub language: String,
    pub canonical_kind: Option<CanonicalKind>,
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
impl ArtefactCopyLineage {
    async fn source(&self, ctx: &Context<'_>) -> Result<Option<Artefact>> {
        ctx.data_unchecked::<DataLoaders>()
            .load_artefact_by_id(self.source_artefact_id.as_ref(), &self.scope)
            .await
            .map_err(|err| {
                backend_error(format!(
                    "failed to resolve source artefact {}: {err:#}",
                    self.source_artefact_id.as_ref()
                ))
            })
    }

    async fn destination(&self, ctx: &Context<'_>) -> Result<Option<Artefact>> {
        ctx.data_unchecked::<DataLoaders>()
            .load_artefact_by_id(self.dest_artefact_id.as_ref(), &self.scope)
            .await
            .map_err(|err| {
                backend_error(format!(
                    "failed to resolve destination artefact {}: {err:#}",
                    self.dest_artefact_id.as_ref()
                ))
            })
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
        first: Option<i32>,
        after: Option<String>,
        last: Option<i32>,
        before: Option<String>,
    ) -> Result<ArtefactConnection> {
        let pagination = ConnectionPagination::from_graphql(
            50,
            first,
            after.as_deref(),
            last,
            before.as_deref(),
        )?;
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
        let page = paginate_items(&artefacts, &pagination, |artefact| artefact.cursor())?;
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
        first: Option<i32>,
        after: Option<String>,
        last: Option<i32>,
        before: Option<String>,
    ) -> Result<DependencyEdgeConnection> {
        let pagination = ConnectionPagination::from_graphql(
            50,
            first,
            after.as_deref(),
            last,
            before.as_deref(),
        )?;
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

    async fn incoming_deps(
        &self,
        ctx: &Context<'_>,
        filter: Option<DepsFilterInput>,
        first: Option<i32>,
        after: Option<String>,
        last: Option<i32>,
        before: Option<String>,
    ) -> Result<DependencyEdgeConnection> {
        let pagination = ConnectionPagination::from_graphql(
            50,
            first,
            after.as_deref(),
            last,
            before.as_deref(),
        )?;
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

    async fn clones(
        &self,
        ctx: &Context<'_>,
        filter: Option<ClonesFilterInput>,
        first: Option<i32>,
        after: Option<String>,
        last: Option<i32>,
        before: Option<String>,
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
        let pagination = ConnectionPagination::from_graphql(
            20,
            first,
            after.as_deref(),
            last,
            before.as_deref(),
        )?;

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
        let summary = CloneSummary::from_clones(&clones);
        let page = paginate_items(&clones, &pagination, |clone| clone.cursor())?;
        Ok(CloneConnection::new(
            page.items.into_iter().map(CloneEdge::new).collect(),
            page.page_info,
            page.total_count,
            summary,
        ))
    }

    async fn chat_history(
        &self,
        ctx: &Context<'_>,
        first: Option<i32>,
        after: Option<String>,
        last: Option<i32>,
        before: Option<String>,
    ) -> Result<ChatEntryConnection> {
        let pagination = ConnectionPagination::from_graphql(
            10,
            first,
            after.as_deref(),
            last,
            before.as_deref(),
        )?;
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
        let page = paginate_items(&history, &pagination, |entry| entry.cursor())?;
        Ok(ChatEntryConnection::new(
            page.items.into_iter().map(ChatEntryEdge::new).collect(),
            page.page_info,
            page.total_count,
        ))
    }

    async fn tests(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "minConfidence")] min_confidence: Option<f64>,
        #[graphql(name = "linkageSource")] linkage_source: Option<String>,
        #[graphql(default = 25)] first: i32,
    ) -> Result<Vec<TestHarnessTestsResult>> {
        let args = build_tests_stage_args(min_confidence, linkage_source)?;
        decode_stage_rows(
            "tests",
            StageResolverAdapter::new(ctx.data_unchecked::<DevqlGraphqlContext>().clone(), "tests")
                .resolve(
                    &self.scope,
                    vec![artefact_stage_row(self)],
                    Some(args),
                    stage_limit(first)?,
                )
                .await
                .map_err(|err| map_stage_adapter_error(self.id.as_ref(), "tests", err))?,
        )
    }

    async fn coverage(
        &self,
        ctx: &Context<'_>,
        #[graphql(default = 25)] first: i32,
    ) -> Result<Vec<TestHarnessCoverageResult>> {
        decode_stage_rows(
            "coverage",
            StageResolverAdapter::new(
                ctx.data_unchecked::<DevqlGraphqlContext>().clone(),
                "coverage",
            )
            .resolve(
                &self.scope,
                vec![artefact_stage_row(self)],
                None,
                stage_limit(first)?,
            )
            .await
            .map_err(|err| map_stage_adapter_error(self.id.as_ref(), "coverage", err))?,
        )
    }

    async fn copy_lineage(&self, ctx: &Context<'_>) -> Result<Vec<ArtefactCopyLineage>> {
        ctx.data_unchecked::<DevqlGraphqlContext>()
            .list_artefact_copy_lineage(self.id.as_ref(), &self.scope)
            .await
            .map_err(|err| {
                backend_error(format!(
                    "failed to resolve checkpoint copy lineage for {}: {err:#}",
                    self.id.as_ref()
                ))
            })
    }
}

fn stage_limit(first: i32) -> Result<usize> {
    if first <= 0 {
        return Err(bad_user_input_error("`first` must be greater than 0"));
    }
    Ok(first as usize)
}

fn build_tests_stage_args(
    min_confidence: Option<f64>,
    linkage_source: Option<String>,
) -> Result<Value> {
    if let Some(min_confidence) = min_confidence
        && !(0.0..=1.0).contains(&min_confidence)
    {
        return Err(bad_user_input_error(
            "`minConfidence` must be between 0 and 1",
        ));
    }

    let mut args = serde_json::Map::new();
    if let Some(min_confidence) = min_confidence {
        args.insert("min_confidence".to_string(), json!(min_confidence));
    }
    if let Some(linkage_source) = linkage_source
        && !linkage_source.trim().is_empty()
    {
        args.insert(
            "linkage_source".to_string(),
            Value::String(linkage_source.trim().to_string()),
        );
    }
    Ok(Value::Object(args))
}

fn artefact_stage_row(artefact: &Artefact) -> Value {
    json!({
    "artefact_id": artefact.id.as_ref(),
    "symbol_id": &artefact.symbol_id,
    "symbol_fqn": &artefact.symbol_fqn,
    "canonical_kind": artefact.canonical_kind.map(|kind| kind.as_devql_value()),
    "path": &artefact.path,
    "start_line": artefact.start_line,
    "end_line": artefact.end_line,
        })
}

fn decode_stage_rows<T: DeserializeOwned>(stage: &str, rows: Vec<Value>) -> Result<Vec<T>> {
    rows.into_iter()
        .map(|row| {
            serde_json::from_value(row).map_err(|err| {
                backend_error(format!(
                    "failed to decode `{stage}` stage payload into typed GraphQL result: {err}"
                ))
            })
        })
        .collect()
}

fn map_stage_adapter_error(
    artefact_id: &str,
    stage: &str,
    err: anyhow::Error,
) -> async_graphql::Error {
    let message = format!("{err:#}");
    if message.contains("unsupported DevQL stage")
        || message.contains("ambiguous DevQL stage")
        || message.contains("extension args must")
        || message.contains("requires a resolved commit")
    {
        return bad_user_input_error(message);
    }
    backend_error(format!(
        "failed to resolve `{stage}` stage for artefact {artefact_id}: {message}"
    ))
}

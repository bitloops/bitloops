use async_graphql::{ComplexObject, Context, Result, SimpleObject};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::graphql::pack_adapter::StageResolverAdapter;
use crate::graphql::{
    DevqlGraphqlContext, ResolverScope, backend_error, bad_cursor_error, bad_user_input_error,
};

use super::{
    ArtefactConnection, ArtefactEdge, ArtefactFilterInput, AsOfInput, CheckpointConnection,
    CheckpointEdge, CloneConnection, CloneEdge, ClonesFilterInput, ConnectionPagination,
    DateTimeScalar, DependencyConnectionEdge, DependencyEdgeConnection, DepsFilterInput,
    FileContext, KnowledgeItemConnection, KnowledgeItemEdge, KnowledgeProvider, TemporalScope,
    TestHarnessCommitSummary, TestHarnessCoverageResult, TestHarnessTestsResult, paginate_items,
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
    #[graphql(name = "asOf")]
    async fn as_of(&self, ctx: &Context<'_>, input: AsOfInput) -> Result<TemporalScope> {
        let context = ctx.data_unchecked::<DevqlGraphqlContext>();
        let temporal_scope = context
            .resolve_temporal_scope(&self.scope, &input)
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
                .artefact_cursor_exists(None, filter.as_ref(), &self.scope, cursor)
                .await
                .map_err(|err| {
                    backend_error(format!("failed to query project artefacts: {err:#}"))
                })?;
            if !cursor_exists {
                return Err(bad_cursor_error(format!(
                    "cursor `{cursor}` does not match any result in this connection"
                )));
            }
        }

        let (artefacts, page_info, total_count) = context
            .query_artefact_connection(None, filter.as_ref(), &self.scope, &pagination)
            .await
            .map_err(|err| backend_error(format!("failed to query project artefacts: {err:#}")))?;

        Ok(ArtefactConnection::new(
            artefacts.into_iter().map(ArtefactEdge::new).collect(),
            page_info,
            total_count,
        ))
    }

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
            .list_project_dependency_edges(&self.scope, filter.as_ref())
            .await
            .map_err(|err| {
                backend_error(format!("failed to query project dependency edges: {err:#}"))
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

    #[allow(clippy::too_many_arguments)]
    async fn checkpoints(
        &self,
        ctx: &Context<'_>,
        agent: Option<String>,
        since: Option<DateTimeScalar>,
        first: Option<i32>,
        after: Option<String>,
        last: Option<i32>,
        before: Option<String>,
    ) -> Result<CheckpointConnection> {
        let pagination = ConnectionPagination::from_graphql(
            50,
            first,
            after.as_deref(),
            last,
            before.as_deref(),
        )?;
        let checkpoints = ctx
            .data_unchecked::<DevqlGraphqlContext>()
            .list_checkpoints(&self.scope, agent.as_deref(), since.as_ref())
            .await
            .map_err(|err| {
                backend_error(format!("failed to query project checkpoints: {err:#}"))
            })?;
        let page = paginate_items(&checkpoints, &pagination, |checkpoint| checkpoint.cursor())?;
        Ok(CheckpointConnection::new(
            page.items.into_iter().map(CheckpointEdge::new).collect(),
            page.page_info,
            page.total_count,
        ))
    }

    async fn knowledge(
        &self,
        ctx: &Context<'_>,
        provider: Option<KnowledgeProvider>,
        first: Option<i32>,
        after: Option<String>,
        last: Option<i32>,
        before: Option<String>,
    ) -> Result<KnowledgeItemConnection> {
        let pagination = ConnectionPagination::from_graphql(
            25,
            first,
            after.as_deref(),
            last,
            before.as_deref(),
        )?;
        let items = ctx
            .data_unchecked::<DevqlGraphqlContext>()
            .list_knowledge_items(provider, &self.scope)
            .await
            .map_err(|err| backend_error(format!("failed to query project knowledge: {err:#}")))?;
        let page = paginate_items(&items, &pagination, |item| item.cursor())?;
        Ok(KnowledgeItemConnection::new(
            page.items.into_iter().map(KnowledgeItemEdge::new).collect(),
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
            filter.validate_project_scope()?;
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
            50,
            first,
            after.as_deref(),
            last,
            before.as_deref(),
        )?;

        let clones = ctx
            .data_unchecked::<DevqlGraphqlContext>()
            .list_project_clones(&self.scope, filter.as_ref())
            .await
            .map_err(|err| {
                backend_error(format!(
                    "failed to query project semantic clones for {}: {err:#}",
                    self.path
                ))
            })?;
        let page = paginate_items(&clones, &pagination, |clone| clone.cursor())?;
        Ok(CloneConnection::new(
            page.items.into_iter().map(CloneEdge::new).collect(),
            page.page_info,
            page.total_count,
        ))
    }

    async fn tests(
        &self,
        ctx: &Context<'_>,
        filter: Option<ArtefactFilterInput>,
        #[graphql(name = "minConfidence")] min_confidence: Option<f64>,
        #[graphql(name = "linkageSource")] linkage_source: Option<String>,
        #[graphql(default = 100)] first: i32,
    ) -> Result<Vec<TestHarnessTestsResult>> {
        if let Some(filter) = filter.as_ref() {
            filter.validate()?;
        }

        let artefacts = ctx
            .data_unchecked::<DevqlGraphqlContext>()
            .list_artefacts(None, filter.as_ref(), &self.scope)
            .await
            .map_err(|err| backend_error(format!("failed to query project artefacts: {err:#}")))?;
        let args = build_tests_stage_args(min_confidence, linkage_source)?;
        decode_stage_rows(
            "tests",
            StageResolverAdapter::new(ctx.data_unchecked::<DevqlGraphqlContext>().clone(), "tests")
                .resolve(
                    &self.scope,
                    artefacts
                        .iter()
                        .map(project_stage_row_from_artefact)
                        .collect(),
                    Some(args),
                    stage_limit(first)?,
                )
                .await
                .map_err(|err| map_stage_adapter_error("project tests", err))?,
        )
    }

    async fn coverage(
        &self,
        ctx: &Context<'_>,
        filter: Option<ArtefactFilterInput>,
        #[graphql(default = 100)] first: i32,
    ) -> Result<Vec<TestHarnessCoverageResult>> {
        if let Some(filter) = filter.as_ref() {
            filter.validate()?;
        }

        let artefacts = ctx
            .data_unchecked::<DevqlGraphqlContext>()
            .list_artefacts(None, filter.as_ref(), &self.scope)
            .await
            .map_err(|err| backend_error(format!("failed to query project artefacts: {err:#}")))?;
        decode_stage_rows(
            "coverage",
            StageResolverAdapter::new(
                ctx.data_unchecked::<DevqlGraphqlContext>().clone(),
                "coverage",
            )
            .resolve(
                &self.scope,
                artefacts
                    .iter()
                    .map(project_stage_row_from_artefact)
                    .collect(),
                None,
                stage_limit(first)?,
            )
            .await
            .map_err(|err| map_stage_adapter_error("project coverage", err))?,
        )
    }

    #[graphql(name = "testsSummary")]
    async fn tests_summary(&self, ctx: &Context<'_>) -> Result<TestHarnessCommitSummary> {
        let artefacts = ctx
            .data_unchecked::<DevqlGraphqlContext>()
            .list_artefacts(None, None, &self.scope)
            .await
            .map_err(|err| backend_error(format!("failed to query project artefacts: {err:#}")))?;
        let rows = StageResolverAdapter::new(
            ctx.data_unchecked::<DevqlGraphqlContext>().clone(),
            "test_harness_tests_summary",
        )
        .resolve(
            &self.scope,
            artefacts
                .iter()
                .map(project_stage_row_from_artefact)
                .collect(),
            None,
            1,
        )
        .await
        .map_err(|err| map_stage_adapter_error("project tests summary", err))?;
        decode_stage_single("test_harness_tests_summary", rows)
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

fn project_stage_row_from_artefact(artefact: &super::Artefact) -> Value {
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

fn decode_stage_single<T: DeserializeOwned>(stage: &str, rows: Vec<Value>) -> Result<T> {
    let Some(row) = rows.into_iter().next() else {
        return Err(backend_error(format!(
            "failed to decode `{stage}` stage payload: empty result"
        )));
    };
    serde_json::from_value(row).map_err(|err| {
        backend_error(format!(
            "failed to decode `{stage}` stage payload into typed GraphQL result: {err}"
        ))
    })
}

fn map_stage_adapter_error(scope: &str, err: anyhow::Error) -> async_graphql::Error {
    let message = format!("{err:#}");
    if message.contains("unsupported DevQL stage")
        || message.contains("ambiguous DevQL stage")
        || message.contains("extension args must")
        || message.contains("requires a resolved commit")
    {
        return bad_user_input_error(message);
    }
    backend_error(format!("failed to resolve {scope}: {message}"))
}

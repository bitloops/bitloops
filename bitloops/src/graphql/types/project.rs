use async_graphql::{ComplexObject, Context, Result, SimpleObject};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::graphql::pack_adapter::StageResolverAdapter;
use crate::graphql::{
    DevqlGraphqlContext, ResolverScope, backend_error, bad_cursor_error, bad_user_input_error,
};

use super::{
    ArchitectureContainer, ArchitectureGraph, ArchitectureGraphFilterInput, ArchitectureGraphFlow,
    ArchitectureGraphNode, ArtefactConnection, ArtefactEdge, ArtefactFilterInput, AsOfInput,
    CheckpointConnection, CheckpointEdge, CloneConnection, CloneEdge, CloneSummary,
    ClonesFilterInput, CodeCityArcConnectionResult, CodeCityArcFilterInput,
    CodeCityArchitectureResult, CodeCityFileDetailResult, CodeCitySnapshotStatusResult,
    CodeCityViolationConnectionResult, CodeCityViolationFilterInput, CodeCityWorldResult,
    ConnectionPagination, DateTimeScalar, DependencyConnectionEdge, DependencyEdgeConnection,
    DepsFilterInput, FileContext, KnowledgeItemConnection, KnowledgeItemEdge, KnowledgeProvider,
    NavigationContextFilterInput, NavigationContextSnapshot, TemporalScope,
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

    #[graphql(name = "navigationContext")]
    async fn navigation_context(
        &self,
        ctx: &Context<'_>,
        filter: Option<NavigationContextFilterInput>,
        #[graphql(default = 500)] first: i32,
        after: Option<String>,
    ) -> Result<NavigationContextSnapshot> {
        let first = stage_limit(first)?;
        ctx.data_unchecked::<DevqlGraphqlContext>()
            .list_navigation_context(&self.scope, filter.as_ref(), Some(first), after.as_deref())
            .await
            .map_err(|err| backend_error(format!("failed to query navigation context: {err:#}")))
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
        let summary = CloneSummary::from_clones(&clones);
        let page = paginate_items(&clones, &pagination, |clone| clone.cursor())?;
        Ok(CloneConnection::new(
            page.items.into_iter().map(CloneEdge::new).collect(),
            page.page_info,
            page.total_count,
            summary,
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
            return Err(bad_user_input_error(
                "`clones` does not support historical or temporary `asOf(...)` scopes yet",
            ));
        }

        super::clone::resolve_clone_summary(
            ctx.data_unchecked::<DevqlGraphqlContext>(),
            None,
            filter.as_ref(),
            clone_filter.as_ref(),
            &self.scope,
        )
        .await
        .map_err(|err| backend_error(format!("failed to query project clone summary: {err:#}")))
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

    #[graphql(name = "architectureGraph")]
    async fn architecture_graph(
        &self,
        ctx: &Context<'_>,
        filter: Option<ArchitectureGraphFilterInput>,
        first: Option<i32>,
        after: Option<String>,
    ) -> Result<ArchitectureGraph> {
        let context = ctx.data_unchecked::<DevqlGraphqlContext>();
        let first = optional_positive_limit("first", first)?;
        let filter = normalise_architecture_graph_filter(context, &self.scope, filter)?;
        context
            .list_architecture_graph(&self.scope, filter.as_ref(), first, after.as_deref())
            .await
            .map_err(|err| backend_error(format!("failed to query architecture graph: {err:#}")))
    }

    #[graphql(name = "architectureEntryPoints")]
    async fn architecture_entry_points(
        &self,
        ctx: &Context<'_>,
        kind: Option<String>,
        first: Option<i32>,
    ) -> Result<Vec<ArchitectureGraphNode>> {
        let first = optional_positive_limit("first", first)?;
        ctx.data_unchecked::<DevqlGraphqlContext>()
            .list_architecture_entry_points(&self.scope, kind.as_deref(), first)
            .await
            .map_err(|err| {
                backend_error(format!(
                    "failed to query architecture entry points: {err:#}"
                ))
            })
    }

    #[graphql(name = "architectureFlows")]
    async fn architecture_flows(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "entryPointId")] entry_point_id: Option<String>,
        first: Option<i32>,
    ) -> Result<Vec<ArchitectureGraphFlow>> {
        let first = optional_positive_limit("first", first)?;
        ctx.data_unchecked::<DevqlGraphqlContext>()
            .list_architecture_flows(&self.scope, entry_point_id.as_deref(), first)
            .await
            .map_err(|err| backend_error(format!("failed to query architecture flows: {err:#}")))
    }

    #[graphql(name = "architectureContainers")]
    async fn architecture_containers(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "systemKey")] system_key: Option<String>,
        first: Option<i32>,
    ) -> Result<Vec<ArchitectureContainer>> {
        let first = optional_positive_limit("first", first)?;
        ctx.data_unchecked::<DevqlGraphqlContext>()
            .list_architecture_containers(&self.scope, system_key.as_deref(), first)
            .await
            .map_err(|err| {
                backend_error(format!("failed to query architecture containers: {err:#}"))
            })
    }

    #[allow(clippy::too_many_arguments)]
    #[graphql(name = "codeCityWorld")]
    async fn code_city_world(
        &self,
        ctx: &Context<'_>,
        #[graphql(default = 500)] first: i32,
        #[graphql(name = "includeDependencyArcs")] include_dependency_arcs: Option<bool>,
        #[graphql(name = "includeBoundaries")] include_boundaries: Option<bool>,
        #[graphql(name = "includeArchitecture")] include_architecture: Option<bool>,
        #[graphql(name = "includeMacroEdges")] include_macro_edges: Option<bool>,
        #[graphql(name = "includeZoneDiagnostics")] include_zone_diagnostics: Option<bool>,
        #[graphql(name = "architectureEnabled")] architecture_enabled: Option<bool>,
        #[graphql(name = "includeHealth")] include_health: Option<bool>,
        #[graphql(name = "analysisWindowMonths")] analysis_window_months: Option<i32>,
    ) -> Result<CodeCityWorldResult> {
        if self.scope.temporal_scope().is_some() {
            return Err(bad_user_input_error(
                "`codeCityWorld` does not support historical or temporary `asOf(...)` scopes",
            ));
        }
        if let Some(months) = analysis_window_months
            && !(1..=60).contains(&months)
        {
            return Err(bad_user_input_error(
                "`analysisWindowMonths` must be between 1 and 60",
            ));
        }

        let rows = StageResolverAdapter::new(
            ctx.data_unchecked::<DevqlGraphqlContext>().clone(),
            "codecity_world",
        )
        .resolve(
            &self.scope,
            Vec::new(),
            Some(build_codecity_stage_args(CodeCityStageArgs {
                include_dependency_arcs,
                include_boundaries,
                include_architecture,
                include_macro_edges,
                include_zone_diagnostics,
                architecture_enabled,
                include_health,
                analysis_window_months,
            })),
            stage_limit(first)?,
        )
        .await
        .map_err(|err| map_stage_adapter_error("project codeCityWorld", err))?;
        decode_stage_single("codecity_world", rows)
    }

    #[graphql(name = "codeCitySnapshotStatus")]
    async fn code_city_snapshot_status(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "projectPath")] project_path: Option<String>,
    ) -> Result<CodeCitySnapshotStatusResult> {
        if self.scope.temporal_scope().is_some() {
            return Err(bad_user_input_error(
                "`codeCitySnapshotStatus` does not support historical or temporary `asOf(...)` scopes",
            ));
        }

        let context = ctx.data_unchecked::<DevqlGraphqlContext>();
        let repo_id = context
            .repo_id_for_scope(&self.scope)
            .map_err(|err| backend_error(format!("failed to resolve repository: {err:#}")))?;
        let repo_root = context.repo_root_for_scope(&self.scope).map_err(|err| {
            backend_error(format!("failed to resolve repository checkout: {err:#}"))
        })?;
        let requested_project_path = if let Some(project_path) = project_path {
            let trimmed = project_path.trim();
            if trimmed.is_empty() {
                return Err(bad_user_input_error("`projectPath` must not be empty"));
            }
            crate::capability_packs::codecity::storage::normalise_project_path(Some(trimmed))
        } else {
            crate::capability_packs::codecity::storage::normalise_project_path(
                self.scope.project_path(),
            )
        };
        let snapshot_key = crate::capability_packs::codecity::storage::snapshot_key_for(
            requested_project_path.as_deref(),
        );
        let config = crate::capability_packs::codecity::services::config::CodeCityConfig::default();
        let config_fingerprint = config.fingerprint().map_err(|err| {
            backend_error(format!(
                "failed to fingerprint CodeCity configuration: {err:#}"
            ))
        })?;
        let repo =
            crate::capability_packs::codecity::storage::SqliteCodeCityRepository::open_for_repo_root(
                &repo_root,
            )
            .and_then(|repo| {
                repo.initialise_schema()?;
                Ok(repo)
            })
            .map_err(|err| {
                backend_error(format!("failed to open CodeCity snapshot store: {err:#}"))
            })?;
        let latest_generation = crate::daemon::capability_event_latest_generation(&repo_id)
            .map_err(|err| {
                backend_error(format!("failed to load DevQL generation status: {err:#}"))
            })?;
        let status = repo
            .load_snapshot_status_or_missing(
                &repo_id,
                &snapshot_key,
                requested_project_path.as_deref(),
                &config_fingerprint,
                latest_generation,
            )
            .map_err(|err| {
                backend_error(format!("failed to load CodeCity snapshot status: {err:#}"))
            })?;
        Ok(status.into())
    }

    #[graphql(name = "codeCityArchitecture")]
    async fn code_city_architecture(
        &self,
        ctx: &Context<'_>,
        #[graphql(default = 500)] first: i32,
        #[graphql(name = "includeMacroEdges")] include_macro_edges: Option<bool>,
        #[graphql(name = "includeZoneDiagnostics")] include_zone_diagnostics: Option<bool>,
        #[graphql(name = "architectureEnabled")] architecture_enabled: Option<bool>,
    ) -> Result<CodeCityArchitectureResult> {
        if self.scope.temporal_scope().is_some() {
            return Err(bad_user_input_error(
                "`codeCityArchitecture` does not support historical or temporary `asOf(...)` scopes",
            ));
        }

        let rows = StageResolverAdapter::new(
            ctx.data_unchecked::<DevqlGraphqlContext>().clone(),
            "codecity_architecture",
        )
        .resolve(
            &self.scope,
            Vec::new(),
            Some(build_codecity_stage_args(CodeCityStageArgs {
                include_macro_edges,
                include_zone_diagnostics,
                architecture_enabled,
                ..CodeCityStageArgs::default()
            })),
            stage_limit(first)?,
        )
        .await
        .map_err(|err| map_stage_adapter_error("project codeCityArchitecture", err))?;
        decode_stage_single("codecity_architecture", rows)
    }

    #[graphql(name = "codeCityViolations")]
    async fn code_city_violations(
        &self,
        ctx: &Context<'_>,
        filter: Option<CodeCityViolationFilterInput>,
        first: Option<i32>,
        after: Option<String>,
        last: Option<i32>,
        before: Option<String>,
    ) -> Result<CodeCityViolationConnectionResult> {
        if self.scope.temporal_scope().is_some() {
            return Err(bad_user_input_error(
                "`codeCityViolations` does not support historical or temporary `asOf(...)` scopes",
            ));
        }
        let pagination = ConnectionPagination::from_graphql(
            100,
            first,
            after.as_deref(),
            last,
            before.as_deref(),
        )?;
        let args = build_codecity_violations_args(filter, &pagination);
        let rows = StageResolverAdapter::new(
            ctx.data_unchecked::<DevqlGraphqlContext>().clone(),
            "codecity_violations",
        )
        .resolve(&self.scope, Vec::new(), Some(args), pagination.limit())
        .await
        .map_err(|err| map_stage_adapter_error("project codeCityViolations", err))?;
        decode_stage_single("codecity_violations", rows)
    }

    #[graphql(name = "codeCityFile")]
    async fn code_city_file(
        &self,
        ctx: &Context<'_>,
        path: String,
        #[graphql(name = "incomingFirst")] incoming_first: Option<i32>,
        #[graphql(name = "outgoingFirst")] outgoing_first: Option<i32>,
    ) -> Result<CodeCityFileDetailResult> {
        if self.scope.temporal_scope().is_some() {
            return Err(bad_user_input_error(
                "`codeCityFile` does not support historical or temporary `asOf(...)` scopes",
            ));
        }
        let normalized = ctx
            .data_unchecked::<DevqlGraphqlContext>()
            .resolve_scope_path(&self.scope, &path, false)
            .map_err(bad_user_input_error)?;
        let incoming_first = optional_positive_limit("incomingFirst", incoming_first)?;
        let outgoing_first = optional_positive_limit("outgoingFirst", outgoing_first)?;
        let mut args = serde_json::Map::new();
        args.insert("path".to_string(), Value::String(normalized.clone()));
        if let Some(limit) = incoming_first {
            args.insert(
                "incoming_first".to_string(),
                Value::Number(serde_json::Number::from(limit as i64)),
            );
        }
        if let Some(limit) = outgoing_first {
            args.insert(
                "outgoing_first".to_string(),
                Value::Number(serde_json::Number::from(limit as i64)),
            );
        }
        let rows = StageResolverAdapter::new(
            ctx.data_unchecked::<DevqlGraphqlContext>().clone(),
            "codecity_file_detail",
        )
        .resolve(&self.scope, Vec::new(), Some(Value::Object(args)), 1)
        .await
        .map_err(|err| map_stage_adapter_error("project codeCityFile", err))?;
        decode_stage_single("codecity_file_detail", rows)
    }

    #[graphql(name = "codeCityArcs")]
    async fn code_city_arcs(
        &self,
        ctx: &Context<'_>,
        filter: Option<CodeCityArcFilterInput>,
        first: Option<i32>,
        after: Option<String>,
        last: Option<i32>,
        before: Option<String>,
    ) -> Result<CodeCityArcConnectionResult> {
        if self.scope.temporal_scope().is_some() {
            return Err(bad_user_input_error(
                "`codeCityArcs` does not support historical or temporary `asOf(...)` scopes",
            ));
        }
        let pagination = ConnectionPagination::from_graphql(
            200,
            first,
            after.as_deref(),
            last,
            before.as_deref(),
        )?;
        let args = build_codecity_arcs_args(filter, &pagination);
        let rows = StageResolverAdapter::new(
            ctx.data_unchecked::<DevqlGraphqlContext>().clone(),
            "codecity_arcs",
        )
        .resolve(&self.scope, Vec::new(), Some(args), pagination.limit())
        .await
        .map_err(|err| map_stage_adapter_error("project codeCityArcs", err))?;
        decode_stage_single("codecity_arcs", rows)
    }
}

fn stage_limit(first: i32) -> Result<usize> {
    if first <= 0 {
        return Err(bad_user_input_error("`first` must be greater than 0"));
    }
    Ok(first as usize)
}

fn optional_positive_limit(name: &str, value: Option<i32>) -> Result<Option<usize>> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value <= 0 {
        return Err(bad_user_input_error(format!(
            "`{name}` must be greater than 0"
        )));
    }
    Ok(Some(value as usize))
}

fn normalise_architecture_graph_filter(
    context: &DevqlGraphqlContext,
    scope: &ResolverScope,
    filter: Option<ArchitectureGraphFilterInput>,
) -> Result<Option<ArchitectureGraphFilterInput>> {
    let Some(mut filter) = filter else {
        return Ok(None);
    };
    if let Some(path) = filter.path.as_ref() {
        filter.path = Some(
            context
                .resolve_scope_path(scope, path, false)
                .map_err(bad_user_input_error)?,
        );
    }
    Ok(Some(filter))
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

#[derive(Default)]
struct CodeCityStageArgs {
    include_dependency_arcs: Option<bool>,
    include_boundaries: Option<bool>,
    include_architecture: Option<bool>,
    include_macro_edges: Option<bool>,
    include_zone_diagnostics: Option<bool>,
    architecture_enabled: Option<bool>,
    include_health: Option<bool>,
    analysis_window_months: Option<i32>,
}

fn build_codecity_stage_args(stage_args: CodeCityStageArgs) -> Value {
    let mut args = serde_json::Map::new();
    let CodeCityStageArgs {
        include_dependency_arcs,
        include_boundaries,
        include_architecture,
        include_macro_edges,
        include_zone_diagnostics,
        architecture_enabled,
        include_health,
        analysis_window_months,
    } = stage_args;
    if let Some(include_dependency_arcs) = include_dependency_arcs {
        args.insert(
            "include_dependency_arcs".to_string(),
            Value::Bool(include_dependency_arcs),
        );
    }
    if let Some(include_boundaries) = include_boundaries {
        args.insert(
            "include_boundaries".to_string(),
            Value::Bool(include_boundaries),
        );
    }
    if let Some(include_architecture) = include_architecture {
        args.insert(
            "include_architecture".to_string(),
            Value::Bool(include_architecture),
        );
    }
    if let Some(include_macro_edges) = include_macro_edges {
        args.insert(
            "include_macro_edges".to_string(),
            Value::Bool(include_macro_edges),
        );
    }
    if let Some(include_zone_diagnostics) = include_zone_diagnostics {
        args.insert(
            "include_zone_diagnostics".to_string(),
            Value::Bool(include_zone_diagnostics),
        );
    }
    if let Some(architecture_enabled) = architecture_enabled {
        args.insert(
            "architecture_enabled".to_string(),
            Value::Bool(architecture_enabled),
        );
    }
    if let Some(include_health) = include_health {
        args.insert("include_health".to_string(), Value::Bool(include_health));
    }
    if let Some(analysis_window_months) = analysis_window_months {
        args.insert(
            "analysis_window_months".to_string(),
            Value::Number(serde_json::Number::from(analysis_window_months as i64)),
        );
    }
    Value::Object(args)
}

fn build_codecity_violations_args(
    filter: Option<CodeCityViolationFilterInput>,
    pagination: &ConnectionPagination,
) -> Value {
    let mut args = serde_json::Map::new();
    insert_pagination_args(&mut args, pagination);
    if let Some(filter) = filter {
        if let Some(severity) = filter.severity {
            args.insert(
                "severity".to_string(),
                Value::String(severity.as_stage_value().to_string()),
            );
        }
        if let Some(severities) = filter.severities
            && !severities.is_empty()
        {
            args.insert(
                "severities".to_string(),
                Value::Array(
                    severities
                        .into_iter()
                        .map(|severity| Value::String(severity.as_stage_value().to_string()))
                        .collect(),
                ),
            );
        }
        if let Some(pattern) = filter.pattern {
            args.insert(
                "pattern".to_string(),
                Value::String(pattern.as_stage_value().to_string()),
            );
        }
        if let Some(rule) = filter.rule {
            args.insert(
                "rule".to_string(),
                Value::String(rule.as_stage_value().to_string()),
            );
        }
        insert_optional_string(&mut args, "boundary_id", filter.boundary_id);
        insert_optional_string(&mut args, "path", filter.path);
        insert_optional_string(&mut args, "from_path", filter.from_path);
        insert_optional_string(&mut args, "to_path", filter.to_path);
        if let Some(include_suppressed) = filter.include_suppressed {
            args.insert(
                "include_suppressed".to_string(),
                Value::Bool(include_suppressed),
            );
        }
    }
    Value::Object(args)
}

fn build_codecity_arcs_args(
    filter: Option<CodeCityArcFilterInput>,
    pagination: &ConnectionPagination,
) -> Value {
    let mut args = serde_json::Map::new();
    insert_pagination_args(&mut args, pagination);
    if let Some(filter) = filter {
        if let Some(kind) = filter.kind {
            args.insert(
                "kind".to_string(),
                Value::String(kind.as_stage_value().to_string()),
            );
        }
        if let Some(visibility) = filter.visibility {
            args.insert(
                "visibility".to_string(),
                Value::String(visibility.as_stage_value().to_string()),
            );
        }
        if let Some(severity) = filter.severity {
            args.insert(
                "severity".to_string(),
                Value::String(severity.as_stage_value().to_string()),
            );
        }
        insert_optional_string(&mut args, "boundary_id", filter.boundary_id);
        insert_optional_string(&mut args, "path", filter.path);
        if let Some(direction) = filter.direction {
            args.insert(
                "direction".to_string(),
                Value::String(direction.as_stage_value().to_string()),
            );
        }
        if let Some(include_hidden) = filter.include_hidden {
            args.insert("include_hidden".to_string(), Value::Bool(include_hidden));
        }
    }
    Value::Object(args)
}

fn insert_pagination_args(
    args: &mut serde_json::Map<String, Value>,
    pagination: &ConnectionPagination,
) {
    match pagination {
        ConnectionPagination::Forward { limit, after } => {
            args.insert(
                "first".to_string(),
                Value::Number(serde_json::Number::from(*limit as i64)),
            );
            insert_optional_string(args, "after", after.clone());
        }
        ConnectionPagination::Backward { limit, before } => {
            args.insert(
                "last".to_string(),
                Value::Number(serde_json::Number::from(*limit as i64)),
            );
            insert_optional_string(args, "before", before.clone());
        }
    }
}

fn insert_optional_string(
    args: &mut serde_json::Map<String, Value>,
    key: &str,
    value: Option<String>,
) {
    if let Some(value) = value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        args.insert(key.to_string(), Value::String(value));
    }
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
        || message.contains("unknown CodeCity path")
    {
        return bad_user_input_error(message);
    }
    backend_error(format!("failed to resolve {scope}: {message}"))
}

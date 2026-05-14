use async_graphql::{ComplexObject, Context, Result};
use std::collections::HashMap;

use crate::graphql::context::HistoricalContextSelectionInput;
use crate::graphql::pack_adapter::StageResolverAdapter;
use crate::graphql::{DevqlGraphqlContext, backend_error, bad_user_input_error};

use super::super::{
    ClonesFilterInput, DateTimeScalar, DepsDirection, DepsFilterInput, EdgeKind, HttpContextResult,
    TestHarnessTestsResult,
};
use super::stages::{
    ArchitectureOverviewStageData, CheckpointStageData, CloneStageData, ContextGuidanceCategory,
    ContextGuidanceItem, ContextGuidanceStageData, ContextGuidanceStageResult, DependencyStageData,
    HistoricalContextStageData, TestsStageData,
};
use super::support::{
    CHECKPOINT_STAGE_SCHEMA, CLONE_STAGE_SCHEMA, CONTEXT_GUIDANCE_STAGE_SCHEMA,
    DEPENDENCY_STAGE_SCHEMA, HISTORICAL_CONTEXT_STAGE_SCHEMA, SelectionSummaryStages,
    TESTS_STAGE_SCHEMA, build_architecture_overview_stage, build_checkpoint_summary,
    build_clone_expand_hint, build_clone_summary, build_dependency_expand_hint,
    build_dependency_summary, build_historical_context_summary, build_selection_summary,
    build_tests_stage_args, build_tests_summary, decode_stage_rows, dedup_dependency_edges,
    selection_stage_row_from_artefact,
};
use super::{
    ArtefactSelection, ArtefactSelectionMode, CheckpointStageResult, CloneStageResult,
    DependencyStageResult, HistoricalContextStageResult, HistoricalEvidenceKind, SearchBreakdown,
    TestsStageResult,
};

#[ComplexObject]
impl ArtefactSelection {
    #[graphql(name = "overview")]
    async fn summary(&self, ctx: &Context<'_>) -> Result<super::super::JsonScalar> {
        self.ensure_artefact_selection("overview")?;
        let checkpoints = self.resolve_checkpoint_stage_data(ctx, None, None).await?;
        let clones = self.resolve_clone_stage_data(ctx, None).await?;
        let deps = self
            .resolve_dependency_stage_data(ctx, None, DepsDirection::Both, true)
            .await?;
        let tests = self.resolve_tests_stage_data(ctx, None, None).await?;
        let historical_context = self
            .resolve_historical_context_stage_data(ctx, None, None, None)
            .await?;
        let context_guidance = self
            .resolve_context_guidance_stage_data(ctx, None, None, None, None, None)
            .await?;
        let http = self.resolve_http_context_stage_data(ctx, 10).await?;
        let architecture = self.resolve_architecture_overview_stage_data(ctx).await?;

        Ok(async_graphql::types::Json(build_selection_summary(
            self.artefacts.len(),
            SelectionSummaryStages {
                checkpoints: &checkpoints,
                clones: &clones,
                deps: &deps,
                tests: &tests,
                historical_context: &historical_context,
                context_guidance: &context_guidance,
                http: &http.overview.0,
                architecture: &architecture,
            },
        )))
    }

    async fn artefacts(
        &self,
        #[graphql(default = 20)] first: i32,
    ) -> Result<Vec<super::super::Artefact>> {
        self.ensure_artefact_selection("artefacts")?;
        super::support::take_stage_items(&self.artefacts, first)
    }

    async fn entries(
        &self,
        #[graphql(default = 20)] first: i32,
    ) -> Result<Vec<super::DirectoryEntry>> {
        self.ensure_directory_selection("entries")?;
        super::support::take_stage_items(&self.directory_entries, first)
    }

    #[graphql(name = "searchBreakdown")]
    async fn search_breakdown(
        &self,
        #[graphql(default = 3)] first: i32,
    ) -> Result<Option<SearchBreakdown>> {
        self.ensure_artefact_selection("searchBreakdown")?;
        let Some(search_breakdown) = self.search_breakdown.as_ref() else {
            return Ok(None);
        };

        Ok(Some(SearchBreakdown {
            lexical: super::support::take_stage_items(&search_breakdown.lexical, first)?,
            identity: super::support::take_stage_items(&search_breakdown.identity, first)?,
            code: super::support::take_stage_items(&search_breakdown.code, first)?,
            summary: super::support::take_stage_items(&search_breakdown.summary, first)?,
        }))
    }

    async fn checkpoints(
        &self,
        ctx: &Context<'_>,
        agent: Option<String>,
        since: Option<DateTimeScalar>,
    ) -> Result<CheckpointStageResult> {
        self.ensure_artefact_selection("checkpoints")?;
        Ok(self
            .resolve_checkpoint_stage_data(ctx, agent.as_deref(), since.as_ref())
            .await?
            .into())
    }

    #[graphql(name = "codeMatches")]
    async fn clones(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "relationKind")] relation_kind: Option<String>,
        #[graphql(name = "minScore")] min_score: Option<f64>,
    ) -> Result<CloneStageResult> {
        self.ensure_artefact_selection("codeMatches")?;
        let filter = ClonesFilterInput {
            relation_kind,
            min_score,
            neighbors: None,
        };
        Ok(self
            .resolve_clone_stage_data(ctx, Some(&filter))
            .await?
            .into())
    }

    #[graphql(name = "dependencies")]
    async fn dependencies(
        &self,
        ctx: &Context<'_>,
        kind: Option<EdgeKind>,
        #[graphql(default_with = "DepsDirection::Both")] direction: DepsDirection,
        #[graphql(name = "includeUnresolved", default = true)] include_unresolved: bool,
    ) -> Result<DependencyStageResult> {
        self.ensure_artefact_selection("dependencies")?;
        Ok(self
            .resolve_dependency_stage_data(ctx, kind, direction, include_unresolved)
            .await?
            .into())
    }

    async fn tests(
        &self,
        ctx: &Context<'_>,
        #[graphql(name = "minConfidence")] min_confidence: Option<f64>,
        #[graphql(name = "linkageSource")] linkage_source: Option<String>,
    ) -> Result<TestsStageResult> {
        self.ensure_artefact_selection("tests")?;
        Ok(self
            .resolve_tests_stage_data(ctx, min_confidence, linkage_source)
            .await?
            .into())
    }

    #[graphql(name = "historicalContext")]
    async fn historical_context(
        &self,
        ctx: &Context<'_>,
        agent: Option<String>,
        since: Option<DateTimeScalar>,
        #[graphql(name = "evidenceKind")] evidence_kind: Option<HistoricalEvidenceKind>,
    ) -> Result<HistoricalContextStageResult> {
        self.ensure_artefact_selection("historicalContext")?;
        Ok(self
            .resolve_historical_context_stage_data(
                ctx,
                agent.as_deref(),
                since.as_ref(),
                evidence_kind,
            )
            .await?
            .into())
    }

    #[graphql(name = "contextGuidance")]
    async fn context_guidance(
        &self,
        ctx: &Context<'_>,
        agent: Option<String>,
        since: Option<DateTimeScalar>,
        #[graphql(name = "evidenceKind")] evidence_kind: Option<HistoricalEvidenceKind>,
        category: Option<ContextGuidanceCategory>,
        kind: Option<String>,
    ) -> Result<ContextGuidanceStageResult> {
        self.ensure_artefact_selection("contextGuidance")?;
        let trimmed_kind = match kind.as_deref().map(str::trim) {
            Some("") => return Err(bad_user_input_error("`kind` must be non-empty")),
            Some(value) => Some(value),
            None => None,
        };
        Ok(self
            .resolve_context_guidance_stage_data(
                ctx,
                agent.as_deref(),
                since.as_ref(),
                evidence_kind,
                category,
                trimmed_kind,
            )
            .await?
            .into())
    }

    #[graphql(name = "httpContext")]
    async fn http_context(
        &self,
        ctx: &Context<'_>,
        #[graphql(default = 20)] first: i32,
    ) -> Result<HttpContextResult> {
        self.ensure_artefact_selection("httpContext")?;
        self.resolve_http_context_stage_data(ctx, first).await
    }
}

impl ArtefactSelection {
    fn ensure_artefact_selection(&self, field: &str) -> Result<()> {
        if self.mode == ArtefactSelectionMode::DirectoryEntries {
            return Err(bad_user_input_error(format!(
                "directory paths only support `entries`; `{field}` requires a file path instead"
            )));
        }
        Ok(())
    }

    fn ensure_directory_selection(&self, field: &str) -> Result<()> {
        if self.mode == ArtefactSelectionMode::Artefacts {
            return Err(bad_user_input_error(format!(
                "file paths do not support `{field}`; select a directory path instead"
            )));
        }
        Ok(())
    }

    async fn resolve_checkpoint_stage_data(
        &self,
        ctx: &Context<'_>,
        agent: Option<&str>,
        since: Option<&DateTimeScalar>,
    ) -> Result<CheckpointStageData> {
        let checkpoints = ctx
            .data_unchecked::<DevqlGraphqlContext>()
            .list_selected_checkpoints(&self.scope, &self.symbol_ids(), &self.paths(), agent, since)
            .await
            .map_err(|err| {
                backend_error(format!("failed to resolve selected checkpoints: {err:#}"))
            })?;
        Ok(CheckpointStageData {
            summary: build_checkpoint_summary(&checkpoints),
            schema: (!checkpoints.is_empty()).then(|| CHECKPOINT_STAGE_SCHEMA.to_string()),
            items: checkpoints,
        })
    }

    async fn resolve_clone_stage_data(
        &self,
        ctx: &Context<'_>,
        filter: Option<&ClonesFilterInput>,
    ) -> Result<CloneStageData> {
        if let Some(filter) = filter {
            filter.validate()?;
        }
        let mut clones = ctx
            .data_unchecked::<DevqlGraphqlContext>()
            .list_selected_clones(&self.artefact_ids(), filter, &self.scope)
            .await
            .map_err(|err| backend_error(format!("failed to resolve selected clones: {err:#}")))?;
        clones.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| left.id.as_ref().cmp(right.id.as_ref()))
        });
        let expand_hint = build_clone_expand_hint(clones.len());
        Ok(CloneStageData {
            summary: build_clone_summary(&clones),
            expand_hint,
            schema: (!clones.is_empty()).then(|| CLONE_STAGE_SCHEMA.to_string()),
            items: clones,
        })
    }

    async fn resolve_dependency_stage_data(
        &self,
        ctx: &Context<'_>,
        kind: Option<EdgeKind>,
        direction: DepsDirection,
        include_unresolved: bool,
    ) -> Result<DependencyStageData> {
        let context = ctx.data_unchecked::<DevqlGraphqlContext>();
        let filter = DepsFilterInput {
            kind,
            direction,
            include_unresolved,
        };
        let artefact_ids = self.artefact_ids();

        let outgoing = if matches!(direction, DepsDirection::Out | DepsDirection::Both) {
            context
                .load_dependency_edges_by_artefact_ids(
                    &artefact_ids,
                    DepsDirection::Out,
                    DepsFilterInput {
                        direction: DepsDirection::Out,
                        ..filter
                    },
                    &self.scope,
                )
                .await
                .map_err(|err| {
                    backend_error(format!(
                        "failed to resolve selected outgoing dependency edges: {err:#}"
                    ))
                })?
        } else {
            HashMap::new()
        };
        let incoming = if matches!(direction, DepsDirection::In | DepsDirection::Both) {
            context
                .load_dependency_edges_by_artefact_ids(
                    &artefact_ids,
                    DepsDirection::In,
                    DepsFilterInput {
                        direction: DepsDirection::In,
                        ..filter
                    },
                    &self.scope,
                )
                .await
                .map_err(|err| {
                    backend_error(format!(
                        "failed to resolve selected incoming dependency edges: {err:#}"
                    ))
                })?
        } else {
            HashMap::new()
        };

        let outgoing_edges = dedup_dependency_edges(outgoing.into_values().flatten().collect());
        let incoming_edges = dedup_dependency_edges(incoming.into_values().flatten().collect());
        let mut items = incoming_edges.clone();
        items.extend(outgoing_edges.clone());
        items = dedup_dependency_edges(items);
        items.sort_by(|left, right| {
            left.to_symbol_ref
                .as_deref()
                .unwrap_or("")
                .cmp(right.to_symbol_ref.as_deref().unwrap_or(""))
                .then_with(|| left.id.as_ref().cmp(right.id.as_ref()))
        });
        let expand_hint = build_dependency_expand_hint(items.len());

        Ok(DependencyStageData {
            summary: build_dependency_summary(
                &incoming_edges,
                &outgoing_edges,
                self.artefacts.len(),
                expand_hint.as_ref(),
            ),
            expand_hint,
            schema: (!items.is_empty()).then(|| DEPENDENCY_STAGE_SCHEMA.to_string()),
            items,
        })
    }

    async fn resolve_tests_stage_data(
        &self,
        ctx: &Context<'_>,
        min_confidence: Option<f64>,
        linkage_source: Option<String>,
    ) -> Result<TestsStageData> {
        let rows: Vec<TestHarnessTestsResult> = decode_stage_rows(
            "tests",
            StageResolverAdapter::new(ctx.data_unchecked::<DevqlGraphqlContext>().clone(), "tests")
                .resolve(
                    &self.scope,
                    self.artefacts
                        .iter()
                        .map(selection_stage_row_from_artefact)
                        .collect(),
                    Some(build_tests_stage_args(min_confidence, linkage_source)?),
                    100,
                )
                .await
                .map_err(|err| {
                    backend_error(format!("failed to resolve selected tests: {err:#}"))
                })?,
        )?;
        let mut rows = rows;
        rows.sort_by(|left, right| {
            left.artefact
                .file_path
                .cmp(&right.artefact.file_path)
                .then_with(|| left.artefact.name.cmp(&right.artefact.name))
        });
        Ok(TestsStageData {
            summary: build_tests_summary(&rows, self.artefacts.len()),
            schema: (!rows.is_empty()).then(|| TESTS_STAGE_SCHEMA.to_string()),
            items: rows,
        })
    }

    async fn resolve_historical_context_stage_data(
        &self,
        ctx: &Context<'_>,
        agent: Option<&str>,
        since: Option<&DateTimeScalar>,
        evidence_kind: Option<HistoricalEvidenceKind>,
    ) -> Result<HistoricalContextStageData> {
        let items = ctx
            .data_unchecked::<DevqlGraphqlContext>()
            .list_selected_historical_context(
                &self.scope,
                HistoricalContextSelectionInput {
                    symbol_ids: self.symbol_ids(),
                    paths: self.paths(),
                    agent: agent.map(str::to_string),
                    since: since.map(|value| value.as_str().to_string()),
                    evidence_kind,
                },
            )
            .await
            .map_err(|err| {
                backend_error(format!(
                    "failed to resolve selected historical context: {err:#}"
                ))
            })?;
        Ok(HistoricalContextStageData {
            summary: build_historical_context_summary(&items),
            schema: (!items.is_empty()).then(|| HISTORICAL_CONTEXT_STAGE_SCHEMA.to_string()),
            items,
        })
    }

    async fn resolve_context_guidance_stage_data(
        &self,
        ctx: &Context<'_>,
        agent: Option<&str>,
        since: Option<&DateTimeScalar>,
        evidence_kind: Option<HistoricalEvidenceKind>,
        category: Option<ContextGuidanceCategory>,
        kind: Option<&str>,
    ) -> Result<ContextGuidanceStageData> {
        let rows = StageResolverAdapter::new(
            ctx.data_unchecked::<DevqlGraphqlContext>().clone(),
            "context_guidance",
        )
        .resolve(
            &self.scope,
            self.artefacts
                .iter()
                .map(selection_stage_row_from_artefact)
                .collect(),
            Some(build_context_guidance_stage_args(
                agent,
                since,
                evidence_kind,
                category,
                kind,
            )),
            100,
        )
        .await
        .map_err(|err| {
            backend_error(format!(
                "failed to resolve selected context guidance: {err:#}"
            ))
        })?;
        let payload = rows
            .into_iter()
            .next()
            .unwrap_or_else(|| serde_json::json!({ "overview": { "totalCount": 0 }, "items": [] }));
        let summary = payload
            .get("overview")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({ "totalCount": 0 }));
        let item_values = payload
            .get("items")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        let items: Vec<ContextGuidanceItem> = decode_stage_rows("context_guidance", item_values)?;
        Ok(ContextGuidanceStageData {
            summary,
            schema: (!items.is_empty()).then(|| CONTEXT_GUIDANCE_STAGE_SCHEMA.to_string()),
            items,
        })
    }

    async fn resolve_http_context_stage_data(
        &self,
        ctx: &Context<'_>,
        first: i32,
    ) -> Result<HttpContextResult> {
        if first <= 0 {
            return Err(bad_user_input_error("`first` must be greater than 0"));
        }
        let context = ctx.data_unchecked::<DevqlGraphqlContext>();
        let target_context = context
            .http_context_for_targets(
                &self.scope,
                &self.artefact_ids(),
                &self.symbol_ids(),
                &self.paths(),
                first as usize,
            )
            .await
            .map_err(|err| {
                backend_error(format!("failed to resolve selected HTTP context: {err:#}"))
            })?;
        if !target_context.bundles.is_empty() || !target_context.primitives.is_empty() {
            return Ok(target_context);
        }

        let terms = self.http_search_terms();
        if terms.is_empty() {
            return Ok(target_context);
        }

        context
            .http_context_for_terms(&self.scope, &terms, first as usize)
            .await
            .map_err(|err| {
                backend_error(format!(
                    "failed to resolve selected HTTP context from search terms: {err:#}"
                ))
            })
    }

    async fn resolve_architecture_overview_stage_data(
        &self,
        ctx: &Context<'_>,
    ) -> Result<ArchitectureOverviewStageData> {
        if self.artefacts.is_empty() {
            return Ok(ArchitectureOverviewStageData::unavailable(
                0,
                "empty_selection",
            ));
        }

        let context = ctx.data_unchecked::<DevqlGraphqlContext>();
        let overview = context
            .architecture_overview_for_targets(
                &self.scope,
                &self.artefact_ids(),
                &self.symbol_ids(),
                &self.paths(),
            )
            .await
            .map_err(|err| {
                backend_error(format!(
                    "failed to resolve selected architecture overview: {err:#}"
                ))
            })?;

        Ok(build_architecture_overview_stage(overview))
    }

    fn http_search_terms(&self) -> Vec<String> {
        self.search_query
            .as_deref()
            .map(split_http_selection_terms)
            .unwrap_or_default()
    }
}

pub(super) fn split_http_selection_terms(query: &str) -> Vec<String> {
    query
        .split(|character: char| character.is_whitespace() || character == ',')
        .map(|term| {
            term.trim_matches(|character: char| {
                matches!(
                    character,
                    '"' | '\'' | '`' | '(' | ')' | '[' | ']' | '{' | '}'
                )
            })
        })
        .map(str::trim)
        .filter(|term| !term.is_empty())
        .map(str::to_string)
        .collect()
}

fn build_context_guidance_stage_args(
    agent: Option<&str>,
    since: Option<&DateTimeScalar>,
    evidence_kind: Option<HistoricalEvidenceKind>,
    category: Option<ContextGuidanceCategory>,
    kind: Option<&str>,
) -> serde_json::Value {
    let mut args = serde_json::Map::new();
    if let Some(agent) = agent.map(str::trim).filter(|value| !value.is_empty()) {
        args.insert("agent".to_string(), serde_json::json!(agent));
    }
    if let Some(since) = since {
        args.insert("since".to_string(), serde_json::json!(since.as_str()));
    }
    if let Some(evidence_kind) = evidence_kind {
        args.insert(
            "evidenceKind".to_string(),
            serde_json::json!(historical_evidence_kind_arg(evidence_kind)),
        );
    }
    if let Some(category) = category {
        args.insert(
            "category".to_string(),
            serde_json::json!(context_guidance_category_arg(category)),
        );
    }
    if let Some(kind) = kind {
        args.insert("kind".to_string(), serde_json::json!(kind));
    }
    serde_json::Value::Object(args)
}

fn historical_evidence_kind_arg(kind: HistoricalEvidenceKind) -> &'static str {
    match kind {
        HistoricalEvidenceKind::SymbolProvenance => "SYMBOL_PROVENANCE",
        HistoricalEvidenceKind::FileRelation => "FILE_RELATION",
        HistoricalEvidenceKind::LineOverlap => "LINE_OVERLAP",
    }
}

fn context_guidance_category_arg(category: ContextGuidanceCategory) -> &'static str {
    match category {
        ContextGuidanceCategory::Decision => "DECISION",
        ContextGuidanceCategory::Constraint => "CONSTRAINT",
        ContextGuidanceCategory::Pattern => "PATTERN",
        ContextGuidanceCategory::Risk => "RISK",
        ContextGuidanceCategory::Verification => "VERIFICATION",
        ContextGuidanceCategory::Context => "CONTEXT",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphql::ResolverScope;

    #[test]
    fn directory_selection_rejects_historical_context() {
        let selection = ArtefactSelection::from_directory_entries(
            vec![crate::graphql::types::artefact_selection::DirectoryEntry {
                path: "src".to_string(),
                name: "src".to_string(),
                entry_kind:
                    crate::graphql::types::artefact_selection::DirectoryEntryKind::Directory,
            }],
            ResolverScope::default(),
        );

        let err = selection
            .ensure_artefact_selection("historicalContext")
            .expect_err("directory selection should reject historicalContext");

        assert!(
            err.message
                .contains("directory paths only support `entries`; `historicalContext`")
        );
    }

    #[test]
    fn directory_selection_rejects_context_guidance() {
        let selection = ArtefactSelection::from_directory_entries(
            vec![crate::graphql::types::artefact_selection::DirectoryEntry {
                path: "src".to_string(),
                name: "src".to_string(),
                entry_kind:
                    crate::graphql::types::artefact_selection::DirectoryEntryKind::Directory,
            }],
            ResolverScope::default(),
        );

        let err = selection
            .ensure_artefact_selection("contextGuidance")
            .expect_err("directory selection should reject contextGuidance");

        assert!(
            err.message
                .contains("directory paths only support `entries`; `contextGuidance`")
        );
    }
}

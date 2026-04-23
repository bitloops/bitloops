use async_graphql::{ComplexObject, Context, Result};
use std::collections::HashMap;

use crate::graphql::pack_adapter::StageResolverAdapter;
use crate::graphql::{DevqlGraphqlContext, backend_error, bad_user_input_error};

use super::super::{
    ClonesFilterInput, DateTimeScalar, DepsDirection, DepsFilterInput, EdgeKind,
    TestHarnessTestsResult,
};
use super::stages::{CheckpointStageData, CloneStageData, DependencyStageData, TestsStageData};
use super::support::{
    CHECKPOINT_STAGE_SCHEMA, CLONE_STAGE_SCHEMA, DEPENDENCY_STAGE_SCHEMA, TESTS_STAGE_SCHEMA,
    build_checkpoint_summary, build_clone_expand_hint, build_clone_summary,
    build_dependency_expand_hint, build_dependency_summary, build_selection_summary,
    build_tests_stage_args, build_tests_summary, decode_stage_rows, dedup_dependency_edges,
    selection_stage_row_from_artefact,
};
use super::{
    ArtefactSelection, ArtefactSelectionMode, CheckpointStageResult, CloneStageResult,
    DependencyStageResult, TestsStageResult,
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

        Ok(async_graphql::types::Json(build_selection_summary(
            self.artefacts.len(),
            &checkpoints,
            &clones,
            &deps,
            &tests,
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
            .list_selected_symbol_checkpoints(&self.scope, &self.symbol_ids(), agent, since)
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
}

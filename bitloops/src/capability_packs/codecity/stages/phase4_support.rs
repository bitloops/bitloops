use anyhow::Result;
use serde_json::Value;

use crate::capability_packs::codecity::services::architecture::{
    CodeCityArchitectureAnalysis, analyse_codecity_architecture,
};
use crate::capability_packs::codecity::services::config::CodeCityConfig;
use crate::capability_packs::codecity::services::health::apply_health_overlay;
use crate::capability_packs::codecity::services::phase4::enrich_world_with_phase4;
use crate::capability_packs::codecity::services::source_graph::load_current_source_graph;
use crate::capability_packs::codecity::services::world::build_codecity_world;
use crate::capability_packs::codecity::storage::SqliteCodeCityRepository;
use crate::capability_packs::codecity::types::{
    CodeCityPhase4Snapshot, CodeCityWorldPayload, codecity_current_scope_required_stage_response,
    codecity_source_data_unavailable_stage_response,
};
use crate::host::capability_host::{CapabilityExecutionContext, StageRequest, StageResponse};

pub(super) struct CodeCityPhase4StageData {
    pub repo_id: String,
    pub world: CodeCityWorldPayload,
    pub analysis: CodeCityArchitectureAnalysis,
    pub snapshot: CodeCityPhase4Snapshot,
    pub config: CodeCityConfig,
}

pub(super) fn build_phase4_stage_data(
    stage_id: &str,
    request: &StageRequest,
    ctx: &mut dyn CapabilityExecutionContext,
    config: CodeCityConfig,
) -> Result<Result<CodeCityPhase4StageData, StageResponse>> {
    let repo_id = request
        .payload
        .get("query_context")
        .and_then(|query_context| query_context.get("repo_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(ctx.repo().repo_id.as_str())
        .to_string();

    let resolved_commit = request
        .payload
        .get("query_context")
        .and_then(|query_context| query_context.get("resolved_commit_sha"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if resolved_commit.is_some() {
        return Ok(Err(codecity_current_scope_required_stage_response(
            stage_id,
        )));
    }

    let project_path = request
        .payload
        .get("query_context")
        .and_then(|query_context| query_context.get("project_path"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != ".")
        .map(str::to_string);

    let source = match load_current_source_graph(
        ctx.host_relational(),
        &repo_id,
        project_path.as_deref(),
        &config,
    ) {
        Ok(source) => source,
        Err(err) if is_source_data_unavailable_error(&err) => {
            return Ok(Err(codecity_source_data_unavailable_stage_response(
                stage_id,
                format!("{err:#}"),
            )));
        }
        Err(err) => return Err(err),
    };

    let current_head = ctx
        .git_history()
        .resolve_head(ctx.repo_root())
        .unwrap_or(None);
    let mut world = build_codecity_world(
        &source,
        &repo_id,
        current_head,
        config.clone(),
        ctx.repo_root(),
    )?;
    apply_health_overlay(
        &mut world,
        &source,
        &config,
        ctx.repo_root(),
        ctx.git_history(),
        ctx.test_harness_store(),
    )?;
    let analysis = analyse_codecity_architecture(&source, &config, ctx.repo_root());
    let snapshot = enrich_world_with_phase4(&source, &analysis, &mut world, &config);

    if let Ok(repo) =
        SqliteCodeCityRepository::open_for_repo_root(ctx.repo_root()).and_then(|repo| {
            repo.initialise_schema()?;
            Ok(repo)
        })
    {
        let _ = repo.replace_phase4_snapshot(&snapshot);
    }

    Ok(Ok(CodeCityPhase4StageData {
        repo_id,
        world,
        analysis,
        snapshot,
        config,
    }))
}

fn is_source_data_unavailable_error(err: &anyhow::Error) -> bool {
    let message = format!("{err:#}");
    message.contains("run DevQL sync first")
        || message.contains("current_file_state")
        || message.contains("artefacts_current")
        || message.contains("artefact_edges_current")
}

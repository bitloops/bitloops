use anyhow::Result;
use serde_json::{Value, json};

use crate::capability_packs::codecity::services::architecture::analyse_codecity_architecture;
use crate::capability_packs::codecity::services::config::CodeCityConfig;
use crate::capability_packs::codecity::services::source_graph::load_current_source_graph;
use crate::capability_packs::codecity::types::{
    CODECITY_ARCHITECTURE_STAGE_ID, CODECITY_CAPABILITY_ID, CodeCityArchitecturePayload,
    CodeCityArchitectureStageSummary, codecity_current_scope_required_stage_response,
    codecity_source_data_unavailable_stage_response,
};
use crate::host::capability_host::{
    BoxFuture, CapabilityExecutionContext, StageHandler, StageRequest, StageResponse,
};

pub struct CodeCityArchitectureStageHandler;

impl StageHandler for CodeCityArchitectureStageHandler {
    fn execute<'a>(
        &'a self,
        request: StageRequest,
        ctx: &'a mut dyn CapabilityExecutionContext,
    ) -> BoxFuture<'a, Result<StageResponse>> {
        Box::pin(async move {
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
                return Ok(codecity_current_scope_required_stage_response(
                    CODECITY_ARCHITECTURE_STAGE_ID,
                ));
            }

            let project_path = request
                .payload
                .get("query_context")
                .and_then(|query_context| query_context.get("project_path"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty() && *value != ".")
                .map(str::to_string);

            let args = request
                .payload
                .get("args")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let config = CodeCityConfig::from_stage_args(&args)?;
            let config_fingerprint = config.fingerprint()?;

            let source = match load_current_source_graph(
                ctx.host_relational(),
                &repo_id,
                project_path.as_deref(),
                &config,
            ) {
                Ok(source) => source,
                Err(err) if is_source_data_unavailable_error(&err) => {
                    return Ok(codecity_source_data_unavailable_stage_response(
                        CODECITY_ARCHITECTURE_STAGE_ID,
                        format!("{err:#}"),
                    ));
                }
                Err(err) => return Err(err),
            };

            let analysis = analyse_codecity_architecture(&source, &config, ctx.repo_root());
            let payload = CodeCityArchitecturePayload {
                capability: CODECITY_CAPABILITY_ID.to_string(),
                stage: CODECITY_ARCHITECTURE_STAGE_ID.to_string(),
                status: "ok".to_string(),
                repo_id: repo_id.clone(),
                commit_sha: None,
                config_fingerprint,
                summary: CodeCityArchitectureStageSummary {
                    boundary_count: analysis.boundaries.len(),
                    macro_edge_count: analysis.macro_graph.edge_count,
                    macro_topology: analysis.macro_graph.topology,
                    primary_pattern: analysis.summary_report.primary_pattern,
                    mud_warning_count: analysis
                        .boundary_reports
                        .iter()
                        .filter(|report| {
                            report.scores.ball_of_mud > config.architecture.mud_warning_threshold
                        })
                        .count(),
                },
                macro_graph: config
                    .include_macro_edges
                    .then_some(analysis.macro_graph.clone()),
                architecture: analysis.summary_report.clone(),
                boundaries: analysis.boundaries.clone(),
                boundary_reports: analysis.boundary_reports.clone(),
                diagnostics: analysis.diagnostics.clone(),
            };

            Ok(StageResponse::new(
                serde_json::to_value(payload)?,
                format!("codecity architecture for repo {repo_id}"),
            ))
        })
    }
}

fn is_source_data_unavailable_error(err: &anyhow::Error) -> bool {
    let message = format!("{err:#}");
    message.contains("run DevQL sync first")
        || message.contains("current_file_state")
        || message.contains("artefacts_current")
        || message.contains("artefact_edges_current")
}

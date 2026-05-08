use anyhow::Result;
use serde_json::{Value, json};

use crate::capability_packs::codecity::services::boundary_detection::detect_boundaries;
use crate::capability_packs::codecity::services::config::CodeCityConfig;
use crate::capability_packs::codecity::services::source_graph::load_current_source_graph;
use crate::capability_packs::codecity::types::{
    CODECITY_BOUNDARIES_STAGE_ID, CODECITY_CAPABILITY_ID, CodeCityBoundariesPayload,
    CodeCityFileBoundaryAssignment, codecity_current_scope_required_stage_response,
    codecity_source_data_unavailable_stage_response,
};
use crate::host::capability_host::{
    BoxFuture, CapabilityExecutionContext, StageHandler, StageRequest, StageResponse,
};

pub struct CodeCityBoundariesStageHandler;

impl StageHandler for CodeCityBoundariesStageHandler {
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
                    CODECITY_BOUNDARIES_STAGE_ID,
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
                        CODECITY_BOUNDARIES_STAGE_ID,
                        format!("{err:#}"),
                    ));
                }
                Err(err) => return Err(err),
            };

            let result = detect_boundaries(&source, &config, ctx.repo_root());
            let payload = CodeCityBoundariesPayload {
                capability: CODECITY_CAPABILITY_ID.to_string(),
                stage: CODECITY_BOUNDARIES_STAGE_ID.to_string(),
                status: "ok".to_string(),
                repo_id: repo_id.clone(),
                commit_sha: None,
                config_fingerprint,
                boundaries: result.boundaries,
                file_to_boundary: result
                    .file_to_boundary
                    .into_iter()
                    .map(|(path, boundary_id)| CodeCityFileBoundaryAssignment { path, boundary_id })
                    .collect(),
                diagnostics: result.diagnostics,
            };

            Ok(StageResponse::new(
                serde_json::to_value(payload)?,
                format!("codecity boundaries for repo {repo_id}"),
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

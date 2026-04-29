use anyhow::Result;
use serde_json::json;

use super::phase4_support::build_phase4_stage_data;
use super::violations::{positive_usize_arg, string_arg};
use crate::capability_packs::codecity::services::config::CodeCityConfig;
use crate::capability_packs::codecity::services::phase4::file_detail;
use crate::capability_packs::codecity::types::CODECITY_FILE_DETAIL_STAGE_ID;
use crate::host::capability_host::{
    BoxFuture, CapabilityExecutionContext, StageHandler, StageRequest, StageResponse,
};

pub struct CodeCityFileDetailStageHandler;

impl StageHandler for CodeCityFileDetailStageHandler {
    fn execute<'a>(
        &'a self,
        request: StageRequest,
        ctx: &'a mut dyn CapabilityExecutionContext,
    ) -> BoxFuture<'a, Result<StageResponse>> {
        Box::pin(async move {
            let args = request
                .payload
                .get("args")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let path = string_arg(&args, "path").unwrap_or_default();
            let data = match build_phase4_stage_data(
                CODECITY_FILE_DETAIL_STAGE_ID,
                &request,
                ctx,
                CodeCityConfig::default(),
            )? {
                Ok(data) => data,
                Err(response) => return Ok(response),
            };
            let incoming_limit = positive_usize_arg(&args, "incoming_first")
                .unwrap_or(data.config.selection.incoming_limit);
            let outgoing_limit = positive_usize_arg(&args, "outgoing_first")
                .unwrap_or(data.config.selection.outgoing_limit);
            let Some(payload) = file_detail(
                &path,
                &data.world,
                &data.analysis,
                &data.snapshot,
                incoming_limit,
                outgoing_limit,
            ) else {
                return Ok(StageResponse::new(
                    json!({
                        "status": "failed",
                        "reason": "codecity_file_not_found",
                        "path": path,
                    }),
                    format!("unknown CodeCity path `{path}`"),
                ));
            };

            Ok(StageResponse::new(
                serde_json::to_value(payload)?,
                format!(
                    "codecity file detail for repo {} path {}",
                    data.repo_id, path
                ),
            ))
        })
    }
}

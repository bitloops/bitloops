use anyhow::Result;
use serde_json::json;

use super::snapshot_support::{build_snapshot_stage_data, empty_file_detail_payload};
use super::violations::{positive_usize_arg, string_arg};
use crate::capability_packs::codecity::services::architecture_diagnostics::file_detail;
use crate::capability_packs::codecity::services::config::CodeCityConfig;
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
            let data = match build_snapshot_stage_data(
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
            let Some(world) = data.world.as_ref() else {
                let payload = empty_file_detail_payload(
                    path.clone(),
                    "missing".to_string(),
                    data.snapshot_status.clone(),
                );
                return Ok(StageResponse::new(
                    serde_json::to_value(payload)?,
                    format!(
                        "codecity file detail for repo {}: snapshot missing",
                        data.repo_id
                    ),
                ));
            };
            let Some(payload) = file_detail(
                &path,
                data.snapshot_status.clone(),
                world,
                &data.snapshot,
                incoming_limit,
                outgoing_limit,
            ) else {
                let payload = empty_file_detail_payload(
                    path.clone(),
                    "not_found".to_string(),
                    world.snapshot_status.clone(),
                );
                return Ok(StageResponse::new(
                    serde_json::to_value(payload)?,
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

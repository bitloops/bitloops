use anyhow::Result;
use serde_json::{Value, json};

use super::phase4_support::build_phase4_stage_data;
use super::violations::{bool_arg, parse_severity, positive_usize_arg, string_arg};
use crate::capability_packs::codecity::services::config::CodeCityConfig;
use crate::capability_packs::codecity::services::phase4::arcs_connection;
use crate::capability_packs::codecity::types::{
    CODECITY_ARCS_STAGE_ID, CodeCityArcFilter, CodeCityArcKind, CodeCityArcVisibility,
    CodeCityDependencyDirection,
};
use crate::host::capability_host::{
    BoxFuture, CapabilityExecutionContext, StageHandler, StageRequest, StageResponse,
};

pub struct CodeCityArcsStageHandler;

impl StageHandler for CodeCityArcsStageHandler {
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
            let data = match build_phase4_stage_data(
                CODECITY_ARCS_STAGE_ID,
                &request,
                ctx,
                CodeCityConfig::default(),
            )? {
                Ok(data) => data,
                Err(response) => return Ok(response),
            };
            let first = positive_usize_arg(&args, "first")
                .unwrap_or_else(|| request.limit().unwrap_or(200));
            let last = positive_usize_arg(&args, "last");
            let after = string_arg(&args, "after");
            let before = string_arg(&args, "before");
            let filter = arc_filter_from_args(&args);
            let payload = arcs_connection(
                &data.snapshot.render_arcs,
                &filter,
                first,
                after.as_deref(),
                last,
                before.as_deref(),
            );
            Ok(StageResponse::new(
                serde_json::to_value(payload)?,
                format!(
                    "codecity arcs for repo {}: total={}",
                    data.repo_id,
                    data.snapshot.render_arcs.len()
                ),
            ))
        })
    }
}

fn arc_filter_from_args(args: &Value) -> CodeCityArcFilter {
    CodeCityArcFilter {
        kind: string_arg(args, "kind").and_then(|value| parse_kind(&value)),
        visibility: string_arg(args, "visibility").and_then(|value| parse_visibility(&value)),
        severity: string_arg(args, "severity").and_then(|value| parse_severity(&value)),
        boundary_id: string_arg(args, "boundary_id"),
        path: string_arg(args, "path"),
        direction: string_arg(args, "direction").and_then(|value| parse_direction(&value)),
        include_hidden: bool_arg(args, "include_hidden").unwrap_or(false),
    }
}

fn parse_kind(value: &str) -> Option<CodeCityArcKind> {
    match value {
        "dependency" => Some(CodeCityArcKind::Dependency),
        "violation" => Some(CodeCityArcKind::Violation),
        "cross_boundary" => Some(CodeCityArcKind::CrossBoundary),
        "cycle" => Some(CodeCityArcKind::Cycle),
        "bridge" => Some(CodeCityArcKind::Bridge),
        _ => None,
    }
}

fn parse_visibility(value: &str) -> Option<CodeCityArcVisibility> {
    match value {
        "hidden_by_default" => Some(CodeCityArcVisibility::HiddenByDefault),
        "visible_on_selection" => Some(CodeCityArcVisibility::VisibleOnSelection),
        "visible_at_medium_zoom" => Some(CodeCityArcVisibility::VisibleAtMediumZoom),
        "visible_at_world_zoom" => Some(CodeCityArcVisibility::VisibleAtWorldZoom),
        "always_visible" => Some(CodeCityArcVisibility::AlwaysVisible),
        _ => None,
    }
}

fn parse_direction(value: &str) -> Option<CodeCityDependencyDirection> {
    match value {
        "incoming" => Some(CodeCityDependencyDirection::Incoming),
        "outgoing" => Some(CodeCityDependencyDirection::Outgoing),
        "both" => Some(CodeCityDependencyDirection::Both),
        _ => None,
    }
}

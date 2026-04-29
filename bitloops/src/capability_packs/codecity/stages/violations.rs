use anyhow::Result;
use serde_json::{Value, json};

use super::phase4_support::build_phase4_stage_data;
use crate::capability_packs::codecity::services::config::CodeCityConfig;
use crate::capability_packs::codecity::services::phase4::violations_connection;
use crate::capability_packs::codecity::types::{
    CODECITY_VIOLATIONS_STAGE_ID, CodeCityViolationFilter, CodeCityViolationPattern,
    CodeCityViolationRule, CodeCityViolationSeverity,
};
use crate::host::capability_host::{
    BoxFuture, CapabilityExecutionContext, StageHandler, StageRequest, StageResponse,
};

pub struct CodeCityViolationsStageHandler;

impl StageHandler for CodeCityViolationsStageHandler {
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
            let config = CodeCityConfig::default();
            let data =
                match build_phase4_stage_data(CODECITY_VIOLATIONS_STAGE_ID, &request, ctx, config)?
                {
                    Ok(data) => data,
                    Err(response) => return Ok(response),
                };
            let first = positive_usize_arg(&args, "first")
                .unwrap_or_else(|| request.limit().unwrap_or(100));
            let last = positive_usize_arg(&args, "last");
            let after = string_arg(&args, "after");
            let before = string_arg(&args, "before");
            let filter = violation_filter_from_args(&args);
            let payload = violations_connection(
                &data.snapshot.violations,
                &filter,
                first,
                after.as_deref(),
                last,
                before.as_deref(),
            );
            Ok(StageResponse::new(
                serde_json::to_value(payload)?,
                format!(
                    "codecity violations for repo {}: total={}",
                    data.repo_id,
                    data.snapshot.violations.len()
                ),
            ))
        })
    }
}

pub(super) fn violation_filter_from_args(args: &Value) -> CodeCityViolationFilter {
    CodeCityViolationFilter {
        severity: string_arg(args, "severity").and_then(|value| parse_severity(&value)),
        severities: array_string_arg(args, "severities")
            .into_iter()
            .filter_map(|value| parse_severity(&value))
            .collect(),
        pattern: string_arg(args, "pattern").and_then(|value| parse_pattern(&value)),
        rule: string_arg(args, "rule").and_then(|value| parse_rule(&value)),
        boundary_id: string_arg(args, "boundary_id"),
        path: string_arg(args, "path"),
        from_path: string_arg(args, "from_path"),
        to_path: string_arg(args, "to_path"),
        include_suppressed: bool_arg(args, "include_suppressed").unwrap_or(false),
    }
}

pub(super) fn positive_usize_arg(args: &Value, key: &str) -> Option<usize> {
    args.get(key)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value > 0)
}

pub(super) fn string_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(super) fn bool_arg(args: &Value, key: &str) -> Option<bool> {
    args.get(key).and_then(Value::as_bool)
}

fn array_string_arg(args: &Value, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn parse_severity(value: &str) -> Option<CodeCityViolationSeverity> {
    match value {
        "high" => Some(CodeCityViolationSeverity::High),
        "medium" => Some(CodeCityViolationSeverity::Medium),
        "low" => Some(CodeCityViolationSeverity::Low),
        "info" => Some(CodeCityViolationSeverity::Info),
        _ => None,
    }
}

fn parse_pattern(value: &str) -> Option<CodeCityViolationPattern> {
    match value {
        "layered" => Some(CodeCityViolationPattern::Layered),
        "hexagonal" => Some(CodeCityViolationPattern::Hexagonal),
        "modular" => Some(CodeCityViolationPattern::Modular),
        "event_driven" => Some(CodeCityViolationPattern::EventDriven),
        "cross_boundary" => Some(CodeCityViolationPattern::CrossBoundary),
        "cycle" => Some(CodeCityViolationPattern::Cycle),
        "mud" => Some(CodeCityViolationPattern::Mud),
        _ => None,
    }
}

fn parse_rule(value: &str) -> Option<CodeCityViolationRule> {
    match value {
        "layered_upward_dependency" => Some(CodeCityViolationRule::LayeredUpwardDependency),
        "layered_skipped_layer" => Some(CodeCityViolationRule::LayeredSkippedLayer),
        "hexagonal_core_imports_periphery" => {
            Some(CodeCityViolationRule::HexagonalCoreImportsPeriphery)
        }
        "hexagonal_core_imports_external" => {
            Some(CodeCityViolationRule::HexagonalCoreImportsExternal)
        }
        "hexagonal_application_imports_edge" => {
            Some(CodeCityViolationRule::HexagonalApplicationImportsEdge)
        }
        "modular_internal_cross_module_dependency" => {
            Some(CodeCityViolationRule::ModularInternalCrossModuleDependency)
        }
        "modular_broad_bridge_file" => Some(CodeCityViolationRule::ModularBroadBridgeFile),
        "event_driven_direct_peer_dependency" => {
            Some(CodeCityViolationRule::EventDrivenDirectPeerDependency)
        }
        "cross_boundary_cycle" => Some(CodeCityViolationRule::CrossBoundaryCycle),
        "cross_boundary_high_coupling" => Some(CodeCityViolationRule::CrossBoundaryHighCoupling),
        _ => None,
    }
}

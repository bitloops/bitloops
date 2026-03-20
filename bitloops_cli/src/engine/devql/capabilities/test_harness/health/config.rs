use crate::engine::devql::capability_host::{CapabilityHealthContext, CapabilityHealthResult};

use super::super::types::resolve_test_harness_config;

pub fn check_test_harness_config(ctx: &dyn CapabilityHealthContext) -> CapabilityHealthResult {
    let config = match ctx.config_view("test_harness") {
        Ok(view) => view,
        Err(err) => {
            return CapabilityHealthResult::failed(
                "test harness config unavailable",
                err.to_string(),
            );
        }
    };

    let Some(test_harness_config) = resolve_test_harness_config(&config) else {
        return CapabilityHealthResult::failed(
            "test harness config missing",
            "no `test_harness` namespace found in capability config".to_string(),
        );
    };

    if let Some(format) = test_harness_config
        .get("coverage")
        .and_then(|coverage| coverage.get("format"))
        .and_then(serde_json::Value::as_str)
        && !format.eq_ignore_ascii_case("lcov")
    {
        return CapabilityHealthResult::failed(
            "test harness coverage format unsupported",
            format!(
                "coverage format `{format}` is not supported in the current test-harness scaffold; expected `lcov`"
            ),
        );
    }

    CapabilityHealthResult::ok("test harness config valid")
}

use crate::host::capability_host::{
    CapabilityHealthCheck, CapabilityHealthContext, CapabilityHealthResult,
};

pub fn check_navigation_context_config(
    ctx: &dyn CapabilityHealthContext,
) -> CapabilityHealthResult {
    match ctx.config_view("navigation_context") {
        Ok(_) => CapabilityHealthResult::ok("navigation context config available"),
        Err(err) => {
            CapabilityHealthResult::failed("navigation context config unavailable", err.to_string())
        }
    }
}

pub fn check_navigation_context_storage(
    ctx: &dyn CapabilityHealthContext,
) -> CapabilityHealthResult {
    if let Err(err) = ctx.stores().check_relational() {
        return CapabilityHealthResult::failed(
            "navigation context relational store unavailable",
            err.to_string(),
        );
    }
    CapabilityHealthResult::ok("navigation context relational store healthy")
}

pub static NAVIGATION_CONTEXT_HEALTH_CHECKS: &[CapabilityHealthCheck] = &[
    CapabilityHealthCheck {
        name: "navigation_context.config",
        run: check_navigation_context_config,
    },
    CapabilityHealthCheck {
        name: "navigation_context.storage",
        run: check_navigation_context_storage,
    },
];

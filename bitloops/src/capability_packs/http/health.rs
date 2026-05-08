use crate::host::capability_host::{
    CapabilityHealthCheck, CapabilityHealthContext, CapabilityHealthResult,
};

pub fn check_http_config(ctx: &dyn CapabilityHealthContext) -> CapabilityHealthResult {
    match ctx.config_view("http") {
        Ok(_) => CapabilityHealthResult::ok("HTTP capability config available"),
        Err(err) => {
            CapabilityHealthResult::failed("HTTP capability config unavailable", err.to_string())
        }
    }
}

pub fn check_http_storage(ctx: &dyn CapabilityHealthContext) -> CapabilityHealthResult {
    if let Err(err) = ctx.stores().check_relational() {
        return CapabilityHealthResult::failed(
            "HTTP relational store unavailable",
            err.to_string(),
        );
    }
    CapabilityHealthResult::ok("HTTP relational store healthy")
}

pub static HTTP_HEALTH_CHECKS: &[CapabilityHealthCheck] = &[
    CapabilityHealthCheck {
        name: "http.config",
        run: check_http_config,
    },
    CapabilityHealthCheck {
        name: "http.storage",
        run: check_http_storage,
    },
];

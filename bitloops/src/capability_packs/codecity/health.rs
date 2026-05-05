use crate::host::capability_host::{
    CapabilityHealthCheck, CapabilityHealthContext, CapabilityHealthResult,
};

use super::services::config::CodeCityConfig;

pub fn check_codecity_config(_: &dyn CapabilityHealthContext) -> CapabilityHealthResult {
    match CodeCityConfig::default().validate() {
        Ok(()) => CapabilityHealthResult::ok("codecity defaults are valid"),
        Err(err) => CapabilityHealthResult::failed("codecity config invalid", format!("{err:#}")),
    }
}

pub fn check_codecity_source_data(ctx: &dyn CapabilityHealthContext) -> CapabilityHealthResult {
    if let Err(err) = ctx.stores().check_relational() {
        return CapabilityHealthResult::failed(
            "codecity relational store unavailable",
            err.to_string(),
        );
    }

    CapabilityHealthResult::ok(
        "codecity can read host-owned relational source data when current DevQL sync tables exist",
    )
}

pub static CODECITY_HEALTH_CHECKS: &[CapabilityHealthCheck] = &[
    CapabilityHealthCheck {
        name: "codecity.config",
        run: check_codecity_config,
    },
    CapabilityHealthCheck {
        name: "codecity.source_data",
        run: check_codecity_source_data,
    },
];

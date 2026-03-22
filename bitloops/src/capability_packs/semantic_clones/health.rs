use crate::host::capability_host::CapabilityHealthContext;
use crate::host::capability_host::health::{CapabilityHealthCheck, CapabilityHealthResult};

use super::types::SEMANTIC_CLONES_CAPABILITY_ID;

fn check_semantic_clones_config(ctx: &dyn CapabilityHealthContext) -> CapabilityHealthResult {
    match ctx.config_view(SEMANTIC_CLONES_CAPABILITY_ID) {
        Ok(_) => CapabilityHealthResult::ok("semantic_clones capability config view available"),
        Err(err) => CapabilityHealthResult::failed("semantic_clones.config", err.to_string()),
    }
}

pub static SEMANTIC_CLONES_HEALTH_CHECKS: &[CapabilityHealthCheck] = &[CapabilityHealthCheck {
    name: "semantic_clones.config",
    run: check_semantic_clones_config,
}];

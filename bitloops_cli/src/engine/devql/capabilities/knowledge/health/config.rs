use crate::engine::devql::capability_host::{CapabilityHealthContext, CapabilityHealthResult};

pub fn check_knowledge_config(ctx: &dyn CapabilityHealthContext) -> CapabilityHealthResult {
    match ctx.config_view("knowledge") {
        Ok(_) => CapabilityHealthResult::ok("knowledge config present"),
        Err(err) => CapabilityHealthResult::failed(
            "knowledge config invalid",
            err.to_string(),
        ),
    }
}

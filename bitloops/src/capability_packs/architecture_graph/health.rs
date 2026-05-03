use crate::host::capability_host::{
    CapabilityHealthCheck, CapabilityHealthContext, CapabilityHealthResult,
};

pub fn check_architecture_graph_config(
    ctx: &dyn CapabilityHealthContext,
) -> CapabilityHealthResult {
    match ctx.config_view("architecture_graph") {
        Ok(_) => CapabilityHealthResult::ok("architecture graph config available"),
        Err(err) => {
            CapabilityHealthResult::failed("architecture graph config unavailable", err.to_string())
        }
    }
}

pub fn check_architecture_graph_storage(
    ctx: &dyn CapabilityHealthContext,
) -> CapabilityHealthResult {
    if let Err(err) = ctx.stores().check_relational() {
        return CapabilityHealthResult::failed(
            "architecture graph relational store unavailable",
            err.to_string(),
        );
    }
    CapabilityHealthResult::ok("architecture graph relational store healthy")
}

pub static ARCHITECTURE_GRAPH_HEALTH_CHECKS: &[CapabilityHealthCheck] = &[
    CapabilityHealthCheck {
        name: "architecture_graph.config",
        run: check_architecture_graph_config,
    },
    CapabilityHealthCheck {
        name: "architecture_graph.storage",
        run: check_architecture_graph_storage,
    },
];

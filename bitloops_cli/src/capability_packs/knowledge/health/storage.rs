use crate::host::devql::capability_host::{CapabilityHealthContext, CapabilityHealthResult};

pub fn check_knowledge_storage(ctx: &dyn CapabilityHealthContext) -> CapabilityHealthResult {
    let stores = ctx.stores();
    if let Err(err) = stores.check_relational() {
        return CapabilityHealthResult::failed(
            "knowledge relational store unavailable",
            err.to_string(),
        );
    }
    if let Err(err) = stores.check_documents() {
        return CapabilityHealthResult::failed(
            "knowledge documents store unavailable",
            err.to_string(),
        );
    }
    if let Err(err) = stores.check_blobs() {
        return CapabilityHealthResult::failed("knowledge blob store unavailable", err.to_string());
    }

    CapabilityHealthResult::ok("knowledge stores healthy")
}

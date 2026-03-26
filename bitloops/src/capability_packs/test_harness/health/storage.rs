use crate::host::capability_host::{CapabilityHealthContext, CapabilityHealthResult};

pub fn check_test_harness_storage(ctx: &dyn CapabilityHealthContext) -> CapabilityHealthResult {
    let stores = ctx.stores();
    if let Err(err) = stores.check_relational() {
        return CapabilityHealthResult::failed(
            "test harness relational store unavailable",
            err.to_string(),
        );
    }
    if let Err(err) = stores.check_documents() {
        return CapabilityHealthResult::failed(
            "test harness documents store unavailable",
            err.to_string(),
        );
    }
    if let Err(err) = stores.check_blobs() {
        return CapabilityHealthResult::failed(
            "test harness blob store unavailable",
            err.to_string(),
        );
    }

    CapabilityHealthResult::ok("test harness stores healthy")
}

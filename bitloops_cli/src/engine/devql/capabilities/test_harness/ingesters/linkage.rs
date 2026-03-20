use crate::engine::devql::capability_host::{
    BoxFuture, CapabilityIngestContext, IngestRequest, IngestResult, IngesterHandler,
};

use super::super::types::dependency_gated_ingest_result;

pub struct LinkageIngester;

impl IngesterHandler for LinkageIngester {
    fn ingest<'a>(
        &'a self,
        request: IngestRequest,
        _ctx: &'a mut dyn CapabilityIngestContext,
    ) -> BoxFuture<'a, anyhow::Result<IngestResult>> {
        Box::pin(async move {
            Ok(dependency_gated_ingest_result(
                "test_harness.linkage",
                request.payload,
            ))
        })
    }
}

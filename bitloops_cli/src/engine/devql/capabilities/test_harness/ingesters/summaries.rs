use crate::engine::devql::capability_host::{
    BoxFuture, CapabilityIngestContext, IngestRequest, IngestResult, IngesterHandler,
};

use super::super::types::{TEST_HARNESS_SUMMARIES_INGESTER_ID, dependency_gated_ingest_result};

pub struct SummariesIngester;

impl IngesterHandler for SummariesIngester {
    fn ingest<'a>(
        &'a self,
        request: IngestRequest,
        _ctx: &'a mut dyn CapabilityIngestContext,
    ) -> BoxFuture<'a, anyhow::Result<IngestResult>> {
        Box::pin(async move {
            Ok(dependency_gated_ingest_result(
                TEST_HARNESS_SUMMARIES_INGESTER_ID,
                request.payload,
            ))
        })
    }
}

use anyhow::Context;
use serde::Deserialize;
use serde_json::json;

use crate::capability_packs::test_harness::storage::TestHarnessRepository;
use crate::host::capability_host::{
    BoxFuture, CapabilityIngestContext, IngestRequest, IngestResult, IngesterHandler,
};

use super::super::types::TEST_HARNESS_CLASSIFICATION_INGESTER_ID;

#[derive(Debug, Deserialize)]
struct ClassificationIngestPayload {
    commit_sha: String,
}

pub struct ClassificationIngester;

impl IngesterHandler for ClassificationIngester {
    fn ingest<'a>(
        &'a self,
        request: IngestRequest,
        ctx: &'a mut dyn CapabilityIngestContext,
    ) -> BoxFuture<'a, anyhow::Result<IngestResult>> {
        Box::pin(async move {
            let Some(store) = ctx.test_harness_store() else {
                return Ok(IngestResult::new(
                    json!({
                        "capability": "test_harness",
                        "ingester": TEST_HARNESS_CLASSIFICATION_INGESTER_ID,
                        "status": "failed",
                        "reason": "test_harness_relational_store_unavailable",
                    }),
                    "test harness relational store is not available; configure stores.relational, create the database, and ensure the daemon is running (`bitloops start`).",
                ));
            };

            let payload: ClassificationIngestPayload = request
                .parse_json()
                .context("parse test_harness.classification ingest payload")?;

            let mut g = store
                .lock()
                .map_err(|e| anyhow::anyhow!("test harness store lock poisoned: {e}"))?;
            let inserted = TestHarnessRepository::rebuild_classifications_from_coverage(
                &mut *g,
                payload.commit_sha.as_str(),
            )?;

            let human = format!(
                "rebuilt {} coverage-derived test classifications for commit {}",
                inserted, payload.commit_sha
            );
            Ok(IngestResult::new(
                json!({
                    "capability": "test_harness",
                    "ingester": TEST_HARNESS_CLASSIFICATION_INGESTER_ID,
                    "status": "ok",
                    "commit_sha": payload.commit_sha,
                    "classifications_inserted": inserted,
                }),
                human,
            ))
        })
    }
}

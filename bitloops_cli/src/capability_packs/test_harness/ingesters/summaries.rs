use std::sync::{Arc, Mutex};

use anyhow::Context;
use serde::Deserialize;
use serde_json::json;

use crate::engine::devql::capability_host::{
    BoxFuture, CapabilityIngestContext, IngestRequest, IngestResult, IngesterHandler,
};
use crate::engine::test_harness::BitloopsTestHarnessRepository;
use crate::repository::TestHarnessQueryRepository;

use super::super::types::TEST_HARNESS_SUMMARIES_INGESTER_ID;

#[derive(Debug, Deserialize)]
struct SummariesIngestPayload {
    commit_sha: String,
}

pub struct SummariesIngester(pub Option<Arc<Mutex<BitloopsTestHarnessRepository>>>);

impl IngesterHandler for SummariesIngester {
    fn ingest<'a>(
        &'a self,
        request: IngestRequest,
        _ctx: &'a mut dyn CapabilityIngestContext,
    ) -> BoxFuture<'a, anyhow::Result<IngestResult>> {
        let store = self.0.clone();
        Box::pin(async move {
            let Some(store) = store else {
                return Ok(IngestResult::new(
                    json!({
                        "capability": "test_harness",
                        "ingester": TEST_HARNESS_SUMMARIES_INGESTER_ID,
                        "status": "failed",
                        "reason": "test_harness_relational_store_unavailable",
                    }),
                    "test harness relational store is not available; configure stores.relational, create the database, and run `bitloops testlens init` if needed.",
                ));
            };

            let payload: SummariesIngestPayload = request
                .parse_json()
                .context("parse test_harness.summaries ingest payload")?;

            let g = store
                .lock()
                .map_err(|e| anyhow::anyhow!("test harness store lock poisoned: {e}"))?;
            let counts = TestHarnessQueryRepository::load_test_harness_commit_counts(
                &*g,
                payload.commit_sha.as_str(),
            )?;

            let coverage_present =
                TestHarnessQueryRepository::coverage_exists_for_commit(&*g, &payload.commit_sha)?;

            let human = format!(
                "test harness snapshot for commit {}: suites={}, scenarios={}, links={}, classifications={}, coverage_captures={}, coverage_hits={}, coverage_indexed={}",
                payload.commit_sha,
                counts.test_suites,
                counts.test_scenarios,
                counts.test_links,
                counts.test_classifications,
                counts.coverage_captures,
                counts.coverage_hits,
                coverage_present
            );

            Ok(IngestResult::new(
                json!({
                    "capability": "test_harness",
                    "ingester": TEST_HARNESS_SUMMARIES_INGESTER_ID,
                    "status": "ok",
                    "commit_sha": payload.commit_sha,
                    "counts": {
                        "test_suites": counts.test_suites,
                        "test_scenarios": counts.test_scenarios,
                        "test_links": counts.test_links,
                        "test_classifications": counts.test_classifications,
                        "coverage_captures": counts.coverage_captures,
                        "coverage_hits": counts.coverage_hits,
                    },
                    "coverage_present": coverage_present,
                }),
                human,
            ))
        })
    }
}

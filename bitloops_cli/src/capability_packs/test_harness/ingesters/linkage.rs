use std::sync::{Arc, Mutex};

use anyhow::Context;
use serde::Deserialize;
use serde_json::json;

use crate::capability_packs::test_harness::ingest::tests;
use crate::capability_packs::test_harness::storage::BitloopsTestHarnessRepository;
use crate::host::devql::capability_host::{
    BoxFuture, CapabilityIngestContext, IngestRequest, IngestResult, IngesterHandler,
};

use super::super::types::TEST_HARNESS_LINKAGE_INGESTER_ID;

#[derive(Debug, Deserialize)]
struct LinkageIngestPayload {
    commit_sha: String,
}

pub struct LinkageIngester(pub Option<Arc<Mutex<BitloopsTestHarnessRepository>>>);

impl IngesterHandler for LinkageIngester {
    fn ingest<'a>(
        &'a self,
        request: IngestRequest,
        ctx: &'a mut dyn CapabilityIngestContext,
    ) -> BoxFuture<'a, anyhow::Result<IngestResult>> {
        let store = self.0.clone();
        Box::pin(async move {
            let Some(store) = store else {
                return Ok(IngestResult::new(
                    json!({
                        "capability": "test_harness",
                        "ingester": TEST_HARNESS_LINKAGE_INGESTER_ID,
                        "status": "failed",
                        "reason": "test_harness_relational_store_unavailable",
                    }),
                    "test harness relational store is not available; configure stores.relational, create the database, and run `bitloops testlens init` if needed.",
                ));
            };

            let payload: LinkageIngestPayload = request
                .parse_json()
                .context("parse test_harness.linkage ingest payload")?;

            let mut g = store
                .lock()
                .map_err(|e| anyhow::anyhow!("test harness store lock poisoned: {e}"))?;
            let summary = tests::execute(&mut *g, ctx.repo_root(), payload.commit_sha.as_str())?;

            let human = tests::format_summary(&payload.commit_sha, &summary);
            Ok(IngestResult::new(
                json!({
                    "capability": "test_harness",
                    "ingester": TEST_HARNESS_LINKAGE_INGESTER_ID,
                    "status": "ok",
                    "commit_sha": payload.commit_sha,
                    "summary": {
                        "files": summary.files,
                        "suites": summary.suites,
                        "scenarios": summary.scenarios,
                        "links": summary.links,
                        "enumeration_status": summary.enumeration_status,
                        "enumerated_scenarios": summary.enumerated_scenarios,
                    }
                }),
                human,
            ))
        })
    }
}

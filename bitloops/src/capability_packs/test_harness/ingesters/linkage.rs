use anyhow::Context;
use serde::Deserialize;
use serde_json::json;

use crate::capability_packs::test_harness::ingest::tests;
use crate::host::capability_host::{
    BoxFuture, CapabilityIngestContext, IngestRequest, IngestResult, IngesterHandler,
};

use super::super::types::TEST_HARNESS_LINKAGE_INGESTER_ID;

#[derive(Debug, Deserialize)]
struct LinkageIngestPayload {
    commit_sha: String,
}

pub struct LinkageIngester;

impl IngesterHandler for LinkageIngester {
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
                        "ingester": TEST_HARNESS_LINKAGE_INGESTER_ID,
                        "status": "failed",
                        "reason": "test_harness_relational_store_unavailable",
                    }),
                    "test harness relational store is not available; configure stores.relational, create the database, and ensure the daemon is running (`bitloops start`).",
                ));
            };

            let payload: LinkageIngestPayload = request
                .parse_json()
                .context("parse test_harness.linkage ingest payload")?;

            let relational = ctx.host_relational();
            let mut g = store
                .lock()
                .map_err(|e| anyhow::anyhow!("test harness store lock poisoned: {e}"))?;
            let summary = tests::execute(
                &mut *g,
                relational,
                ctx.repo_root(),
                payload.commit_sha.as_str(),
                ctx.languages(),
            )?;

            let human = tests::format_summary(&payload.commit_sha, &summary);
            Ok(IngestResult::new(
                json!({
                    "capability": "test_harness",
                    "ingester": TEST_HARNESS_LINKAGE_INGESTER_ID,
                    "status": "ok",
                    "commit_sha": payload.commit_sha,
                    "summary": {
                        "files": summary.files,
                        "test_artefacts": summary.test_artefacts,
                        "test_edges": summary.test_edges,
                        "enumeration_status": summary.enumeration_status,
                        "enumerated_scenarios": summary.enumerated_scenarios,
                    }
                }),
                human,
            ))
        })
    }
}

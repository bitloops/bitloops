use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::json;

use crate::app::commands::ingest_coverage;
use crate::domain::{CoverageFormat, ScopeKind};
use crate::engine::devql::capability_host::{
    BoxFuture, CapabilityIngestContext, IngestRequest, IngestResult, IngesterHandler,
};
use crate::engine::test_harness::BitloopsTestHarnessRepository;

use super::super::types::TEST_HARNESS_COVERAGE_INGESTER_ID;

#[derive(Debug, Deserialize)]
struct CoverageIngestPayload {
    coverage_path: String,
    commit_sha: String,
    scope_kind: String,
    tool: String,
    test_artefact_id: Option<String>,
    /// `lcov` or `llvm-json` (see [`CoverageFormat::from_str`]).
    format: String,
}

pub struct CoverageIngestIngester(pub Option<Arc<Mutex<BitloopsTestHarnessRepository>>>);

impl IngesterHandler for CoverageIngestIngester {
    fn ingest<'a>(
        &'a self,
        request: IngestRequest,
        ctx: &'a mut dyn CapabilityIngestContext,
    ) -> BoxFuture<'a, Result<IngestResult>> {
        let store = self.0.clone();
        Box::pin(async move {
            let Some(store) = store else {
                return Ok(IngestResult::new(
                    json!({
                        "capability": "test_harness",
                        "ingester": TEST_HARNESS_COVERAGE_INGESTER_ID,
                        "status": "failed",
                        "reason": "test_harness_relational_store_unavailable",
                    }),
                    "test harness relational store is not available; configure stores.relational, create the database, and run `bitloops testlens init` if needed.",
                ));
            };

            let payload: CoverageIngestPayload = request
                .parse_json()
                .context("parse test_harness.coverage ingest payload")?;

            let scope_kind: ScopeKind = payload
                .scope_kind
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid scope_kind in payload"))?;
            let format: CoverageFormat = payload
                .format
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid coverage format in payload"))?;

            if scope_kind == ScopeKind::TestScenario {
                if payload.test_artefact_id.is_none() {
                    bail!("test_artefact_id is required when scope_kind is test-scenario");
                }
                if format == CoverageFormat::Lcov {
                    bail!("LCOV is not supported for scope=test-scenario; use llvm-json");
                }
            }

            let path = PathBuf::from(&payload.coverage_path);
            let coverage_path = if path.is_absolute() {
                path
            } else {
                ctx.repo_root().join(&path)
            };

            let mut g = store
                .lock()
                .map_err(|e| anyhow::anyhow!("test harness store lock poisoned: {e}"))?;
            let summary = ingest_coverage::execute(
                &mut *g,
                &coverage_path,
                &payload.commit_sha,
                scope_kind,
                &payload.tool,
                payload.test_artefact_id.as_deref(),
                format,
            )?;

            let human = ingest_coverage::format_summary(&payload.commit_sha, &summary);
            Ok(IngestResult::new(
                json!({
                    "capability": "test_harness",
                    "ingester": TEST_HARNESS_COVERAGE_INGESTER_ID,
                    "status": "ok",
                    "commit_sha": payload.commit_sha,
                    "summary": {
                        "format": summary.format.as_str(),
                        "scope_kind": summary.scope_kind.to_string(),
                        "hits": summary.hits,
                        "classifications": summary.classifications,
                        "diagnostics": summary.diagnostics,
                    }
                }),
                human,
            ))
        })
    }
}

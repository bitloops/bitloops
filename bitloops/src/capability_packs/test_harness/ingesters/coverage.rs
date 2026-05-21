use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::json;

use crate::capability_packs::test_harness::ingest::coverage;
use crate::host::capability_host::{
    BoxFuture, CapabilityIngestContext, IngestRequest, IngestResult, IngesterHandler,
};
use crate::host::checkpoints::strategy::manual_commit::{run_git, try_head_hash};
use crate::models::{CoverageFormat, ScopeKind};

use super::super::types::TEST_HARNESS_COVERAGE_INGESTER_ID;

#[derive(Debug, Deserialize)]
struct CoverageIngestPayload {
    coverage_path: String,
    commit_sha: Option<String>,
    #[serde(default = "default_scope_kind")]
    scope_kind: String,
    tool: String,
    test_artefact_id: Option<String>,
    /// `lcov` or `llvm-json` (see [`CoverageFormat::from_str`]).
    format: String,
}

pub struct CoverageIngestIngester;

impl IngesterHandler for CoverageIngestIngester {
    fn ingest<'a>(
        &'a self,
        request: IngestRequest,
        ctx: &'a mut dyn CapabilityIngestContext,
    ) -> BoxFuture<'a, Result<IngestResult>> {
        Box::pin(async move {
            let Some(store) = ctx.test_harness_store() else {
                return Ok(IngestResult::new(
                    json!({
                        "capability": "test_harness",
                        "ingester": TEST_HARNESS_COVERAGE_INGESTER_ID,
                        "status": "failed",
                        "reason": "test_harness_relational_store_unavailable",
                    }),
                    "test harness relational store is not available; configure stores.relational, create the database, and ensure the daemon is running (`bitloops start`).",
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

            let commit_sha = payload
                .commit_sha
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty());
            let relational = ctx.host_relational();

            if let Some(commit_sha) = commit_sha {
                let mut g = store
                    .lock()
                    .map_err(|e| anyhow::anyhow!("test harness store lock poisoned: {e}"))?;
                let summary = coverage::execute(
                    &mut *g,
                    relational,
                    &coverage_path,
                    commit_sha,
                    scope_kind,
                    &payload.tool,
                    payload.test_artefact_id.as_deref(),
                    format,
                )?;

                let human = coverage::format_summary(commit_sha, &summary);
                return Ok(IngestResult::new(
                    json!({
                        "capability": "test_harness",
                        "ingester": TEST_HARNESS_COVERAGE_INGESTER_ID,
                        "status": "ok",
                        "reference_mode": "commit",
                        "commit_sha": commit_sha,
                        "summary": {
                            "format": summary.format.as_str(),
                            "scope_kind": summary.scope_kind.to_string(),
                            "hits": summary.hits,
                            "classifications": summary.classifications,
                            "diagnostics": summary.diagnostics,
                        }
                    }),
                    human,
                ));
            }

            let observed_head_sha = try_head_hash(ctx.repo_root())
                .context("resolve current git HEAD for coverage provenance")?;
            let stored_commit_sha = observed_head_sha
                .clone()
                .unwrap_or_else(|| "current".to_string());
            let provenance = coverage::CurrentCoverageProvenance {
                observed_head_sha: observed_head_sha.clone(),
                repo_dirty: repo_dirty(ctx.repo_root()),
                coverage_file_modified_at_unix: file_modified_unix(&coverage_path),
                ingested_at_unix: unix_now(),
                coverage_path: coverage_path.display().to_string(),
            };
            let mut g = store
                .lock()
                .map_err(|e| anyhow::anyhow!("test harness store lock poisoned: {e}"))?;
            let summary = coverage::execute_current(
                &mut *g,
                relational,
                &coverage_path,
                &ctx.repo().repo_id,
                scope_kind,
                &payload.tool,
                payload.test_artefact_id.as_deref(),
                format,
                provenance,
            )?;

            let human = coverage::format_current_summary(&summary, observed_head_sha.as_deref());
            Ok(IngestResult::new(
                json!({
                    "capability": "test_harness",
                    "ingester": TEST_HARNESS_COVERAGE_INGESTER_ID,
                    "status": "ok",
                    "reference_mode": "current",
                    "commit_sha": stored_commit_sha,
                    "observed_head_sha": observed_head_sha,
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

fn default_scope_kind() -> String {
    ScopeKind::Workspace.to_string()
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn file_modified_unix(path: &Path) -> Option<u64> {
    std::fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs())
}

fn repo_dirty(repo_root: &Path) -> Option<bool> {
    run_git(
        repo_root,
        &["status", "--porcelain=v1", "--untracked-files=all"],
    )
    .ok()
    .map(|status| !status.is_empty())
}

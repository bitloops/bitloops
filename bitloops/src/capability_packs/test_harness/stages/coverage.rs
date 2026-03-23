use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use anyhow::anyhow;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::capability_packs::test_harness::storage::{
    BitloopsTestHarnessRepository, TestHarnessQueryRepository,
};
use crate::capability_packs::test_harness::types::test_harness_relational_store_unavailable_stage_response;
use crate::host::capability_host::{
    BoxFuture, CapabilityExecutionContext, StageHandler, StageRequest, StageResponse,
};

#[derive(Debug, Deserialize)]
struct CoverageStagePayload {
    #[serde(default)]
    input_rows: Vec<Value>,
    #[serde(default)]
    args: BTreeMap<String, String>,
    #[serde(default)]
    query_context: CoverageQueryContext,
}

#[derive(Debug, Default, Deserialize)]
struct CoverageQueryContext {
    resolved_commit_sha: Option<String>,
}

pub struct CoverageStageHandler(pub Option<Arc<Mutex<BitloopsTestHarnessRepository>>>);

impl StageHandler for CoverageStageHandler {
    fn execute<'a>(
        &'a self,
        request: StageRequest,
        ctx: &'a mut dyn CapabilityExecutionContext,
    ) -> BoxFuture<'a, anyhow::Result<StageResponse>> {
        let store = self.0.clone();
        Box::pin(async move {
            let Some(store) = store else {
                return Ok(test_harness_relational_store_unavailable_stage_response());
            };

            let payload: CoverageStagePayload = request.parse_json()?;
            let _ = payload.args;
            let repo_id = ctx.repo().repo_id.clone();
            let commit_sha = payload.query_context.resolved_commit_sha;

            let g = store
                .lock()
                .map_err(|e| anyhow!("test harness store lock poisoned: {e}"))?;

            let metadata = g.load_stage_coverage_metadata(&repo_id, commit_sha.as_deref())?;
            let coverage_source = metadata
                .as_ref()
                .map(|m| m.coverage_source.as_str())
                .unwrap_or("lcov")
                .to_string();
            let branch_truth = metadata.as_ref().map(|m| m.branch_truth).unwrap_or(0);

            let mut out = Vec::with_capacity(payload.input_rows.len());
            for input_row in payload.input_rows {
                let Some(row_obj) = input_row.as_object() else {
                    continue;
                };

                let artefact_id = row_obj
                    .get("artefact_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if artefact_id.is_empty() {
                    continue;
                }

                let artefact = json!({
                    "artefact_id": artefact_id,
                    "name": row_obj.get("symbol_fqn").and_then(Value::as_str).unwrap_or(artefact_id),
                    "kind": row_obj.get("canonical_kind").and_then(Value::as_str).unwrap_or("unknown"),
                    "file_path": row_obj.get("path").and_then(Value::as_str).unwrap_or(""),
                    "start_line": row_obj.get("start_line").and_then(Value::as_i64).unwrap_or(0),
                    "end_line": row_obj.get("end_line").and_then(Value::as_i64).unwrap_or(0),
                });

                let line_records = g.load_stage_line_coverage(
                    &repo_id,
                    artefact_id,
                    commit_sha.as_deref(),
                )?;
                let total_lines = line_records.len();
                let mut uncovered_lines = Vec::new();
                let mut covered_line_count = 0usize;
                for rec in &line_records {
                    if rec.covered {
                        covered_line_count += 1;
                    } else {
                        uncovered_lines.push(rec.line);
                    }
                }
                let line_coverage_pct = if total_lines > 0 {
                    (covered_line_count as f64 / total_lines as f64) * 100.0
                } else {
                    0.0
                };

                let branch_records = g.load_stage_branch_coverage(
                    &repo_id,
                    artefact_id,
                    commit_sha.as_deref(),
                )?;
                let total_branches = branch_records.len();
                let mut uncovered_branch_count = 0usize;
                let mut branches = Vec::with_capacity(total_branches);
                for rec in &branch_records {
                    if !rec.covered {
                        uncovered_branch_count += 1;
                    }
                    branches.push(json!({
                        "line": rec.line,
                        "block": 0,
                        "branch": rec.branch_id,
                        "covered": rec.covered,
                        "hit_count": rec.hit_count,
                    }));
                }
                let branch_coverage_pct = if total_branches > 0 {
                    let covered_branches = total_branches - uncovered_branch_count;
                    (covered_branches as f64 / total_branches as f64) * 100.0
                } else {
                    0.0
                };

                let coverage = json!({
                    "coverage_source": coverage_source,
                    "line_coverage_pct": line_coverage_pct,
                    "branch_coverage_pct": branch_coverage_pct,
                    "line_data_available": !line_records.is_empty(),
                    "branch_data_available": branch_truth == 1 || !branch_records.is_empty(),
                    "uncovered_lines": uncovered_lines,
                    "branches": branches,
                });

                let summary = json!({
                    "uncovered_line_count": uncovered_lines.len(),
                    "uncovered_branch_count": uncovered_branch_count,
                    "diagnostic_count": 0,
                });

                out.push(json!({
                    "artefact": artefact,
                    "coverage": coverage,
                    "summary": summary,
                }));
            }

            Ok(StageResponse::json(Value::Array(out)))
        })
    }
}

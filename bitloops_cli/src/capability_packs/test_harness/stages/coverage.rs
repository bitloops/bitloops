use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::host::devql::capability_host::{
    BoxFuture, CapabilityExecutionContext, DevqlSubqueryOptions, StageHandler, StageRequest,
    StageResponse, execute_devql_subquery,
};

use super::super::types::{
    TEST_HARNESS_CAPABILITY_ID, TEST_HARNESS_CORE_BRANCH_COVERAGE_STAGE_ID,
    TEST_HARNESS_CORE_COVERAGE_METADATA_STAGE_ID, TEST_HARNESS_CORE_LINE_COVERAGE_STAGE_ID,
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

pub struct CoverageStageHandler;

impl StageHandler for CoverageStageHandler {
    fn execute<'a>(
        &'a self,
        request: StageRequest,
        ctx: &'a mut dyn CapabilityExecutionContext,
    ) -> BoxFuture<'a, anyhow::Result<StageResponse>> {
        Box::pin(async move {
            let payload: CoverageStagePayload = request.parse_json()?;
            let _ = payload.args;
            let repo_identity = ctx.repo().identity.clone();
            let commit_sha = payload.query_context.resolved_commit_sha.as_deref();

            let metadata_query = build_core_coverage_metadata_query(&repo_identity, commit_sha);
            let metadata_rows = execute_devql_subquery(
                ctx,
                &request,
                &metadata_query,
                DevqlSubqueryOptions::new(TEST_HARNESS_CAPABILITY_ID),
            )
            .await?;
            let metadata = as_array_rows(metadata_rows).into_iter().next();
            let coverage_source = metadata
                .as_ref()
                .and_then(|value| value.get("coverage_source"))
                .and_then(Value::as_str)
                .unwrap_or("lcov")
                .to_string();
            let branch_truth = metadata
                .as_ref()
                .and_then(|value| value.get("branch_truth"))
                .map(parse_i64)
                .unwrap_or(0);

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

                let line_query =
                    build_core_line_coverage_query(&repo_identity, commit_sha, artefact_id);
                let line_rows = execute_devql_subquery(
                    ctx,
                    &request,
                    &line_query,
                    DevqlSubqueryOptions::new(TEST_HARNESS_CAPABILITY_ID),
                )
                .await?;
                let line_rows = as_array_rows(line_rows);
                let total_lines = line_rows.len();
                let mut uncovered_lines = Vec::new();
                let mut covered_line_count = 0usize;
                for row in &line_rows {
                    let Some(row_obj) = row.as_object() else {
                        continue;
                    };
                    if parse_i64_field(row_obj, "covered_any") == 1 {
                        covered_line_count += 1;
                    } else {
                        uncovered_lines.push(parse_i64_field(row_obj, "line"));
                    }
                }
                let line_coverage_pct = if total_lines > 0 {
                    (covered_line_count as f64 / total_lines as f64) * 100.0
                } else {
                    0.0
                };

                let branch_query =
                    build_core_branch_coverage_query(&repo_identity, commit_sha, artefact_id);
                let branch_rows = execute_devql_subquery(
                    ctx,
                    &request,
                    &branch_query,
                    DevqlSubqueryOptions::new(TEST_HARNESS_CAPABILITY_ID),
                )
                .await?;
                let branch_rows = as_array_rows(branch_rows);
                let total_branches = branch_rows.len();
                let mut uncovered_branch_count = 0usize;
                let mut branches = Vec::with_capacity(total_branches);
                for row in &branch_rows {
                    let Some(row_obj) = row.as_object() else {
                        continue;
                    };
                    let is_covered = parse_i64_field(row_obj, "covered_any") == 1;
                    if !is_covered {
                        uncovered_branch_count += 1;
                    }
                    branches.push(json!({
                        "line": parse_i64_field(row_obj, "line"),
                        "block": 0,
                        "branch": parse_i64_field(row_obj, "branch_id"),
                        "covered": is_covered,
                        "hit_count": parse_i64_field(row_obj, "hit_count"),
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
                    "line_data_available": !line_rows.is_empty(),
                    "branch_data_available": branch_truth == 1 || !branch_rows.is_empty(),
                    "uncovered_lines": uncovered_lines,
                    "branches": branches,
                });

                let summary = json!({
                    "uncovered_line_count": coverage
                        .get("uncovered_lines")
                        .and_then(Value::as_array)
                        .map(Vec::len)
                        .unwrap_or(0),
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

fn build_core_coverage_metadata_query(repo_identity: &str, commit_sha: Option<&str>) -> String {
    let mut parts = vec![format!("repo({})", quote_devql(repo_identity))];
    if let Some(commit_sha) = commit_sha {
        parts.push(format!("asOf(commit:{})", quote_devql(commit_sha)));
    }
    parts.push(format!("{TEST_HARNESS_CORE_COVERAGE_METADATA_STAGE_ID}()"));
    parts.push("limit(1)".to_string());
    parts.join("->")
}

fn build_core_line_coverage_query(
    repo_identity: &str,
    commit_sha: Option<&str>,
    artefact_id: &str,
) -> String {
    let mut parts = vec![format!("repo({})", quote_devql(repo_identity))];
    if let Some(commit_sha) = commit_sha {
        parts.push(format!("asOf(commit:{})", quote_devql(commit_sha)));
    }
    parts.push(format!(
        "{}(artefact_id:{})",
        TEST_HARNESS_CORE_LINE_COVERAGE_STAGE_ID,
        quote_devql(artefact_id),
    ));
    parts.join("->")
}

fn build_core_branch_coverage_query(
    repo_identity: &str,
    commit_sha: Option<&str>,
    artefact_id: &str,
) -> String {
    let mut parts = vec![format!("repo({})", quote_devql(repo_identity))];
    if let Some(commit_sha) = commit_sha {
        parts.push(format!("asOf(commit:{})", quote_devql(commit_sha)));
    }
    parts.push(format!(
        "{}(artefact_id:{})",
        TEST_HARNESS_CORE_BRANCH_COVERAGE_STAGE_ID,
        quote_devql(artefact_id),
    ));
    parts.join("->")
}

fn quote_devql(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "'"))
}

fn as_array_rows(value: Value) -> Vec<Value> {
    match value {
        Value::Array(rows) => rows,
        other => vec![other],
    }
}

fn parse_i64(value: &Value) -> i64 {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|item| i64::try_from(item).ok()))
        .unwrap_or(0)
}

fn parse_i64_field(row_obj: &serde_json::Map<String, Value>, key: &str) -> i64 {
    row_obj.get(key).map(parse_i64).unwrap_or(0)
}

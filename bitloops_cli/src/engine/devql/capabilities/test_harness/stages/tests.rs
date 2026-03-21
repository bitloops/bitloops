use std::collections::BTreeMap;

use anyhow::anyhow;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::engine::devql::capability_host::{
    BoxFuture, CapabilityExecutionContext, DevqlSubqueryOptions, StageHandler, StageRequest,
    StageResponse, execute_devql_subquery,
};

use super::super::types::{TEST_HARNESS_CAPABILITY_ID, TEST_HARNESS_CORE_TEST_LINKS_STAGE_ID};

#[derive(Debug, Deserialize)]
struct TestsStagePayload {
    #[serde(default)]
    input_rows: Vec<Value>,
    #[serde(default)]
    args: BTreeMap<String, String>,
}

pub struct TestsStageHandler;

impl StageHandler for TestsStageHandler {
    fn execute<'a>(
        &'a self,
        request: StageRequest,
        ctx: &'a mut dyn CapabilityExecutionContext,
    ) -> BoxFuture<'a, anyhow::Result<StageResponse>> {
        Box::pin(async move {
            let payload: TestsStagePayload = request.parse_json()?;
            let limit = request.limit().unwrap_or(100).max(1);

            let min_confidence = payload
                .args
                .get("min_confidence")
                .map(|value| {
                    value
                        .parse::<f64>()
                        .map(|parsed| parsed.clamp(0.0, 1.0))
                        .map_err(|_| {
                            anyhow!("tests(min_confidence:...) must be a valid numeric value")
                        })
                })
                .transpose()?;
            let linkage_source = payload.args.get("linkage_source").map(String::as_str);

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

                let core_query = build_core_test_links_query(
                    &ctx.repo().identity,
                    artefact_id,
                    min_confidence,
                    linkage_source,
                    limit,
                );
                let subquery_rows = execute_devql_subquery(
                    ctx,
                    &request,
                    &core_query,
                    DevqlSubqueryOptions::new(TEST_HARNESS_CAPABILITY_ID),
                )
                .await?;
                let covering_tests_rows = as_array_rows(subquery_rows)
                    .into_iter()
                    .filter_map(|row| {
                        let row_obj = row.as_object()?;
                        Some(json!({
                            "test_id": row_obj.get("test_id").and_then(Value::as_str).unwrap_or_default(),
                            "test_name": row_obj.get("test_name").and_then(Value::as_str).unwrap_or_default(),
                            "suite_name": row_obj.get("suite_name").cloned().unwrap_or(Value::Null),
                            "file_path": row_obj.get("file_path").and_then(Value::as_str).unwrap_or_default(),
                            "confidence": row_obj.get("confidence").cloned().unwrap_or(Value::Null),
                            "discovery_source": row_obj.get("discovery_source").and_then(Value::as_str).unwrap_or_default(),
                            "linkage_source": row_obj.get("linkage_source").and_then(Value::as_str).unwrap_or_default(),
                            "linkage_status": row_obj.get("linkage_status").and_then(Value::as_str).unwrap_or_default(),
                        }))
                    })
                    .collect::<Vec<_>>();

                let summary = json!({
                    "total_covering_tests": covering_tests_rows.len(),
                    "cross_cutting": false,
                    "data_sources": ["static_source"],
                    "diagnostic_count": 0,
                });

                out.push(json!({
                    "artefact": artefact,
                    "covering_tests": covering_tests_rows,
                    "summary": summary,
                }));
            }

            Ok(StageResponse::json(Value::Array(out)))
        })
    }
}

fn build_core_test_links_query(
    repo_identity: &str,
    artefact_id: &str,
    min_confidence: Option<f64>,
    linkage_source: Option<&str>,
    limit: usize,
) -> String {
    let mut args = vec![format!("artefact_id:{}", quote_devql(artefact_id))];
    if let Some(min_confidence) = min_confidence {
        args.push(format!("min_confidence:{min_confidence}"));
    }
    if let Some(linkage_source) = linkage_source {
        args.push(format!("linkage_source:{}", quote_devql(linkage_source)));
    }

    format!(
        "repo({})->{}({})->limit({})",
        quote_devql(repo_identity),
        TEST_HARNESS_CORE_TEST_LINKS_STAGE_ID,
        args.join(","),
        limit.max(1),
    )
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

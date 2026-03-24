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
struct TestsStagePayload {
    #[serde(default)]
    input_rows: Vec<Value>,
    #[serde(default)]
    args: BTreeMap<String, String>,
}

pub struct TestsStageHandler(pub Option<Arc<Mutex<BitloopsTestHarnessRepository>>>);

impl StageHandler for TestsStageHandler {
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
            let repo_id = ctx.repo().repo_id.clone();

            let g = store
                .lock()
                .map_err(|e| anyhow!("test harness store lock poisoned: {e}"))?;

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

                let covering = g.load_stage_covering_tests(
                    &repo_id,
                    artefact_id,
                    min_confidence,
                    linkage_source,
                    limit,
                )?;

                let covering_tests_rows: Vec<Value> = covering
                    .into_iter()
                    .map(|rec| {
                        json!({
                            "test_id": rec.test_id,
                            "test_name": rec.test_name,
                            "suite_name": rec.suite_name,
                            "file_path": rec.file_path,
                            "confidence": rec.confidence,
                            "discovery_source": rec.discovery_source,
                            "linkage_source": rec.linkage_source,
                            "linkage_status": rec.linkage_status,
                        })
                    })
                    .collect();

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

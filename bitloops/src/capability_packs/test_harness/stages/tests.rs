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

fn execute_tests_stage<R: TestHarnessQueryRepository + ?Sized>(
    store: &R,
    request: StageRequest,
    ctx: &mut dyn CapabilityExecutionContext,
) -> anyhow::Result<StageResponse> {
    let payload: TestsStagePayload = request.parse_json()?;
    let limit = request.limit().unwrap_or(100).max(1);

    let min_confidence = payload
        .args
        .get("min_confidence")
        .map(|value| {
            value
                .parse::<f64>()
                .map(|parsed| parsed.clamp(0.0, 1.0))
                .map_err(|_| anyhow!("tests(min_confidence:...) must be a valid numeric value"))
        })
        .transpose()?;
    let linkage_source = payload.args.get("linkage_source").map(String::as_str);
    let repo_id = ctx.repo().repo_id.clone();

    // Upstream core owns the contract row shape here: artefact_id, symbol_id,
    // symbol_fqn, canonical_kind, path, start_line, end_line.
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
        let production_symbol_id = row_obj
            .get("symbol_id")
            .and_then(Value::as_str)
            .unwrap_or(artefact_id);

        let artefact = json!({
            "artefact_id": artefact_id,
            "name": row_obj.get("symbol_fqn").and_then(Value::as_str).unwrap_or(artefact_id),
            "kind": row_obj.get("canonical_kind").and_then(Value::as_str).unwrap_or("unknown"),
            "file_path": row_obj.get("path").and_then(Value::as_str).unwrap_or(""),
            "start_line": row_obj.get("start_line").and_then(Value::as_i64).unwrap_or(0),
            "end_line": row_obj.get("end_line").and_then(Value::as_i64).unwrap_or(0),
        });

        let covering = store.load_stage_covering_tests(
            &repo_id,
            production_symbol_id,
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
}

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

            let g = store
                .lock()
                .map_err(|e| anyhow!("test harness store lock poisoned: {e}"))?;
            execute_tests_stage(&*g, request, ctx)
        })
    }
}

#[cfg(test)]
mod guardrail_tests {
    use std::collections::BTreeMap;
    use std::path::Path;
    use std::sync::Mutex;

    use anyhow::Result;
    use serde_json::{Value, json};

    use super::execute_tests_stage;
    use crate::capability_packs::test_harness::storage::TestHarnessQueryRepository;
    use crate::host::capability_host::gateways::{CanonicalGraphGateway, RelationalGateway};
    use crate::host::capability_host::runtime_contexts::LocalCanonicalGraphGateway;
    use crate::host::capability_host::{CapabilityExecutionContext, StageRequest, StageResponse};
    use crate::host::devql::RepoIdentity;
    use crate::models::{StageCoveringTestRecord, TestHarnessCommitCounts};

    struct DummyExecCtx {
        repo: RepoIdentity,
        graph: LocalCanonicalGraphGateway,
    }

    impl CapabilityExecutionContext for DummyExecCtx {
        fn repo(&self) -> &RepoIdentity {
            &self.repo
        }

        fn repo_root(&self) -> &Path {
            Path::new(".")
        }

        fn graph(&self) -> &dyn CanonicalGraphGateway {
            &self.graph
        }

        fn host_relational(&self) -> &dyn RelationalGateway {
            panic!("test context does not provide host relational access")
        }
    }

    fn test_repo() -> RepoIdentity {
        RepoIdentity {
            provider: "local".to_string(),
            organization: "bitloops".to_string(),
            name: "bitloops-cli".to_string(),
            identity: "local/bitloops/bitloops-cli".to_string(),
            repo_id: "repo-1".to_string(),
        }
    }

    #[derive(Debug, Default)]
    struct FakeRepo {
        calls: Mutex<Vec<StageCall>>,
        responses: BTreeMap<String, Vec<StageCoveringTestRecord>>,
    }

    #[derive(Debug, Clone, PartialEq)]
    struct StageCall {
        repo_id: String,
        production_symbol_id: String,
        min_confidence: Option<f64>,
        linkage_source: Option<String>,
        limit: usize,
    }

    impl FakeRepo {
        fn with_response(
            mut self,
            production_symbol_id: &str,
            response: Vec<StageCoveringTestRecord>,
        ) -> Self {
            self.responses
                .insert(production_symbol_id.to_string(), response);
            self
        }

        fn calls(&self) -> Vec<StageCall> {
            self.calls.lock().expect("calls lock").clone()
        }
    }

    impl TestHarnessQueryRepository for FakeRepo {
        fn find_artefact(
            &self,
            _commit_sha: &str,
            _artefact_query: &str,
        ) -> Result<crate::models::QueriedArtefactRecord> {
            unreachable!("unused in tests stage guardrails")
        }

        fn list_artefacts(
            &self,
            _commit_sha: &str,
            _kind: Option<&str>,
        ) -> Result<Vec<crate::models::ListedArtefactRecord>> {
            unreachable!("unused in tests stage guardrails")
        }

        fn load_covering_tests(
            &self,
            _commit_sha: &str,
            _production_symbol_id: &str,
        ) -> Result<Vec<crate::models::CoveringTestRecord>> {
            unreachable!("unused in tests stage guardrails")
        }

        fn load_linked_fan_out_by_test(
            &self,
            _commit_sha: &str,
        ) -> Result<std::collections::HashMap<String, i64>> {
            unreachable!("unused in tests stage guardrails")
        }

        fn coverage_exists_for_commit(&self, _commit_sha: &str) -> Result<bool> {
            unreachable!("unused in tests stage guardrails")
        }

        fn load_coverage_pair_stats(
            &self,
            _commit_sha: &str,
            _test_symbol_id: &str,
            _production_symbol_id: &str,
        ) -> Result<crate::models::CoveragePairStats> {
            unreachable!("unused in tests stage guardrails")
        }

        fn load_latest_test_run(
            &self,
            _commit_sha: &str,
            _test_symbol_id: &str,
        ) -> Result<Option<crate::models::LatestTestRunRecord>> {
            unreachable!("unused in tests stage guardrails")
        }

        fn load_coverage_summary(
            &self,
            _commit_sha: &str,
            _production_symbol_id: &str,
        ) -> Result<Option<crate::models::CoverageSummaryRecord>> {
            unreachable!("unused in tests stage guardrails")
        }

        fn load_test_harness_commit_counts(
            &self,
            _commit_sha: &str,
        ) -> Result<TestHarnessCommitCounts> {
            unreachable!("unused in tests stage guardrails")
        }

        fn load_stage_covering_tests(
            &self,
            repo_id: &str,
            production_symbol_id: &str,
            min_confidence: Option<f64>,
            linkage_source: Option<&str>,
            limit: usize,
        ) -> Result<Vec<StageCoveringTestRecord>> {
            self.calls.lock().expect("calls lock").push(StageCall {
                repo_id: repo_id.to_string(),
                production_symbol_id: production_symbol_id.to_string(),
                min_confidence,
                linkage_source: linkage_source.map(str::to_string),
                limit,
            });

            Ok(self
                .responses
                .get(production_symbol_id)
                .cloned()
                .unwrap_or_default())
        }

        fn load_stage_line_coverage(
            &self,
            _repo_id: &str,
            _production_symbol_id: &str,
            _commit_sha: Option<&str>,
        ) -> Result<Vec<crate::models::StageLineCoverageRecord>> {
            unreachable!("unused in tests stage guardrails")
        }

        fn load_stage_branch_coverage(
            &self,
            _repo_id: &str,
            _production_symbol_id: &str,
            _commit_sha: Option<&str>,
        ) -> Result<Vec<crate::models::StageBranchCoverageRecord>> {
            unreachable!("unused in tests stage guardrails")
        }

        fn load_stage_coverage_metadata(
            &self,
            _repo_id: &str,
            _commit_sha: Option<&str>,
        ) -> Result<Option<crate::models::StageCoverageMetadataRecord>> {
            unreachable!("unused in tests stage guardrails")
        }
    }

    fn stage_row(
        artefact_id: Option<&str>,
        symbol_id: &str,
        symbol_fqn: &str,
        canonical_kind: &str,
        path: &str,
        start_line: i64,
        end_line: i64,
    ) -> Value {
        let mut row = json!({
            "symbol_id": symbol_id,
            "symbol_fqn": symbol_fqn,
            "canonical_kind": canonical_kind,
            "path": path,
            "start_line": start_line,
            "end_line": end_line,
        });
        if let Some(artefact_id) = artefact_id {
            row["artefact_id"] = json!(artefact_id);
        }
        row
    }

    async fn execute(repo: &FakeRepo, payload: Value) -> StageResponse {
        let mut ctx = DummyExecCtx {
            repo: test_repo(),
            graph: LocalCanonicalGraphGateway,
        };
        execute_tests_stage(repo, StageRequest::new(payload), &mut ctx).expect("stage execution")
    }

    #[tokio::test]
    async fn tests_stage_skips_rows_without_artefact_id() {
        let repo = FakeRepo::default().with_response(
            "symbol-a",
            vec![StageCoveringTestRecord {
                test_id: "test-1".into(),
                test_name: "covers_a".into(),
                suite_name: Some("suite".into()),
                file_path: "tests/a.rs".into(),
                confidence: 0.91,
                discovery_source: "static".into(),
                linkage_source: "coverage".into(),
                linkage_status: "linked".into(),
            }],
        );

        let resp = execute(
            &repo,
            json!({
                "input_rows": [
                    stage_row(None, "symbol-skip", "tests::skip", "test_case", "tests/skip.rs", 1, 2),
                    stage_row(Some("artefact-a"), "symbol-a", "tests::a", "test_case", "tests/a.rs", 10, 12)
                ],
                "limit": 10
            }),
        )
        .await;

        let rows = resp.payload.as_array().expect("array");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["artefact"]["artefact_id"], "artefact-a");
        assert_eq!(rows[0]["covering_tests"].as_array().unwrap().len(), 1);
        assert_eq!(repo.calls().len(), 1);
        assert_eq!(repo.calls()[0].production_symbol_id, "symbol-a");
    }

    #[tokio::test]
    async fn tests_stage_uses_input_rows_and_stage_queries_only() {
        let repo = FakeRepo::default()
            .with_response(
                "symbol-a",
                vec![StageCoveringTestRecord {
                    test_id: "test-a".into(),
                    test_name: "covers_a".into(),
                    suite_name: Some("suite-a".into()),
                    file_path: "tests/a.rs".into(),
                    confidence: 0.8,
                    discovery_source: "static".into(),
                    linkage_source: "coverage".into(),
                    linkage_status: "linked".into(),
                }],
            )
            .with_response(
                "symbol-b",
                vec![StageCoveringTestRecord {
                    test_id: "test-b".into(),
                    test_name: "covers_b".into(),
                    suite_name: Some("suite-b".into()),
                    file_path: "tests/b.rs".into(),
                    confidence: 0.82,
                    discovery_source: "static".into(),
                    linkage_source: "coverage".into(),
                    linkage_status: "linked".into(),
                }],
            );

        let resp = execute(
            &repo,
            json!({
                "input_rows": [
                    stage_row(Some("artefact-a"), "symbol-a", "tests::a", "test_case", "tests/a.rs", 10, 12),
                    stage_row(Some("artefact-b"), "symbol-b", "tests::b", "test_case", "tests/b.rs", 20, 22)
                ],
                "limit": 10
            }),
        )
        .await;

        let rows = resp.payload.as_array().expect("array");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["artefact"]["artefact_id"], "artefact-a");
        assert_eq!(rows[1]["artefact"]["artefact_id"], "artefact-b");
        assert_eq!(repo.calls().len(), 2);
        assert_eq!(repo.calls()[0].production_symbol_id, "symbol-a");
        assert_eq!(repo.calls()[1].production_symbol_id, "symbol-b");
    }

    #[tokio::test]
    async fn tests_stage_falls_back_to_artefact_id_when_symbol_id_is_missing() {
        let repo = FakeRepo::default().with_response(
            "artefact-a",
            vec![StageCoveringTestRecord {
                test_id: "test-a".into(),
                test_name: "covers_a".into(),
                suite_name: None,
                file_path: "tests/a.rs".into(),
                confidence: 0.8,
                discovery_source: "static".into(),
                linkage_source: "coverage".into(),
                linkage_status: "linked".into(),
            }],
        );

        let resp = execute(
            &repo,
            json!({
                "input_rows": [
                    {"artefact_id": "artefact-a", "symbol_fqn": "tests::a", "canonical_kind": "test_case", "path": "tests/a.rs", "start_line": 10, "end_line": 12},
                    "ignore-me"
                ],
                "limit": 10
            }),
        )
        .await;

        let rows = resp.payload.as_array().expect("array");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["artefact"]["artefact_id"], "artefact-a");
        assert_eq!(repo.calls().len(), 1);
        assert_eq!(repo.calls()[0].production_symbol_id, "artefact-a");
    }

    #[tokio::test]
    async fn tests_stage_forwards_min_confidence_and_linkage_source() {
        let repo = FakeRepo::default().with_response(
            "symbol-a",
            vec![StageCoveringTestRecord {
                test_id: "test-a".into(),
                test_name: "covers_a".into(),
                suite_name: None,
                file_path: "tests/a.rs".into(),
                confidence: 0.5,
                discovery_source: "static".into(),
                linkage_source: "coverage".into(),
                linkage_status: "linked".into(),
            }],
        );

        let _ = execute(
            &repo,
            json!({
                "input_rows": [
                    stage_row(Some("artefact-a"), "symbol-a", "tests::a", "test_case", "tests/a.rs", 10, 12)
                ],
                "limit": 7,
                "args": {
                    "min_confidence": "0.75",
                    "linkage_source": "coverage_ingest"
                }
            }),
        )
        .await;

        let call = repo.calls().pop().expect("call");
        assert_eq!(call.repo_id, "repo-1");
        assert_eq!(call.production_symbol_id, "symbol-a");
        assert_eq!(call.min_confidence, Some(0.75));
        assert_eq!(call.linkage_source.as_deref(), Some("coverage_ingest"));
        assert_eq!(call.limit, 7);
    }
}

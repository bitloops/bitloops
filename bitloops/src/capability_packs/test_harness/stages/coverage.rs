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

fn execute_coverage_stage<R: TestHarnessQueryRepository + ?Sized>(
    store: &R,
    request: StageRequest,
    ctx: &mut dyn CapabilityExecutionContext,
) -> anyhow::Result<StageResponse> {
    let payload: CoverageStagePayload = request.parse_json()?;
    let _ = payload.args;
    let repo_id = ctx.repo().repo_id.clone();
    let commit_sha = payload.query_context.resolved_commit_sha;

    let metadata = store.load_stage_coverage_metadata(&repo_id, commit_sha.as_deref())?;
    let coverage_source = metadata
        .as_ref()
        .map(|m| m.coverage_source.as_str())
        .unwrap_or("lcov")
        .to_string();
    let branch_truth = metadata.as_ref().map(|m| m.branch_truth).unwrap_or(0);

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
        let Some(production_symbol_id) = row_obj.get("symbol_id").and_then(Value::as_str) else {
            continue;
        };

        let artefact = json!({
            "artefact_id": artefact_id,
            "name": row_obj.get("symbol_fqn").and_then(Value::as_str).unwrap_or(artefact_id),
            "kind": row_obj.get("canonical_kind").and_then(Value::as_str).unwrap_or("unknown"),
            "file_path": row_obj.get("path").and_then(Value::as_str).unwrap_or(""),
            "start_line": row_obj.get("start_line").and_then(Value::as_i64).unwrap_or(0),
            "end_line": row_obj.get("end_line").and_then(Value::as_i64).unwrap_or(0),
        });

        let line_records = store.load_stage_line_coverage(
            &repo_id,
            production_symbol_id,
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

        let branch_records = store.load_stage_branch_coverage(
            &repo_id,
            production_symbol_id,
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
}

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

            let g = store
                .lock()
                .map_err(|e| anyhow!("test harness store lock poisoned: {e}"))?;
            execute_coverage_stage(&*g, request, ctx)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;
    use std::sync::Mutex;

    use anyhow::Result;
    use serde_json::{Value, json};

    use super::execute_coverage_stage;
    use crate::capability_packs::test_harness::storage::TestHarnessQueryRepository;
    use crate::host::capability_host::gateways::{CanonicalGraphGateway, RelationalGateway};
    use crate::host::capability_host::runtime_contexts::LocalCanonicalGraphGateway;
    use crate::host::capability_host::{CapabilityExecutionContext, StageRequest, StageResponse};
    use crate::host::devql::RepoIdentity;
    use crate::models::{
        StageBranchCoverageRecord, StageCoverageMetadataRecord, StageLineCoverageRecord,
        TestHarnessCommitCounts,
    };

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
        metadata_calls: Mutex<Vec<(String, Option<String>)>>,
        line_calls: Mutex<Vec<(String, String, Option<String>)>>,
        branch_calls: Mutex<Vec<(String, String, Option<String>)>>,
        metadata: Option<StageCoverageMetadataRecord>,
        line_responses: BTreeMap<String, Vec<StageLineCoverageRecord>>,
        branch_responses: BTreeMap<String, Vec<StageBranchCoverageRecord>>,
    }

    impl FakeRepo {
        fn with_metadata(mut self, metadata: Option<StageCoverageMetadataRecord>) -> Self {
            self.metadata = metadata;
            self
        }

        fn with_line_response(
            mut self,
            production_symbol_id: &str,
            response: Vec<StageLineCoverageRecord>,
        ) -> Self {
            self.line_responses
                .insert(production_symbol_id.to_string(), response);
            self
        }

        fn with_branch_response(
            mut self,
            production_symbol_id: &str,
            response: Vec<StageBranchCoverageRecord>,
        ) -> Self {
            self.branch_responses
                .insert(production_symbol_id.to_string(), response);
            self
        }

        fn metadata_calls(&self) -> Vec<(String, Option<String>)> {
            self.metadata_calls.lock().expect("metadata lock").clone()
        }

        fn line_calls(&self) -> Vec<(String, String, Option<String>)> {
            self.line_calls.lock().expect("line lock").clone()
        }

        fn branch_calls(&self) -> Vec<(String, String, Option<String>)> {
            self.branch_calls.lock().expect("branch lock").clone()
        }
    }

    impl TestHarnessQueryRepository for FakeRepo {
        fn load_covering_tests(
            &self,
            _commit_sha: &str,
            _production_symbol_id: &str,
        ) -> Result<Vec<crate::models::CoveringTestRecord>> {
            unreachable!("unused in coverage stage guardrails")
        }

        fn load_linked_fan_out_by_test(
            &self,
            _commit_sha: &str,
        ) -> Result<std::collections::HashMap<String, i64>> {
            unreachable!("unused in coverage stage guardrails")
        }

        fn coverage_exists_for_commit(&self, _commit_sha: &str) -> Result<bool> {
            unreachable!("unused in coverage stage guardrails")
        }

        fn load_coverage_pair_stats(
            &self,
            _commit_sha: &str,
            _test_symbol_id: &str,
            _production_symbol_id: &str,
        ) -> Result<crate::models::CoveragePairStats> {
            unreachable!("unused in coverage stage guardrails")
        }

        fn load_latest_test_run(
            &self,
            _commit_sha: &str,
            _test_symbol_id: &str,
        ) -> Result<Option<crate::models::LatestTestRunRecord>> {
            unreachable!("unused in coverage stage guardrails")
        }

        fn load_coverage_summary(
            &self,
            _commit_sha: &str,
            _production_symbol_id: &str,
        ) -> Result<Option<crate::models::CoverageSummaryRecord>> {
            unreachable!("unused in coverage stage guardrails")
        }

        fn load_test_harness_commit_counts(
            &self,
            _commit_sha: &str,
        ) -> Result<TestHarnessCommitCounts> {
            unreachable!("unused in coverage stage guardrails")
        }

        fn load_stage_covering_tests(
            &self,
            _repo_id: &str,
            _production_symbol_id: &str,
            _commit_sha: Option<&str>,
            _min_confidence: Option<f64>,
            _linkage_source: Option<&str>,
            _limit: usize,
        ) -> Result<Vec<crate::models::StageCoveringTestRecord>> {
            unreachable!("unused in coverage stage guardrails")
        }

        fn load_stage_line_coverage(
            &self,
            repo_id: &str,
            production_symbol_id: &str,
            commit_sha: Option<&str>,
        ) -> Result<Vec<StageLineCoverageRecord>> {
            self.line_calls.lock().expect("line lock").push((
                repo_id.to_string(),
                production_symbol_id.to_string(),
                commit_sha.map(str::to_string),
            ));

            Ok(self
                .line_responses
                .get(production_symbol_id)
                .cloned()
                .unwrap_or_default())
        }

        fn load_stage_branch_coverage(
            &self,
            repo_id: &str,
            production_symbol_id: &str,
            commit_sha: Option<&str>,
        ) -> Result<Vec<StageBranchCoverageRecord>> {
            self.branch_calls.lock().expect("branch lock").push((
                repo_id.to_string(),
                production_symbol_id.to_string(),
                commit_sha.map(str::to_string),
            ));

            Ok(self
                .branch_responses
                .get(production_symbol_id)
                .cloned()
                .unwrap_or_default())
        }

        fn load_stage_coverage_metadata(
            &self,
            repo_id: &str,
            commit_sha: Option<&str>,
        ) -> Result<Option<StageCoverageMetadataRecord>> {
            self.metadata_calls
                .lock()
                .expect("metadata lock")
                .push((repo_id.to_string(), commit_sha.map(str::to_string)));
            Ok(self.metadata.clone())
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
        execute_coverage_stage(repo, StageRequest::new(payload), &mut ctx).expect("stage execution")
    }

    #[tokio::test]
    async fn coverage_stage_uses_input_rows_and_stage_queries_only() {
        let repo = FakeRepo::default()
            .with_metadata(Some(StageCoverageMetadataRecord {
                coverage_source: "llvm-json".into(),
                branch_truth: 1,
            }))
            .with_line_response(
                "symbol-a",
                vec![
                    StageLineCoverageRecord {
                        line: 10,
                        covered: true,
                    },
                    StageLineCoverageRecord {
                        line: 11,
                        covered: false,
                    },
                ],
            )
            .with_line_response(
                "symbol-b",
                vec![StageLineCoverageRecord {
                    line: 20,
                    covered: true,
                }],
            )
            .with_branch_response(
                "symbol-a",
                vec![
                    StageBranchCoverageRecord {
                        line: 10,
                        branch_id: 1,
                        covered: true,
                        hit_count: 2,
                    },
                    StageBranchCoverageRecord {
                        line: 11,
                        branch_id: 2,
                        covered: false,
                        hit_count: 0,
                    },
                ],
            )
            .with_branch_response(
                "symbol-b",
                vec![StageBranchCoverageRecord {
                    line: 20,
                    branch_id: 3,
                    covered: true,
                    hit_count: 1,
                }],
            );

        let resp = execute(
            &repo,
            json!({
                "input_rows": [
                    stage_row(Some("artefact-a"), "symbol-a", "tests::a", "test_case", "tests/a.rs", 10, 12),
                    stage_row(Some("artefact-b"), "symbol-b", "tests::b", "test_case", "tests/b.rs", 20, 22)
                ]
            }),
        )
        .await;

        let rows = resp.payload.as_array().expect("array");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["artefact"]["artefact_id"], "artefact-a");
        assert_eq!(rows[1]["artefact"]["artefact_id"], "artefact-b");
        let coverage = rows[0]["coverage"].as_object().expect("coverage object");
        assert_eq!(coverage["coverage_source"], "llvm-json");
        assert_eq!(coverage["branch_data_available"], true);
        let branches = coverage["branches"].as_array().expect("branches array");
        assert_eq!(branches.len(), 2);
        assert_eq!(branches[0]["branch"], 1);
        assert_eq!(branches[1]["branch"], 2);
        assert_eq!(repo.metadata_calls().len(), 1);
        assert_eq!(repo.line_calls().len(), 2);
        assert_eq!(repo.branch_calls().len(), 2);
        assert_eq!(repo.line_calls()[0].1, "symbol-a");
        assert_eq!(repo.line_calls()[1].1, "symbol-b");
    }

    #[tokio::test]
    async fn coverage_stage_requires_symbol_id_in_input_rows() {
        let repo = FakeRepo::default()
            .with_metadata(Some(StageCoverageMetadataRecord {
                coverage_source: "llvm-json".into(),
                branch_truth: 1,
            }))
            .with_line_response(
                "artefact-a",
                vec![StageLineCoverageRecord {
                    line: 42,
                    covered: true,
                }],
            )
            .with_branch_response(
                "artefact-a",
                vec![StageBranchCoverageRecord {
                    line: 42,
                    branch_id: 7,
                    covered: true,
                    hit_count: 1,
                }],
            );

        let resp = execute(
            &repo,
            json!({
                "input_rows": [
                    {"artefact_id": "artefact-a", "symbol_fqn": "tests::a", "canonical_kind": "test_case", "path": "tests/a.rs", "start_line": 10, "end_line": 12},
                    123
                ]
            }),
        )
        .await;

        let rows = resp.payload.as_array().expect("array");
        assert!(rows.is_empty());
        assert_eq!(repo.metadata_calls().len(), 1);
        assert!(repo.line_calls().is_empty());
        assert!(repo.branch_calls().is_empty());
    }

    #[tokio::test]
    async fn coverage_stage_handles_missing_coverage_data() {
        let repo = FakeRepo::default();

        let resp = execute(
            &repo,
            json!({
                "input_rows": [
                    stage_row(Some("artefact-a"), "symbol-a", "tests::a", "test_case", "tests/a.rs", 10, 12)
                ]
            }),
        )
        .await;

        let rows = resp.payload.as_array().expect("array");
        assert_eq!(rows.len(), 1);
        let coverage = &rows[0]["coverage"];
        assert_eq!(coverage["coverage_source"], "lcov");
        assert_eq!(coverage["line_coverage_pct"], 0.0);
        assert_eq!(coverage["branch_coverage_pct"], 0.0);
        assert_eq!(coverage["line_data_available"], false);
        assert_eq!(coverage["branch_data_available"], false);
        assert_eq!(coverage["uncovered_lines"].as_array().unwrap().len(), 0);
        assert_eq!(coverage["branches"].as_array().unwrap().len(), 0);
        assert_eq!(rows[0]["summary"]["uncovered_line_count"], 0);
        assert_eq!(rows[0]["summary"]["uncovered_branch_count"], 0);
    }

    #[tokio::test]
    async fn coverage_stage_uses_resolved_commit_for_metadata() {
        let repo = FakeRepo::default()
            .with_metadata(Some(StageCoverageMetadataRecord {
                coverage_source: "llvm-json".into(),
                branch_truth: 1,
            }))
            .with_line_response(
                "symbol-a",
                vec![StageLineCoverageRecord {
                    line: 10,
                    covered: false,
                }],
            )
            .with_branch_response(
                "symbol-a",
                vec![StageBranchCoverageRecord {
                    line: 10,
                    branch_id: 1,
                    covered: false,
                    hit_count: 0,
                }],
            );

        let _ = execute(
            &repo,
            json!({
                "input_rows": [
                    stage_row(Some("artefact-a"), "symbol-a", "tests::a", "test_case", "tests/a.rs", 10, 12)
                ],
                "query_context": {
                    "resolved_commit_sha": "deadbeef"
                }
            }),
        )
        .await;

        assert_eq!(
            repo.metadata_calls(),
            vec![("repo-1".to_string(), Some("deadbeef".to_string()))]
        );
        assert_eq!(
            repo.line_calls(),
            vec![(
                "repo-1".to_string(),
                "symbol-a".to_string(),
                Some("deadbeef".to_string())
            )]
        );
        assert_eq!(
            repo.branch_calls(),
            vec![(
                "repo-1".to_string(),
                "symbol-a".to_string(),
                Some("deadbeef".to_string())
            )]
        );
    }
}

//! `test_harness_tests_summary`: commit-scoped row counts and coverage presence from the harness store.
//!
//! Distinct from the per-artefact `summary` object on the `tests()` stage (linkage counts only).

use std::sync::{Arc, Mutex};

use anyhow::Result;
use serde_json::{Value, json};

use crate::capability_packs::test_harness::storage::{
    BitloopsTestHarnessRepository, TestHarnessQueryRepository,
};
use crate::capability_packs::test_harness::types::{
    TEST_HARNESS_CAPABILITY_ID, TEST_HARNESS_TESTS_SUMMARY_STAGE_ID,
    test_harness_commit_sha_required_response,
    test_harness_relational_store_unavailable_stage_response,
};
use crate::host::capability_host::{
    BoxFuture, CapabilityExecutionContext, StageHandler, StageRequest, StageResponse,
};

fn build_test_harness_commit_snapshot(
    repo: &impl TestHarnessQueryRepository,
    commit_sha: &str,
) -> Result<(Value, String)> {
    let counts = repo.load_test_harness_commit_counts(commit_sha)?;
    let coverage_present = repo.coverage_exists_for_commit(commit_sha)?;

    let human = format!(
        "test harness snapshot for commit {commit_sha}: test_artefacts={}, test_artefact_edges={}, classifications={}, coverage_captures={}, coverage_hits={}, coverage_indexed={}",
        counts.test_artefacts,
        counts.test_artefact_edges,
        counts.test_classifications,
        counts.coverage_captures,
        counts.coverage_hits,
        coverage_present
    );

    let value = json!({
        "capability": TEST_HARNESS_CAPABILITY_ID,
        "stage": TEST_HARNESS_TESTS_SUMMARY_STAGE_ID,
        "status": "ok",
        "commit_sha": commit_sha,
        "counts": {
            "test_artefacts": counts.test_artefacts,
            "test_artefact_edges": counts.test_artefact_edges,
            "test_classifications": counts.test_classifications,
            "coverage_captures": counts.coverage_captures,
            "coverage_hits": counts.coverage_hits,
        },
        "coverage_present": coverage_present,
    });

    Ok((value, human))
}

pub struct TestsSummaryStageHandler(pub Option<Arc<Mutex<BitloopsTestHarnessRepository>>>);

impl StageHandler for TestsSummaryStageHandler {
    fn execute<'a>(
        &'a self,
        request: StageRequest,
        _ctx: &'a mut dyn CapabilityExecutionContext,
    ) -> BoxFuture<'a, anyhow::Result<StageResponse>> {
        let store = self.0.clone();
        Box::pin(async move {
            let Some(store) = store else {
                return Ok(test_harness_relational_store_unavailable_stage_response());
            };

            let commit_sha = request
                .payload
                .get("query_context")
                .and_then(|qc| qc.get("resolved_commit_sha"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty());

            let Some(commit_sha) = commit_sha else {
                return Ok(test_harness_commit_sha_required_response(request.limit()));
            };

            let g = store
                .lock()
                .map_err(|e| anyhow::anyhow!("test harness store lock poisoned: {e}"))?;
            let (payload, human) = build_test_harness_commit_snapshot(&*g, commit_sha)?;
            Ok(StageResponse::new(payload, human))
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    use serde_json::json;
    use tempfile::TempDir;

    use super::TestsSummaryStageHandler;
    use crate::capability_packs::test_harness::storage::{
        BitloopsTestHarnessRepository, SqliteTestHarnessRepository, init_test_domain_database,
    };
    use crate::capability_packs::test_harness::types::TEST_HARNESS_TESTS_SUMMARY_STAGE_ID;
    use crate::host::capability_host::gateways::{CanonicalGraphGateway, RelationalGateway};
    use crate::host::capability_host::runtime_contexts::LocalCanonicalGraphGateway;
    use crate::host::capability_host::{CapabilityExecutionContext, StageHandler, StageRequest};
    use crate::host::devql::RepoIdentity;
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

    #[tokio::test]
    async fn summary_stage_without_store_reports_unavailable() {
        let handler = TestsSummaryStageHandler(None);
        let mut ctx = DummyExecCtx {
            repo: test_repo(),
            graph: LocalCanonicalGraphGateway,
        };
        let req = StageRequest::new(json!({
            "limit": 10,
            "query_context": { "resolved_commit_sha": "abc123" }
        }));
        let resp = handler.execute(req, &mut ctx).await.unwrap();
        assert_eq!(
            resp.payload["reason"],
            "test_harness_relational_store_unavailable"
        );
        assert_eq!(resp.payload["stage"], TEST_HARNESS_TESTS_SUMMARY_STAGE_ID);
    }

    #[tokio::test]
    async fn summary_stage_requires_resolved_commit() {
        let temp = TempDir::new().expect("tempdir");
        let db_path = temp.path().join("harness.db");
        init_test_domain_database(&db_path).expect("init");
        let repo = BitloopsTestHarnessRepository::Sqlite(
            SqliteTestHarnessRepository::open_existing(&db_path).expect("open"),
        );
        let handler = TestsSummaryStageHandler(Some(Arc::new(Mutex::new(repo))));
        let mut ctx = DummyExecCtx {
            repo: test_repo(),
            graph: LocalCanonicalGraphGateway,
        };
        let req = StageRequest::new(json!({
            "limit": 5,
            "query_context": { "resolved_commit_sha": null }
        }));
        let resp = handler.execute(req, &mut ctx).await.unwrap();
        assert_eq!(resp.payload["reason"], "test_harness_commit_sha_required");
    }

    #[tokio::test]
    async fn summary_stage_returns_counts_for_resolved_commit() {
        let temp = TempDir::new().expect("tempdir");
        let db_path = temp.path().join("harness.db");
        init_test_domain_database(&db_path).expect("init");
        let repo = BitloopsTestHarnessRepository::Sqlite(
            SqliteTestHarnessRepository::open_existing(&db_path).expect("open"),
        );
        let handler = TestsSummaryStageHandler(Some(Arc::new(Mutex::new(repo))));
        let mut ctx = DummyExecCtx {
            repo: test_repo(),
            graph: LocalCanonicalGraphGateway,
        };
        let req = StageRequest::new(json!({
            "limit": 10,
            "query_context": { "resolved_commit_sha": "deadbeef" }
        }));
        let resp = handler.execute(req, &mut ctx).await.unwrap();
        assert_eq!(resp.payload["status"], "ok");
        assert_eq!(resp.payload["commit_sha"], "deadbeef");
        assert_eq!(resp.payload["stage"], TEST_HARNESS_TESTS_SUMMARY_STAGE_ID);
        let counts = resp.payload["counts"].as_object().expect("counts object");
        assert_eq!(counts["test_artefacts"], 0);
        assert_eq!(counts["test_artefact_edges"], 0);
        assert!(resp.human_output.contains("deadbeef"));
    }
}

use serde_json::{Value, json};

use crate::host::capability_host::{CapabilityConfigView, StageResponse};

pub const TEST_HARNESS_CAPABILITY_ID: &str = "test_harness";
pub const TEST_HARNESS_CURRENT_STATE_CONSUMER_ID: &str = "test_harness.current_state";
pub const TEST_HARNESS_TESTS_STAGE_ID: &str = "tests";
pub const TEST_HARNESS_TESTS_SUMMARY_STAGE_ID: &str = "test_harness_tests_summary";
pub const TEST_HARNESS_COVERAGE_STAGE_ID: &str = "coverage";
pub const TEST_HARNESS_LINKAGE_INGESTER_ID: &str = "test_harness.linkage";
pub const TEST_HARNESS_COVERAGE_INGESTER_ID: &str = "test_harness.coverage";
pub const TEST_HARNESS_CLASSIFICATION_INGESTER_ID: &str = "test_harness.classification";
pub const TEST_HARNESS_TESTS_EXPAND_HINT_INTENT: &str =
    "Inspect concrete covering tests for selected artefacts";
pub const TEST_HARNESS_TESTS_EXPAND_HINT_TEMPLATE: &str = "bitloops devql query '{ selectArtefacts(by: { symbolFqn: \"<symbol-fqn>\" }) { tests { summary items(first: 20) { coveringTests { testName suiteName filePath startLine endLine } } } } }'";

pub fn test_harness_tests_expand_hint_json() -> Value {
    json!({
        "intent": TEST_HARNESS_TESTS_EXPAND_HINT_INTENT,
        "template": TEST_HARNESS_TESTS_EXPAND_HINT_TEMPLATE,
    })
}

pub fn resolve_test_harness_config(view: &CapabilityConfigView) -> Option<&Value> {
    view.scoped()
        .or_else(|| view.root().get(TEST_HARNESS_CAPABILITY_ID))
        .or_else(|| {
            view.root().as_object().and_then(|root| {
                if root.contains_key("dependencies")
                    || root.contains_key("coverage")
                    || root.contains_key("thresholds")
                {
                    Some(view.root())
                } else {
                    None
                }
            })
        })
}

pub fn test_harness_relational_store_unavailable_stage_response() -> StageResponse {
    StageResponse::new(
        json!({
            "capability": TEST_HARNESS_CAPABILITY_ID,
            "stage": TEST_HARNESS_TESTS_SUMMARY_STAGE_ID,
            "status": "failed",
            "reason": "test_harness_relational_store_unavailable",
        }),
        "test harness relational store is not available; configure stores.relational, create the database, and ensure the daemon is running (`bitloops start`).",
    )
}

pub fn test_harness_commit_sha_required_response(limit: Option<usize>) -> StageResponse {
    StageResponse::new(
        json!({
            "capability": TEST_HARNESS_CAPABILITY_ID,
            "stage": TEST_HARNESS_TESTS_SUMMARY_STAGE_ID,
            "status": "failed",
            "reason": "test_harness_commit_sha_required",
            "limit": limit,
        }),
        "test_harness_tests_summary requires a resolved commit (use asOf(ref:...) or asOf(commit:...) in the DevQL query).",
    )
}

use serde_json::{Value, json};

use crate::engine::devql::capability_host::{CapabilityConfigView, StageResponse};

pub const TEST_HARNESS_CAPABILITY_ID: &str = "test_harness";
pub const TEST_HARNESS_TESTS_STAGE_ID: &str = "tests";
pub const TEST_HARNESS_TESTS_STAGE_ALIAS_ID: &str = "test_harness_tests";
pub const TEST_HARNESS_TESTS_SUMMARY_STAGE_ID: &str = "test_harness_tests_summary";
pub const TEST_HARNESS_COVERAGE_STAGE_ID: &str = "coverage";
pub const TEST_HARNESS_COVERAGE_STAGE_ALIAS_ID: &str = "test_harness_coverage";
pub const TEST_HARNESS_CORE_TEST_LINKS_STAGE_ID: &str = "__core_test_links";
pub const TEST_HARNESS_CORE_LINE_COVERAGE_STAGE_ID: &str = "__core_line_coverage";
pub const TEST_HARNESS_CORE_BRANCH_COVERAGE_STAGE_ID: &str = "__core_branch_coverage";
pub const TEST_HARNESS_CORE_COVERAGE_METADATA_STAGE_ID: &str = "__core_coverage_metadata";
pub const TEST_HARNESS_LINKAGE_INGESTER_ID: &str = "test_harness.linkage";
pub const TEST_HARNESS_COVERAGE_INGESTER_ID: &str = "test_harness.coverage";
pub const TEST_HARNESS_CLASSIFICATION_INGESTER_ID: &str = "test_harness.classification";
pub const TEST_HARNESS_SUMMARIES_INGESTER_ID: &str = "test_harness.summaries";
pub const TEST_HARNESS_DEPENDENCY_GATED_REASON: &str = "Test Harness capability-pack scaffold is registered, but runtime behaviour is dependency-gated until coverage adapters, test-discovery adapters, and language-aware test discovery are integrated.";

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

pub fn dependency_gated_stage_response(
    stage_name: &'static str,
    limit: Option<usize>,
) -> StageResponse {
    StageResponse::json(json!({
        "capability": TEST_HARNESS_CAPABILITY_ID,
        "stage": stage_name,
        "status": "dependency_gated",
        "limit": limit,
        "rows": [],
        "reason": TEST_HARNESS_DEPENDENCY_GATED_REASON,
    }))
}

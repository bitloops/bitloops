use async_graphql::{Interface, SimpleObject};
use serde::Deserialize;

use crate::capability_packs::test_harness::types::{
    TEST_HARNESS_TESTS_EXPAND_HINT_INTENT, TEST_HARNESS_TESTS_EXPAND_HINT_TEMPLATE,
};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, SimpleObject)]
pub struct TestHarnessArtefactRef {
    pub artefact_id: String,
    pub name: String,
    pub kind: String,
    pub file_path: String,
    pub start_line: i32,
    pub end_line: i32,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct TestHarnessLastRun {
    pub status: String,
    pub duration_ms: Option<i64>,
    pub commit_sha: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct TestHarnessCoveringTest {
    pub test_id: String,
    pub test_name: String,
    pub suite_name: Option<String>,
    pub file_path: String,
    pub start_line: i32,
    pub end_line: i32,
    pub confidence: f64,
    pub discovery_source: String,
    pub linkage_source: String,
    pub linkage_status: String,
    pub classification: Option<String>,
    pub classification_source: Option<String>,
    pub fan_out: Option<i32>,
    pub last_run: Option<TestHarnessLastRun>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, SimpleObject)]
pub struct TestHarnessTestsExpandHint {
    pub intent: String,
    pub template: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Interface)]
#[graphql(field(name = "intent", ty = "String"))]
#[graphql(field(name = "template", ty = "String"))]
pub enum ExpandHint {
    TestHarnessTestsExpandHint(TestHarnessTestsExpandHint),
}

impl Default for TestHarnessTestsExpandHint {
    fn default() -> Self {
        Self {
            intent: TEST_HARNESS_TESTS_EXPAND_HINT_INTENT.to_string(),
            template: TEST_HARNESS_TESTS_EXPAND_HINT_TEMPLATE.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, SimpleObject)]
pub struct TestHarnessTestsSummary {
    pub total_covering_tests: i32,
    pub cross_cutting: bool,
    pub data_sources: Vec<String>,
    pub diagnostic_count: i32,
    #[serde(default)]
    pub expand_hint: TestHarnessTestsExpandHint,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct TestHarnessTestsResult {
    pub artefact: TestHarnessArtefactRef,
    pub covering_tests: Vec<TestHarnessCoveringTest>,
    pub summary: TestHarnessTestsSummary,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, SimpleObject)]
pub struct TestHarnessCoverageBranch {
    pub line: i32,
    pub block: i32,
    pub branch: i32,
    pub covered: bool,
    pub hit_count: i32,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct TestHarnessCoverage {
    pub coverage_source: String,
    pub line_coverage_pct: f64,
    pub branch_coverage_pct: f64,
    pub line_data_available: bool,
    pub branch_data_available: bool,
    pub uncovered_lines: Vec<i32>,
    pub branches: Vec<TestHarnessCoverageBranch>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, SimpleObject)]
pub struct TestHarnessCoverageSummary {
    pub uncovered_line_count: i32,
    pub uncovered_branch_count: i32,
    pub diagnostic_count: i32,
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct TestHarnessCoverageResult {
    pub artefact: TestHarnessArtefactRef,
    pub coverage: TestHarnessCoverage,
    pub summary: TestHarnessCoverageSummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, SimpleObject)]
pub struct TestHarnessCommitCounts {
    pub test_artefacts: i32,
    pub test_artefact_edges: i32,
    pub test_classifications: i32,
    pub coverage_captures: i32,
    pub coverage_hits: i32,
}

#[derive(Debug, Clone, PartialEq, Deserialize, SimpleObject)]
pub struct TestHarnessCommitSummary {
    pub capability: String,
    pub stage: String,
    pub status: String,
    pub commit_sha: String,
    pub counts: TestHarnessCommitCounts,
    pub coverage_present: bool,
}

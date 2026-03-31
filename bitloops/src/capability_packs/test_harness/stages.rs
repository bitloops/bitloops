mod coverage;
mod tests;
mod tests_summary;

use crate::host::capability_host::StageRegistration;

use super::types::{
    TEST_HARNESS_COVERAGE_STAGE_ALIAS_ID, TEST_HARNESS_COVERAGE_STAGE_ID,
    TEST_HARNESS_TESTS_STAGE_ALIAS_ID, TEST_HARNESS_TESTS_STAGE_ID,
    TEST_HARNESS_TESTS_SUMMARY_STAGE_ID,
};
pub use coverage::CoverageStageHandler;
pub use tests::TestsStageHandler;
pub use tests_summary::TestsSummaryStageHandler;

pub fn build_tests_stage() -> StageRegistration {
    StageRegistration::new(
        "test_harness",
        TEST_HARNESS_TESTS_STAGE_ID,
        std::sync::Arc::new(TestsStageHandler),
    )
}

pub fn build_tests_summary_stage() -> StageRegistration {
    StageRegistration::new(
        "test_harness",
        TEST_HARNESS_TESTS_SUMMARY_STAGE_ID,
        std::sync::Arc::new(TestsSummaryStageHandler),
    )
}

pub fn build_coverage_stage() -> StageRegistration {
    StageRegistration::new(
        "test_harness",
        TEST_HARNESS_COVERAGE_STAGE_ID,
        std::sync::Arc::new(CoverageStageHandler),
    )
}

pub fn build_tests_stage_alias() -> StageRegistration {
    StageRegistration::new(
        "test_harness",
        TEST_HARNESS_TESTS_STAGE_ALIAS_ID,
        std::sync::Arc::new(TestsStageHandler),
    )
}

pub fn build_coverage_stage_alias() -> StageRegistration {
    StageRegistration::new(
        "test_harness",
        TEST_HARNESS_COVERAGE_STAGE_ALIAS_ID,
        std::sync::Arc::new(CoverageStageHandler),
    )
}

mod coverage;
mod tests;
mod tests_summary;

use crate::engine::devql::capability_host::StageRegistration;

pub use coverage::CoverageStageHandler;
pub use tests::TestsStageHandler;
pub use tests_summary::TestsSummaryStageHandler;

pub fn build_tests_stage() -> StageRegistration {
    StageRegistration::new(
        "test_harness",
        "tests",
        std::sync::Arc::new(TestsStageHandler),
    )
}

pub fn build_tests_summary_stage() -> StageRegistration {
    StageRegistration::new(
        "test_harness",
        "tests.summary",
        std::sync::Arc::new(TestsSummaryStageHandler),
    )
}

pub fn build_coverage_stage() -> StageRegistration {
    StageRegistration::new(
        "test_harness",
        "coverage",
        std::sync::Arc::new(CoverageStageHandler),
    )
}

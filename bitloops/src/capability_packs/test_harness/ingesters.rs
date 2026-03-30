mod classification;
mod coverage;
mod linkage;

use crate::host::capability_host::IngesterRegistration;

use super::types::{
    TEST_HARNESS_CLASSIFICATION_INGESTER_ID, TEST_HARNESS_COVERAGE_INGESTER_ID,
    TEST_HARNESS_LINKAGE_INGESTER_ID,
};

pub use classification::ClassificationIngester;
pub use coverage::CoverageIngestIngester;
pub use linkage::LinkageIngester;

pub fn build_linkage_ingester() -> IngesterRegistration {
    IngesterRegistration::new(
        "test_harness",
        TEST_HARNESS_LINKAGE_INGESTER_ID,
        std::sync::Arc::new(LinkageIngester),
    )
}

pub fn build_coverage_ingester() -> IngesterRegistration {
    IngesterRegistration::new(
        "test_harness",
        TEST_HARNESS_COVERAGE_INGESTER_ID,
        std::sync::Arc::new(CoverageIngestIngester),
    )
}

pub fn build_classification_ingester() -> IngesterRegistration {
    IngesterRegistration::new(
        "test_harness",
        TEST_HARNESS_CLASSIFICATION_INGESTER_ID,
        std::sync::Arc::new(ClassificationIngester),
    )
}

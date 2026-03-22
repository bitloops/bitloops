mod classification;
mod coverage;
mod linkage;
mod summaries;

use std::sync::{Arc, Mutex};

use crate::host::devql::capability_host::IngesterRegistration;
use crate::host::test_harness::BitloopsTestHarnessRepository;

use super::types::{
    TEST_HARNESS_CLASSIFICATION_INGESTER_ID, TEST_HARNESS_COVERAGE_INGESTER_ID,
    TEST_HARNESS_LINKAGE_INGESTER_ID, TEST_HARNESS_SUMMARIES_INGESTER_ID,
};

pub use classification::ClassificationIngester;
pub use coverage::CoverageIngestIngester;
pub use linkage::LinkageIngester;
pub use summaries::SummariesIngester;

pub fn build_linkage_ingester(
    store: Option<Arc<Mutex<BitloopsTestHarnessRepository>>>,
) -> IngesterRegistration {
    IngesterRegistration::new(
        "test_harness",
        TEST_HARNESS_LINKAGE_INGESTER_ID,
        std::sync::Arc::new(LinkageIngester(store)),
    )
}

pub fn build_coverage_ingester(
    store: Option<Arc<Mutex<BitloopsTestHarnessRepository>>>,
) -> IngesterRegistration {
    IngesterRegistration::new(
        "test_harness",
        TEST_HARNESS_COVERAGE_INGESTER_ID,
        std::sync::Arc::new(CoverageIngestIngester(store)),
    )
}

pub fn build_classification_ingester(
    store: Option<Arc<Mutex<BitloopsTestHarnessRepository>>>,
) -> IngesterRegistration {
    IngesterRegistration::new(
        "test_harness",
        TEST_HARNESS_CLASSIFICATION_INGESTER_ID,
        std::sync::Arc::new(ClassificationIngester(store)),
    )
}

pub fn build_summaries_ingester(
    store: Option<Arc<Mutex<BitloopsTestHarnessRepository>>>,
) -> IngesterRegistration {
    IngesterRegistration::new(
        "test_harness",
        TEST_HARNESS_SUMMARIES_INGESTER_ID,
        std::sync::Arc::new(SummariesIngester(store)),
    )
}

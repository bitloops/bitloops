mod classification;
mod coverage;
mod linkage;
mod summaries;

use crate::engine::devql::capability_host::IngesterRegistration;

pub use classification::ClassificationIngester;
pub use coverage::CoverageIngestIngester;
pub use linkage::LinkageIngester;
pub use summaries::SummariesIngester;

pub fn build_linkage_ingester() -> IngesterRegistration {
    IngesterRegistration::new(
        "test_harness",
        "test_harness.linkage",
        std::sync::Arc::new(LinkageIngester),
    )
}

pub fn build_coverage_ingester() -> IngesterRegistration {
    IngesterRegistration::new(
        "test_harness",
        "test_harness.coverage",
        std::sync::Arc::new(CoverageIngestIngester),
    )
}

pub fn build_classification_ingester() -> IngesterRegistration {
    IngesterRegistration::new(
        "test_harness",
        "test_harness.classification",
        std::sync::Arc::new(ClassificationIngester),
    )
}

pub fn build_summaries_ingester() -> IngesterRegistration {
    IngesterRegistration::new(
        "test_harness",
        "test_harness.summaries",
        std::sync::Arc::new(SummariesIngester),
    )
}

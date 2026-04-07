mod summary;

use crate::host::capability_host::StageRegistration;

use super::types::{SEMANTIC_CLONES_CAPABILITY_ID, SEMANTIC_CLONES_SUMMARY_STAGE_ID};
pub use summary::CloneSummaryStageHandler;

pub fn build_summary_stage() -> StageRegistration {
    StageRegistration::new(
        SEMANTIC_CLONES_CAPABILITY_ID,
        SEMANTIC_CLONES_SUMMARY_STAGE_ID,
        std::sync::Arc::new(CloneSummaryStageHandler),
    )
}

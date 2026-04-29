mod architecture;
mod arcs;
mod boundaries;
mod file_detail;
mod phase4_support;
mod violations;
mod world;

use crate::host::capability_host::StageRegistration;

use super::types::{
    CODECITY_ARCHITECTURE_STAGE_ID, CODECITY_ARCS_STAGE_ID, CODECITY_BOUNDARIES_STAGE_ID,
    CODECITY_CAPABILITY_ID, CODECITY_FILE_DETAIL_STAGE_ID, CODECITY_VIOLATIONS_STAGE_ID,
    CODECITY_WORLD_STAGE_ID,
};
pub use architecture::CodeCityArchitectureStageHandler;
pub use arcs::CodeCityArcsStageHandler;
pub use boundaries::CodeCityBoundariesStageHandler;
pub use file_detail::CodeCityFileDetailStageHandler;
pub use violations::CodeCityViolationsStageHandler;
pub use world::CodeCityWorldStageHandler;

pub fn build_codecity_world_stage() -> StageRegistration {
    StageRegistration::new(
        CODECITY_CAPABILITY_ID,
        CODECITY_WORLD_STAGE_ID,
        std::sync::Arc::new(CodeCityWorldStageHandler),
    )
}

pub fn build_codecity_boundaries_stage() -> StageRegistration {
    StageRegistration::new(
        CODECITY_CAPABILITY_ID,
        CODECITY_BOUNDARIES_STAGE_ID,
        std::sync::Arc::new(CodeCityBoundariesStageHandler),
    )
}

pub fn build_codecity_architecture_stage() -> StageRegistration {
    StageRegistration::new(
        CODECITY_CAPABILITY_ID,
        CODECITY_ARCHITECTURE_STAGE_ID,
        std::sync::Arc::new(CodeCityArchitectureStageHandler),
    )
}

pub fn build_codecity_violations_stage() -> StageRegistration {
    StageRegistration::new(
        CODECITY_CAPABILITY_ID,
        CODECITY_VIOLATIONS_STAGE_ID,
        std::sync::Arc::new(CodeCityViolationsStageHandler),
    )
}

pub fn build_codecity_file_detail_stage() -> StageRegistration {
    StageRegistration::new(
        CODECITY_CAPABILITY_ID,
        CODECITY_FILE_DETAIL_STAGE_ID,
        std::sync::Arc::new(CodeCityFileDetailStageHandler),
    )
}

pub fn build_codecity_arcs_stage() -> StageRegistration {
    StageRegistration::new(
        CODECITY_CAPABILITY_ID,
        CODECITY_ARCS_STAGE_ID,
        std::sync::Arc::new(CodeCityArcsStageHandler),
    )
}

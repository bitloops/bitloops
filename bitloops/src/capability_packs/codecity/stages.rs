mod architecture;
mod boundaries;
mod world;

use crate::host::capability_host::StageRegistration;

use super::types::{
    CODECITY_ARCHITECTURE_STAGE_ID, CODECITY_BOUNDARIES_STAGE_ID, CODECITY_CAPABILITY_ID,
    CODECITY_WORLD_STAGE_ID,
};
pub use architecture::CodeCityArchitectureStageHandler;
pub use boundaries::CodeCityBoundariesStageHandler;
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

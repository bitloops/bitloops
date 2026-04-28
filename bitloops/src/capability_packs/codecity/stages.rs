mod world;

use crate::host::capability_host::StageRegistration;

use super::types::{CODECITY_CAPABILITY_ID, CODECITY_WORLD_STAGE_ID};
pub use world::CodeCityWorldStageHandler;

pub fn build_codecity_world_stage() -> StageRegistration {
    StageRegistration::new(
        CODECITY_CAPABILITY_ID,
        CODECITY_WORLD_STAGE_ID,
        std::sync::Arc::new(CodeCityWorldStageHandler),
    )
}

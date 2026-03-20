use crate::engine::devql::capability_host::{
    BoxFuture, CapabilityExecutionContext, StageHandler, StageRequest, StageResponse,
};

use super::super::types::{TEST_HARNESS_COVERAGE_STAGE_ID, dependency_gated_stage_response};

pub struct CoverageStageHandler;

impl StageHandler for CoverageStageHandler {
    fn execute<'a>(
        &'a self,
        request: StageRequest,
        _ctx: &'a mut dyn CapabilityExecutionContext,
    ) -> BoxFuture<'a, anyhow::Result<StageResponse>> {
        Box::pin(async move {
            Ok(dependency_gated_stage_response(
                TEST_HARNESS_COVERAGE_STAGE_ID,
                request.limit(),
            ))
        })
    }
}

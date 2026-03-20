use crate::engine::devql::capability_host::{
    BoxFuture, CapabilityExecutionContext, StageHandler, StageRequest, StageResponse,
};

use super::super::types::{TEST_HARNESS_TESTS_STAGE_ID, dependency_gated_stage_response};

pub struct TestsStageHandler;

impl StageHandler for TestsStageHandler {
    fn execute<'a>(
        &'a self,
        request: StageRequest,
        _ctx: &'a mut dyn CapabilityExecutionContext,
    ) -> BoxFuture<'a, anyhow::Result<StageResponse>> {
        Box::pin(async move {
            Ok(dependency_gated_stage_response(
                TEST_HARNESS_TESTS_STAGE_ID,
                request.limit(),
            ))
        })
    }
}

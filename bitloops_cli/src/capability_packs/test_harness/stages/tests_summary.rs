use crate::host::devql::capability_host::{
    BoxFuture, CapabilityExecutionContext, StageHandler, StageRequest, StageResponse,
};

use super::super::types::{TEST_HARNESS_TESTS_SUMMARY_STAGE_ID, dependency_gated_stage_response};

pub struct TestsSummaryStageHandler;

impl StageHandler for TestsSummaryStageHandler {
    fn execute<'a>(
        &'a self,
        request: StageRequest,
        _ctx: &'a mut dyn CapabilityExecutionContext,
    ) -> BoxFuture<'a, anyhow::Result<StageResponse>> {
        Box::pin(async move {
            Ok(dependency_gated_stage_response(
                TEST_HARNESS_TESTS_SUMMARY_STAGE_ID,
                request.limit(),
            ))
        })
    }
}

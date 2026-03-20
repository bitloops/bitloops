use crate::engine::devql::capability_host::{
    BoxFuture, CapabilityExecutionContext, StageHandler, StageRequest, StageResponse,
};

use super::super::types::dependency_gated_stage_response;

pub struct TestsSummaryStageHandler;

impl StageHandler for TestsSummaryStageHandler {
    fn execute<'a>(
        &'a self,
        request: StageRequest,
        _ctx: &'a mut dyn CapabilityExecutionContext,
    ) -> BoxFuture<'a, anyhow::Result<StageResponse>> {
        Box::pin(async move {
            Ok(dependency_gated_stage_response(
                "tests.summary",
                request.limit(),
            ))
        })
    }
}

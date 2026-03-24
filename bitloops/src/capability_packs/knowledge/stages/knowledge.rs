use std::sync::Arc;

use crate::host::capability_host::{
    KnowledgeExecutionContext, KnowledgeStageHandler, StageRequest, StageResponse,
};

use super::super::services::KnowledgeServices;

pub struct KnowledgeStageHandlerImpl {
    services: Arc<KnowledgeServices>,
}

impl KnowledgeStageHandlerImpl {
    pub fn new(services: Arc<KnowledgeServices>) -> Self {
        Self { services }
    }
}

impl KnowledgeStageHandler for KnowledgeStageHandlerImpl {
    fn execute<'a>(
        &'a self,
        request: StageRequest,
        ctx: &'a mut dyn KnowledgeExecutionContext,
    ) -> crate::host::capability_host::BoxFuture<'a, anyhow::Result<StageResponse>> {
        Box::pin(async move {
            let repo = ctx.repo().clone();
            let rows = self
                .services
                .retrieval
                .list_repository_knowledge(&repo, &request, ctx)?;
            Ok(StageResponse::json(serde_json::Value::Array(rows)))
        })
    }
}

use std::sync::Arc;

use crate::engine::devql::capability_host::{
    KnowledgeExecutionContext, KnowledgeStage, StageRequest, StageResponse,
};

use super::super::services::KnowledgeServices;

pub struct KnowledgeStageHandler {
    services: Arc<KnowledgeServices>,
}

impl KnowledgeStageHandler {
    pub fn new(services: Arc<KnowledgeServices>) -> Self {
        Self { services }
    }
}

impl KnowledgeStage for KnowledgeStageHandler {
    fn execute<'a>(
        &'a self,
        request: StageRequest,
        ctx: &'a mut dyn KnowledgeExecutionContext,
    ) -> crate::engine::devql::capability_host::BoxFuture<'a, anyhow::Result<StageResponse>> {
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

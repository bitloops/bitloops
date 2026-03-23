use std::sync::Arc;

use anyhow::Result;
use serde::Deserialize;
use serde_json::json;

use crate::host::capability_host::{
    CapabilityIngestContext, IngestRequest, IngestResult, IngesterHandler,
};

use super::super::services::KnowledgeServices;
use super::super::types::format_knowledge_associate_result;

#[derive(Debug, Clone, Deserialize)]
struct KnowledgeAssociateInput {
    source_ref: String,
    target_ref: String,
}

pub struct KnowledgeAssociateIngester {
    services: Arc<KnowledgeServices>,
}

impl KnowledgeAssociateIngester {
    pub fn new(services: Arc<KnowledgeServices>) -> Self {
        Self { services }
    }
}

impl IngesterHandler for KnowledgeAssociateIngester {
    fn ingest<'a>(
        &'a self,
        request: IngestRequest,
        ctx: &'a mut dyn CapabilityIngestContext,
    ) -> super::super::types::BoxFuture<'a, Result<IngestResult>> {
        Box::pin(async move {
            let input: KnowledgeAssociateInput = request.parse_json()?;
            let result = self
                .services
                .relations
                .associate_by_refs(ctx, &input.source_ref, &input.target_ref)
                .await?;

            Ok(IngestResult::new(
                json!({
                    "association": result,
                }),
                format_knowledge_associate_result(&result),
            ))
        })
    }
}

use std::sync::Arc;

use anyhow::Result;
use serde_json::json;

use crate::engine::devql::capability_host::{
    IngestRequest, IngestResult, KnowledgeIngestContext, KnowledgeIngester,
};

use super::super::services::KnowledgeServices;
use super::super::types::{ListVersionsRequest, format_knowledge_versions_result};

pub struct KnowledgeVersionsIngester {
    services: Arc<KnowledgeServices>,
}

impl KnowledgeVersionsIngester {
    pub fn new(services: Arc<KnowledgeServices>) -> Self {
        Self { services }
    }
}

impl KnowledgeIngester for KnowledgeVersionsIngester {
    fn ingest<'a>(
        &'a self,
        request: IngestRequest,
        ctx: &'a mut dyn KnowledgeIngestContext,
    ) -> super::super::types::BoxFuture<'a, Result<IngestResult>> {
        Box::pin(async move {
            let input: ListVersionsRequest = request.parse_json()?;
            let result = self.services.retrieval.list_versions(input, ctx).await?;

            Ok(IngestResult::new(
                json!({
                    "versions": result,
                }),
                format_knowledge_versions_result(&result),
            ))
        })
    }
}

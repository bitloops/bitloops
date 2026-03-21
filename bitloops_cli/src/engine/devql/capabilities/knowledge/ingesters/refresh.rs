use std::sync::Arc;

use anyhow::Result;
use serde_json::json;

use crate::engine::devql::capability_host::{
    IngestRequest, IngestResult, KnowledgeIngestContext, KnowledgeIngester,
};

use super::super::services::KnowledgeServices;
use super::super::types::{RefreshSourceRequest, format_knowledge_refresh_result};

pub struct KnowledgeRefreshIngester {
    services: Arc<KnowledgeServices>,
}

impl KnowledgeRefreshIngester {
    pub fn new(services: Arc<KnowledgeServices>) -> Self {
        Self { services }
    }
}

impl KnowledgeIngester for KnowledgeRefreshIngester {
    fn ingest<'a>(
        &'a self,
        request: IngestRequest,
        ctx: &'a mut dyn KnowledgeIngestContext,
    ) -> super::super::types::BoxFuture<'a, Result<IngestResult>> {
        Box::pin(async move {
            let input: RefreshSourceRequest = request.parse_json()?;
            let result = self.services.ingestion.refresh_source(input, ctx).await?;

            Ok(IngestResult::new(
                json!({
                    "refresh": result,
                }),
                format_knowledge_refresh_result(&result),
            ))
        })
    }
}

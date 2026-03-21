use std::sync::Arc;

use anyhow::Result;
use serde::Deserialize;
use serde_json::json;

use crate::engine::devql::capability_host::{
    IngestRequest, IngestResult, KnowledgeIngestContext, KnowledgeIngester,
};

use super::super::services::KnowledgeServices;
use super::super::types::{IngestKnowledgeRequest, format_knowledge_add_result};

#[derive(Debug, Clone, Deserialize)]
struct KnowledgeAddInput {
    url: String,
    commit: Option<String>,
}

pub struct KnowledgeAddIngester {
    services: Arc<KnowledgeServices>,
}

impl KnowledgeAddIngester {
    pub fn new(services: Arc<KnowledgeServices>) -> Self {
        Self { services }
    }
}

impl KnowledgeIngester for KnowledgeAddIngester {
    fn ingest<'a>(
        &'a self,
        request: IngestRequest,
        ctx: &'a mut dyn KnowledgeIngestContext,
    ) -> super::super::types::BoxFuture<'a, Result<IngestResult>> {
        Box::pin(async move {
            let input: KnowledgeAddInput = request.parse_json()?;
            let ingest_result = self
                .services
                .ingestion
                .ingest_source(
                    IngestKnowledgeRequest {
                        url: input.url.clone(),
                    },
                    ctx,
                )
                .await?;

            let association_result = if let Some(commit) = input.commit.as_deref() {
                Some(
                    self.services
                        .relations
                        .associate_to_commit(ctx, &ingest_result, commit)
                        .await?,
                )
            } else {
                None
            };

            Ok(IngestResult::new(
                json!({
                    "ingest": ingest_result,
                    "association": association_result,
                }),
                format_knowledge_add_result(&ingest_result, association_result.as_ref()),
            ))
        })
    }
}

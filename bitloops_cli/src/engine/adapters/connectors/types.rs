use std::future::Future;
use std::pin::Pin;

use serde_json::Value;

use crate::config::ProviderConfig;
use crate::engine::devql::capabilities::knowledge::{
    FetchedKnowledgeDocument, KnowledgePayloadData, ParsedKnowledgeUrl,
};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone, PartialEq)]
pub struct ExternalKnowledgeRecord {
    pub provider: String,
    pub source_kind: String,
    pub canonical_external_id: String,
    pub canonical_url: String,
    pub title: String,
    pub state: Option<String>,
    pub author: Option<String>,
    pub updated_at: Option<String>,
    pub body_preview: Option<String>,
    pub normalized_fields: Value,
    pub payload: KnowledgePayloadData,
}

pub trait ConnectorContext: Send + Sync {
    fn provider_config(&self) -> &ProviderConfig;
}

pub trait KnowledgeConnectorAdapter: Send + Sync {
    fn can_handle(&self, parsed: &ParsedKnowledgeUrl) -> bool;

    fn fetch<'a>(
        &'a self,
        parsed: &'a ParsedKnowledgeUrl,
        ctx: &'a dyn ConnectorContext,
    ) -> BoxFuture<'a, anyhow::Result<ExternalKnowledgeRecord>>;
}

impl From<ExternalKnowledgeRecord> for FetchedKnowledgeDocument {
    fn from(value: ExternalKnowledgeRecord) -> Self {
        Self {
            external_id: value.canonical_external_id,
            title: value.title,
            web_url: value.canonical_url,
            state: value.state,
            author: value.author,
            updated_at: value.updated_at,
            body_preview: value.body_preview,
            normalized_fields: value.normalized_fields,
            payload: value.payload,
        }
    }
}

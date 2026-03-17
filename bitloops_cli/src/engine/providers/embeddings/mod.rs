mod embeddings_http;
mod embeddings_local;

use anyhow::Result;

const DEFAULT_EMBEDDING_PROVIDER: &str = "local";
const DEFAULT_LOCAL_EMBEDDING_MODEL: &str = "jinaai/jina-embeddings-v2-base-code";
const DEFAULT_VOYAGE_EMBEDDING_MODEL: &str = "voyage-code-3";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingInputType {
    Document,
    Query,
}

impl EmbeddingInputType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Document => "document",
            Self::Query => "query",
        }
    }
}

pub trait EmbeddingProvider: Send + Sync {
    fn provider_name(&self) -> &str;
    fn model_name(&self) -> &str;
    fn output_dimension(&self) -> Option<usize>;
    fn cache_key(&self) -> String;
    fn embed(&self, input: &str, input_type: EmbeddingInputType) -> Result<Vec<f32>>;
}

pub fn build_embedding_provider(
    provider: &str,
    model: String,
    api_key: Option<String>,
) -> Result<Box<dyn EmbeddingProvider>> {
    if embeddings_local::supports_provider(provider) {
        embeddings_local::build(provider, model)
    } else {
        embeddings_http::build(provider, model, api_key)
    }
}

pub fn default_embedding_provider() -> &'static str {
    DEFAULT_EMBEDDING_PROVIDER
}

pub fn default_embedding_model(provider: &str) -> Option<&'static str> {
    match provider {
        "local" | "jina" | "jina_local" => Some(DEFAULT_LOCAL_EMBEDDING_MODEL),
        "voyage" => Some(DEFAULT_VOYAGE_EMBEDDING_MODEL),
        _ => None,
    }
}

pub fn embedding_provider_requires_api_key(provider: &str) -> bool {
    matches!(provider, "voyage" | "openai")
}

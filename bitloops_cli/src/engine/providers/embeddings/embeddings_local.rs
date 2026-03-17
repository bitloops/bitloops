use std::sync::Mutex;

use anyhow::{Context, Result, anyhow, bail};
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

use super::{EmbeddingInputType, EmbeddingProvider};

const DEFAULT_LOCAL_EMBEDDING_MODEL: &str = "jinaai/jina-embeddings-v2-base-code";

pub(super) fn supports_provider(provider: &str) -> bool {
    matches!(provider, "local" | "jina" | "jina_local")
}

pub(super) fn build(
    provider: &str,
    model: String,
    output_dimension: Option<usize>,
) -> Result<Box<dyn EmbeddingProvider>> {
    if output_dimension.is_some() {
        bail!(
            "BITLOOPS_DEVQL_EMBEDDING_OUTPUT_DIMENSION is not supported for local embedding provider `{provider}`"
        );
    }

    let resolved_model = resolve_local_embedding_model(&model)?;
    let embedder = TextEmbedding::try_new(
        InitOptions::new(resolved_model).with_show_download_progress(false),
    )
        .with_context(|| format!("loading local embedding model `{model}`"))?;

    Ok(Box::new(LocalEmbeddingsProvider {
        provider: provider.to_string(),
        model,
        embedder: Mutex::new(embedder),
    }))
}

struct LocalEmbeddingsProvider {
    provider: String,
    model: String,
    embedder: Mutex<TextEmbedding>,
}

impl EmbeddingProvider for LocalEmbeddingsProvider {
    fn provider_name(&self) -> &str {
        &self.provider
    }

    fn model_name(&self) -> &str {
        &self.model
    }

    fn output_dimension(&self) -> Option<usize> {
        None
    }

    fn cache_key(&self) -> String {
        format!("provider={}::model={}", self.provider, self.model)
    }

    fn embed(&self, input: &str, _input_type: EmbeddingInputType) -> Result<Vec<f32>> {
        let input = input.trim().to_string();
        if input.is_empty() {
            bail!("embedding input cannot be empty");
        }

        let mut embedder = self
            .embedder
            .lock()
            .map_err(|_| anyhow!("local embedding provider mutex was poisoned"))?;
        let outputs = embedder
            .embed(vec![input], None)
            .with_context(|| format!("running local embedding model `{}`", self.model))?;
        let output = outputs
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("local embedding provider returned no vectors"))?;
        if output.is_empty() {
            bail!("local embedding provider returned an empty vector");
        }

        Ok(output)
    }
}

fn resolve_local_embedding_model(model: &str) -> Result<EmbeddingModel> {
    match model.trim().to_ascii_lowercase().as_str() {
        "jinaai/jina-embeddings-v2-base-code"
        | "jina-embeddings-v2-base-code"
        | "jinaembeddingsv2basecode" => Ok(EmbeddingModel::JinaEmbeddingsV2BaseCode),
        other => bail!(
            "unsupported local embedding model `{other}`. Use `{DEFAULT_LOCAL_EMBEDDING_MODEL}`"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_provider_supports_local_and_jina_aliases() {
        assert!(supports_provider("local"));
        assert!(supports_provider("jina"));
        assert!(supports_provider("jina_local"));
        assert!(!supports_provider("voyage"));
    }

    #[test]
    fn local_provider_resolves_jina_code_model_aliases() {
        assert!(matches!(
            resolve_local_embedding_model("jinaai/jina-embeddings-v2-base-code")
                .expect("canonical model should resolve"),
            EmbeddingModel::JinaEmbeddingsV2BaseCode
        ));
        assert!(matches!(
            resolve_local_embedding_model("JinaEmbeddingsV2BaseCode")
                .expect("enum-like alias should resolve"),
            EmbeddingModel::JinaEmbeddingsV2BaseCode
        ));
    }
}

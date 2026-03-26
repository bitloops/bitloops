use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result, anyhow, bail};
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

use super::{EmbeddingInputType, EmbeddingProvider};
use crate::utils::paths;

const DEFAULT_LOCAL_EMBEDDING_MODEL: &str = "jinaai/jina-embeddings-v2-base-code";

pub(super) fn supports_provider(provider: &str) -> bool {
    matches!(provider, "local" | "jina" | "jina_local")
}

pub(super) fn build(
    provider: &str,
    model: String,
    repo_root: Option<&Path>,
) -> Result<Box<dyn EmbeddingProvider>> {
    let resolved_model = resolve_local_embedding_model(&model)?;
    let repo_root =
        repo_root.ok_or_else(|| anyhow!("local embedding provider requires repo root"))?;
    let init_options = build_init_options(resolved_model, repo_root)?;
    let embedder = TextEmbedding::try_new(init_options)
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

fn build_init_options(model: EmbeddingModel, repo_root: &Path) -> Result<InitOptions> {
    let cache_dir = default_local_embedding_cache_dir(repo_root);
    fs::create_dir_all(&cache_dir)
        .with_context(|| format!("creating local embedding cache at {}", cache_dir.display()))?;
    Ok(InitOptions::new(model)
        .with_show_download_progress(false)
        .with_cache_dir(cache_dir))
}

fn default_local_embedding_cache_dir(repo_root: &Path) -> PathBuf {
    paths::default_embedding_model_cache_dir(repo_root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

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

    #[test]
    fn local_provider_rejects_unknown_models() {
        let err = resolve_local_embedding_model("voyage-code-3")
            .expect_err("unsupported model should fail");
        assert!(
            err.to_string()
                .contains("unsupported local embedding model")
        );
    }

    #[test]
    fn local_provider_build_rejects_unknown_model_before_loading_runtime() {
        let err = build("local", "voyage-code-3".to_string(), None)
            .err()
            .expect("unsupported model should fail before loading embedder");
        assert!(
            err.to_string()
                .contains("unsupported local embedding model")
        );
    }

    #[test]
    fn local_provider_defaults_cache_dir_under_bitloops_embeddings() {
        let cache_dir = default_local_embedding_cache_dir(Path::new("/repo"));
        assert_eq!(
            cache_dir,
            PathBuf::from("/repo/.bitloops/embeddings/models")
        );
    }

    #[test]
    fn local_provider_build_requires_repo_root() {
        let err = build(
            "local",
            "jinaai/jina-embeddings-v2-base-code".to_string(),
            None,
        )
        .err()
        .expect("local provider should require repo root");
        assert!(err.to_string().contains("requires repo root"));
    }

    #[test]
    fn local_provider_build_init_options_use_bitloops_cache_dir_when_repo_is_known() {
        let temp = tempfile::tempdir().expect("temp dir");
        let options = build_init_options(EmbeddingModel::JinaEmbeddingsV2BaseCode, temp.path())
            .expect("init options");

        assert_eq!(
            options.cache_dir,
            temp.path().join(".bitloops/embeddings/models")
        );
        assert!(options.cache_dir.exists());
        assert!(!options.show_download_progress);
    }
}

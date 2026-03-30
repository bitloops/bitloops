use std::fs;
use std::panic::{self, AssertUnwindSafe};
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
    cache_dir: Option<&Path>,
) -> Result<Box<dyn EmbeddingProvider>> {
    let resolved_model = resolve_local_embedding_model(&model)?;
    let repo_root =
        repo_root.ok_or_else(|| anyhow!("local embedding provider requires repo root"))?;
    let init_options = build_init_options(resolved_model, repo_root, cache_dir)?;
    let embedder = guard_ort_panic(
        || TextEmbedding::try_new(init_options),
        &format!("loading local embedding model `{model}`"),
    )?;

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
        let outputs = guard_ort_panic(
            || embedder.embed(vec![input], None),
            &format!("running local embedding model `{}`", self.model),
        )?;
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

fn build_init_options(
    model: EmbeddingModel,
    repo_root: &Path,
    cache_dir: Option<&Path>,
) -> Result<InitOptions> {
    let cache_dir = cache_dir
        .map(Path::to_path_buf)
        .unwrap_or_else(|| default_local_embedding_cache_dir(repo_root));
    fs::create_dir_all(&cache_dir)
        .with_context(|| format!("creating local embedding cache at {}", cache_dir.display()))?;
    Ok(InitOptions::new(model)
        .with_show_download_progress(false)
        .with_cache_dir(cache_dir))
}

fn default_local_embedding_cache_dir(repo_root: &Path) -> PathBuf {
    paths::default_embedding_model_cache_dir(repo_root)
}

fn guard_ort_panic<T, F>(action: F, context: &str) -> Result<T>
where
    F: FnOnce() -> Result<T>,
{
    match panic::catch_unwind(AssertUnwindSafe(action)) {
        Ok(result) => result.with_context(|| context.to_string()),
        Err(payload) => Err(anyhow!(
            "{context}: {}",
            format_embedding_runtime_panic(payload)
        )),
    }
}

fn format_embedding_runtime_panic(payload: Box<dyn std::any::Any + Send>) -> String {
    let message = match payload.downcast::<String>() {
        Ok(message) => *message,
        Err(payload) => match payload.downcast::<&'static str>() {
            Ok(message) => (*message).to_string(),
            Err(_) => "local embedding runtime panicked".to_string(),
        },
    };

    if message.contains("Failed to load ONNX Runtime dylib") {
        format!(
            "{message}. Install or point `ORT_DYLIB_PATH` at `libonnxruntime.dylib`, or disable local embeddings with `BITLOOPS_DEVQL_EMBEDDING_PROVIDER=none`."
        )
    } else {
        message
    }
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
        let err = build("local", "voyage-code-3".to_string(), None, None)
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
            None,
        )
        .err()
        .expect("local provider should require repo root");
        assert!(err.to_string().contains("requires repo root"));
    }

    #[test]
    fn local_provider_build_init_options_use_bitloops_cache_dir_when_repo_is_known() {
        let temp = tempfile::tempdir().expect("temp dir");
        let options =
            build_init_options(EmbeddingModel::JinaEmbeddingsV2BaseCode, temp.path(), None)
                .expect("init options");

        assert_eq!(
            options.cache_dir,
            temp.path().join(".bitloops/embeddings/models")
        );
        assert!(options.cache_dir.exists());
        assert!(!options.show_download_progress);
    }

    #[test]
    fn guard_ort_panic_wraps_dynload_panics_with_guidance() {
        let err = guard_ort_panic(
            || -> Result<()> {
                panic!("Failed to load ONNX Runtime dylib: dlopen failed");
            },
            "loading local embedding model `jinaai/jina-embeddings-v2-base-code`",
        )
        .expect_err("panic should be converted into an error");

        let text = err.to_string();
        assert!(text.contains("Failed to load ONNX Runtime dylib"));
        assert!(text.contains("ORT_DYLIB_PATH"));
        assert!(text.contains("BITLOOPS_DEVQL_EMBEDDING_PROVIDER=none"));
    }

    #[test]
    fn guard_ort_panic_preserves_regular_errors() {
        let err = guard_ort_panic(
            || -> Result<()> { Err(anyhow!("embed failed")) },
            "running local embedding model `jinaai/jina-embeddings-v2-base-code`",
        )
        .expect_err("regular error should be preserved");

        let text = format!("{err:#}");
        assert!(text.contains("embed failed"));
        assert!(
            text.contains("running local embedding model `jinaai/jina-embeddings-v2-base-code`")
        );
    }
}

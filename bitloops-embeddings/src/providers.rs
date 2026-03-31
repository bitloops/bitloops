use anyhow::{Context, Result, anyhow, bail};
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use bitloops_embeddings_protocol::{EmbeddingInput, EmbeddingInputType, ProviderDescriptor};

use crate::config::EmbeddingProfileConfig;

const DEFAULT_LOCAL_MODEL: &str = "jinaai/jina-embeddings-v2-base-code";

pub trait EmbeddingRuntimeProvider: Send + Sync {
    fn provider_name(&self) -> &str;
    fn model_name(&self) -> &str;
    fn output_dimension(&self) -> Option<usize>;
    fn cache_dir(&self) -> Option<&Path>;
    fn cache_key(&self) -> String;
    fn embed(&self, input: &str, input_type: EmbeddingInputType) -> Result<Vec<f32>>;
}

pub fn build_provider(
    profile: &EmbeddingProfileConfig,
    repo_root: Option<&Path>,
) -> Result<Box<dyn EmbeddingRuntimeProvider>> {
    match profile {
        EmbeddingProfileConfig::LocalFastembed { model, cache_dir } => {
            build_local_provider(model.as_deref(), cache_dir.as_deref(), repo_root)
        }
        EmbeddingProfileConfig::OpenAi {
            model,
            api_key,
            base_url,
        } => build_http_provider("openai", model, Some(api_key), base_url.as_deref()),
        EmbeddingProfileConfig::Voyage {
            model,
            api_key,
            base_url,
        } => build_http_provider("voyage", model, Some(api_key), base_url.as_deref()),
    }
}

pub fn describe_provider(
    provider: &dyn EmbeddingRuntimeProvider,
) -> ProviderDescriptor {
    ProviderDescriptor {
        kind: provider.provider_name().to_string(),
        provider_name: provider.provider_name().to_string(),
        model_name: provider.model_name().to_string(),
        output_dimension: provider.output_dimension(),
        cache_dir: provider.cache_dir().map(|path| path.display().to_string()),
    }
}

pub fn build_batch_vectors(
    provider: &dyn EmbeddingRuntimeProvider,
    inputs: &[EmbeddingInput],
) -> Result<Vec<Vec<f32>>> {
    let mut out = Vec::with_capacity(inputs.len());
    for input in inputs {
        out.push(provider.embed(&input.text, input.input_type)?);
    }
    Ok(out)
}

fn build_local_provider(
    model: Option<&str>,
    cache_dir: Option<&Path>,
    repo_root: Option<&Path>,
) -> Result<Box<dyn EmbeddingRuntimeProvider>> {
    let resolved_model = resolve_local_embedding_model(model)?;
    let repo_root = repo_root.ok_or_else(|| anyhow!("local embedding profile requires repo_root"))?;
    let cache_dir = cache_dir
        .map(Path::to_path_buf)
        .unwrap_or_else(|| default_local_embedding_cache_dir(repo_root));
    std::fs::create_dir_all(&cache_dir)
        .with_context(|| format!("creating embeddings cache at {}", cache_dir.display()))?;

    let init_options = InitOptions::new(resolved_model)
        .with_cache_dir(cache_dir.clone())
        .with_show_download_progress(false);
    let embedder = guard_runtime_panic(
        || TextEmbedding::try_new(init_options),
        "loading local embedding runtime",
    )?;

    Ok(Box::new(LocalEmbeddingProvider {
        model: resolved_model_name(model),
        embedder: Mutex::new(embedder),
        cache_dir,
    }))
}

struct LocalEmbeddingProvider {
    model: String,
    embedder: Mutex<TextEmbedding>,
    cache_dir: PathBuf,
}

impl EmbeddingRuntimeProvider for LocalEmbeddingProvider {
    fn provider_name(&self) -> &str {
        "local_fastembed"
    }

    fn model_name(&self) -> &str {
        &self.model
    }

    fn output_dimension(&self) -> Option<usize> {
        None
    }

    fn cache_dir(&self) -> Option<&Path> {
        Some(&self.cache_dir)
    }

    fn cache_key(&self) -> String {
        format!("provider=local_fastembed::model={}", self.model)
    }

    fn embed(&self, input: &str, _input_type: EmbeddingInputType) -> Result<Vec<f32>> {
        let input = input.trim();
        if input.is_empty() {
            bail!("embedding input cannot be empty");
        }

        let mut embedder = self
            .embedder
            .lock()
            .map_err(|_| anyhow!("local embedding mutex was poisoned"))?;
        let outputs = guard_runtime_panic(
            || embedder.embed(vec![input.to_string()], None),
            "running local embedding runtime",
        )?;
        let output = outputs
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("local embedding runtime returned no vectors"))?;
        if output.is_empty() {
            bail!("local embedding runtime returned an empty vector");
        }

        Ok(output)
    }
}

fn resolve_local_embedding_model(model: Option<&str>) -> Result<EmbeddingModel> {
    let model = model.unwrap_or(DEFAULT_LOCAL_MODEL).trim();
    match model.to_ascii_lowercase().as_str() {
        "jinaai/jina-embeddings-v2-base-code"
        | "jina-embeddings-v2-base-code"
        | "jinaembeddingsv2basecode" => Ok(EmbeddingModel::JinaEmbeddingsV2BaseCode),
        other => bail!("unsupported local embedding model `{other}`"),
    }
}

fn resolved_model_name(model: Option<&str>) -> String {
    model
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_LOCAL_MODEL)
        .to_string()
}

fn default_local_embedding_cache_dir(repo_root: &Path) -> PathBuf {
    repo_root.join(".bitloops/embeddings/models")
}

fn build_http_provider(
    provider: &str,
    model: &str,
    api_key: Option<&String>,
    base_url: Option<&str>,
) -> Result<Box<dyn EmbeddingRuntimeProvider>> {
    let endpoint = resolve_http_endpoint(provider, base_url)?;
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(60))
        .build()
        .context("building embeddings HTTP client")?;

    Ok(Box::new(HttpEmbeddingProvider {
        provider: provider.to_string(),
        model: model.to_string(),
        api_key: api_key.cloned(),
        endpoint,
        output_dimension: default_output_dimension(provider),
        client,
    }))
}

struct HttpEmbeddingProvider {
    provider: String,
    model: String,
    api_key: Option<String>,
    endpoint: String,
    output_dimension: Option<usize>,
    client: Client,
}

impl EmbeddingRuntimeProvider for HttpEmbeddingProvider {
    fn provider_name(&self) -> &str {
        &self.provider
    }

    fn model_name(&self) -> &str {
        &self.model
    }

    fn output_dimension(&self) -> Option<usize> {
        self.output_dimension
    }

    fn cache_dir(&self) -> Option<&Path> {
        None
    }

    fn cache_key(&self) -> String {
        match self.output_dimension {
            Some(output_dimension) => format!(
                "provider={}::model={}::dimension={output_dimension}",
                self.provider, self.model
            ),
            None => format!("provider={}::model={}", self.provider, self.model),
        }
    }

    fn embed(&self, input: &str, input_type: EmbeddingInputType) -> Result<Vec<f32>> {
        let mut request = self.client.post(&self.endpoint);
        if let Some(api_key) = self.api_key.as_deref().filter(|value| !value.is_empty()) {
            request = request.bearer_auth(api_key);
        }

        let response = request
            .json(&build_embedding_payload(
                &self.provider,
                &self.model,
                input,
                input_type,
                self.output_dimension,
            ))
            .send()
            .with_context(|| {
                format!(
                    "sending embedding request to provider={} model={}",
                    self.provider, self.model
                )
            })?;

        let status = response.status();
        let value: Value = response
            .json()
            .with_context(|| format!("parsing embedding response from {}", self.provider))?;
        if !status.is_success() {
            let detail = value
                .get("error")
                .and_then(Value::as_object)
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
                .or_else(|| value.get("detail").and_then(Value::as_str))
                .unwrap_or("request failed");
            bail!(
                "embedding provider request failed: provider={}, model={}, status={}, detail={}",
                self.provider,
                self.model,
                status,
                detail
            );
        }

        extract_embedding(&value).with_context(|| {
            format!(
                "reading embedding vector from provider={} model={}",
                self.provider, self.model
            )
        })
    }
}

fn resolve_http_endpoint(provider: &str, base_url: Option<&str>) -> Result<String> {
    match provider {
        "voyage" => Ok(base_url
            .map(|value| value.trim_end_matches('/').to_string())
            .unwrap_or_else(|| "https://api.voyageai.com/v1/embeddings".to_string())),
        "openai" => Ok(base_url
            .map(|value| value.trim_end_matches('/').to_string())
            .unwrap_or_else(|| "https://api.openai.com/v1/embeddings".to_string())),
        other => bail!("unsupported embedding provider `{other}`"),
    }
}

fn default_output_dimension(provider: &str) -> Option<usize> {
    match provider {
        "voyage" => Some(1024),
        _ => None,
    }
}

fn build_embedding_payload(
    provider: &str,
    model: &str,
    input: &str,
    input_type: EmbeddingInputType,
    output_dimension: Option<usize>,
) -> Value {
    let mut payload = json!({
        "input": [input],
        "model": model,
    });

    match provider {
        "voyage" => {
            payload["input_type"] = json!(input_type.as_str());
            payload["truncation"] = json!(true);
            if let Some(output_dimension) = output_dimension {
                payload["output_dimension"] = json!(output_dimension);
            }
        }
        _ => {
            if let Some(output_dimension) = output_dimension {
                payload["dimensions"] = json!(output_dimension);
            }
        }
    }

    payload
}

fn extract_embedding(value: &Value) -> Result<Vec<f32>> {
    let embedding = value
        .pointer("/data/0/embedding")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("response did not include `/data/0/embedding` array"))?;

    let mut out = Vec::with_capacity(embedding.len());
    for item in embedding {
        let Some(number) = item.as_f64() else {
            bail!("embedding response contained non-numeric value");
        };
        let value = number as f32;
        if !value.is_finite() {
            bail!("embedding response contained non-finite value");
        }
        out.push(value);
    }

    if out.is_empty() {
        bail!("embedding response returned an empty vector");
    }

    Ok(out)
}

fn guard_runtime_panic<T, F>(action: F, context: &str) -> Result<T>
where
    F: FnOnce() -> Result<T>,
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(action)) {
        Ok(result) => result.with_context(|| context.to_string()),
        Err(payload) => Err(anyhow!("{context}: {}", format_panic(payload))),
    }
}

fn format_panic(payload: Box<dyn std::any::Any + Send>) -> String {
    let message = match payload.downcast::<String>() {
        Ok(message) => *message,
        Err(payload) => match payload.downcast::<&'static str>() {
            Ok(message) => (*message).to_string(),
            Err(_) => "embedding runtime panicked".to_string(),
        },
    };

    if message.contains("Failed to load ONNX Runtime dylib") {
        format!("{message}. Install ONNX Runtime or update the cached local model.")
    } else {
        message
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_model_resolution_accepts_aliases() {
        assert!(matches!(
            resolve_local_embedding_model(Some("jinaai/jina-embeddings-v2-base-code"))
                .expect("canonical model"),
            EmbeddingModel::JinaEmbeddingsV2BaseCode
        ));
        assert!(matches!(
            resolve_local_embedding_model(Some("JinaEmbeddingsV2BaseCode"))
                .expect("alias"),
            EmbeddingModel::JinaEmbeddingsV2BaseCode
        ));
    }

    #[test]
    fn http_endpoints_match_provider_defaults() {
        assert_eq!(
            resolve_http_endpoint("openai", None).expect("openai endpoint"),
            "https://api.openai.com/v1/embeddings"
        );
        assert_eq!(
            resolve_http_endpoint("voyage", None).expect("voyage endpoint"),
            "https://api.voyageai.com/v1/embeddings"
        );
    }

    #[test]
    fn embedding_payload_contains_provider_specific_fields() {
        let payload = build_embedding_payload(
            "voyage",
            "voyage-code-3",
            "fn normalize_email() {}",
            EmbeddingInputType::Document,
            Some(1024),
        );

        assert_eq!(payload["input_type"], "document");
        assert_eq!(payload["output_dimension"], 1024);
        assert_eq!(payload["truncation"], true);
    }

    #[test]
    fn guard_runtime_panic_preserves_regular_errors() {
        let err = guard_runtime_panic(|| -> Result<()> { Err(anyhow!("embed failed")) }, "test")
            .expect_err("expected error");
        assert!(format!("{err:#}").contains("embed failed"));
    }
}

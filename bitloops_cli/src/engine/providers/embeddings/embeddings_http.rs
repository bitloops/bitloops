use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};

use super::{EmbeddingInputType, EmbeddingProvider};

pub(super) fn build(
    provider: &str,
    model: String,
    api_key: Option<String>,
    base_url: Option<&str>,
    output_dimension: Option<usize>,
) -> Result<Box<dyn EmbeddingProvider>> {
    let endpoint = resolve_endpoint(provider, base_url)?;
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(60))
        .build()
        .context("building embedding HTTP client")?;

    Ok(Box::new(EmbeddingsHttpProvider {
        provider: provider.to_string(),
        model,
        api_key,
        endpoint,
        output_dimension,
        client,
    }))
}

pub(super) fn resolve_endpoint(provider: &str, base_url: Option<&str>) -> Result<String> {
    if let Some(base_url) = base_url.map(str::trim).filter(|value| !value.is_empty()) {
        return Ok(base_url.to_string());
    }

    match provider {
        "voyage" => Ok("https://api.voyageai.com/v1/embeddings".to_string()),
        "openai" => Ok("https://api.openai.com/v1/embeddings".to_string()),
        "qodo" | "openai_compatible" | "custom" => {
            bail!("BITLOOPS_DEVQL_EMBEDDING_BASE_URL is required for embedding provider `{provider}`")
        }
        other => bail!(
            "unsupported embedding provider `{other}`. Use `local`, `voyage`, `qodo`, `openai`, or `openai_compatible` with BITLOOPS_DEVQL_EMBEDDING_BASE_URL"
        ),
    }
}

struct EmbeddingsHttpProvider {
    provider: String,
    model: String,
    api_key: Option<String>,
    endpoint: String,
    output_dimension: Option<usize>,
    client: reqwest::blocking::Client,
}

impl EmbeddingProvider for EmbeddingsHttpProvider {
    fn provider_name(&self) -> &str {
        &self.provider
    }

    fn model_name(&self) -> &str {
        &self.model
    }

    fn output_dimension(&self) -> Option<usize> {
        self.output_dimension
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
        "qodo" => {
            payload["input"] = json!([format_qodo_input(input, input_type)]);
            if let Some(output_dimension) = output_dimension {
                payload["dimensions"] = json!(output_dimension);
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

fn format_qodo_input(input: &str, input_type: EmbeddingInputType) -> String {
    match input_type {
        EmbeddingInputType::Query => {
            format!(
                "Instruct: Given a question, retrieve relevant code snippets that best answer the question\nQuery: {}",
                input.trim()
            )
        }
        EmbeddingInputType::Document => input.to_string(),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embeddings_http_resolves_known_endpoints_and_errors() {
        assert_eq!(
            resolve_endpoint("voyage", None).expect("voyage endpoint"),
            "https://api.voyageai.com/v1/embeddings"
        );
        assert_eq!(
            resolve_endpoint("openai", None).expect("openai endpoint"),
            "https://api.openai.com/v1/embeddings"
        );
        assert_eq!(
            resolve_endpoint("custom", Some(" http://localhost:11434/v1/embeddings "))
                .expect("custom base url"),
            "http://localhost:11434/v1/embeddings"
        );
        assert!(
            resolve_endpoint("openai_compatible", None)
                .expect_err("missing base url should fail")
                .to_string()
                .contains("BITLOOPS_DEVQL_EMBEDDING_BASE_URL is required")
        );
        assert!(
            resolve_endpoint("qodo", None)
                .expect_err("missing base url should fail")
                .to_string()
                .contains("BITLOOPS_DEVQL_EMBEDDING_BASE_URL is required")
        );
    }

    #[test]
    fn embeddings_http_builds_voyage_payload_with_dimension_and_input_type() {
        let payload = build_embedding_payload(
            "voyage",
            "voyage-code-3",
            "fn normalize_email() {}",
            EmbeddingInputType::Document,
            Some(1024),
        );
        assert_eq!(payload["model"], "voyage-code-3");
        assert_eq!(payload["input_type"], "document");
        assert_eq!(payload["output_dimension"], 1024);
        assert_eq!(payload["truncation"], true);
    }

    #[test]
    fn embeddings_http_builds_openai_payload_with_dimensions() {
        let payload = build_embedding_payload(
            "openai",
            "text-embedding-3-large",
            "fn normalize_email() {}",
            EmbeddingInputType::Document,
            Some(1536),
        );
        assert_eq!(payload["model"], "text-embedding-3-large");
        assert_eq!(payload["dimensions"], 1536);
        assert!(payload.get("input_type").is_none());
    }

    #[test]
    fn embeddings_http_builds_qodo_payload_with_query_instruction() {
        let payload = build_embedding_payload(
            "qodo",
            "Qodo/Qodo-Embed-1-1.5B",
            "normalize email helper",
            EmbeddingInputType::Query,
            None,
        );

        assert_eq!(payload["model"], "Qodo/Qodo-Embed-1-1.5B");
        assert_eq!(
            payload["input"][0],
            "Instruct: Given a question, retrieve relevant code snippets that best answer the question\nQuery: normalize email helper"
        );
        assert!(payload.get("dimensions").is_none());
        assert!(payload.get("input_type").is_none());
    }

    #[test]
    fn embeddings_http_builds_qodo_payload_without_instruction_for_documents() {
        let payload = build_embedding_payload(
            "qodo",
            "Qodo/Qodo-Embed-1-1.5B",
            "fn normalize_email() {}",
            EmbeddingInputType::Document,
            Some(1536),
        );

        assert_eq!(payload["input"][0], "fn normalize_email() {}");
        assert_eq!(payload["dimensions"], 1536);
    }

    #[test]
    fn embeddings_http_extracts_embedding_vector() {
        let payload = json!({
            "data": [
                { "embedding": [0.1, -0.2, 0.3] }
            ]
        });
        assert_eq!(
            extract_embedding(&payload).expect("embedding should parse"),
            vec![0.1_f32, -0.2_f32, 0.3_f32]
        );
    }

    #[test]
    fn embeddings_http_rejects_missing_embedding_payload() {
        let payload = json!({ "data": [] });
        assert!(
            extract_embedding(&payload)
                .expect_err("missing embedding should fail")
                .to_string()
                .contains("/data/0/embedding")
        );
    }
}

use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use super::LlmProvider;

pub(super) fn build(
    provider: &str,
    model: String,
    api_key: String,
    base_url: Option<&str>,
) -> Result<Box<dyn LlmProvider>> {
    let endpoint = resolve_endpoint(provider, base_url)?;
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()
        .context("building semantic summary HTTP client")?;

    Ok(Box::new(OpenAiCompatibleLlmProvider {
        provider: provider.to_string(),
        model,
        endpoint,
        api_key,
        client,
    }))
}

pub(super) fn resolve_endpoint(provider: &str, base_url: Option<&str>) -> Result<String> {
    if let Some(base_url) = base_url.map(str::trim).filter(|value| !value.is_empty()) {
        return Ok(base_url.to_string());
    }

    match provider {
        "openai" => Ok("https://api.openai.com/v1/chat/completions".to_string()),
        "openrouter" => Ok("https://openrouter.ai/api/v1/chat/completions".to_string()),
        "openai_compatible" | "custom" => {
            bail!("BITLOOPS_DEVQL_SEMANTIC_BASE_URL is required for semantic provider `{provider}`")
        }
        other => bail!(
            "unsupported semantic provider `{other}`. Use `openai`, `openrouter`, or `openai_compatible` with BITLOOPS_DEVQL_SEMANTIC_BASE_URL"
        ),
    }
}

struct OpenAiCompatibleLlmProvider {
    provider: String,
    model: String,
    endpoint: String,
    api_key: String,
    client: reqwest::blocking::Client,
}

impl LlmProvider for OpenAiCompatibleLlmProvider {
    fn complete(&self, system_prompt: &str, user_prompt: &str) -> Option<String> {
        let payload = json!({
            "model": self.model,
            "temperature": 0.1,
            "messages": [
                {
                    "role": "system",
                    "content": system_prompt,
                },
                {
                    "role": "user",
                    "content": user_prompt,
                }
            ]
        });

        let response = self
            .client
            .post(&self.endpoint)
            .bearer_auth(&self.api_key)
            .json(&payload)
            .send()
            .ok()?;
        if !response.status().is_success() {
            log::warn!(
                "semantic summary provider request failed: provider={}, model={}, status={}",
                self.provider,
                self.model,
                response.status()
            );
            return None;
        }

        let value: Value = response.json().ok()?;
        extract_message_content(&value)
    }

    fn descriptor(&self) -> String {
        format!("{}:{}", self.provider, self.model)
    }

    fn prompt_version(&self, base_prompt_version: &str) -> String {
        format!(
            "{base_prompt_version}::provider={}::model={}",
            self.provider, self.model
        )
    }
}

fn extract_message_content(value: &Value) -> Option<String> {
    let content = value.pointer("/choices/0/message/content")?;
    match content {
        Value::String(text) => Some(text.clone()),
        Value::Array(items) => {
            let mut parts = Vec::new();
            for item in items {
                if let Some(text) = item.get("text").and_then(Value::as_str) {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        parts.push(trimmed.to_string());
                    }
                }
            }
            if parts.is_empty() {
                None
            } else {
                Some(parts.join("\n"))
            }
        }
        _ => None,
    }
}

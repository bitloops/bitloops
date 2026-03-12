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
        let response = self
            .client
            .post(&self.endpoint)
            .bearer_auth(&self.api_key)
            .json(&build_chat_completion_payload(
                &self.model,
                system_prompt,
                user_prompt,
            ))
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

fn build_chat_completion_payload(model: &str, system_prompt: &str, user_prompt: &str) -> Value {
    json!({
        "model": model,
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
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_llm_provider_resolves_known_endpoints_and_errors() {
        assert_eq!(
            resolve_endpoint("openai", None).expect("openai endpoint"),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            resolve_endpoint("openrouter", None).expect("openrouter endpoint"),
            "https://openrouter.ai/api/v1/chat/completions"
        );
        assert_eq!(
            resolve_endpoint(
                "custom",
                Some(" http://localhost:11434/v1/chat/completions ")
            )
            .expect("custom base url"),
            "http://localhost:11434/v1/chat/completions"
        );
        assert!(
            resolve_endpoint("openai_compatible", None)
                .expect_err("missing base url should fail")
                .to_string()
                .contains("BITLOOPS_DEVQL_SEMANTIC_BASE_URL is required")
        );
        assert!(
            resolve_endpoint("unsupported", None)
                .expect_err("unsupported provider should fail")
                .to_string()
                .contains("unsupported semantic provider")
        );
    }

    #[test]
    fn semantic_llm_provider_builds_payload_and_extracts_message_content() {
        let payload = build_chat_completion_payload("gpt-test", "system prompt", "user prompt");
        assert_eq!(payload["model"], "gpt-test");
        assert_eq!(payload["messages"][0]["role"], "system");
        assert_eq!(payload["messages"][1]["content"], "user prompt");

        let string_content = json!({
            "choices": [{ "message": { "content": "summary text" } }]
        });
        assert_eq!(
            extract_message_content(&string_content).as_deref(),
            Some("summary text")
        );

        let array_content = json!({
            "choices": [{
                "message": {
                    "content": [
                        { "text": " first " },
                        { "text": "" },
                        { "text": "second" }
                    ]
                }
            }]
        });
        assert_eq!(
            extract_message_content(&array_content).as_deref(),
            Some("first\nsecond")
        );
    }

    #[test]
    fn semantic_llm_provider_build_exposes_descriptor_and_prompt_version() {
        let provider = build(
            "openai",
            "gpt-test".to_string(),
            "test-key".to_string(),
            None,
        )
        .expect("provider should build");
        assert_eq!(provider.descriptor(), "openai:gpt-test");
        assert_eq!(
            provider.prompt_version("semantic-summary-v1"),
            "semantic-summary-v1::provider=openai::model=gpt-test"
        );
    }

    #[test]
    fn semantic_llm_provider_extract_message_content_returns_none_when_no_text_exists() {
        let payload = json!({
            "choices": [{
                "message": {
                    "content": [
                        { "type": "tool_use", "name": "search" }
                    ]
                }
            }]
        });
        assert!(
            extract_message_content(&payload).is_none(),
            "message content arrays without text blocks should be ignored"
        );
    }
}

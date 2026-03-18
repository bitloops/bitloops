mod chat_completions_http;

use anyhow::Result;

pub trait LlmProvider: Send + Sync {
    fn complete(&self, system_prompt: &str, user_prompt: &str) -> Option<String>;
    fn descriptor(&self) -> String;
}

pub fn build_llm_provider(
    provider: &str,
    model: String,
    api_key: String,
    base_url: Option<&str>,
) -> Result<Box<dyn LlmProvider>> {
    chat_completions_http::build(provider, model, api_key, base_url)
}

pub fn resolve_semantic_summary_endpoint(provider: &str, base_url: Option<&str>) -> Result<String> {
    chat_completions_http::resolve_endpoint(provider, base_url)
}

mod openai_compatible;

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
    openai_compatible::build(provider, model, api_key, base_url)
}

pub fn resolve_semantic_summary_endpoint(provider: &str, base_url: Option<&str>) -> Result<String> {
    openai_compatible::resolve_endpoint(provider, base_url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn llm_wrapper_builds_provider() {
        let provider = build_llm_provider(
            "openai",
            "gpt-test".to_string(),
            "test-key".to_string(),
            None,
        )
        .expect("provider should build");

        assert_eq!(provider.descriptor(), "openai:gpt-test");
    }

    #[test]
    fn llm_wrapper_resolves_endpoint() {
        assert_eq!(
            resolve_semantic_summary_endpoint("openai", None).expect("openai endpoint"),
            "https://api.openai.com/v1/chat/completions"
        );
    }
}

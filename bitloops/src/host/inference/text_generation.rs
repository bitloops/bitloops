use anyhow::{Result, anyhow};

use crate::adapters::model_providers::llm::LlmProvider;

use super::TextGenerationService;

pub(super) struct LlmTextGenerationService {
    pub(super) inner: Box<dyn LlmProvider>,
}

impl TextGenerationService for LlmTextGenerationService {
    fn descriptor(&self) -> String {
        self.inner.descriptor()
    }

    fn complete(&self, system_prompt: &str, user_prompt: &str) -> Result<String> {
        self.inner
            .complete(system_prompt, user_prompt)
            .ok_or_else(|| {
                anyhow!(
                    "text-generation provider `{}` returned no content",
                    self.descriptor()
                )
            })
    }
}

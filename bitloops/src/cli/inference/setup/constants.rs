pub(super) const DEFAULT_OLLAMA_BASE_URL: &str = "http://127.0.0.1:11434";
pub(super) const DEFAULT_OLLAMA_CHAT_BASE_URL: &str = "http://127.0.0.1:11434/api/chat";
pub(super) const DEFAULT_SUMMARY_TEMPERATURE: &str = "0.1";
pub(super) const DEFAULT_SUMMARY_MAX_OUTPUT_TOKENS: i64 = 200;
pub(super) const DEFAULT_SUMMARY_PROFILE_NAME: &str = "summary_local";
pub(super) const DEFAULT_PLATFORM_SUMMARY_PROFILE_NAME: &str = "summary_llm";
pub(super) const DEFAULT_PLATFORM_SUMMARY_MODEL: &str = "ministral-3-3b-instruct";
pub(super) const DEFAULT_PLATFORM_SUMMARY_API_KEY: &str = "${BITLOOPS_PLATFORM_GATEWAY_TOKEN}";
pub(super) const DEFAULT_CONTEXT_GUIDANCE_MAX_OUTPUT_TOKENS: i64 = 4096;
pub(super) const DEFAULT_CONTEXT_GUIDANCE_PROFILE_NAME: &str = "guidance_local";
pub(super) const DEFAULT_PLATFORM_CONTEXT_GUIDANCE_PROFILE_NAME: &str = "guidance_llm";
pub(super) const DEFAULT_PLATFORM_CONTEXT_GUIDANCE_MODEL: &str = "ministral-3-3b-instruct";
pub(super) const DEFAULT_ARCHITECTURE_FACT_SYNTHESIS_MAX_OUTPUT_TOKENS: i64 = 4096;
pub(super) const DEFAULT_ARCHITECTURE_ROLE_ADJUDICATION_MAX_OUTPUT_TOKENS: i64 = 1024;
pub(super) const DEFAULT_LOCAL_ARCHITECTURE_FACT_SYNTHESIS_PROFILE_NAME: &str =
    "architecture_fact_synthesis_local";
pub(super) const DEFAULT_LOCAL_ARCHITECTURE_ROLE_ADJUDICATION_PROFILE_NAME: &str =
    "architecture_role_adjudication_local";
pub(super) const DEFAULT_PLATFORM_ARCHITECTURE_FACT_SYNTHESIS_PROFILE_NAME: &str =
    "architecture_fact_synthesis";
pub(super) const DEFAULT_PLATFORM_ARCHITECTURE_ROLE_ADJUDICATION_PROFILE_NAME: &str =
    "architecture_role_adjudication";
pub(crate) const DEFAULT_PLATFORM_CONTEXT_GUIDANCE_API_KEY_ENV: &str =
    "BITLOOPS_PLATFORM_GATEWAY_TOKEN";
pub(super) const OLLAMA_CHAT_DRIVER: &str = "ollama_chat";
pub(super) const PLATFORM_CHAT_COMPLETIONS_URL_ENV: &str = "BITLOOPS_PLATFORM_CHAT_COMPLETIONS_URL";
pub(super) const PLATFORM_GATEWAY_URL_ENV: &str = "BITLOOPS_PLATFORM_GATEWAY_URL";
pub(super) const STRUCTURED_GENERATION_TASK: &str = "structured_generation";
pub(super) const TEXT_GENERATION_TASK: &str = "text_generation";

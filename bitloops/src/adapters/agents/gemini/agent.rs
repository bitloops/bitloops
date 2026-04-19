mod analysis;
mod cli_agent;
mod config;
mod hooks;
mod time;

pub use cli_agent::{GeminiCliAgent, new_gemini_agent};
pub use config::{
    GEMINI_SETTINGS_FILE_NAME, GeminiHookEntry, GeminiHookMatcher, GeminiHooks, GeminiHooksConfig,
    GeminiSettings, HOOK_NAME_AFTER_AGENT, HOOK_NAME_AFTER_MODEL, HOOK_NAME_AFTER_TOOL,
    HOOK_NAME_BEFORE_AGENT, HOOK_NAME_BEFORE_MODEL, HOOK_NAME_BEFORE_TOOL,
    HOOK_NAME_BEFORE_TOOL_SELECTION, HOOK_NAME_NOTIFICATION, HOOK_NAME_PRE_COMPRESS,
    HOOK_NAME_SESSION_END, HOOK_NAME_SESSION_START,
};

#[cfg(test)]
use super::transcript::{GeminiMessage, GeminiTranscript};

#[cfg(test)]
#[path = "skills_integration_tests.rs"]
mod skills_integration_tests;

#[cfg(test)]
#[path = "agent_tests.rs"]
mod tests;

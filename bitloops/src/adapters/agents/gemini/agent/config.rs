use serde::{Deserialize, Serialize};

pub const HOOK_NAME_SESSION_START: &str = "session-start";
pub const HOOK_NAME_SESSION_END: &str = "session-end";
pub const HOOK_NAME_BEFORE_AGENT: &str = "before-agent";
pub const HOOK_NAME_AFTER_AGENT: &str = "after-agent";
pub const HOOK_NAME_BEFORE_MODEL: &str = "before-model";
pub const HOOK_NAME_AFTER_MODEL: &str = "after-model";
pub const HOOK_NAME_BEFORE_TOOL_SELECTION: &str = "before-tool-selection";
pub const HOOK_NAME_BEFORE_TOOL: &str = "before-tool";
pub const HOOK_NAME_AFTER_TOOL: &str = "after-tool";
pub const HOOK_NAME_PRE_COMPRESS: &str = "pre-compress";
pub const HOOK_NAME_NOTIFICATION: &str = "notification";

pub const GEMINI_SETTINGS_FILE_NAME: &str = "settings.json";

pub(crate) const BITLOOPS_HOOK_PREFIXES: [&str; 2] = ["bitloops ", "cargo run -- "];

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GeminiSettings {
    #[serde(rename = "hooksConfig", default)]
    pub hooks_config: GeminiHooksConfig,
    #[serde(default)]
    pub hooks: GeminiHooks,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GeminiHooksConfig {
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GeminiHooks {
    #[serde(rename = "SessionStart", default)]
    pub session_start: Vec<GeminiHookMatcher>,
    #[serde(rename = "SessionEnd", default)]
    pub session_end: Vec<GeminiHookMatcher>,
    #[serde(rename = "BeforeAgent", default)]
    pub before_agent: Vec<GeminiHookMatcher>,
    #[serde(rename = "AfterAgent", default)]
    pub after_agent: Vec<GeminiHookMatcher>,
    #[serde(rename = "BeforeModel", default)]
    pub before_model: Vec<GeminiHookMatcher>,
    #[serde(rename = "AfterModel", default)]
    pub after_model: Vec<GeminiHookMatcher>,
    #[serde(rename = "BeforeToolSelection", default)]
    pub before_tool_selection: Vec<GeminiHookMatcher>,
    #[serde(rename = "BeforeTool", default)]
    pub before_tool: Vec<GeminiHookMatcher>,
    #[serde(rename = "AfterTool", default)]
    pub after_tool: Vec<GeminiHookMatcher>,
    #[serde(rename = "PreCompress", default)]
    pub pre_compress: Vec<GeminiHookMatcher>,
    #[serde(rename = "Notification", default)]
    pub notification: Vec<GeminiHookMatcher>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GeminiHookMatcher {
    #[serde(default)]
    pub matcher: String,
    #[serde(default)]
    pub hooks: Vec<GeminiHookEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GeminiHookEntry {
    #[serde(default)]
    pub name: String,
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub command: String,
}

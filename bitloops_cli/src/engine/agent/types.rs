use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::time::SystemTime;

#[derive(Clone, Debug, Default)]
pub struct Event;

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum HookType {
    #[default]
    SessionStart,
    SessionEnd,
    UserPromptSubmit,
    Stop,
    PreToolUse,
    PostToolUse,
}

impl HookType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SessionStart => "session_start",
            Self::SessionEnd => "session_end",
            Self::UserPromptSubmit => "user_prompt_submit",
            Self::Stop => "stop",
            Self::PreToolUse => "pre_tool_use",
            Self::PostToolUse => "post_tool_use",
        }
    }
}

#[derive(Clone, Debug)]
pub struct HookInput {
    pub hook_type: HookType,
    pub session_id: String,
    pub session_ref: String,
    pub timestamp: SystemTime,
    pub user_prompt: String,
    pub tool_name: String,
    pub tool_use_id: String,
    pub tool_input: Vec<u8>,
    pub tool_response: Vec<u8>,
    pub raw_data: HashMap<String, Value>,
}

impl Default for HookInput {
    fn default() -> Self {
        Self {
            hook_type: HookType::SessionStart,
            session_id: String::new(),
            session_ref: String::new(),
            timestamp: SystemTime::UNIX_EPOCH,
            user_prompt: String::new(),
            tool_name: String::new(),
            tool_use_id: String::new(),
            tool_input: Vec::new(),
            tool_response: Vec::new(),
            raw_data: HashMap::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct SessionChange {
    pub session_id: String,
    pub session_ref: String,
    pub event_type: HookType,
    pub timestamp: SystemTime,
}

impl Default for SessionChange {
    fn default() -> Self {
        Self {
            session_id: String::new(),
            session_ref: String::new(),
            event_type: HookType::SessionStart,
            timestamp: SystemTime::UNIX_EPOCH,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: i32,
    pub cache_creation_tokens: i32,
    pub cache_read_tokens: i32,
    pub output_tokens: i32,
    pub api_call_count: i32,
    pub subagent_tokens: Option<Box<TokenUsage>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum EntryType {
    #[default]
    User,
    Assistant,
    Tool,
    System,
}

impl EntryType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
            Self::System => "system",
        }
    }
}

pub const AGENT_NAME_CLAUDE_CODE: &str = "claude-code";
pub const AGENT_NAME_COPILOT: &str = "copilot";
pub const AGENT_NAME_CURSOR: &str = "cursor";
pub const AGENT_NAME_GEMINI: &str = "gemini";
pub const AGENT_NAME_OPEN_CODE: &str = "opencode";

pub const AGENT_TYPE_CLAUDE_CODE: &str = AGENT_NAME_CLAUDE_CODE;
pub const AGENT_TYPE_COPILOT: &str = AGENT_NAME_COPILOT;
pub const AGENT_TYPE_CURSOR: &str = AGENT_NAME_CURSOR;
pub const AGENT_TYPE_GEMINI: &str = "gemini-cli";
pub const AGENT_TYPE_OPEN_CODE: &str = AGENT_NAME_OPEN_CODE;
pub const AGENT_TYPE_UNKNOWN: &str = "unknown";

pub const DEFAULT_AGENT_NAME: &str = AGENT_NAME_CLAUDE_CODE;

pub fn canonical_agent_key(agent: &str) -> String {
    let key = agent.trim().to_ascii_lowercase();
    match key.as_str() {
        AGENT_NAME_CLAUDE_CODE => AGENT_TYPE_CLAUDE_CODE.to_string(),
        AGENT_NAME_COPILOT | "copilot-cli" | "github-copilot" => AGENT_TYPE_COPILOT.to_string(),
        AGENT_NAME_CURSOR => AGENT_TYPE_CURSOR.to_string(),
        AGENT_NAME_GEMINI | AGENT_TYPE_GEMINI => AGENT_TYPE_GEMINI.to_string(),
        AGENT_NAME_OPEN_CODE | "open-code" => AGENT_TYPE_OPEN_CODE.to_string(),
        AGENT_TYPE_UNKNOWN => AGENT_TYPE_UNKNOWN.to_string(),
        _ => key,
    }
}

pub fn agent_display_name(agent: &str) -> String {
    match canonical_agent_key(agent).as_str() {
        AGENT_TYPE_CLAUDE_CODE => "Claude Code".to_string(),
        AGENT_TYPE_COPILOT => "Copilot".to_string(),
        AGENT_TYPE_CURSOR => "Cursor".to_string(),
        AGENT_TYPE_GEMINI => "Gemini CLI".to_string(),
        AGENT_TYPE_OPEN_CODE => "OpenCode".to_string(),
        AGENT_TYPE_UNKNOWN => "Unknown Agent".to_string(),
        "" => String::new(),
        other => other.to_string(),
    }
}

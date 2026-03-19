use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CopilotHooksFile {
    #[serde(default)]
    pub version: i32,
    #[serde(default)]
    pub hooks: CopilotHooks,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CopilotHooks {
    #[serde(rename = "userPromptSubmitted", default)]
    pub user_prompt_submitted: Vec<CopilotHookEntry>,
    #[serde(rename = "sessionStart", default)]
    pub session_start: Vec<CopilotHookEntry>,
    #[serde(rename = "agentStop", default)]
    pub agent_stop: Vec<CopilotHookEntry>,
    #[serde(rename = "sessionEnd", default)]
    pub session_end: Vec<CopilotHookEntry>,
    #[serde(rename = "subagentStop", default)]
    pub subagent_stop: Vec<CopilotHookEntry>,
    #[serde(rename = "preToolUse", default)]
    pub pre_tool_use: Vec<CopilotHookEntry>,
    #[serde(rename = "postToolUse", default)]
    pub post_tool_use: Vec<CopilotHookEntry>,
    #[serde(rename = "errorOccurred", default)]
    pub error_occurred: Vec<CopilotHookEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CopilotHookEntry {
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub bash: String,
    #[serde(default)]
    pub comment: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default, rename = "timeoutSec")]
    pub timeout_sec: i32,
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct CopilotUserPromptSubmittedRaw {
    #[serde(default)]
    pub timestamp: i64,
    #[serde(default)]
    pub cwd: String,
    #[serde(default, rename = "sessionId")]
    pub session_id: String,
    #[serde(default)]
    pub prompt: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct CopilotSessionStartRaw {
    #[serde(default)]
    pub timestamp: i64,
    #[serde(default)]
    pub cwd: String,
    #[serde(default, rename = "sessionId")]
    pub session_id: String,
    #[serde(default)]
    pub source: String,
    #[serde(default, rename = "initialPrompt")]
    pub initial_prompt: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct CopilotAgentStopRaw {
    #[serde(default)]
    pub timestamp: i64,
    #[serde(default)]
    pub cwd: String,
    #[serde(default, rename = "sessionId")]
    pub session_id: String,
    #[serde(default, rename = "transcriptPath")]
    pub transcript_path: String,
    #[serde(default, rename = "stopReason")]
    pub stop_reason: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct CopilotSessionEndRaw {
    #[serde(default)]
    pub timestamp: i64,
    #[serde(default)]
    pub cwd: String,
    #[serde(default, rename = "sessionId")]
    pub session_id: String,
    #[serde(default)]
    pub reason: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct CopilotSubagentStopRaw {
    #[serde(default)]
    pub timestamp: i64,
    #[serde(default)]
    pub cwd: String,
    #[serde(default, rename = "sessionId")]
    pub session_id: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct CopilotToolHookRaw {
    #[serde(default)]
    pub timestamp: i64,
    #[serde(default)]
    pub cwd: String,
    #[serde(default, rename = "sessionId")]
    pub session_id: String,
    #[serde(default, rename = "transcriptPath")]
    pub transcript_path: String,
    #[serde(default, rename = "toolName")]
    pub tool_name: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct CopilotErrorOccurredRaw {
    #[serde(default)]
    pub timestamp: i64,
    #[serde(default)]
    pub cwd: String,
    #[serde(default, rename = "sessionId")]
    pub session_id: String,
    #[serde(default)]
    pub message: String,
}

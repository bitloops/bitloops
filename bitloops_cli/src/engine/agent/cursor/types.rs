use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CursorHooksFile {
    #[serde(default)]
    pub version: i32,
    #[serde(default)]
    pub hooks: CursorHooks,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CursorHooks {
    #[serde(rename = "sessionStart", default)]
    pub session_start: Vec<CursorHookEntry>,
    #[serde(rename = "sessionEnd", default)]
    pub session_end: Vec<CursorHookEntry>,
    #[serde(rename = "beforeSubmitPrompt", default)]
    pub before_submit_prompt: Vec<CursorHookEntry>,
    #[serde(rename = "beforeShellExecution", default)]
    pub before_shell_execution: Vec<CursorHookEntry>,
    #[serde(rename = "afterShellExecution", default)]
    pub after_shell_execution: Vec<CursorHookEntry>,
    #[serde(rename = "stop", default)]
    pub stop: Vec<CursorHookEntry>,
    #[serde(rename = "preCompact", default)]
    pub pre_compact: Vec<CursorHookEntry>,
    #[serde(rename = "subagentStart", default)]
    pub subagent_start: Vec<CursorHookEntry>,
    #[serde(rename = "subagentStop", default)]
    pub subagent_stop: Vec<CursorHookEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CursorHookEntry {
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub matcher: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct CursorSessionInfoRaw {
    #[serde(default)]
    pub conversation_id: String,
    #[serde(default)]
    pub transcript_path: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct CursorBeforeSubmitPromptRaw {
    #[serde(default)]
    pub conversation_id: String,
    #[serde(default)]
    pub transcript_path: Option<String>,
    #[serde(default)]
    pub prompt: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct CursorBeforeShellExecutionRaw {
    #[serde(default)]
    pub conversation_id: String,
    #[serde(default)]
    pub transcript_path: Option<String>,
    #[serde(default)]
    pub command: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct CursorAfterShellExecutionRaw {
    #[serde(default)]
    pub conversation_id: String,
    #[serde(default)]
    pub transcript_path: Option<String>,
    #[serde(default)]
    pub command: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct CursorSubagentRaw {
    #[serde(default)]
    pub conversation_id: String,
    #[serde(default)]
    pub transcript_path: Option<String>,
    #[serde(default)]
    pub subagent_id: String,
    #[serde(default)]
    pub subagent_type: String,
    #[serde(default)]
    pub task: String,
}

use serde::Deserialize;
use serde_json::Value;

// ── Stdin JSON input types ────────────────────────────────────────────────────

/// Used by session-start, stop, session-end.
///
#[derive(Debug, Deserialize)]
pub struct SessionInfoInput {
    pub session_id: String,
    pub transcript_path: String,
}

/// Used by user-prompt-submit.
///
#[derive(Debug, Deserialize)]
pub struct UserPromptSubmitInput {
    pub session_id: String,
    pub transcript_path: String,
    pub prompt: String,
}

/// Used by pre-task.
///
#[derive(Debug, Deserialize)]
pub struct TaskHookInput {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub transcript_path: String,
    #[serde(default)]
    pub tool_use_id: String,
    #[serde(default)]
    pub tool_input: Option<Value>,
}

/// Used by post-task.
///
#[derive(Debug, Deserialize)]
pub struct PostTaskInput {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub transcript_path: String,
    #[serde(default)]
    pub tool_use_id: String,
    #[serde(default)]
    pub tool_input: Option<Value>,
    #[serde(default)]
    pub tool_response: TaskToolResponse,
}

#[derive(Debug, Deserialize, Default)]
pub struct TaskToolResponse {
    #[serde(default, rename = "agentId")]
    pub agent_id: String,
}

/// Used by post-todo.
///
#[derive(Debug, Deserialize)]
pub struct PostTodoInput {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub transcript_path: String,
    #[serde(default)]
    pub tool_use_id: String,
    #[serde(default)]
    pub tool_name: String,
    #[serde(default)]
    pub tool_input: Option<Value>,
}

/// Used by post-todo parsing tests.
#[cfg(test)]
#[derive(Debug, Deserialize)]
pub(super) struct SubagentCheckpointHookInput {
    #[serde(default)]
    pub(super) session_id: String,
    #[serde(default)]
    pub(super) transcript_path: String,
    #[serde(default)]
    pub(super) tool_name: String,
    #[serde(default)]
    pub(super) tool_use_id: String,
    #[serde(default)]
    pub(super) tool_input: Option<Value>,
    #[serde(default)]
    pub(super) tool_response: Option<Value>,
}

use anyhow::{Result, anyhow};

use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LifecycleEventType {
    SessionStart,
    TurnStart,
    TurnEnd,
    Compaction,
    SessionEnd,
    ToolInvocationObserved,
    ToolResultObserved,
    SubagentStart,
    SubagentEnd,
    TodoCheckpoint,
    Unknown(i32),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LifecycleEvent {
    pub event_type: Option<LifecycleEventType>,
    pub session_id: String,
    pub session_ref: String,
    pub source: String,
    pub prompt: String,
    pub tool_name: String,
    pub tool_use_id: String,
    pub tool_input: Option<Value>,
    pub tool_response: Option<Value>,
    pub subagent_id: String,
    pub model: String,
    pub finalize_open_turn: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PrePromptState {
    pub transcript_offset: usize,
}

pub const UNKNOWN_SESSION_ID: &str = "unknown";

/// Session ID policy rationale, invariants, and usage rules are documented in
/// `SESSION_ID_POLICY.md` in this directory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionIdPolicy {
    Strict,
    PreserveEmpty,
    FallbackUnknown,
}

pub fn apply_session_id_policy(session_id: &str, policy: SessionIdPolicy) -> Result<String> {
    let trimmed = session_id.trim();
    match policy {
        SessionIdPolicy::Strict => {
            if trimmed.is_empty() {
                Err(anyhow!("session_id is required"))
            } else {
                Ok(trimmed.to_string())
            }
        }
        SessionIdPolicy::PreserveEmpty => Ok(trimmed.to_string()),
        SessionIdPolicy::FallbackUnknown => {
            if trimmed.is_empty() {
                Ok(UNKNOWN_SESSION_ID.to_string())
            } else {
                Ok(trimmed.to_string())
            }
        }
    }
}

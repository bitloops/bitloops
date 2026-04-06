use anyhow::{Result, anyhow};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LifecycleEventType {
    SessionStart,
    TurnStart,
    TurnEnd,
    Compaction,
    SessionEnd,
    SubagentStart,
    SubagentEnd,
    Unknown(i32),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LifecycleEvent {
    pub event_type: Option<LifecycleEventType>,
    pub session_id: String,
    pub session_ref: String,
    pub prompt: String,
    pub tool_use_id: String,
    pub subagent_id: String,
    pub model: String,
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

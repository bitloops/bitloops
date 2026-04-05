//! Agent hook runtime shared by agent adapters.
//!
//! Shared top-level hook command routing lives in `crate::host::hooks::dispatcher`.

use uuid::Uuid;

use crate::adapters::agents::{
    AGENT_NAME_CLAUDE_CODE, AGENT_NAME_CODEX, AGENT_NAME_CURSOR, AGENT_TYPE_CLAUDE_CODE,
    AGENT_TYPE_CODEX, AGENT_TYPE_CURSOR,
};
use crate::host::checkpoints::session::phase::{
    Event, NoOpActionHandler, TransitionContext, apply_transition, transition_with_context,
};
use crate::host::checkpoints::session::state::SessionState;
#[cfg(test)]
use crate::telemetry::logging;

mod handlers;
mod helpers;
mod interactions;
mod types;

#[cfg(test)]
use self::helpers::*;
#[cfg(test)]
use crate::host::checkpoints::session::backend::SessionBackend;
#[cfg(test)]
use crate::host::checkpoints::session::state::{PrePromptState, PreTaskState};
#[cfg(test)]
use crate::host::checkpoints::strategy::Strategy;
#[cfg(test)]
use anyhow::Result;
pub use handlers::*;
#[cfg(test)]
use serde_json::Value;
pub use types::*;

#[derive(Debug, Clone, Copy)]
pub struct HookAgentProfile {
    pub agent_name: &'static str,
    pub agent_type: &'static str,
}

pub const CLAUDE_HOOK_AGENT_PROFILE: HookAgentProfile = HookAgentProfile {
    agent_name: AGENT_NAME_CLAUDE_CODE,
    agent_type: AGENT_TYPE_CLAUDE_CODE,
};

pub const CURSOR_HOOK_AGENT_PROFILE: HookAgentProfile = HookAgentProfile {
    agent_name: AGENT_NAME_CURSOR,
    agent_type: AGENT_TYPE_CURSOR,
};

pub const CODEX_HOOK_AGENT_PROFILE: HookAgentProfile = HookAgentProfile {
    agent_name: AGENT_NAME_CODEX,
    agent_type: AGENT_TYPE_CODEX,
};

fn apply_session_transition(state: &mut SessionState, event: Event) {
    let result = transition_with_context(state.phase, event, TransitionContext::default());
    let mut handler = NoOpActionHandler;
    if let Err(err) = apply_transition(state, result, &mut handler) {
        eprintln!("[bitloops] Warning: session transition failed ({event}): {err}");
    }
}

fn generate_turn_id() -> String {
    let id = Uuid::new_v4().simple().to_string();
    id[..12].to_string()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "agent_runtime_tests.rs"]
mod tests;

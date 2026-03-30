use std::sync::OnceLock;

use regex::Regex;
use serde::Deserialize;

use crate::adapters::agents::{Agent, HookType};

pub const HOOK_NAME_SESSION_START: &str = "session-start";
pub const HOOK_NAME_SESSION_END: &str = "session-end";
pub const HOOK_NAME_TURN_START: &str = "turn-start";
pub const HOOK_NAME_TURN_END: &str = "turn-end";
pub const HOOK_NAME_COMPACTION: &str = "compaction";

#[derive(Debug, Default, Deserialize)]
pub(super) struct SessionInfoRaw {
    #[serde(default)]
    pub(super) session_id: String,
    #[serde(default)]
    pub(super) transcript_path: String,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct TurnStartRaw {
    #[serde(default)]
    pub(super) session_id: String,
    #[serde(default)]
    pub(super) transcript_path: String,
    #[serde(default)]
    pub(super) prompt: String,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct OpenCodeAgent;

pub fn new_open_code_agent() -> Box<dyn Agent + Send + Sync> {
    Box::new(OpenCodeAgent)
}

impl OpenCodeAgent {
    pub fn get_supported_hooks(&self) -> Vec<HookType> {
        vec![
            HookType::SessionStart,
            HookType::SessionEnd,
            HookType::UserPromptSubmit,
            HookType::Stop,
        ]
    }
}

pub fn sanitize_path_for_opencode(path: &str) -> String {
    static NON_ALPHANUMERIC: OnceLock<Regex> = OnceLock::new();
    NON_ALPHANUMERIC
        .get_or_init(|| Regex::new(r"[^a-zA-Z0-9]").expect("regex must compile"))
        .replace_all(path, "-")
        .to_string()
}

#[cfg(test)]
#[path = "open_code_agent_tests.rs"]
mod tests;

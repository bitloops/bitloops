use std::io::Read;

use anyhow::{Context, Result};

use crate::host::checkpoints::lifecycle::{
    LifecycleEvent, LifecycleEventType, SessionIdPolicy, apply_session_id_policy,
};

use super::types::parse_codex_session_info;

pub const HOOK_NAME_SESSION_START: &str = "session-start";
pub const HOOK_NAME_STOP: &str = "stop";

pub fn parse_hook_event(hook_name: &str, stdin: &mut dyn Read) -> Result<Option<LifecycleEvent>> {
    match hook_name {
        HOOK_NAME_SESSION_START => {
            let raw = parse_hook_input(stdin)?;
            Ok(Some(LifecycleEvent {
                event_type: Some(LifecycleEventType::SessionStart),
                session_id: apply_session_id_policy(&raw.session_id, SessionIdPolicy::Strict)
                    .context("codex session-start requires non-empty session_id")?,
                session_ref: raw.transcript_path,
                model: raw.model,
                ..LifecycleEvent::default()
            }))
        }
        HOOK_NAME_STOP => {
            let raw = parse_hook_input(stdin)?;
            Ok(Some(LifecycleEvent {
                event_type: Some(LifecycleEventType::TurnEnd),
                session_id: apply_session_id_policy(
                    &raw.session_id,
                    SessionIdPolicy::FallbackUnknown,
                )?,
                session_ref: raw.transcript_path,
                model: raw.model,
                ..LifecycleEvent::default()
            }))
        }
        _ => Ok(None),
    }
}

fn parse_hook_input(stdin: &mut dyn Read) -> Result<super::types::CodexSessionInfoRaw> {
    let mut raw = String::new();
    stdin
        .read_to_string(&mut raw)
        .context("reading codex hook input")?;
    parse_codex_session_info(&raw)
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;

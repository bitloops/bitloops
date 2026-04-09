use std::io::Read;

use anyhow::{Context, Result};

use crate::host::checkpoints::lifecycle::{
    LifecycleEvent, LifecycleEventType, SessionIdPolicy, apply_session_id_policy,
};

use super::types::{
    CodexSessionInfoRaw, CodexToolHookRaw, CodexUserPromptSubmitRaw, parse_codex_session_info,
    parse_codex_tool_hook, parse_codex_user_prompt_submit,
};

pub const HOOK_NAME_SESSION_START: &str =
    crate::host::checkpoints::lifecycle::adapters::CODEX_HOOK_SESSION_START;
pub const HOOK_NAME_USER_PROMPT_SUBMIT: &str =
    crate::host::checkpoints::lifecycle::adapters::CODEX_HOOK_USER_PROMPT_SUBMIT;
pub const HOOK_NAME_PRE_TOOL_USE: &str =
    crate::host::checkpoints::lifecycle::adapters::CODEX_HOOK_PRE_TOOL_USE;
pub const HOOK_NAME_POST_TOOL_USE: &str =
    crate::host::checkpoints::lifecycle::adapters::CODEX_HOOK_POST_TOOL_USE;
pub const HOOK_NAME_STOP: &str = crate::host::checkpoints::lifecycle::adapters::CODEX_HOOK_STOP;

pub fn parse_hook_event(hook_name: &str, stdin: &mut dyn Read) -> Result<Option<LifecycleEvent>> {
    match hook_name {
        HOOK_NAME_SESSION_START => {
            let raw = parse_session_info_input(stdin)?;
            Ok(Some(LifecycleEvent {
                event_type: Some(LifecycleEventType::SessionStart),
                session_id: apply_session_id_policy(&raw.session_id, SessionIdPolicy::Strict)
                    .context("codex session-start requires non-empty session_id")?,
                session_ref: raw.transcript_path,
                model: raw.model,
                ..LifecycleEvent::default()
            }))
        }
        HOOK_NAME_USER_PROMPT_SUBMIT => {
            let raw = parse_user_prompt_submit_input(stdin)?;
            Ok(Some(LifecycleEvent {
                event_type: Some(LifecycleEventType::TurnStart),
                session_id: apply_session_id_policy(&raw.session_id, SessionIdPolicy::Strict)
                    .context("codex user-prompt-submit requires non-empty session_id")?,
                session_ref: raw.transcript_path,
                prompt: raw.prompt,
                model: raw.model,
                ..LifecycleEvent::default()
            }))
        }
        HOOK_NAME_PRE_TOOL_USE | HOOK_NAME_POST_TOOL_USE => {
            let _raw = parse_tool_hook_input(stdin)?;
            Ok(None)
        }
        HOOK_NAME_STOP => {
            let raw = parse_session_info_input(stdin)?;
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

fn parse_session_info_input(stdin: &mut dyn Read) -> Result<CodexSessionInfoRaw> {
    let mut raw = String::new();
    stdin
        .read_to_string(&mut raw)
        .context("reading codex hook input")?;
    parse_codex_session_info(&raw)
}

fn parse_user_prompt_submit_input(stdin: &mut dyn Read) -> Result<CodexUserPromptSubmitRaw> {
    let mut raw = String::new();
    stdin
        .read_to_string(&mut raw)
        .context("reading codex user-prompt-submit input")?;
    parse_codex_user_prompt_submit(&raw)
}

fn parse_tool_hook_input(stdin: &mut dyn Read) -> Result<CodexToolHookRaw> {
    let mut raw = String::new();
    stdin
        .read_to_string(&mut raw)
        .context("reading codex tool hook input")?;
    parse_codex_tool_hook(&raw)
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;

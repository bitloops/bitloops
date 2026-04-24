use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result};

use crate::adapters::agents::Agent;
use crate::host::checkpoints::lifecycle::{
    LifecycleEvent, LifecycleEventType, SessionIdPolicy, apply_session_id_policy,
};
use crate::host::checkpoints::session::create_session_backend_or_local;

use super::agent::CodexAgent;
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

pub fn resolve_transcript_ref(session_id: &str, raw_path: Option<&str>) -> String {
    if let Some(path) = raw_path
        && !path.trim().is_empty()
    {
        return path.to_string();
    }

    let session_id = session_id.trim();
    if session_id.is_empty()
        || session_id == crate::host::checkpoints::lifecycle::UNKNOWN_SESSION_ID
    {
        return String::new();
    }

    if let Some(path) = resolve_transcript_ref_from_state(session_id) {
        return path;
    }

    let repo_root = crate::utils::paths::repo_root().ok();
    let repo_path = repo_root
        .as_ref()
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_default();

    let agent = CodexAgent;
    let Ok(session_dir) = agent.get_session_dir(&repo_path) else {
        return String::new();
    };
    let resolved = agent.resolve_session_file(&session_dir, session_id);
    if Path::new(&resolved).is_file() {
        return resolved;
    }

    String::new()
}

pub fn parse_hook_event(hook_name: &str, stdin: &mut dyn Read) -> Result<Option<LifecycleEvent>> {
    match hook_name {
        HOOK_NAME_SESSION_START => {
            let raw = parse_session_info_input(stdin)?;
            let session_id = apply_session_id_policy(&raw.session_id, SessionIdPolicy::Strict)
                .context("codex session-start requires non-empty session_id")?;
            Ok(Some(LifecycleEvent {
                event_type: Some(LifecycleEventType::SessionStart),
                session_id: session_id.clone(),
                session_ref: resolve_transcript_ref(&session_id, Some(&raw.transcript_path)),
                model: raw.model,
                ..LifecycleEvent::default()
            }))
        }
        HOOK_NAME_USER_PROMPT_SUBMIT => {
            let raw = parse_user_prompt_submit_input(stdin)?;
            let session_id = apply_session_id_policy(&raw.session_id, SessionIdPolicy::Strict)
                .context("codex user-prompt-submit requires non-empty session_id")?;
            Ok(Some(LifecycleEvent {
                event_type: Some(LifecycleEventType::TurnStart),
                session_id: session_id.clone(),
                session_ref: resolve_transcript_ref(&session_id, Some(&raw.transcript_path)),
                prompt: raw.prompt,
                model: raw.model,
                ..LifecycleEvent::default()
            }))
        }
        HOOK_NAME_PRE_TOOL_USE | HOOK_NAME_POST_TOOL_USE => {
            let raw = parse_tool_hook_input(stdin)?;
            let session_id = apply_session_id_policy(&raw.session_id, SessionIdPolicy::Strict)
                .context("codex tool hook requires non-empty session_id")?;
            let event_type = match (hook_name, raw.tool_name.eq_ignore_ascii_case("task")) {
                (HOOK_NAME_PRE_TOOL_USE, true) => LifecycleEventType::SubagentStart,
                (HOOK_NAME_POST_TOOL_USE, true) => LifecycleEventType::SubagentEnd,
                (HOOK_NAME_PRE_TOOL_USE, false) => LifecycleEventType::ToolInvocationObserved,
                (HOOK_NAME_POST_TOOL_USE, false) => LifecycleEventType::ToolResultObserved,
                _ => return Ok(None),
            };
            Ok(Some(LifecycleEvent {
                event_type: Some(event_type),
                session_id: session_id.clone(),
                session_ref: resolve_transcript_ref(&session_id, Some(&raw.transcript_path)),
                tool_name: raw.tool_name.clone(),
                tool_use_id: raw.tool_use_id,
                tool_input: raw.tool_input,
                tool_response: raw.tool_response.clone(),
                subagent_id: task_agent_id(raw.tool_response.as_ref()),
                model: raw.model,
                ..LifecycleEvent::default()
            }))
        }
        HOOK_NAME_STOP => {
            let raw = parse_session_info_input(stdin)?;
            let session_id =
                apply_session_id_policy(&raw.session_id, SessionIdPolicy::FallbackUnknown)?;
            Ok(Some(LifecycleEvent {
                event_type: Some(LifecycleEventType::TurnEnd),
                session_id: session_id.clone(),
                session_ref: resolve_transcript_ref(&session_id, Some(&raw.transcript_path)),
                model: raw.model,
                ..LifecycleEvent::default()
            }))
        }
        _ => Ok(None),
    }
}

fn task_agent_id(tool_response: Option<&serde_json::Value>) -> String {
    tool_response
        .and_then(|value| {
            value
                .pointer("/agentId")
                .and_then(serde_json::Value::as_str)
                .or_else(|| {
                    value
                        .pointer("/agent_id")
                        .and_then(serde_json::Value::as_str)
                })
        })
        .unwrap_or_default()
        .trim()
        .to_string()
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

fn resolve_transcript_ref_from_state(session_id: &str) -> Option<String> {
    let repo_root = crate::utils::paths::repo_root().ok()?;
    let backend = create_session_backend_or_local(&repo_root);

    if let Ok(Some(pre_prompt)) = backend.load_pre_prompt(session_id)
        && !pre_prompt.transcript_path.trim().is_empty()
    {
        return Some(pre_prompt.transcript_path);
    }

    if let Ok(Some(session)) = backend.load_session(session_id)
        && !session.transcript_path.trim().is_empty()
    {
        return Some(session.transcript_path);
    }

    None
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;

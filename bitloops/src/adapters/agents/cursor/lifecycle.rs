use anyhow::Result;

use crate::adapters::agents::Agent;
use crate::host::checkpoints::lifecycle::{
    LifecycleEvent, LifecycleEventType, SessionIdPolicy, apply_session_id_policy,
    read_and_parse_hook_input,
};
use crate::host::checkpoints::session::state::PRE_PROMPT_SOURCE_CURSOR_SHELL;

use super::agent::CursorAgent;
use super::types::{
    CursorAfterShellExecutionRaw, CursorBeforeShellExecutionRaw, CursorBeforeSubmitPromptRaw,
    CursorSessionInfoRaw, CursorSubagentRaw,
};

pub const HOOK_NAME_SESSION_START: &str = "session-start";
pub const HOOK_NAME_SESSION_END: &str = "session-end";
pub const HOOK_NAME_BEFORE_SUBMIT_PROMPT: &str = "before-submit-prompt";
pub const HOOK_NAME_BEFORE_SHELL_EXECUTION: &str = "before-shell-execution";
pub const HOOK_NAME_AFTER_SHELL_EXECUTION: &str = "after-shell-execution";
pub const HOOK_NAME_STOP: &str = "stop";
pub const HOOK_NAME_PRE_COMPACT: &str = "pre-compact";
pub const HOOK_NAME_SUBAGENT_START: &str = "subagent-start";
pub const HOOK_NAME_SUBAGENT_STOP: &str = "subagent-stop";

pub fn resolve_transcript_ref(conversation_id: &str, raw_path: Option<&str>) -> String {
    if let Some(path) = raw_path
        && !path.trim().is_empty()
    {
        return path.to_string();
    }

    let Ok(repo_root) = crate::utils::paths::repo_root() else {
        return String::new();
    };

    let agent = CursorAgent;
    let Ok(session_dir) = agent.get_session_dir(repo_root.to_string_lossy().as_ref()) else {
        return String::new();
    };
    agent.resolve_session_file(&session_dir, conversation_id)
}

pub fn parse_hook_event(
    hook_name: &str,
    stdin: &mut dyn std::io::Read,
) -> Result<Option<LifecycleEvent>> {
    match hook_name {
        HOOK_NAME_SESSION_START => {
            let raw: CursorSessionInfoRaw = read_and_parse_hook_input(stdin)?;
            let session_id =
                apply_session_id_policy(&raw.conversation_id, SessionIdPolicy::Strict)?;
            Ok(Some(LifecycleEvent {
                event_type: Some(LifecycleEventType::SessionStart),
                session_id,
                session_ref: raw.transcript_path.unwrap_or_default(),
                model: raw.model,
                ..LifecycleEvent::default()
            }))
        }
        HOOK_NAME_BEFORE_SUBMIT_PROMPT => {
            let raw: CursorBeforeSubmitPromptRaw = read_and_parse_hook_input(stdin)?;
            let session_id =
                apply_session_id_policy(&raw.conversation_id, SessionIdPolicy::Strict)?;
            Ok(Some(LifecycleEvent {
                event_type: Some(LifecycleEventType::TurnStart),
                session_id: session_id.clone(),
                session_ref: resolve_transcript_ref(&session_id, raw.transcript_path.as_deref()),
                prompt: raw.prompt,
                model: raw.model,
                ..LifecycleEvent::default()
            }))
        }
        HOOK_NAME_BEFORE_SHELL_EXECUTION => {
            let raw: CursorBeforeShellExecutionRaw = read_and_parse_hook_input(stdin)?;
            let session_id =
                apply_session_id_policy(&raw.conversation_id, SessionIdPolicy::Strict)?;
            let command = raw.command.trim();
            Ok(Some(LifecycleEvent {
                event_type: Some(LifecycleEventType::TurnStart),
                session_id: session_id.clone(),
                session_ref: resolve_transcript_ref(&session_id, raw.transcript_path.as_deref()),
                source: PRE_PROMPT_SOURCE_CURSOR_SHELL.to_string(),
                prompt: if command.is_empty() {
                    "Run shell command".to_string()
                } else {
                    format!("Run shell command: {command}")
                },
                model: raw.model,
                ..LifecycleEvent::default()
            }))
        }
        HOOK_NAME_AFTER_SHELL_EXECUTION => {
            let raw: CursorAfterShellExecutionRaw = read_and_parse_hook_input(stdin)?;
            let session_id =
                apply_session_id_policy(&raw.conversation_id, SessionIdPolicy::PreserveEmpty)?;
            Ok(Some(LifecycleEvent {
                event_type: Some(LifecycleEventType::TurnEnd),
                session_id: session_id.clone(),
                session_ref: resolve_transcript_ref(&session_id, raw.transcript_path.as_deref()),
                source: PRE_PROMPT_SOURCE_CURSOR_SHELL.to_string(),
                model: raw.model,
                ..LifecycleEvent::default()
            }))
        }
        HOOK_NAME_STOP => {
            let raw: CursorSessionInfoRaw = read_and_parse_hook_input(stdin)?;
            let session_id =
                apply_session_id_policy(&raw.conversation_id, SessionIdPolicy::FallbackUnknown)?;
            Ok(Some(LifecycleEvent {
                event_type: Some(LifecycleEventType::TurnEnd),
                session_id: session_id.clone(),
                session_ref: resolve_transcript_ref(&session_id, raw.transcript_path.as_deref()),
                model: raw.model,
                ..LifecycleEvent::default()
            }))
        }
        HOOK_NAME_SESSION_END => {
            let raw: CursorSessionInfoRaw = read_and_parse_hook_input(stdin)?;
            let session_id =
                apply_session_id_policy(&raw.conversation_id, SessionIdPolicy::PreserveEmpty)?;
            Ok(Some(LifecycleEvent {
                event_type: Some(LifecycleEventType::SessionEnd),
                session_id: session_id.clone(),
                session_ref: resolve_transcript_ref(&session_id, raw.transcript_path.as_deref()),
                model: raw.model,
                finalize_open_turn: true,
                ..LifecycleEvent::default()
            }))
        }
        HOOK_NAME_PRE_COMPACT => {
            let raw: CursorSessionInfoRaw = read_and_parse_hook_input(stdin)?;
            Ok(Some(LifecycleEvent {
                event_type: Some(LifecycleEventType::Compaction),
                session_id: apply_session_id_policy(
                    &raw.conversation_id,
                    SessionIdPolicy::PreserveEmpty,
                )?,
                session_ref: raw.transcript_path.unwrap_or_default(),
                model: raw.model,
                ..LifecycleEvent::default()
            }))
        }
        HOOK_NAME_SUBAGENT_START => {
            let raw: CursorSubagentRaw = read_and_parse_hook_input(stdin)?;
            if raw.task.trim().is_empty() {
                return Ok(None);
            }
            Ok(Some(LifecycleEvent {
                event_type: Some(LifecycleEventType::SubagentStart),
                session_id: apply_session_id_policy(
                    &raw.conversation_id,
                    SessionIdPolicy::PreserveEmpty,
                )?,
                session_ref: raw.transcript_path.unwrap_or_default(),
                tool_use_id: raw.subagent_id.clone(),
                subagent_id: raw.subagent_id,
                model: raw.model,
                ..LifecycleEvent::default()
            }))
        }
        HOOK_NAME_SUBAGENT_STOP => {
            let raw: CursorSubagentRaw = read_and_parse_hook_input(stdin)?;
            if raw.task.trim().is_empty() {
                return Ok(None);
            }
            Ok(Some(LifecycleEvent {
                event_type: Some(LifecycleEventType::SubagentEnd),
                session_id: apply_session_id_policy(
                    &raw.conversation_id,
                    SessionIdPolicy::PreserveEmpty,
                )?,
                session_ref: raw.transcript_path.unwrap_or_default(),
                tool_use_id: raw.subagent_id.clone(),
                subagent_id: raw.subagent_id,
                model: raw.model,
                ..LifecycleEvent::default()
            }))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_unknown_hook_returns_none() {
        let mut input = std::io::Cursor::new(br#"{}"#.as_slice());
        let parsed = parse_hook_event("unknown", &mut input).expect("parse");
        assert!(parsed.is_none());
    }

    #[test]
    fn parse_before_submit_prompt_maps_turn_start() {
        let mut input = std::io::Cursor::new(
            br#"{"conversation_id":"c1","transcript_path":"/tmp/t.jsonl","prompt":"hello","modelSlug":"gpt-5.4-mini"}"#
                .as_slice(),
        );
        let parsed = parse_hook_event(HOOK_NAME_BEFORE_SUBMIT_PROMPT, &mut input)
            .expect("parse")
            .expect("event");
        assert_eq!(parsed.event_type, Some(LifecycleEventType::TurnStart));
        assert_eq!(parsed.session_id, "c1");
        assert_eq!(parsed.prompt, "hello");
        assert_eq!(parsed.session_ref, "/tmp/t.jsonl");
        assert_eq!(parsed.model, "gpt-5.4-mini");
    }

    #[test]
    fn parse_before_shell_execution_maps_shell_turn_start() {
        let mut input = std::io::Cursor::new(
            br#"{"conversation_id":"c1","transcript_path":"/tmp/t.jsonl","command":"npm test","model":"gpt-5.4"}"#
                .as_slice(),
        );
        let parsed = parse_hook_event(HOOK_NAME_BEFORE_SHELL_EXECUTION, &mut input)
            .expect("parse")
            .expect("event");
        assert_eq!(parsed.event_type, Some(LifecycleEventType::TurnStart));
        assert_eq!(parsed.source, PRE_PROMPT_SOURCE_CURSOR_SHELL);
        assert_eq!(parsed.prompt, "Run shell command: npm test");
        assert_eq!(parsed.model, "gpt-5.4");
    }

    #[test]
    fn parse_session_start_rejects_empty_session_id() {
        let mut input = std::io::Cursor::new(
            br#"{"conversation_id":"   ","transcript_path":"/tmp/t.jsonl"}"#.as_slice(),
        );
        let err = parse_hook_event(HOOK_NAME_SESSION_START, &mut input).expect_err("expected err");
        assert!(
            err.to_string().contains("session_id is required"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn parse_before_submit_prompt_rejects_empty_session_id() {
        let mut input = std::io::Cursor::new(
            br#"{"conversation_id":" ","transcript_path":"/tmp/t.jsonl","prompt":"hello"}"#
                .as_slice(),
        );
        let err =
            parse_hook_event(HOOK_NAME_BEFORE_SUBMIT_PROMPT, &mut input).expect_err("expected err");
        assert!(
            err.to_string().contains("session_id is required"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn parse_stop_defaults_empty_session_id_to_unknown() {
        let mut input = std::io::Cursor::new(
            br#"{"conversation_id":" ","transcript_path":"/tmp/t.jsonl"}"#.as_slice(),
        );
        let parsed = parse_hook_event(HOOK_NAME_STOP, &mut input)
            .expect("parse")
            .expect("event");
        assert_eq!(
            parsed.session_id,
            crate::host::checkpoints::lifecycle::UNKNOWN_SESSION_ID
        );
    }

    #[test]
    fn parse_subagent_without_task_is_noop() {
        let mut input = std::io::Cursor::new(
            br#"{"conversation_id":"c1","subagent_id":"s1","task":""}"#.as_slice(),
        );
        let parsed = parse_hook_event(HOOK_NAME_SUBAGENT_START, &mut input).expect("parse");
        assert!(parsed.is_none());
    }
}

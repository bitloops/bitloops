use anyhow::Result;

use crate::engine::agent::Agent;
use crate::engine::lifecycle::{LifecycleEvent, LifecycleEventType, read_and_parse_hook_input};

use super::agent::CursorAgent;
use super::types::{CursorBeforeSubmitPromptRaw, CursorSessionInfoRaw, CursorSubagentRaw};

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

    let Ok(repo_root) = crate::engine::paths::repo_root() else {
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
            Ok(Some(LifecycleEvent {
                event_type: Some(LifecycleEventType::SessionStart),
                session_id: raw.conversation_id,
                session_ref: raw.transcript_path.unwrap_or_default(),
                ..LifecycleEvent::default()
            }))
        }
        HOOK_NAME_BEFORE_SUBMIT_PROMPT => {
            let raw: CursorBeforeSubmitPromptRaw = read_and_parse_hook_input(stdin)?;
            Ok(Some(LifecycleEvent {
                event_type: Some(LifecycleEventType::TurnStart),
                session_id: raw.conversation_id.clone(),
                session_ref: resolve_transcript_ref(
                    &raw.conversation_id,
                    raw.transcript_path.as_deref(),
                ),
                prompt: raw.prompt,
                ..LifecycleEvent::default()
            }))
        }
        HOOK_NAME_STOP => {
            let raw: CursorSessionInfoRaw = read_and_parse_hook_input(stdin)?;
            Ok(Some(LifecycleEvent {
                event_type: Some(LifecycleEventType::TurnEnd),
                session_id: raw.conversation_id.clone(),
                session_ref: resolve_transcript_ref(
                    &raw.conversation_id,
                    raw.transcript_path.as_deref(),
                ),
                ..LifecycleEvent::default()
            }))
        }
        HOOK_NAME_SESSION_END => {
            let raw: CursorSessionInfoRaw = read_and_parse_hook_input(stdin)?;
            Ok(Some(LifecycleEvent {
                event_type: Some(LifecycleEventType::SessionEnd),
                session_id: raw.conversation_id.clone(),
                session_ref: resolve_transcript_ref(
                    &raw.conversation_id,
                    raw.transcript_path.as_deref(),
                ),
                ..LifecycleEvent::default()
            }))
        }
        HOOK_NAME_PRE_COMPACT => {
            let raw: CursorSessionInfoRaw = read_and_parse_hook_input(stdin)?;
            Ok(Some(LifecycleEvent {
                event_type: Some(LifecycleEventType::Compaction),
                session_id: raw.conversation_id,
                session_ref: raw.transcript_path.unwrap_or_default(),
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
                session_id: raw.conversation_id,
                session_ref: raw.transcript_path.unwrap_or_default(),
                tool_use_id: raw.subagent_id.clone(),
                subagent_id: raw.subagent_id,
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
                session_id: raw.conversation_id,
                session_ref: raw.transcript_path.unwrap_or_default(),
                tool_use_id: raw.subagent_id.clone(),
                subagent_id: raw.subagent_id,
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
            br#"{"conversation_id":"c1","transcript_path":"/tmp/t.jsonl","prompt":"hello"}"#
                .as_slice(),
        );
        let parsed = parse_hook_event(HOOK_NAME_BEFORE_SUBMIT_PROMPT, &mut input)
            .expect("parse")
            .expect("event");
        assert_eq!(parsed.event_type, Some(LifecycleEventType::TurnStart));
        assert_eq!(parsed.session_id, "c1");
        assert_eq!(parsed.prompt, "hello");
        assert_eq!(parsed.session_ref, "/tmp/t.jsonl");
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

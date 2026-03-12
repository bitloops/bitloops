use std::io::Read;

use anyhow::Result;

use crate::engine::agent::Agent;
use crate::engine::lifecycle::{LifecycleEvent, LifecycleEventType, read_and_parse_hook_input};

use super::agent::CopilotCliAgent;
use super::types::{
    CopilotAgentStopRaw, CopilotSessionEndRaw, CopilotSessionStartRaw, CopilotSubagentStopRaw,
    CopilotUserPromptSubmittedRaw,
};

pub const HOOK_NAME_USER_PROMPT_SUBMITTED: &str = "user-prompt-submitted";
pub const HOOK_NAME_SESSION_START: &str = "session-start";
pub const HOOK_NAME_AGENT_STOP: &str = "agent-stop";
pub const HOOK_NAME_SESSION_END: &str = "session-end";
pub const HOOK_NAME_SUBAGENT_STOP: &str = "subagent-stop";
pub const HOOK_NAME_PRE_TOOL_USE: &str = "pre-tool-use";
pub const HOOK_NAME_POST_TOOL_USE: &str = "post-tool-use";
pub const HOOK_NAME_ERROR_OCCURRED: &str = "error-occurred";

pub fn resolve_transcript_ref(session_id: &str) -> String {
    if session_id.trim().is_empty() {
        return String::new();
    }

    let agent = CopilotCliAgent;
    let Ok(session_dir) = agent.get_session_dir("") else {
        return String::new();
    };
    agent.resolve_session_file(&session_dir, session_id)
}

pub fn parse_hook_event(hook_name: &str, stdin: &mut dyn Read) -> Result<Option<LifecycleEvent>> {
    match hook_name {
        HOOK_NAME_USER_PROMPT_SUBMITTED => {
            let raw: CopilotUserPromptSubmittedRaw = read_and_parse_hook_input(stdin)?;
            Ok(Some(LifecycleEvent {
                event_type: Some(LifecycleEventType::TurnStart),
                session_id: raw.session_id.clone(),
                session_ref: resolve_transcript_ref(&raw.session_id),
                prompt: raw.prompt,
                ..LifecycleEvent::default()
            }))
        }
        HOOK_NAME_SESSION_START => {
            let raw: CopilotSessionStartRaw = read_and_parse_hook_input(stdin)?;
            Ok(Some(LifecycleEvent {
                event_type: Some(LifecycleEventType::SessionStart),
                session_id: raw.session_id,
                ..LifecycleEvent::default()
            }))
        }
        HOOK_NAME_AGENT_STOP => {
            let raw: CopilotAgentStopRaw = read_and_parse_hook_input(stdin)?;
            let transcript_path = if raw.transcript_path.trim().is_empty() {
                resolve_transcript_ref(&raw.session_id)
            } else {
                raw.transcript_path
            };
            Ok(Some(LifecycleEvent {
                event_type: Some(LifecycleEventType::TurnEnd),
                session_id: raw.session_id,
                session_ref: transcript_path,
                ..LifecycleEvent::default()
            }))
        }
        HOOK_NAME_SESSION_END => {
            let raw: CopilotSessionEndRaw = read_and_parse_hook_input(stdin)?;
            let session_ref = resolve_transcript_ref(&raw.session_id);
            Ok(Some(LifecycleEvent {
                event_type: Some(LifecycleEventType::SessionEnd),
                session_id: raw.session_id,
                session_ref,
                ..LifecycleEvent::default()
            }))
        }
        HOOK_NAME_SUBAGENT_STOP => {
            let raw: CopilotSubagentStopRaw = read_and_parse_hook_input(stdin)?;
            Ok(Some(LifecycleEvent {
                event_type: Some(LifecycleEventType::SubagentEnd),
                session_id: raw.session_id,
                ..LifecycleEvent::default()
            }))
        }
        HOOK_NAME_PRE_TOOL_USE | HOOK_NAME_POST_TOOL_USE | HOOK_NAME_ERROR_OCCURRED => Ok(None),
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::process_state::with_env_var;

    #[test]
    fn resolves_transcript_path_from_session_id() {
        with_env_var(
            "BITLOOPS_TEST_COPILOT_SESSION_DIR",
            Some("/tmp/copilot-session-state"),
            || {
                assert_eq!(
                    resolve_transcript_ref("session-1"),
                    "/tmp/copilot-session-state/session-1/events.jsonl"
                );
            },
        );
    }

    #[test]
    fn parses_user_prompt_submitted_as_turn_start() {
        with_env_var(
            "BITLOOPS_TEST_COPILOT_SESSION_DIR",
            Some("/tmp/copilot-session-state"),
            || {
                let mut stdin = std::io::Cursor::new(
                    br#"{"sessionId":"session-1","prompt":"hello"}"#.as_slice(),
                );
                let event = parse_hook_event(HOOK_NAME_USER_PROMPT_SUBMITTED, &mut stdin)
                    .expect("parse")
                    .expect("event");
                assert_eq!(event.event_type, Some(LifecycleEventType::TurnStart));
                assert_eq!(event.prompt, "hello");
                assert_eq!(
                    event.session_ref,
                    "/tmp/copilot-session-state/session-1/events.jsonl"
                );
            },
        );
    }

    #[test]
    fn parses_agent_stop_as_turn_end() {
        let mut stdin = std::io::Cursor::new(
            br#"{"sessionId":"session-1","transcriptPath":"/tmp/events.jsonl"}"#.as_slice(),
        );
        let event = parse_hook_event(HOOK_NAME_AGENT_STOP, &mut stdin)
            .expect("parse")
            .expect("event");
        assert_eq!(event.event_type, Some(LifecycleEventType::TurnEnd));
        assert_eq!(event.session_ref, "/tmp/events.jsonl");
    }

    #[test]
    fn pass_through_hooks_return_none() {
        for hook_name in [
            HOOK_NAME_PRE_TOOL_USE,
            HOOK_NAME_POST_TOOL_USE,
            HOOK_NAME_ERROR_OCCURRED,
        ] {
            let mut stdin = std::io::Cursor::new(br#"{}"#.as_slice());
            assert!(
                parse_hook_event(hook_name, &mut stdin)
                    .expect("parse")
                    .is_none()
            );
        }
    }
}

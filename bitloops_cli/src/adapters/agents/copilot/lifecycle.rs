use std::io::Read;

use anyhow::Result;

use crate::adapters::agents::Agent;
use crate::host::lifecycle::{LifecycleEvent, LifecycleEventType, read_and_parse_hook_input};

use super::agent::CopilotCliAgent;
use super::transcript::{extract_model_from_events, parse_events_from_offset};
use super::types::{
    CopilotAgentStopRaw, CopilotErrorOccurredRaw, CopilotSessionEndRaw, CopilotSessionStartRaw,
    CopilotSubagentStopRaw, CopilotToolHookRaw, CopilotUserPromptSubmittedRaw,
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
                session_id: raw.session_id.clone(),
                session_ref: resolve_transcript_ref(&raw.session_id),
                prompt: raw.initial_prompt,
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
            let model = read_model_from_transcript_path(&transcript_path);
            Ok(Some(LifecycleEvent {
                event_type: Some(LifecycleEventType::TurnEnd),
                session_id: raw.session_id,
                session_ref: transcript_path,
                model,
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
        HOOK_NAME_PRE_TOOL_USE | HOOK_NAME_POST_TOOL_USE => {
            let _: CopilotToolHookRaw = read_and_parse_hook_input(stdin)?;
            Ok(None)
        }
        HOOK_NAME_ERROR_OCCURRED => {
            let _: CopilotErrorOccurredRaw = read_and_parse_hook_input(stdin)?;
            Ok(None)
        }
        _ => Ok(None),
    }
}

fn read_model_from_transcript_path(transcript_path: &str) -> String {
    if transcript_path.trim().is_empty() {
        return String::new();
    }

    let Ok(data) = std::fs::read(transcript_path) else {
        return String::new();
    };
    let Ok((events, _)) = parse_events_from_offset(&data, 0) else {
        return String::new();
    };
    extract_model_from_events(&events)
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
    fn parses_session_start_with_prompt_and_resolved_transcript() {
        with_env_var(
            "BITLOOPS_TEST_COPILOT_SESSION_DIR",
            Some("/tmp/copilot-session-state"),
            || {
                let mut stdin = std::io::Cursor::new(
                    br#"{"sessionId":"session-1","initialPrompt":"bootstrap"}"#.as_slice(),
                );
                let event = parse_hook_event(HOOK_NAME_SESSION_START, &mut stdin)
                    .expect("parse")
                    .expect("event");
                assert_eq!(event.event_type, Some(LifecycleEventType::SessionStart));
                assert_eq!(event.prompt, "bootstrap");
                assert_eq!(
                    event.session_ref,
                    "/tmp/copilot-session-state/session-1/events.jsonl"
                );
            },
        );
    }

    #[test]
    fn agent_stop_falls_back_to_resolved_transcript_path() {
        with_env_var(
            "BITLOOPS_TEST_COPILOT_SESSION_DIR",
            Some("/tmp/copilot-session-state"),
            || {
                let mut stdin = std::io::Cursor::new(
                    br#"{"sessionId":"session-1","transcriptPath":""}"#.as_slice(),
                );
                let event = parse_hook_event(HOOK_NAME_AGENT_STOP, &mut stdin)
                    .expect("parse")
                    .expect("event");
                assert_eq!(
                    event.session_ref,
                    "/tmp/copilot-session-state/session-1/events.jsonl"
                );
            },
        );
    }

    #[test]
    fn agent_stop_extracts_model_from_transcript() {
        let dir = tempfile::tempdir().expect("tempdir");
        let transcript_path = dir.path().join("events.jsonl");
        std::fs::write(
            &transcript_path,
            br#"{"type":"session.model_change","data":{"newModel":"gpt-5.1"}}
"#,
        )
        .expect("write");

        let input = format!(
            r#"{{"sessionId":"session-1","transcriptPath":"{}"}}"#,
            transcript_path.display()
        );
        let mut stdin = std::io::Cursor::new(input.into_bytes());
        let event = parse_hook_event(HOOK_NAME_AGENT_STOP, &mut stdin)
            .expect("parse")
            .expect("event");
        assert_eq!(event.model, "gpt-5.1");
    }

    #[test]
    fn agent_stop_missing_transcript_yields_empty_model() {
        let mut stdin = std::io::Cursor::new(
            br#"{"sessionId":"session-1","transcriptPath":"/tmp/missing-events.jsonl"}"#.as_slice(),
        );
        let event = parse_hook_event(HOOK_NAME_AGENT_STOP, &mut stdin)
            .expect("parse")
            .expect("event");
        assert_eq!(event.model, "");
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

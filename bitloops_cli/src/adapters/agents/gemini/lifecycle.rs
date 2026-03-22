use std::io::Read;

use anyhow::Result;
use serde::Deserialize;

use crate::host::checkpoints::lifecycle::{
    LifecycleEvent, LifecycleEventType, read_and_parse_hook_input,
};

use super::agent::{
    HOOK_NAME_AFTER_AGENT, HOOK_NAME_AFTER_MODEL, HOOK_NAME_AFTER_TOOL, HOOK_NAME_BEFORE_AGENT,
    HOOK_NAME_BEFORE_MODEL, HOOK_NAME_BEFORE_TOOL, HOOK_NAME_BEFORE_TOOL_SELECTION,
    HOOK_NAME_NOTIFICATION, HOOK_NAME_PRE_COMPRESS, HOOK_NAME_SESSION_END, HOOK_NAME_SESSION_START,
};

#[derive(Debug, Deserialize, Default)]
struct SessionHookRaw {
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    transcript_path: String,
}

#[derive(Debug, Deserialize, Default)]
struct AgentHookRaw {
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    transcript_path: String,
    #[serde(default)]
    prompt: String,
}

/// Translates a Gemini hook into a normalized lifecycle LifecycleEvent.
/// Returns None if the hook has no lifecycle significance (pass-through hooks).
pub fn parse_hook_event(hook_name: &str, stdin: &mut dyn Read) -> Result<Option<LifecycleEvent>> {
    match hook_name {
        HOOK_NAME_SESSION_START => {
            let raw: SessionHookRaw = read_and_parse_hook_input(stdin)?;
            Ok(Some(LifecycleEvent {
                event_type: Some(LifecycleEventType::SessionStart),
                session_id: raw.session_id,
                session_ref: raw.transcript_path,
                ..LifecycleEvent::default()
            }))
        }
        HOOK_NAME_BEFORE_AGENT => {
            let raw: AgentHookRaw = read_and_parse_hook_input(stdin)?;
            Ok(Some(LifecycleEvent {
                event_type: Some(LifecycleEventType::TurnStart),
                session_id: raw.session_id,
                session_ref: raw.transcript_path,
                prompt: raw.prompt,
                ..LifecycleEvent::default()
            }))
        }
        HOOK_NAME_AFTER_AGENT => {
            let raw: AgentHookRaw = read_and_parse_hook_input(stdin)?;
            Ok(Some(LifecycleEvent {
                event_type: Some(LifecycleEventType::TurnEnd),
                session_id: raw.session_id,
                session_ref: raw.transcript_path,
                ..LifecycleEvent::default()
            }))
        }
        HOOK_NAME_SESSION_END => {
            let raw: SessionHookRaw = read_and_parse_hook_input(stdin)?;
            Ok(Some(LifecycleEvent {
                event_type: Some(LifecycleEventType::SessionEnd),
                session_id: raw.session_id,
                session_ref: raw.transcript_path,
                ..LifecycleEvent::default()
            }))
        }
        HOOK_NAME_PRE_COMPRESS => {
            let raw: SessionHookRaw = read_and_parse_hook_input(stdin)?;
            Ok(Some(LifecycleEvent {
                event_type: Some(LifecycleEventType::Compaction),
                session_id: raw.session_id,
                session_ref: raw.transcript_path,
                ..LifecycleEvent::default()
            }))
        }
        HOOK_NAME_BEFORE_TOOL
        | HOOK_NAME_AFTER_TOOL
        | HOOK_NAME_BEFORE_MODEL
        | HOOK_NAME_AFTER_MODEL
        | HOOK_NAME_BEFORE_TOOL_SELECTION
        | HOOK_NAME_NOTIFICATION => Ok(None),
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_unknown_hook_returns_none() {
        let mut input = std::io::Cursor::new(br#"{}"#.as_slice());
        let result = parse_hook_event("unknown-hook", &mut input).expect("parse");
        assert!(result.is_none());
    }

    #[test]
    fn parse_session_start_maps_session_start() {
        let mut input = std::io::Cursor::new(
            br#"{"session_id":"s1","transcript_path":"/tmp/t.json"}"#.as_slice(),
        );
        let event = parse_hook_event(HOOK_NAME_SESSION_START, &mut input)
            .expect("parse")
            .expect("event");
        assert_eq!(event.event_type, Some(LifecycleEventType::SessionStart));
        assert_eq!(event.session_id, "s1");
        assert_eq!(event.session_ref, "/tmp/t.json");
        assert!(event.prompt.is_empty());
    }

    #[test]
    fn parse_before_agent_maps_turn_start_with_prompt() {
        let mut input = std::io::Cursor::new(
            br#"{"session_id":"s1","transcript_path":"/tmp/t.json","prompt":"hello world"}"#
                .as_slice(),
        );
        let event = parse_hook_event(HOOK_NAME_BEFORE_AGENT, &mut input)
            .expect("parse")
            .expect("event");
        assert_eq!(event.event_type, Some(LifecycleEventType::TurnStart));
        assert_eq!(event.session_id, "s1");
        assert_eq!(event.session_ref, "/tmp/t.json");
        assert_eq!(event.prompt, "hello world");
    }

    #[test]
    fn parse_after_agent_maps_turn_end() {
        let mut input = std::io::Cursor::new(
            br#"{"session_id":"s1","transcript_path":"/tmp/t.json","prompt":"ignored"}"#.as_slice(),
        );
        let event = parse_hook_event(HOOK_NAME_AFTER_AGENT, &mut input)
            .expect("parse")
            .expect("event");
        assert_eq!(event.event_type, Some(LifecycleEventType::TurnEnd));
        assert_eq!(event.session_id, "s1");
        assert_eq!(event.session_ref, "/tmp/t.json");
    }

    #[test]
    fn parse_session_end_maps_session_end() {
        let mut input = std::io::Cursor::new(
            br#"{"session_id":"s1","transcript_path":"/tmp/t.json"}"#.as_slice(),
        );
        let event = parse_hook_event(HOOK_NAME_SESSION_END, &mut input)
            .expect("parse")
            .expect("event");
        assert_eq!(event.event_type, Some(LifecycleEventType::SessionEnd));
    }

    #[test]
    fn parse_pre_compress_maps_compaction() {
        let mut input = std::io::Cursor::new(
            br#"{"session_id":"s1","transcript_path":"/tmp/t.json"}"#.as_slice(),
        );
        let event = parse_hook_event(HOOK_NAME_PRE_COMPRESS, &mut input)
            .expect("parse")
            .expect("event");
        assert_eq!(event.event_type, Some(LifecycleEventType::Compaction));
    }

    #[test]
    fn pass_through_hooks_return_none() {
        for hook in &[
            HOOK_NAME_BEFORE_TOOL,
            HOOK_NAME_AFTER_TOOL,
            HOOK_NAME_BEFORE_MODEL,
            HOOK_NAME_AFTER_MODEL,
            HOOK_NAME_BEFORE_TOOL_SELECTION,
            HOOK_NAME_NOTIFICATION,
        ] {
            let mut input = std::io::Cursor::new(br#"{}"#.as_slice());
            let result = parse_hook_event(hook, &mut input).expect("parse");
            assert!(result.is_none(), "expected None for hook {hook}");
        }
    }
}

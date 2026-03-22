use super::*;
use crate::host::lifecycle::LifecycleEventType;

#[test]
fn parse_unknown_hook_returns_none() {
    let mut input = std::io::Cursor::new(br#"{}"#.as_slice());
    let parsed = parse_hook_event("unknown", &mut input).expect("parse");
    assert!(parsed.is_none());
}

#[test]
fn parse_session_start_maps_session_start_event() {
    let mut input = std::io::Cursor::new(
        br#"{"session_id":"codex-session-1","transcript_path":"/tmp/codex-1.jsonl"}"#.as_slice(),
    );
    let parsed = parse_hook_event(HOOK_NAME_SESSION_START, &mut input)
        .expect("parse")
        .expect("event");
    assert_eq!(parsed.event_type, Some(LifecycleEventType::SessionStart));
    assert_eq!(parsed.session_id, "codex-session-1");
    assert_eq!(parsed.session_ref, "/tmp/codex-1.jsonl");
}

#[test]
fn parse_stop_maps_turn_end_event() {
    let mut input = std::io::Cursor::new(
        br#"{"sessionId":"codex-session-2","transcriptPath":"/tmp/codex-2.jsonl"}"#.as_slice(),
    );
    let parsed = parse_hook_event(HOOK_NAME_STOP, &mut input)
        .expect("parse")
        .expect("event");
    assert_eq!(parsed.event_type, Some(LifecycleEventType::TurnEnd));
    assert_eq!(parsed.session_id, "codex-session-2");
    assert_eq!(parsed.session_ref, "/tmp/codex-2.jsonl");
}

#[test]
fn parse_session_start_rejects_empty_session_id() {
    let mut input = std::io::Cursor::new(br#"{"transcript_path":"/tmp/codex-3.jsonl"}"#.as_slice());
    let err = parse_hook_event(HOOK_NAME_SESSION_START, &mut input).expect_err("expected error");
    assert!(
        err.to_string()
            .contains("codex session-start requires non-empty session_id"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn parse_stop_defaults_empty_session_id_to_unknown() {
    let mut input = std::io::Cursor::new(br#"{"transcript_path":"/tmp/codex-4.jsonl"}"#.as_slice());
    let parsed = parse_hook_event(HOOK_NAME_STOP, &mut input)
        .expect("parse")
        .expect("event");
    assert_eq!(parsed.event_type, Some(LifecycleEventType::TurnEnd));
    assert_eq!(
        parsed.session_id,
        crate::host::lifecycle::UNKNOWN_SESSION_ID
    );
    assert_eq!(parsed.session_ref, "/tmp/codex-4.jsonl");
}

#[test]
fn parse_invalid_payload_errors() {
    let mut input = std::io::Cursor::new(br#"{"session_id":"broken""#.as_slice());
    let err = parse_hook_event(HOOK_NAME_SESSION_START, &mut input).expect_err("expected error");
    assert!(
        err.to_string().contains("failed to parse codex hook input"),
        "unexpected error: {err:#}"
    );
}

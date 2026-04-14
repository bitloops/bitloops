use super::*;
use crate::host::checkpoints::lifecycle::LifecycleEventType;

#[test]
fn parse_unknown_hook_returns_none() {
    let mut input = std::io::Cursor::new(br#"{}"#.as_slice());
    let parsed = parse_hook_event("unknown", &mut input).expect("parse");
    assert!(parsed.is_none());
}

#[test]
fn parse_session_start_maps_session_start_event() {
    let mut input = std::io::Cursor::new(
        br#"{"session_id":"codex-session-1","transcript_path":"/tmp/codex-1.jsonl","modelSlug":"gpt-5.4-codex"}"#.as_slice(),
    );
    let parsed = parse_hook_event(HOOK_NAME_SESSION_START, &mut input)
        .expect("parse")
        .expect("event");
    assert_eq!(parsed.event_type, Some(LifecycleEventType::SessionStart));
    assert_eq!(parsed.session_id, "codex-session-1");
    assert_eq!(parsed.session_ref, "/tmp/codex-1.jsonl");
    assert_eq!(parsed.model, "gpt-5.4-codex");
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
fn parse_user_prompt_submit_maps_turn_start_event() {
    let mut input = std::io::Cursor::new(
        br#"{"session_id":"codex-session-3","transcript_path":"/tmp/codex-3.jsonl","prompt":"Refactor tracked file","model":"gpt-5.4-codex"}"#.as_slice(),
    );
    let parsed = parse_hook_event(HOOK_NAME_USER_PROMPT_SUBMIT, &mut input)
        .expect("parse")
        .expect("event");
    assert_eq!(parsed.event_type, Some(LifecycleEventType::TurnStart));
    assert_eq!(parsed.session_id, "codex-session-3");
    assert_eq!(parsed.session_ref, "/tmp/codex-3.jsonl");
    assert_eq!(parsed.prompt, "Refactor tracked file");
    assert_eq!(parsed.model, "gpt-5.4-codex");
}

#[test]
fn parse_user_prompt_submit_rejects_empty_session_id() {
    let mut input = std::io::Cursor::new(
        br#"{"prompt":"Refactor tracked file","transcript_path":"/tmp/codex-4.jsonl"}"#.as_slice(),
    );
    let err =
        parse_hook_event(HOOK_NAME_USER_PROMPT_SUBMIT, &mut input).expect_err("expected error");
    assert!(
        err.to_string()
            .contains("codex user-prompt-submit requires non-empty session_id"),
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
        crate::host::checkpoints::lifecycle::UNKNOWN_SESSION_ID
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

#[test]
fn parse_pre_tool_use_accepts_bash_payload_and_returns_none() {
    let mut input = std::io::Cursor::new(
        br#"{"session_id":"codex-session-ptu","transcript_path":"/tmp/codex-ptu.jsonl","tool_name":"Bash","tool_use_id":"toolu_1","tool_input":{"command":"git status"}}"#.as_slice(),
    );
    let parsed = parse_hook_event(HOOK_NAME_PRE_TOOL_USE, &mut input).expect("parse");
    assert!(
        parsed.is_none(),
        "tool hooks should be parsed but not mapped to lifecycle events yet"
    );
}

#[test]
fn parse_post_tool_use_accepts_bash_payload_and_returns_none() {
    let mut input = std::io::Cursor::new(
        br#"{"session_id":"codex-session-post","transcript_path":"/tmp/codex-post.jsonl","tool_name":"Bash","tool_use_id":"toolu_2","tool_input":{"command":"git status"},"tool_response":"clean"}"#.as_slice(),
    );
    let parsed = parse_hook_event(HOOK_NAME_POST_TOOL_USE, &mut input).expect("parse");
    assert!(
        parsed.is_none(),
        "tool hooks should be parsed but not mapped to lifecycle events yet"
    );
}

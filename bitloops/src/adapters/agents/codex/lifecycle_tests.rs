use super::*;
use crate::host::checkpoints::lifecycle::LifecycleEventType;
use crate::test_support::process_state::with_env_var;

fn seed_date_sharded_codex_session(session_id: &str) -> (tempfile::TempDir, String, String) {
    let root = tempfile::tempdir().expect("tempdir");
    let sessions_dir = root.path().join("sessions");
    let day_dir = sessions_dir.join("2026").join("04").join("16");
    std::fs::create_dir_all(&day_dir).expect("create session dir");
    let transcript_path = day_dir.join(format!("rollout-2026-04-16T16-03-59-{session_id}.jsonl"));
    std::fs::write(&transcript_path, "{}\n").expect("write transcript");
    std::fs::write(
        root.path().join("session_index.jsonl"),
        format!(
            r#"{{"id":"{session_id}","thread_name":"Investigate checkpoint loss","updated_at":"2026-04-16T13:04:37.997446Z"}}"#
        ),
    )
    .expect("write session index");

    (
        root,
        sessions_dir.to_string_lossy().to_string(),
        transcript_path.to_string_lossy().to_string(),
    )
}

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
fn parse_session_start_resolves_missing_transcript_path_from_date_sharded_store() {
    let session_id = "019d9664-2636-79c0-9658-f76bfb8af4b4";
    let (_root, session_dir, transcript_path) = seed_date_sharded_codex_session(session_id);

    with_env_var(
        "BITLOOPS_TEST_CODEX_SESSION_DIR",
        Some(&session_dir),
        || {
            let mut input =
                std::io::Cursor::new(format!(r#"{{"sessionId":"{session_id}"}}"#).into_bytes());
            let parsed = parse_hook_event(HOOK_NAME_SESSION_START, &mut input)
                .expect("parse")
                .expect("event");
            assert_eq!(parsed.session_ref, transcript_path);
        },
    );
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
fn parse_stop_resolves_missing_transcript_path_from_date_sharded_store() {
    let session_id = "019d9664-2636-79c0-9658-f76bfb8af4b4";
    let (_root, session_dir, transcript_path) = seed_date_sharded_codex_session(session_id);

    with_env_var(
        "BITLOOPS_TEST_CODEX_SESSION_DIR",
        Some(&session_dir),
        || {
            let mut input =
                std::io::Cursor::new(format!(r#"{{"sessionId":"{session_id}"}}"#).into_bytes());
            let parsed = parse_hook_event(HOOK_NAME_STOP, &mut input)
                .expect("parse")
                .expect("event");
            assert_eq!(parsed.session_ref, transcript_path);
        },
    );
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
fn parse_user_prompt_submit_resolves_missing_transcript_path_from_date_sharded_store() {
    let session_id = "019d9664-2636-79c0-9658-f76bfb8af4b4";
    let (_root, session_dir, transcript_path) = seed_date_sharded_codex_session(session_id);

    with_env_var(
        "BITLOOPS_TEST_CODEX_SESSION_DIR",
        Some(&session_dir),
        || {
            let mut input = std::io::Cursor::new(
                format!(r#"{{"sessionId":"{session_id}","prompt":"Refactor tracked file"}}"#)
                    .into_bytes(),
            );
            let parsed = parse_hook_event(HOOK_NAME_USER_PROMPT_SUBMIT, &mut input)
                .expect("parse")
                .expect("event");
            assert_eq!(parsed.session_ref, transcript_path);
        },
    );
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
fn parse_pre_tool_use_accepts_bash_payload_and_maps_tool_invocation_event() {
    let mut input = std::io::Cursor::new(
        br#"{"session_id":"codex-session-ptu","transcript_path":"/tmp/codex-ptu.jsonl","tool_name":"Bash","tool_use_id":"toolu_1","tool_input":{"command":"git status"}}"#.as_slice(),
    );
    let parsed = parse_hook_event(HOOK_NAME_PRE_TOOL_USE, &mut input)
        .expect("parse")
        .expect("event");
    assert_eq!(
        parsed.event_type,
        Some(LifecycleEventType::ToolInvocationObserved)
    );
    assert_eq!(parsed.tool_name, "Bash");
    assert_eq!(parsed.tool_use_id, "toolu_1");
    assert_eq!(
        parsed.tool_input,
        Some(serde_json::json!({"command":"git status"}))
    );
}

#[test]
fn parse_post_tool_use_accepts_bash_payload_and_maps_tool_result_event() {
    let mut input = std::io::Cursor::new(
        br#"{"session_id":"codex-session-post","transcript_path":"/tmp/codex-post.jsonl","tool_name":"Bash","tool_use_id":"toolu_2","tool_input":{"command":"git status"},"tool_response":"clean"}"#.as_slice(),
    );
    let parsed = parse_hook_event(HOOK_NAME_POST_TOOL_USE, &mut input)
        .expect("parse")
        .expect("event");
    assert_eq!(
        parsed.event_type,
        Some(LifecycleEventType::ToolResultObserved)
    );
    assert_eq!(parsed.tool_name, "Bash");
    assert_eq!(parsed.tool_use_id, "toolu_2");
    assert_eq!(parsed.tool_response, Some(serde_json::json!("clean")));
}

// --- Codex Desktop auxiliary-rollout gating ---------------------------------
//
// Codex Desktop fires hooks for its own background rollouts (title generation,
// ambient suggestions, internal probes) but never writes a JSONL transcript
// for those session_ids. It signals this by sending `transcript_path: null`
// in the hook payload. `parse_hook_event` should still produce a
// `LifecycleEvent`, but it must mark the row as auxiliary so broad dashboard
// views can filter it while preserving the session for debugging.

#[test]
fn parse_session_start_with_null_transcript_path_marks_auxiliary() {
    let mut input = std::io::Cursor::new(
        br#"{"session_id":"019e3f97-2b1f-7930-b6f1-7cdbbc385553","transcript_path":null,"cwd":"/workspace","hook_event_name":"SessionStart","model":"gpt-5.4-mini","permission_mode":"bypassPermissions","source":"startup"}"#.as_slice(),
    );
    let parsed = parse_hook_event(HOOK_NAME_SESSION_START, &mut input)
        .expect("parse")
        .expect("event");
    assert_eq!(parsed.event_type, Some(LifecycleEventType::SessionStart));
    assert_eq!(parsed.session_id, "019e3f97-2b1f-7930-b6f1-7cdbbc385553");
    assert!(parsed.is_auxiliary);
}

#[test]
fn parse_user_prompt_submit_with_null_transcript_path_marks_auxiliary() {
    // This payload mirrors what Codex Desktop sends for its title-generation
    // pass — `transcript_path: null` plus an internal "You are a helpful
    // assistant…" prompt. It should still dispatch, but the event must be
    // flagged as auxiliary so the dashboard can filter it.
    let mut input = std::io::Cursor::new(
        br#"{"session_id":"019e3f97-2b1f-7930-b6f1-7cdbbc385553","turn_id":"019e3f97-2b61-7083-8f79-826b8614c848","transcript_path":null,"cwd":"/workspace","hook_event_name":"UserPromptSubmit","model":"gpt-5.4-mini","permission_mode":"bypassPermissions","prompt":"You are a helpful assistant. You will be presented with a user prompt..."}"#.as_slice(),
    );
    let parsed = parse_hook_event(HOOK_NAME_USER_PROMPT_SUBMIT, &mut input)
        .expect("parse")
        .expect("event");
    assert_eq!(parsed.event_type, Some(LifecycleEventType::TurnStart));
    assert!(parsed.is_auxiliary);
}

#[test]
fn parse_stop_with_null_transcript_path_marks_auxiliary() {
    let mut input = std::io::Cursor::new(
        br#"{"session_id":"019e3f97-2b1f-7930-b6f1-7cdbbc385553","turn_id":"019e3f97-2b61-7083-8f79-826b8614c848","transcript_path":null,"cwd":"/workspace","hook_event_name":"Stop","model":"gpt-5.4-mini","permission_mode":"bypassPermissions","stop_hook_active":false}"#.as_slice(),
    );
    let parsed = parse_hook_event(HOOK_NAME_STOP, &mut input)
        .expect("parse")
        .expect("event");
    assert_eq!(parsed.event_type, Some(LifecycleEventType::TurnEnd));
    assert!(parsed.is_auxiliary);
}

#[test]
fn parse_pre_tool_use_with_null_transcript_path_marks_auxiliary() {
    let mut input = std::io::Cursor::new(
        br#"{"session_id":"019e3f97-2b1f-7930-b6f1-7cdbbc385553","turn_id":"t-1","transcript_path":null,"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"pwd"},"tool_use_id":"call_1"}"#.as_slice(),
    );
    let parsed = parse_hook_event(HOOK_NAME_PRE_TOOL_USE, &mut input)
        .expect("parse")
        .expect("event");
    assert_eq!(
        parsed.event_type,
        Some(LifecycleEventType::ToolInvocationObserved)
    );
    assert!(parsed.is_auxiliary);
}

#[test]
fn parse_post_tool_use_with_null_transcript_path_marks_auxiliary() {
    let mut input = std::io::Cursor::new(
        br#"{"session_id":"019e3f97-2b1f-7930-b6f1-7cdbbc385553","turn_id":"t-1","transcript_path":null,"hook_event_name":"PostToolUse","tool_name":"Bash","tool_use_id":"call_1","tool_input":{"command":"pwd"},"tool_response":"ok"}"#.as_slice(),
    );
    let parsed = parse_hook_event(HOOK_NAME_POST_TOOL_USE, &mut input)
        .expect("parse")
        .expect("event");
    assert_eq!(
        parsed.event_type,
        Some(LifecycleEventType::ToolResultObserved)
    );
    assert!(parsed.is_auxiliary);
}

#[test]
fn parse_session_start_with_string_transcript_path_still_dispatches() {
    // Real Codex sessions always carry a string transcript_path. The gate
    // must not affect them. This is the exact shape captured from Codex
    // Desktop for the "update readme title to codex" user prompt.
    let mut input = std::io::Cursor::new(
        br#"{"session_id":"019e3f96-d775-7082-8227-bef62830a23e","transcript_path":"/Users/wayneomoga/.codex/sessions/2026/05/19/rollout-2026-05-19T12-35-10-019e3f96-d775-7082-8227-bef62830a23e.jsonl","cwd":"/workspace","hook_event_name":"SessionStart","model":"gpt-5.4","permission_mode":"default","source":"startup"}"#.as_slice(),
    );
    let parsed = parse_hook_event(HOOK_NAME_SESSION_START, &mut input)
        .expect("parse")
        .expect("event");
    assert_eq!(parsed.event_type, Some(LifecycleEventType::SessionStart));
    assert_eq!(parsed.session_id, "019e3f96-d775-7082-8227-bef62830a23e");
    assert_eq!(parsed.model, "gpt-5.4");
    assert!(!parsed.is_auxiliary);
}

#[test]
fn parse_unknown_hook_with_invalid_payload_still_returns_none() {
    let mut input = std::io::Cursor::new(br#"{"session_id":"broken""#.as_slice());
    let parsed = parse_hook_event("unknown", &mut input).expect("unknown hooks should be ignored");
    assert!(parsed.is_none());
}

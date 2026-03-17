use super::adapters::{
    CLAUDE_HOOK_POST_TASK, CLAUDE_HOOK_POST_TODO, CLAUDE_HOOK_PRE_TASK, CLAUDE_HOOK_SESSION_END,
    CLAUDE_HOOK_SESSION_START, CLAUDE_HOOK_STOP, CLAUDE_HOOK_USER_PROMPT_SUBMIT,
    CODEX_HOOK_SESSION_START, CODEX_HOOK_STOP, COPILOT_HOOK_AGENT_STOP, COPILOT_HOOK_SESSION_END,
    COPILOT_HOOK_SESSION_START, COPILOT_HOOK_SUBAGENT_STOP, COPILOT_HOOK_USER_PROMPT_SUBMITTED,
    CURSOR_HOOK_BEFORE_SUBMIT_PROMPT, CURSOR_HOOK_PRE_COMPACT, CURSOR_HOOK_SESSION_END,
    CURSOR_HOOK_SESSION_START, CURSOR_HOOK_STOP, CURSOR_HOOK_SUBAGENT_START,
    CURSOR_HOOK_SUBAGENT_STOP, GEMINI_HOOK_AFTER_AGENT, GEMINI_HOOK_AFTER_MODEL,
    GEMINI_HOOK_AFTER_TOOL, GEMINI_HOOK_BEFORE_AGENT, GEMINI_HOOK_BEFORE_MODEL,
    GEMINI_HOOK_BEFORE_TOOL, GEMINI_HOOK_BEFORE_TOOL_SELECTION, GEMINI_HOOK_NOTIFICATION,
    GEMINI_HOOK_PRE_COMPRESS, GEMINI_HOOK_SESSION_END, GEMINI_HOOK_SESSION_START,
    OPENCODE_HOOK_COMPACTION, OPENCODE_HOOK_SESSION_END, OPENCODE_HOOK_SESSION_START,
    OPENCODE_HOOK_TURN_END, OPENCODE_HOOK_TURN_START,
};
use super::adapters::{
    ClaudeCodeLifecycleAdapter, CodexLifecycleAdapter, CopilotCliLifecycleAdapter,
    CursorLifecycleAdapter, GeminiCliLifecycleAdapter, OpenCodeLifecycleAdapter,
};
use super::{
    LifecycleAgentAdapter, LifecycleEvent, LifecycleEventType, PrePromptState, SessionIdPolicy,
    UNKNOWN_SESSION_ID, apply_session_id_policy, create_context_file, dispatch_lifecycle_event,
    handle_lifecycle_compaction, handle_lifecycle_session_end, handle_lifecycle_session_start,
    handle_lifecycle_subagent_end, handle_lifecycle_subagent_start, handle_lifecycle_turn_end,
    handle_lifecycle_turn_start, read_and_parse_hook_input, resolve_transcript_offset,
};

use crate::engine::session::create_session_backend_or_local;
use crate::engine::session::phase::SessionPhase;
use crate::engine::session::state::SessionState;
use crate::test_support::git_fixtures::ensure_test_store_backends;
use crate::test_support::process_state::{git_command, with_cwd, with_git_env_cleared};
use serde::Deserialize;
use std::collections::HashSet;
use std::io::Cursor;

fn sample_event(event_type: LifecycleEventType) -> LifecycleEvent {
    LifecycleEvent {
        event_type: Some(event_type),
        session_id: String::from("session-123"),
        session_ref: String::from("/tmp/transcript.jsonl"),
        prompt: String::from("hello"),
        tool_use_id: String::from("toolu_123"),
        subagent_id: String::from("subagent-1"),
    }
}

#[test]
fn test_apply_session_id_policy_strict_rejects_empty() {
    let err = apply_session_id_policy("  ", SessionIdPolicy::Strict).expect_err("expected error");
    assert!(err.to_string().contains("session_id is required"));
}

#[test]
fn test_apply_session_id_policy_turn_end_fallback_uses_unknown() {
    let session_id = apply_session_id_policy("", SessionIdPolicy::FallbackUnknown).expect("policy");
    assert_eq!(session_id, UNKNOWN_SESSION_ID);
}

fn setup_git_repo(dir: &tempfile::TempDir) {
    let run = |args: &[&str]| {
        let out = git_command()
            .args(args)
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(out.status.success(), "git {:?} failed", args);
    };
    run(&["init"]);
    run(&["config", "user.email", "t@t.com"]);
    run(&["config", "user.name", "Test"]);
    std::fs::write(dir.path().join("README.md"), "init").unwrap();
    run(&["add", "."]);
    run(&["commit", "-m", "initial"]);
    ensure_test_store_backends(dir.path());
}

// CLI-866
#[test]
fn test_dispatch_lifecycle_event_nil_agent() {
    let event = sample_event(LifecycleEventType::TurnStart);
    let err = dispatch_lifecycle_event(None, Some(&event)).unwrap_err();
    assert!(err.to_string().contains("agent is required"));
}

#[test]
fn test_dispatch_lifecycle_event_nil_event() {
    let adapter = ClaudeCodeLifecycleAdapter;
    let err = dispatch_lifecycle_event(Some(&adapter), None).unwrap_err();
    assert!(err.to_string().contains("event is required"));
}

#[test]
fn test_dispatch_lifecycle_event_unknown_event_type() {
    let adapter = ClaudeCodeLifecycleAdapter;
    let event = sample_event(LifecycleEventType::Unknown(999));
    let err = dispatch_lifecycle_event(Some(&adapter), Some(&event)).unwrap_err();
    assert!(err.to_string().contains("unknown lifecycle event type"));
}

// CLI-867
#[test]
fn test_handle_lifecycle_session_start_empty_session_id() {
    let adapter = ClaudeCodeLifecycleAdapter;
    let mut event = sample_event(LifecycleEventType::SessionStart);
    event.session_id.clear();

    let err = handle_lifecycle_session_start(&adapter, &event).unwrap_err();
    assert!(err.to_string().contains("no session_id"));
}

#[test]
fn test_handle_lifecycle_turn_start_empty_session_id() {
    let adapter = ClaudeCodeLifecycleAdapter;
    let mut event = sample_event(LifecycleEventType::TurnStart);
    event.session_id.clear();

    let err = handle_lifecycle_turn_start(&adapter, &event).unwrap_err();
    assert!(err.to_string().contains("no session_id"));
}

// CLI-868
#[test]
fn test_handle_lifecycle_turn_end_empty_transcript_ref() {
    let adapter = ClaudeCodeLifecycleAdapter;
    let mut event = sample_event(LifecycleEventType::TurnEnd);
    event.session_ref.clear();

    let err = handle_lifecycle_turn_end(&adapter, &event).unwrap_err();
    assert!(err.to_string().contains("transcript file not specified"));
}

#[test]
fn test_handle_lifecycle_turn_end_nonexistent_transcript() {
    let adapter = ClaudeCodeLifecycleAdapter;
    let mut event = sample_event(LifecycleEventType::TurnEnd);
    event.session_ref = String::from("/nonexistent/path/to/transcript.jsonl");

    let err = handle_lifecycle_turn_end(&adapter, &event).unwrap_err();
    assert!(err.to_string().contains("transcript file not found"));
}

#[test]
fn test_handle_lifecycle_turn_end_empty_repository() {
    let dir = tempfile::tempdir().unwrap();
    with_git_env_cleared(|| {
        let init = git_command()
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(init.status.success(), "git init failed");
    });
    std::fs::write(dir.path().join("transcript.jsonl"), "{}\n").unwrap();

    let adapter = ClaudeCodeLifecycleAdapter;
    let mut event = sample_event(LifecycleEventType::TurnEnd);
    event.session_ref = dir
        .path()
        .join("transcript.jsonl")
        .to_string_lossy()
        .to_string();

    with_cwd(dir.path(), || {
        let err = handle_lifecycle_turn_end(&adapter, &event).unwrap_err();
        assert!(err.to_string().contains("empty repository"));
    });
}

// CLI-869
#[test]
fn test_handle_lifecycle_compaction_resets_transcript_offset() {
    let adapter = ClaudeCodeLifecycleAdapter;
    let event = sample_event(LifecycleEventType::Compaction);

    handle_lifecycle_compaction(&adapter, &event)
        .expect("compaction should reset transcript offset and succeed");
}

#[test]
fn test_handle_lifecycle_compaction_applies_phase_transition_and_persists_reset() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    with_cwd(dir.path(), || {
        let backend = create_session_backend_or_local(dir.path());
        backend
            .save_session(&SessionState {
                session_id: "session-compaction".to_string(),
                phase: SessionPhase::Active,
                files_touched: vec!["tracked.txt".to_string()],
                checkpoint_transcript_start: 77,
                ..Default::default()
            })
            .unwrap();

        let adapter = ClaudeCodeLifecycleAdapter;
        let mut event = sample_event(LifecycleEventType::Compaction);
        event.session_id = "session-compaction".to_string();

        handle_lifecycle_compaction(&adapter, &event)
            .expect("compaction should update phase state and reset transcript offset");

        let saved = backend
            .load_session("session-compaction")
            .unwrap()
            .expect("session should still exist");
        assert_eq!(saved.phase, SessionPhase::Active);
        assert_eq!(saved.checkpoint_transcript_start, 0);
        assert!(saved.last_interaction_time.is_some());
    });
}

#[test]
fn test_handle_lifecycle_session_end_empty_session_id() {
    let adapter = ClaudeCodeLifecycleAdapter;
    let mut event = sample_event(LifecycleEventType::SessionEnd);
    event.session_id.clear();

    handle_lifecycle_session_end(&adapter, &event)
        .expect("session end with empty session id should be a safe no-op");
}

// CLI-870
#[test]
fn test_resolve_transcript_offset_prefers_pre_prompt_state() {
    let pre_state = PrePromptState {
        transcript_offset: 42,
    };

    assert_eq!(
        42,
        resolve_transcript_offset(Some(&pre_state), "session-123")
    );
}

#[test]
fn test_resolve_transcript_offset_nil_pre_prompt_state() {
    assert_eq!(0, resolve_transcript_offset(None, "session-123"));
}

#[test]
fn test_resolve_transcript_offset_zero_offset_in_pre_prompt_state() {
    let pre_state = PrePromptState {
        transcript_offset: 0,
    };

    assert_eq!(
        0,
        resolve_transcript_offset(Some(&pre_state), "session-123")
    );
}

// CLI-871
#[test]
fn test_create_context_file_format() {
    let temp_dir = tempfile::tempdir().unwrap();
    let path = temp_dir.path().join("context.md");

    create_context_file(
        &path,
        "feat: add philosophy",
        "session-123",
        &[
            String::from("What is the meaning of life?"),
            String::from("Follow-up"),
        ],
        "Summary text",
    )
    .unwrap();

    let content = std::fs::read_to_string(path).unwrap();
    assert!(content.contains("# Session Context"));
    assert!(content.contains("Session ID: session-123"));
    assert!(content.contains("Commit Message: feat: add philosophy"));
    assert!(content.contains("## Prompts"));
    assert!(content.contains("### Prompt 1"));
    assert!(content.contains("### Prompt 2"));
    assert!(content.contains("## Summary"));
}

#[test]
fn test_create_context_file_empty_prompts() {
    let temp_dir = tempfile::tempdir().unwrap();
    let path = temp_dir.path().join("context.md");

    create_context_file(&path, "fix: bug", "session-456", &[], "").unwrap();

    let content = std::fs::read_to_string(path).unwrap();
    assert!(content.contains("# Session Context"));
    assert!(!content.contains("## Prompts"));
    assert!(!content.contains("## Summary"));
}

// CLI-872
#[test]
fn test_dispatch_lifecycle_event_routes_to_correct_handler() {
    let adapter = ClaudeCodeLifecycleAdapter;

    let cases = vec![
        (
            "session start empty id",
            LifecycleEvent {
                event_type: Some(LifecycleEventType::SessionStart),
                session_id: String::new(),
                ..sample_event(LifecycleEventType::SessionStart)
            },
            "no session_id",
            true,
        ),
        (
            "turn end empty transcript",
            LifecycleEvent {
                event_type: Some(LifecycleEventType::TurnEnd),
                session_ref: String::new(),
                ..sample_event(LifecycleEventType::TurnEnd)
            },
            "transcript file not specified",
            true,
        ),
        (
            "compaction no-op",
            sample_event(LifecycleEventType::Compaction),
            "",
            false,
        ),
        (
            "session end empty id no-op",
            LifecycleEvent {
                event_type: Some(LifecycleEventType::SessionEnd),
                session_id: String::new(),
                ..sample_event(LifecycleEventType::SessionEnd)
            },
            "",
            false,
        ),
        (
            "subagent start",
            sample_event(LifecycleEventType::SubagentStart),
            "",
            false,
        ),
        (
            "subagent end",
            sample_event(LifecycleEventType::SubagentEnd),
            "",
            false,
        ),
    ];

    for (name, event, message, should_error) in cases {
        let result = dispatch_lifecycle_event(Some(&adapter), Some(&event));
        if should_error {
            let err = result.expect_err(name);
            assert!(err.to_string().contains(message), "{name}: {err}");
        } else {
            result.expect(name);
        }
    }
}

// CLI-873
#[test]
fn test_parse_hook_event_session_start_claude() {
    let adapter = ClaudeCodeLifecycleAdapter;
    let mut stdin = Cursor::new(
        r#"{"session_id":"test-session-123","transcript_path":"/tmp/transcript.jsonl"}"#,
    );

    let event = adapter
        .parse_hook_event(CLAUDE_HOOK_SESSION_START, &mut stdin)
        .unwrap()
        .expect("event should exist");

    assert_eq!(Some(LifecycleEventType::SessionStart), event.event_type);
    assert_eq!("test-session-123", event.session_id);
    assert_eq!("/tmp/transcript.jsonl", event.session_ref);
}

#[test]
fn test_parse_hook_event_turn_start_claude() {
    let adapter = ClaudeCodeLifecycleAdapter;
    let mut stdin = Cursor::new(
        r#"{"session_id":"sess-456","transcript_path":"/tmp/t.jsonl","prompt":"Hello world"}"#,
    );

    let event = adapter
        .parse_hook_event(CLAUDE_HOOK_USER_PROMPT_SUBMIT, &mut stdin)
        .unwrap()
        .expect("event should exist");

    assert_eq!(Some(LifecycleEventType::TurnStart), event.event_type);
    assert_eq!("sess-456", event.session_id);
    assert_eq!("Hello world", event.prompt);
}

#[test]
fn test_parse_hook_event_turn_end_claude() {
    let adapter = ClaudeCodeLifecycleAdapter;
    let mut stdin = Cursor::new(r#"{"session_id":"sess-789","transcript_path":"/tmp/stop.jsonl"}"#);

    let event = adapter
        .parse_hook_event(CLAUDE_HOOK_STOP, &mut stdin)
        .unwrap()
        .expect("event should exist");

    assert_eq!(Some(LifecycleEventType::TurnEnd), event.event_type);
    assert_eq!("sess-789", event.session_id);
}

#[test]
fn test_parse_hook_event_session_end_claude() {
    let adapter = ClaudeCodeLifecycleAdapter;
    let mut stdin =
        Cursor::new(r#"{"session_id":"ending-session","transcript_path":"/tmp/end.jsonl"}"#);

    let event = adapter
        .parse_hook_event(CLAUDE_HOOK_SESSION_END, &mut stdin)
        .unwrap()
        .expect("event should exist");

    assert_eq!(Some(LifecycleEventType::SessionEnd), event.event_type);
    assert_eq!("ending-session", event.session_id);
}

// CLI-874
#[test]
fn test_parse_hook_event_subagent_start_claude() {
    let adapter = ClaudeCodeLifecycleAdapter;
    let mut stdin = Cursor::new(
        r#"{"session_id":"main-session","transcript_path":"/tmp/main.jsonl","tool_use_id":"toolu_abc123","tool_input":{"description":"test task"}}"#,
    );

    let event = adapter
        .parse_hook_event(CLAUDE_HOOK_PRE_TASK, &mut stdin)
        .unwrap()
        .expect("event should exist");

    assert_eq!(Some(LifecycleEventType::SubagentStart), event.event_type);
    assert_eq!("main-session", event.session_id);
    assert_eq!("toolu_abc123", event.tool_use_id);
}

#[test]
fn test_parse_hook_event_subagent_end_claude() {
    let adapter = ClaudeCodeLifecycleAdapter;
    let mut stdin = Cursor::new(
        r#"{"session_id":"main-session","transcript_path":"/tmp/main.jsonl","tool_use_id":"toolu_xyz789","tool_input":{"prompt":"task done"},"tool_response":{"agentId":"agent-subagent-001"}}"#,
    );

    let event = adapter
        .parse_hook_event(CLAUDE_HOOK_POST_TASK, &mut stdin)
        .unwrap()
        .expect("event should exist");

    assert_eq!(Some(LifecycleEventType::SubagentEnd), event.event_type);
    assert_eq!("toolu_xyz789", event.tool_use_id);
    assert_eq!("agent-subagent-001", event.subagent_id);
}

#[test]
fn test_parse_hook_event_subagent_end_no_agent_id_claude() {
    let adapter = ClaudeCodeLifecycleAdapter;
    let mut stdin = Cursor::new(
        r#"{"session_id":"main-session","transcript_path":"/tmp/main.jsonl","tool_use_id":"toolu_no_agent","tool_input":{},"tool_response":{}}"#,
    );

    let event = adapter
        .parse_hook_event(CLAUDE_HOOK_POST_TASK, &mut stdin)
        .unwrap()
        .expect("event should exist");

    assert_eq!(Some(LifecycleEventType::SubagentEnd), event.event_type);
    assert_eq!("", event.subagent_id);
}

// CLI-875
#[test]
fn test_parse_hook_event_post_todo_returns_nil_claude() {
    let adapter = ClaudeCodeLifecycleAdapter;
    let mut stdin =
        Cursor::new(r#"{"session_id":"todo-session","transcript_path":"/tmp/todo.jsonl"}"#);

    let event = adapter
        .parse_hook_event(CLAUDE_HOOK_POST_TODO, &mut stdin)
        .unwrap();

    assert!(event.is_none());
}

#[test]
fn test_parse_hook_event_unknown_hook_returns_nil_claude() {
    let adapter = ClaudeCodeLifecycleAdapter;
    let mut stdin =
        Cursor::new(r#"{"session_id":"unknown","transcript_path":"/tmp/unknown.jsonl"}"#);

    let event = adapter
        .parse_hook_event("unknown-hook-name", &mut stdin)
        .unwrap();
    assert!(event.is_none());
}

// CLI-876
#[test]
fn test_parse_hook_event_empty_input_claude() {
    let adapter = ClaudeCodeLifecycleAdapter;
    let mut stdin = Cursor::new("");

    let err = adapter
        .parse_hook_event(CLAUDE_HOOK_SESSION_START, &mut stdin)
        .unwrap_err();

    assert!(err.to_string().contains("empty hook input"));
}

#[test]
fn test_parse_hook_event_malformed_json_claude() {
    let adapter = ClaudeCodeLifecycleAdapter;
    let mut stdin = Cursor::new(r#"{"session_id":"test","transcript_path":INVALID}"#);

    let err = adapter
        .parse_hook_event(CLAUDE_HOOK_SESSION_START, &mut stdin)
        .unwrap_err();

    assert!(err.to_string().contains("failed to parse hook input"));
}

// CLI-877
#[test]
fn test_parse_hook_event_all_hook_types_claude() {
    let adapter = ClaudeCodeLifecycleAdapter;

    let test_cases = vec![
        (
            CLAUDE_HOOK_SESSION_START,
            Some(LifecycleEventType::SessionStart),
            false,
            r#"{"session_id":"s1","transcript_path":"/t"}"#,
        ),
        (
            CLAUDE_HOOK_USER_PROMPT_SUBMIT,
            Some(LifecycleEventType::TurnStart),
            false,
            r#"{"session_id":"s2","transcript_path":"/t","prompt":"hi"}"#,
        ),
        (
            CLAUDE_HOOK_STOP,
            Some(LifecycleEventType::TurnEnd),
            false,
            r#"{"session_id":"s3","transcript_path":"/t"}"#,
        ),
        (
            CLAUDE_HOOK_SESSION_END,
            Some(LifecycleEventType::SessionEnd),
            false,
            r#"{"session_id":"s4","transcript_path":"/t"}"#,
        ),
        (
            CLAUDE_HOOK_PRE_TASK,
            Some(LifecycleEventType::SubagentStart),
            false,
            r#"{"session_id":"s5","transcript_path":"/t","tool_use_id":"t1","tool_input":{}}"#,
        ),
        (
            CLAUDE_HOOK_POST_TASK,
            Some(LifecycleEventType::SubagentEnd),
            false,
            r#"{"session_id":"s6","transcript_path":"/t","tool_use_id":"t2","tool_input":{},"tool_response":{}}"#,
        ),
        (
            CLAUDE_HOOK_POST_TODO,
            None,
            true,
            r#"{"session_id":"s7","transcript_path":"/t"}"#,
        ),
    ];

    for (hook_name, expected_type, expect_nil, input) in test_cases {
        let mut stdin = Cursor::new(input);
        let event = adapter.parse_hook_event(hook_name, &mut stdin).unwrap();
        if expect_nil {
            assert!(event.is_none(), "{hook_name}");
            continue;
        }

        let event = event.expect("event should exist");
        assert_eq!(expected_type, event.event_type, "{hook_name}");
    }
}

// CLI-878
#[derive(Debug, Deserialize)]
struct SessionInfoRaw {
    session_id: String,
    transcript_path: String,
}

#[test]
fn test_read_and_parse_valid_input_claude() {
    let mut stdin =
        Cursor::new(r#"{"session_id":"test-123","transcript_path":"/path/to/transcript"}"#);
    let parsed: SessionInfoRaw = read_and_parse_hook_input(&mut stdin).unwrap();
    assert_eq!("test-123", parsed.session_id);
    assert_eq!("/path/to/transcript", parsed.transcript_path);
}

#[test]
fn test_read_and_parse_empty_input_claude() {
    let mut stdin = Cursor::new("");
    let err = read_and_parse_hook_input::<SessionInfoRaw>(&mut stdin).unwrap_err();
    assert!(err.to_string().contains("empty hook input"));
}

#[test]
fn test_read_and_parse_invalid_json_claude() {
    let mut stdin = Cursor::new("not valid json");
    let err = read_and_parse_hook_input::<SessionInfoRaw>(&mut stdin).unwrap_err();
    assert!(err.to_string().contains("failed to parse hook input"));
}

#[test]
fn test_read_and_parse_partial_json_claude() {
    let mut stdin = Cursor::new(r#"{"session_id":"partial-only"}"#);
    let parsed: SessionInfoRaw = read_and_parse_hook_input(&mut stdin).unwrap();
    assert_eq!("partial-only", parsed.session_id);
    assert_eq!("", parsed.transcript_path);
}

#[test]
fn test_read_and_parse_extra_fields_claude() {
    let mut stdin = Cursor::new(
        r#"{"session_id":"test","transcript_path":"/t","extra_field":"ignored","another":123}"#,
    );
    let parsed: SessionInfoRaw = read_and_parse_hook_input(&mut stdin).unwrap();
    assert_eq!("test", parsed.session_id);
}

// CLI-879
#[test]
fn test_parse_hook_event_session_start_gemini() {
    let adapter = GeminiCliLifecycleAdapter;
    let mut stdin =
        Cursor::new(r#"{"session_id":"gemini-session-123","transcript_path":"/tmp/gemini.json"}"#);

    let event = adapter
        .parse_hook_event(GEMINI_HOOK_SESSION_START, &mut stdin)
        .unwrap()
        .expect("event should exist");

    assert_eq!(Some(LifecycleEventType::SessionStart), event.event_type);
    assert_eq!("gemini-session-123", event.session_id);
}

#[test]
fn test_parse_hook_event_turn_start_gemini() {
    let adapter = GeminiCliLifecycleAdapter;
    let mut stdin = Cursor::new(
        r#"{"session_id":"sess-456","transcript_path":"/tmp/t.json","prompt":"Hello Gemini"}"#,
    );

    let event = adapter
        .parse_hook_event(GEMINI_HOOK_BEFORE_AGENT, &mut stdin)
        .unwrap()
        .expect("event should exist");

    assert_eq!(Some(LifecycleEventType::TurnStart), event.event_type);
    assert_eq!("Hello Gemini", event.prompt);
}

#[test]
fn test_parse_hook_event_turn_end_gemini() {
    let adapter = GeminiCliLifecycleAdapter;
    let mut stdin = Cursor::new(r#"{"session_id":"sess-789","transcript_path":"/tmp/after.json"}"#);

    let event = adapter
        .parse_hook_event(GEMINI_HOOK_AFTER_AGENT, &mut stdin)
        .unwrap()
        .expect("event should exist");

    assert_eq!(Some(LifecycleEventType::TurnEnd), event.event_type);
    assert_eq!("sess-789", event.session_id);
}

#[test]
fn test_parse_hook_event_session_end_gemini() {
    let adapter = GeminiCliLifecycleAdapter;
    let mut stdin =
        Cursor::new(r#"{"session_id":"ending-session","transcript_path":"/tmp/end.json"}"#);

    let event = adapter
        .parse_hook_event(GEMINI_HOOK_SESSION_END, &mut stdin)
        .unwrap()
        .expect("event should exist");

    assert_eq!(Some(LifecycleEventType::SessionEnd), event.event_type);
    assert_eq!("ending-session", event.session_id);
}

#[test]
fn test_parse_hook_event_compaction_gemini() {
    let adapter = GeminiCliLifecycleAdapter;
    let mut stdin =
        Cursor::new(r#"{"session_id":"compress-session","transcript_path":"/tmp/compress.json"}"#);

    let event = adapter
        .parse_hook_event(GEMINI_HOOK_PRE_COMPRESS, &mut stdin)
        .unwrap()
        .expect("event should exist");

    assert_eq!(Some(LifecycleEventType::Compaction), event.event_type);
}

// CLI-880
#[test]
fn test_parse_hook_event_pass_through_hooks_return_nil_gemini() {
    let adapter = GeminiCliLifecycleAdapter;
    let pass_through = vec![
        GEMINI_HOOK_BEFORE_TOOL,
        GEMINI_HOOK_AFTER_TOOL,
        GEMINI_HOOK_BEFORE_MODEL,
        GEMINI_HOOK_AFTER_MODEL,
        GEMINI_HOOK_BEFORE_TOOL_SELECTION,
        GEMINI_HOOK_NOTIFICATION,
    ];

    for hook in pass_through {
        let mut stdin = Cursor::new(r#"{"session_id":"test","transcript_path":"/t"}"#);
        let event = adapter.parse_hook_event(hook, &mut stdin).unwrap();
        assert!(event.is_none(), "{hook}");
    }
}

#[test]
fn test_parse_hook_event_unknown_hook_returns_nil_gemini() {
    let adapter = GeminiCliLifecycleAdapter;
    let mut stdin =
        Cursor::new(r#"{"session_id":"unknown","transcript_path":"/tmp/unknown.json"}"#);
    let event = adapter
        .parse_hook_event("unknown-hook-name", &mut stdin)
        .unwrap();
    assert!(event.is_none());
}

// CLI-881
#[test]
fn test_parse_hook_event_empty_input_gemini() {
    let adapter = GeminiCliLifecycleAdapter;
    let mut stdin = Cursor::new("");
    let err = adapter
        .parse_hook_event(GEMINI_HOOK_SESSION_START, &mut stdin)
        .unwrap_err();
    assert!(err.to_string().contains("empty hook input"));
}

#[test]
fn test_parse_hook_event_malformed_json_gemini() {
    let adapter = GeminiCliLifecycleAdapter;
    let mut stdin = Cursor::new(r#"{"session_id":"test","transcript_path":INVALID}"#);
    let err = adapter
        .parse_hook_event(GEMINI_HOOK_SESSION_START, &mut stdin)
        .unwrap_err();
    assert!(err.to_string().contains("failed to parse hook input"));
}

// CLI-882
#[test]
fn test_parse_hook_event_all_lifecycle_hooks_gemini() {
    let adapter = GeminiCliLifecycleAdapter;
    let test_cases = vec![
        (
            GEMINI_HOOK_SESSION_START,
            Some(LifecycleEventType::SessionStart),
            false,
            r#"{"session_id":"s1","transcript_path":"/t"}"#,
        ),
        (
            GEMINI_HOOK_BEFORE_AGENT,
            Some(LifecycleEventType::TurnStart),
            false,
            r#"{"session_id":"s2","transcript_path":"/t","prompt":"hi"}"#,
        ),
        (
            GEMINI_HOOK_AFTER_AGENT,
            Some(LifecycleEventType::TurnEnd),
            false,
            r#"{"session_id":"s3","transcript_path":"/t"}"#,
        ),
        (
            GEMINI_HOOK_SESSION_END,
            Some(LifecycleEventType::SessionEnd),
            false,
            r#"{"session_id":"s4","transcript_path":"/t"}"#,
        ),
        (
            GEMINI_HOOK_PRE_COMPRESS,
            Some(LifecycleEventType::Compaction),
            false,
            r#"{"session_id":"s5","transcript_path":"/t"}"#,
        ),
        (
            GEMINI_HOOK_BEFORE_TOOL,
            None,
            true,
            r#"{"session_id":"s6","transcript_path":"/t"}"#,
        ),
        (
            GEMINI_HOOK_AFTER_TOOL,
            None,
            true,
            r#"{"session_id":"s7","transcript_path":"/t"}"#,
        ),
        (
            GEMINI_HOOK_BEFORE_MODEL,
            None,
            true,
            r#"{"session_id":"s8","transcript_path":"/t"}"#,
        ),
        (
            GEMINI_HOOK_AFTER_MODEL,
            None,
            true,
            r#"{"session_id":"s9","transcript_path":"/t"}"#,
        ),
        (
            GEMINI_HOOK_BEFORE_TOOL_SELECTION,
            None,
            true,
            r#"{"session_id":"s10","transcript_path":"/t"}"#,
        ),
        (
            GEMINI_HOOK_NOTIFICATION,
            None,
            true,
            r#"{"session_id":"s11","transcript_path":"/t"}"#,
        ),
    ];

    for (hook_name, expected_type, expect_nil, input) in test_cases {
        let mut stdin = Cursor::new(input);
        let event = adapter.parse_hook_event(hook_name, &mut stdin).unwrap();
        if expect_nil {
            assert!(event.is_none(), "{hook_name}");
            continue;
        }

        let event = event.expect("event should exist");
        assert_eq!(expected_type, event.event_type, "{hook_name}");
    }
}

// CLI-883
#[derive(Debug, Deserialize)]
struct GeminiAgentHookInputRaw {
    session_id: String,
    transcript_path: String,
    #[serde(default)]
    hook_event_name: String,
    #[serde(default)]
    prompt: String,
}

#[test]
fn test_read_and_parse_valid_input_gemini() {
    let mut stdin = Cursor::new(
        r#"{"session_id":"test-123","transcript_path":"/path/to/transcript","cwd":"/home/user"}"#,
    );
    let parsed: SessionInfoRaw = read_and_parse_hook_input(&mut stdin).unwrap();
    assert_eq!("test-123", parsed.session_id);
    assert_eq!("/path/to/transcript", parsed.transcript_path);
}

#[test]
fn test_read_and_parse_empty_input_gemini() {
    let mut stdin = Cursor::new("");
    let err = read_and_parse_hook_input::<SessionInfoRaw>(&mut stdin).unwrap_err();
    assert!(err.to_string().contains("empty hook input"));
}

#[test]
fn test_read_and_parse_invalid_json_gemini() {
    let mut stdin = Cursor::new("not valid json");
    let err = read_and_parse_hook_input::<SessionInfoRaw>(&mut stdin).unwrap_err();
    assert!(err.to_string().contains("failed to parse hook input"));
}

#[test]
fn test_read_and_parse_partial_json_gemini() {
    let mut stdin = Cursor::new(r#"{"session_id":"partial-only"}"#);
    let parsed: SessionInfoRaw = read_and_parse_hook_input(&mut stdin).unwrap();
    assert_eq!("partial-only", parsed.session_id);
    assert_eq!("", parsed.transcript_path);
}

#[test]
fn test_read_and_parse_extra_fields_gemini() {
    let mut stdin = Cursor::new(
        r#"{"session_id":"test","transcript_path":"/t","extra_field":"ignored","another":123}"#,
    );
    let parsed: SessionInfoRaw = read_and_parse_hook_input(&mut stdin).unwrap();
    assert_eq!("test", parsed.session_id);
}

#[test]
fn test_read_and_parse_agent_hook_input_gemini() {
    let mut stdin = Cursor::new(
        r#"{"session_id":"agent-session","transcript_path":"/path/to/agent.json","hook_event_name":"before-agent","prompt":"User's question here"}"#,
    );
    let parsed: GeminiAgentHookInputRaw = read_and_parse_hook_input(&mut stdin).unwrap();
    assert_eq!("agent-session", parsed.session_id);
    assert_eq!("/path/to/agent.json", parsed.transcript_path);
    assert_eq!("User's question here", parsed.prompt);
    assert_eq!("before-agent", parsed.hook_event_name);
}

#[test]
fn test_parse_hook_event_session_start_copilot() {
    let adapter = CopilotCliLifecycleAdapter;
    let mut stdin = Cursor::new(r#"{"sessionId":"copilot-session-1"}"#);
    let event = adapter
        .parse_hook_event(COPILOT_HOOK_SESSION_START, &mut stdin)
        .unwrap()
        .expect("event should exist");

    assert_eq!(Some(LifecycleEventType::SessionStart), event.event_type);
    assert_eq!("copilot-session-1", event.session_id);
}

#[test]
fn test_parse_hook_event_turn_start_copilot() {
    let adapter = CopilotCliLifecycleAdapter;
    let mut stdin = Cursor::new(r#"{"sessionId":"copilot-session-2","prompt":"Ship it"}"#);
    let event = adapter
        .parse_hook_event(COPILOT_HOOK_USER_PROMPT_SUBMITTED, &mut stdin)
        .unwrap()
        .expect("event should exist");

    assert_eq!(Some(LifecycleEventType::TurnStart), event.event_type);
    assert_eq!("copilot-session-2", event.session_id);
    assert_eq!("Ship it", event.prompt);
}

#[test]
fn test_parse_hook_event_turn_end_copilot() {
    let adapter = CopilotCliLifecycleAdapter;
    let mut stdin =
        Cursor::new(r#"{"sessionId":"copilot-session-3","transcriptPath":"/tmp/copilot.jsonl"}"#);
    let event = adapter
        .parse_hook_event(COPILOT_HOOK_AGENT_STOP, &mut stdin)
        .unwrap()
        .expect("event should exist");

    assert_eq!(Some(LifecycleEventType::TurnEnd), event.event_type);
    assert_eq!("copilot-session-3", event.session_id);
    assert_eq!("/tmp/copilot.jsonl", event.session_ref);
}

#[test]
fn test_parse_hook_event_session_end_copilot() {
    let adapter = CopilotCliLifecycleAdapter;
    let mut stdin = Cursor::new(r#"{"sessionId":"copilot-session-4"}"#);
    let event = adapter
        .parse_hook_event(COPILOT_HOOK_SESSION_END, &mut stdin)
        .unwrap()
        .expect("event should exist");

    assert_eq!(Some(LifecycleEventType::SessionEnd), event.event_type);
    assert_eq!("copilot-session-4", event.session_id);
}

#[test]
fn test_parse_hook_event_subagent_end_copilot() {
    let adapter = CopilotCliLifecycleAdapter;
    let mut stdin = Cursor::new(r#"{"sessionId":"copilot-session-5"}"#);
    let event = adapter
        .parse_hook_event(COPILOT_HOOK_SUBAGENT_STOP, &mut stdin)
        .unwrap()
        .expect("event should exist");

    assert_eq!(Some(LifecycleEventType::SubagentEnd), event.event_type);
    assert_eq!("copilot-session-5", event.session_id);
}

// CLI-884
#[test]
fn test_parse_hook_event_session_start_opencode() {
    let adapter = OpenCodeLifecycleAdapter;
    let mut stdin = Cursor::new(
        r#"{"session_id":"sess-abc123","transcript_path":"/tmp/bitloops-opencode/-project/sess-abc123.json"}"#,
    );
    let event = adapter
        .parse_hook_event(OPENCODE_HOOK_SESSION_START, &mut stdin)
        .unwrap()
        .expect("event should exist");

    assert_eq!(Some(LifecycleEventType::SessionStart), event.event_type);
    assert_eq!("sess-abc123", event.session_id);
}

#[test]
fn test_parse_hook_event_turn_start_opencode() {
    let adapter = OpenCodeLifecycleAdapter;
    let mut stdin = Cursor::new(
        r#"{"session_id":"sess-1","transcript_path":"/tmp/t.json","prompt":"Fix the bug in login.ts"}"#,
    );
    let event = adapter
        .parse_hook_event(OPENCODE_HOOK_TURN_START, &mut stdin)
        .unwrap()
        .expect("event should exist");

    assert_eq!(Some(LifecycleEventType::TurnStart), event.event_type);
    assert_eq!("Fix the bug in login.ts", event.prompt);
}

#[test]
fn test_parse_hook_event_turn_end_opencode() {
    let adapter = OpenCodeLifecycleAdapter;
    let mut stdin = Cursor::new(r#"{"session_id":"sess-2","transcript_path":"/tmp/t.json"}"#);
    let event = adapter
        .parse_hook_event(OPENCODE_HOOK_TURN_END, &mut stdin)
        .unwrap()
        .expect("event should exist");

    assert_eq!(Some(LifecycleEventType::TurnEnd), event.event_type);
}

#[test]
fn test_parse_hook_event_compaction_opencode() {
    let adapter = OpenCodeLifecycleAdapter;
    let mut stdin = Cursor::new(r#"{"session_id":"sess-3"}"#);
    let event = adapter
        .parse_hook_event(OPENCODE_HOOK_COMPACTION, &mut stdin)
        .unwrap()
        .expect("event should exist");

    assert_eq!(Some(LifecycleEventType::Compaction), event.event_type);
}

#[test]
fn test_parse_hook_event_session_end_opencode() {
    let adapter = OpenCodeLifecycleAdapter;
    let mut stdin = Cursor::new(r#"{"session_id":"sess-4","transcript_path":"/tmp/t.json"}"#);
    let event = adapter
        .parse_hook_event(OPENCODE_HOOK_SESSION_END, &mut stdin)
        .unwrap()
        .expect("event should exist");

    assert_eq!(Some(LifecycleEventType::SessionEnd), event.event_type);
}

// CLI-885
#[test]
fn test_parse_hook_event_unknown_hook_opencode() {
    let adapter = OpenCodeLifecycleAdapter;
    let mut stdin = Cursor::new("{}");
    let event = adapter
        .parse_hook_event("unknown-hook", &mut stdin)
        .unwrap();
    assert!(event.is_none());
}

#[test]
fn test_parse_hook_event_empty_input_opencode() {
    let adapter = OpenCodeLifecycleAdapter;
    let mut stdin = Cursor::new("");
    let err = adapter
        .parse_hook_event(OPENCODE_HOOK_SESSION_START, &mut stdin)
        .unwrap_err();
    assert!(err.to_string().contains("empty hook input"));
}

#[test]
fn test_parse_hook_event_malformed_json_opencode() {
    let adapter = OpenCodeLifecycleAdapter;
    let mut stdin = Cursor::new("not json");
    let err = adapter
        .parse_hook_event(OPENCODE_HOOK_SESSION_START, &mut stdin)
        .unwrap_err();
    assert!(err.to_string().contains("failed to parse hook input"));
}

// CLI-886
#[test]
fn test_format_resume_command_opencode() {
    let adapter = OpenCodeLifecycleAdapter;
    assert_eq!(
        "opencode -s sess-abc123",
        adapter.format_resume_command("sess-abc123")
    );
}

#[test]
fn test_format_resume_command_empty_opencode() {
    let adapter = OpenCodeLifecycleAdapter;
    assert_eq!("opencode", adapter.format_resume_command(""));
}

#[test]
fn test_hook_names_opencode() {
    let adapter = OpenCodeLifecycleAdapter;
    let names = adapter.hook_names();

    let expected = [
        OPENCODE_HOOK_SESSION_START,
        OPENCODE_HOOK_SESSION_END,
        OPENCODE_HOOK_TURN_START,
        OPENCODE_HOOK_TURN_END,
        OPENCODE_HOOK_COMPACTION,
    ];

    assert_eq!(expected.len(), names.len());

    let actual: HashSet<&str> = names.into_iter().collect();
    for expected_name in expected {
        assert!(actual.contains(expected_name), "missing {expected_name}");
    }
}

#[test]
fn test_parse_hook_event_session_start_codex() {
    let adapter = CodexLifecycleAdapter;
    let mut stdin =
        Cursor::new(r#"{"sessionId":"codex-session-1","transcriptPath":"/tmp/codex.jsonl"}"#);
    let event = adapter
        .parse_hook_event(CODEX_HOOK_SESSION_START, &mut stdin)
        .expect("parse")
        .expect("event");
    assert_eq!(event.event_type, Some(LifecycleEventType::SessionStart));
    assert_eq!(event.session_id, "codex-session-1");
    assert_eq!(event.session_ref, "/tmp/codex.jsonl");
}

#[test]
fn test_parse_hook_event_turn_end_codex() {
    let adapter = CodexLifecycleAdapter;
    let mut stdin =
        Cursor::new(r#"{"session_id":"codex-session-2","transcript_path":"/tmp/codex-2.jsonl"}"#);
    let event = adapter
        .parse_hook_event(CODEX_HOOK_STOP, &mut stdin)
        .expect("parse")
        .expect("event");
    assert_eq!(event.event_type, Some(LifecycleEventType::TurnEnd));
    assert_eq!(event.session_id, "codex-session-2");
    assert_eq!(event.session_ref, "/tmp/codex-2.jsonl");
}

#[test]
fn test_hook_names_codex() {
    let adapter = CodexLifecycleAdapter;
    let names = adapter.hook_names();
    assert_eq!(names, vec![CODEX_HOOK_SESSION_START, CODEX_HOOK_STOP]);
}

#[test]
fn test_parse_hook_event_session_start_cursor() {
    let adapter = CursorLifecycleAdapter;
    let mut stdin = Cursor::new(
        r#"{"conversation_id":"cursor-session-1","transcript_path":"/tmp/cursor.jsonl"}"#,
    );
    let event = adapter
        .parse_hook_event(CURSOR_HOOK_SESSION_START, &mut stdin)
        .expect("parse")
        .expect("event");
    assert_eq!(event.event_type, Some(LifecycleEventType::SessionStart));
    assert_eq!(event.session_id, "cursor-session-1");
    assert_eq!(event.session_ref, "/tmp/cursor.jsonl");
}

#[test]
fn test_parse_hook_event_turn_start_cursor() {
    let adapter = CursorLifecycleAdapter;
    let mut stdin = Cursor::new(
        r#"{"conversation_id":"cursor-session-2","transcript_path":"/tmp/cursor2.jsonl","prompt":"hello"}"#,
    );
    let event = adapter
        .parse_hook_event(CURSOR_HOOK_BEFORE_SUBMIT_PROMPT, &mut stdin)
        .expect("parse")
        .expect("event");
    assert_eq!(event.event_type, Some(LifecycleEventType::TurnStart));
    assert_eq!(event.prompt, "hello");
}

#[test]
fn test_parse_hook_event_compaction_cursor() {
    let adapter = CursorLifecycleAdapter;
    let mut stdin = Cursor::new(
        r#"{"conversation_id":"cursor-session-3","transcript_path":"/tmp/cursor3.jsonl"}"#,
    );
    let event = adapter
        .parse_hook_event(CURSOR_HOOK_PRE_COMPACT, &mut stdin)
        .expect("parse")
        .expect("event");
    assert_eq!(event.event_type, Some(LifecycleEventType::Compaction));
}

#[test]
fn test_parse_hook_event_subagent_no_task_cursor() {
    let adapter = CursorLifecycleAdapter;
    let mut stdin = Cursor::new(r#"{"conversation_id":"cursor-session-4","subagent_id":"a1"}"#);
    let event = adapter
        .parse_hook_event(CURSOR_HOOK_SUBAGENT_START, &mut stdin)
        .expect("parse");
    assert!(event.is_none());
}

#[test]
fn test_parse_hook_event_turn_end_cursor() {
    let adapter = CursorLifecycleAdapter;
    let mut stdin = Cursor::new(
        r#"{"conversation_id":"cursor-session-stop","transcript_path":"/tmp/cursor-stop.jsonl"}"#,
    );
    let event = adapter
        .parse_hook_event(CURSOR_HOOK_STOP, &mut stdin)
        .expect("parse")
        .expect("event");
    assert_eq!(event.event_type, Some(LifecycleEventType::TurnEnd));
    assert_eq!(event.session_id, "cursor-session-stop");
    assert_eq!(event.session_ref, "/tmp/cursor-stop.jsonl");
}

#[test]
fn test_parse_hook_event_session_end_cursor() {
    let adapter = CursorLifecycleAdapter;
    let mut stdin = Cursor::new(
        r#"{"conversation_id":"cursor-session-end","transcript_path":"/tmp/cursor-end.jsonl"}"#,
    );
    let event = adapter
        .parse_hook_event(CURSOR_HOOK_SESSION_END, &mut stdin)
        .expect("parse")
        .expect("event");
    assert_eq!(event.event_type, Some(LifecycleEventType::SessionEnd));
    assert_eq!(event.session_id, "cursor-session-end");
    assert_eq!(event.session_ref, "/tmp/cursor-end.jsonl");
}

#[test]
fn test_parse_hook_event_subagent_stop_cursor() {
    let adapter = CursorLifecycleAdapter;
    let mut stdin = Cursor::new(
        r#"{"conversation_id":"cursor-session-5","transcript_path":"/tmp/cursor5.jsonl","subagent_id":"agent-5","task":"Ship feature"}"#,
    );
    let event = adapter
        .parse_hook_event(CURSOR_HOOK_SUBAGENT_STOP, &mut stdin)
        .expect("parse")
        .expect("event");
    assert_eq!(event.event_type, Some(LifecycleEventType::SubagentEnd));
    assert_eq!(event.session_id, "cursor-session-5");
    assert_eq!(event.session_ref, "/tmp/cursor5.jsonl");
    assert_eq!(event.subagent_id, "agent-5");
    assert_eq!(event.tool_use_id, "agent-5");
}

#[test]
fn test_parse_hook_event_subagent_stop_no_task_cursor() {
    let adapter = CursorLifecycleAdapter;
    let mut stdin = Cursor::new(r#"{"conversation_id":"cursor-session-6","subagent_id":"a6"}"#);
    let event = adapter
        .parse_hook_event(CURSOR_HOOK_SUBAGENT_STOP, &mut stdin)
        .expect("parse");
    assert!(event.is_none());
}

#[test]
fn test_subagent_start_handler_safe_noop() {
    let adapter = ClaudeCodeLifecycleAdapter;
    let event = sample_event(LifecycleEventType::SubagentStart);
    handle_lifecycle_subagent_start(&adapter, &event)
        .expect("subagent start should be a safe no-op in compatibility scaffold");
}

#[test]
fn test_subagent_end_handler_safe_noop() {
    let adapter = ClaudeCodeLifecycleAdapter;
    let event = sample_event(LifecycleEventType::SubagentEnd);
    handle_lifecycle_subagent_end(&adapter, &event)
        .expect("subagent end should be a safe no-op in compatibility scaffold");
}

use super::adapters::route_hook_command_to_lifecycle;
use super::adapters::{
    CLAUDE_HOOK_POST_TASK, CLAUDE_HOOK_POST_TODO, CLAUDE_HOOK_PRE_TASK, CLAUDE_HOOK_SESSION_END,
    CLAUDE_HOOK_SESSION_START, CLAUDE_HOOK_STOP, CLAUDE_HOOK_USER_PROMPT_SUBMIT,
    CODEX_HOOK_POST_TOOL_USE, CODEX_HOOK_PRE_TOOL_USE, CODEX_HOOK_SESSION_START, CODEX_HOOK_STOP,
    CODEX_HOOK_USER_PROMPT_SUBMIT, COPILOT_HOOK_AGENT_STOP, COPILOT_HOOK_SESSION_END,
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
use super::canonical::build_phase3_canonical_request;
use super::{
    LifecycleAgentAdapter, LifecycleEvent, LifecycleEventType, PrePromptState, SessionIdPolicy,
    UNKNOWN_SESSION_ID, apply_session_id_policy, create_context_file, dispatch_lifecycle_event,
    handle_lifecycle_compaction, handle_lifecycle_session_end, handle_lifecycle_session_start,
    handle_lifecycle_subagent_end, handle_lifecycle_subagent_start, handle_lifecycle_turn_end,
    handle_lifecycle_turn_start, read_and_parse_hook_input, resolve_transcript_offset,
};
use crate::adapters::agents::AGENT_NAME_CODEX;
use crate::adapters::agents::canonical::{
    CanonicalContractCompatibility, CanonicalResumableSessionState,
};

use crate::host::checkpoints::session::create_session_backend_or_local;
use crate::host::checkpoints::session::phase::SessionPhase;
use crate::host::checkpoints::session::state::{PendingCheckpointState, SessionState};
use crate::test_support::git_fixtures::ensure_test_store_backends;
use crate::test_support::process_state::{
    git_command, with_cwd, with_git_env_cleared, with_process_state,
};
use serde::Deserialize;
use std::collections::HashSet;
use std::io::Cursor;
use std::path::Path;

fn sample_event(event_type: LifecycleEventType) -> LifecycleEvent {
    LifecycleEvent {
        event_type: Some(event_type),
        session_id: String::from("session-123"),
        session_ref: String::from("/tmp/transcript.jsonl"),
        source: String::new(),
        prompt: String::from("hello"),
        tool_name: String::new(),
        tool_use_id: String::from("toolu_123"),
        tool_input: None,
        subagent_id: String::from("subagent-1"),
        model: String::new(),
        finalize_open_turn: false,
    }
}

fn open_events_duckdb(repo_root: &Path) -> duckdb::Connection {
    let backends = crate::config::resolve_store_backend_config_for_repo(repo_root)
        .expect("resolve store backends");
    let path = backends.events.resolve_duckdb_db_path_for_repo(repo_root);
    duckdb::Connection::open(path).expect("open events duckdb")
}

fn latest_turn_fragment(repo_root: &Path) -> String {
    let conn = open_events_duckdb(repo_root);
    conn.query_row(
        "SELECT transcript_fragment FROM interaction_turns ORDER BY updated_at DESC LIMIT 1",
        [],
        |row| row.get(0),
    )
    .expect("read interaction turn transcript_fragment")
}

fn latest_session_model(repo_root: &Path) -> String {
    let conn = open_events_duckdb(repo_root);
    conn.query_row(
        "SELECT model FROM interaction_sessions ORDER BY updated_at DESC LIMIT 1",
        [],
        |row| row.get(0),
    )
    .expect("read interaction session model")
}

fn latest_turn_model(repo_root: &Path) -> String {
    let conn = open_events_duckdb(repo_root);
    conn.query_row(
        "SELECT model FROM interaction_turns ORDER BY updated_at DESC LIMIT 1",
        [],
        |row| row.get(0),
    )
    .expect("read interaction turn model")
}

fn latest_turn_end_payload(repo_root: &Path) -> serde_json::Value {
    let conn = open_events_duckdb(repo_root);
    let payload: String = conn
        .query_row(
            "SELECT payload FROM interaction_events WHERE event_type = 'turn_end' ORDER BY event_time DESC, event_id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .expect("read turn_end payload");
    serde_json::from_str(&payload).expect("parse turn_end payload")
}

fn codex_response_item_line(role: &str, kind: &str, text: &str) -> String {
    serde_json::json!({
        "type": "response_item",
        "payload": {
            "type": "message",
            "role": role,
            "content": [{
                "type": kind,
                "text": text,
            }],
        }
    })
    .to_string()
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

#[test]
fn test_phase3_canonical_request_enriches_rich_builtin_agents() {
    let mut event = sample_event(LifecycleEventType::TurnStart);
    event.session_id = String::from("gemini-session");
    event.session_ref = String::from("/tmp/gemini-session.jsonl");
    event.prompt = String::from("rich lifecycle path");

    let request = build_phase3_canonical_request("Gemini", &event).expect("request");
    assert_eq!(request.agent.agent_key, "gemini");
    assert_eq!(
        request.compatibility,
        CanonicalContractCompatibility::rich()
    );
    assert!(request.progress.is_some());
    let resumable = request
        .resumable_session
        .as_ref()
        .expect("resumable session");
    assert_eq!(resumable.state, CanonicalResumableSessionState::Resumable);
    assert_eq!(
        resumable.checkpoint.as_deref(),
        Some("/tmp/gemini-session.jsonl")
    );
}

#[test]
fn test_phase3_canonical_request_collapses_simple_builtin_agents() {
    let mut event = sample_event(LifecycleEventType::TurnStart);
    event.session_id = String::from("claude-session");
    event.session_ref = String::from("/tmp/claude-session.jsonl");
    event.prompt = String::from("simple lifecycle path");

    let request = build_phase3_canonical_request("Claude Code", &event).expect("request");
    assert_eq!(request.agent.agent_key, "claude-code");
    assert_eq!(
        request.compatibility,
        CanonicalContractCompatibility::default()
    );
    assert!(request.progress.is_none());
    assert!(request.resumable_session.is_none());
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
    std::fs::write(dir.path().join(".gitignore"), "stores/\n").unwrap();
    ensure_test_store_backends(dir.path());
    run(&["add", "."]);
    run(&["commit", "-m", "initial"]);
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
    assert!(
        err.to_string().contains("no session_id")
            || err.to_string().contains("session_id is required")
    );
}

#[test]
fn test_handle_lifecycle_session_start_persists_session_state() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    with_cwd(dir.path(), || {
        let adapter = CopilotCliLifecycleAdapter;
        let mut event = sample_event(LifecycleEventType::SessionStart);
        event.session_id = "copilot-session-start".to_string();
        event.session_ref = dir
            .path()
            .join("transcript.jsonl")
            .to_string_lossy()
            .to_string();

        handle_lifecycle_session_start(&adapter, &event)
            .expect("session start should persist state");

        let backend = create_session_backend_or_local(dir.path());
        let state = backend
            .load_session("copilot-session-start")
            .unwrap()
            .expect("session should exist");
        assert_eq!(state.transcript_path, event.session_ref);
        assert_eq!(state.agent_type, "copilot");
        assert!(state.last_interaction_time.is_some());
    });
}

#[test]
fn test_handle_lifecycle_turn_start_empty_session_id() {
    let adapter = ClaudeCodeLifecycleAdapter;
    let mut event = sample_event(LifecycleEventType::TurnStart);
    event.session_id.clear();

    let err = handle_lifecycle_turn_start(&adapter, &event).unwrap_err();
    assert!(
        err.to_string().contains("no session_id")
            || err.to_string().contains("session_id is required")
    );
}

#[test]
fn test_handle_lifecycle_turn_start_persists_pre_prompt_and_session_state() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let transcript_path = dir.path().join("copilot-events.jsonl");
    std::fs::write(
        &transcript_path,
        "{\"type\":\"user.message\",\"data\":{\"content\":\"hello\"}}\n",
    )
    .unwrap();

    with_cwd(dir.path(), || {
        let adapter = CopilotCliLifecycleAdapter;
        let mut event = sample_event(LifecycleEventType::TurnStart);
        event.session_id = "copilot-turn-start".to_string();
        event.session_ref = transcript_path.to_string_lossy().to_string();
        event.prompt = "Create file".to_string();

        handle_lifecycle_turn_start(&adapter, &event).expect("turn start should persist state");

        let backend = create_session_backend_or_local(dir.path());
        let pre_prompt = backend
            .load_pre_prompt("copilot-turn-start")
            .unwrap()
            .expect("pre-prompt should exist");
        assert_eq!(pre_prompt.prompt, "Create file");
        assert_eq!(pre_prompt.transcript_path, event.session_ref);

        let state = backend
            .load_session("copilot-turn-start")
            .unwrap()
            .expect("session should exist");
        assert_eq!(state.phase, SessionPhase::Active);
        assert_eq!(state.agent_type, "copilot");
        assert_eq!(state.first_prompt, "Create file");
        assert_eq!(state.transcript_path, event.session_ref);
    });
}

#[test]
fn test_handle_lifecycle_turn_start_prefers_real_prompt_over_bootstrap_prompt() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    with_cwd(dir.path(), || {
        let backend = create_session_backend_or_local(dir.path());
        backend
            .save_session(&SessionState {
                session_id: "copilot-bootstrap".to_string(),
                first_prompt: "Bootstrap prompt".to_string(),
                phase: SessionPhase::Idle,
                ..Default::default()
            })
            .expect("seed session");

        let adapter = CopilotCliLifecycleAdapter;
        let mut event = sample_event(LifecycleEventType::TurnStart);
        event.session_id = "copilot-bootstrap".to_string();
        event.prompt = "Turn prompt".to_string();

        handle_lifecycle_turn_start(&adapter, &event).expect("turn start should succeed");

        let state = backend
            .load_session("copilot-bootstrap")
            .unwrap()
            .expect("session should exist");
        assert_eq!(state.first_prompt, "Turn prompt");
    });
}

#[test]
fn route_codex_user_prompt_submit_persists_pre_prompt_state() -> anyhow::Result<()> {
    let repo = tempfile::tempdir().expect("tempdir");
    setup_git_repo(&repo);
    let session_id = "codex-session-ups";
    let transcript_path = repo.path().join("codex-transcript.jsonl");
    std::fs::write(&transcript_path, "").expect("write transcript");

    with_process_state(Some(repo.path()), &[], || -> anyhow::Result<()> {
        let payload = serde_json::json!({
            "session_id": session_id,
            "transcript_path": transcript_path.to_string_lossy().to_string(),
            "prompt": "Refactor tracked file",
            "model": "gpt-5.4-codex"
        })
        .to_string();
        route_hook_command_to_lifecycle(
            repo.path(),
            AGENT_NAME_CODEX,
            CODEX_HOOK_USER_PROMPT_SUBMIT,
            &payload,
        )?;
        Ok(())
    })?;

    with_process_state(Some(repo.path()), &[], || -> anyhow::Result<()> {
        let backend = create_session_backend_or_local(repo.path());
        let pre_prompt = backend
            .load_pre_prompt(session_id)?
            .expect("pre-prompt should exist");
        assert_eq!(pre_prompt.prompt, "Refactor tracked file");
        assert!(
            crate::adapters::agents::codex::hooks::are_hooks_installed_at(repo.path()),
            "expected codex hooks to be installed for agent-aware self-heal"
        );
        Ok(())
    })?;

    Ok(())
}

// CLI-868
#[test]
fn test_handle_lifecycle_turn_end_empty_transcript_ref() {
    let adapter = ClaudeCodeLifecycleAdapter;
    let mut event = sample_event(LifecycleEventType::TurnEnd);
    event.session_ref.clear();

    let err = handle_lifecycle_turn_end(&adapter, &event).unwrap_err();
    assert!(err.to_string().contains("transcript file not specified"));
    assert!(err.to_string().contains("hook payload transcript_path"));
    assert!(err.to_string().contains("pre-prompt state"));
    assert!(err.to_string().contains("session state"));
}

#[test]
fn test_handle_lifecycle_turn_end_nonexistent_transcript() {
    let adapter = ClaudeCodeLifecycleAdapter;
    let mut event = sample_event(LifecycleEventType::TurnEnd);
    // Use a path whose parent directory cannot exist to ensure immediate failure
    // (no retry). Avoids false positives from other tests creating /tmp subdirs.
    event.session_ref =
        String::from("/nonexistent_bitloops_test_root/no_such_dir/transcript.jsonl");

    let err = handle_lifecycle_turn_end(&adapter, &event).unwrap_err();
    assert!(
        err.to_string().contains("transcript file not found"),
        "unexpected error: {err}"
    );
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

#[test]
fn test_handle_lifecycle_turn_end_persists_transcript_fragment() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let transcript_path = dir.path().join("transcript.jsonl");
    std::fs::write(
        &transcript_path,
        "{\"model\":\"gemini-2.5-pro\",\"messages\":[{\"type\":\"user\",\"content\":\"Update tracked file\"},{\"type\":\"gemini\",\"content\":\"Implemented the change\"}]}",
    )
    .unwrap();
    std::fs::write(dir.path().join("tracked.txt"), "changed\n").unwrap();

    let adapter = GeminiCliLifecycleAdapter;
    let mut event = sample_event(LifecycleEventType::TurnEnd);
    event.session_id = "fragment-session".to_string();
    event.session_ref = transcript_path.to_string_lossy().to_string();
    event.prompt = "Update tracked file".to_string();

    with_cwd(dir.path(), || {
        handle_lifecycle_turn_end(&adapter, &event).expect("turn end should persist fragment");

        let fragment = latest_turn_fragment(dir.path());
        assert!(
            fragment.contains("Implemented the change"),
            "turn row should persist the completed transcript fragment"
        );

        let payload = latest_turn_end_payload(dir.path());
        assert_eq!(
            payload["transcript_fragment"].as_str().unwrap_or_default(),
            fragment,
            "turn_end event payload should mirror the persisted transcript fragment"
        );
        assert_eq!(latest_session_model(dir.path()), "gemini-2.5-pro");
        assert_eq!(latest_turn_model(dir.path()), "gemini-2.5-pro");
    });
}

#[test]
fn route_codex_second_turn_uses_transcript_offsets_for_fragment() -> anyhow::Result<()> {
    let repo = tempfile::tempdir().expect("tempdir");
    setup_git_repo(&repo);
    let session_id = "codex-offset-session";
    let transcript_path = repo.path().join("codex-rollout.jsonl");
    std::fs::write(&transcript_path, "").expect("write transcript");

    with_process_state(Some(repo.path()), &[], || -> anyhow::Result<()> {
        let session_payload = serde_json::json!({
            "sessionId": session_id,
            "transcriptPath": transcript_path.to_string_lossy().to_string(),
        })
        .to_string();
        route_hook_command_to_lifecycle(
            repo.path(),
            AGENT_NAME_CODEX,
            CODEX_HOOK_SESSION_START,
            &session_payload,
        )?;

        let turn_one_payload = serde_json::json!({
            "sessionId": session_id,
            "transcriptPath": transcript_path.to_string_lossy().to_string(),
            "prompt": "Prompt 1",
        })
        .to_string();
        route_hook_command_to_lifecycle(
            repo.path(),
            AGENT_NAME_CODEX,
            CODEX_HOOK_USER_PROMPT_SUBMIT,
            &turn_one_payload,
        )?;
        std::fs::write(
            &transcript_path,
            format!(
                "{}\n{}\n{}\n",
                serde_json::json!({
                    "type": "session_meta",
                    "payload": { "id": session_id }
                }),
                codex_response_item_line("user", "input_text", "Prompt 1"),
                codex_response_item_line("assistant", "output_text", "Answer 1"),
            ),
        )
        .expect("write first turn transcript");
        std::fs::write(repo.path().join("tracked.txt"), "two\n").expect("modify tracked file");
        let turn_one_stop = serde_json::json!({
            "sessionId": session_id,
            "transcriptPath": transcript_path.to_string_lossy().to_string(),
        })
        .to_string();
        route_hook_command_to_lifecycle(
            repo.path(),
            AGENT_NAME_CODEX,
            CODEX_HOOK_STOP,
            &turn_one_stop,
        )?;

        let turn_two_payload = serde_json::json!({
            "sessionId": session_id,
            "transcriptPath": transcript_path.to_string_lossy().to_string(),
            "prompt": "Prompt 2",
        })
        .to_string();
        route_hook_command_to_lifecycle(
            repo.path(),
            AGENT_NAME_CODEX,
            CODEX_HOOK_USER_PROMPT_SUBMIT,
            &turn_two_payload,
        )?;
        std::fs::write(
            &transcript_path,
            format!(
                "{}\n{}\n{}\n{}\n{}\n",
                serde_json::json!({
                    "type": "session_meta",
                    "payload": { "id": session_id }
                }),
                codex_response_item_line("user", "input_text", "Prompt 1"),
                codex_response_item_line("assistant", "output_text", "Answer 1"),
                codex_response_item_line("user", "input_text", "Prompt 2"),
                codex_response_item_line("assistant", "output_text", "Answer 2"),
            ),
        )
        .expect("write second turn transcript");
        std::fs::write(repo.path().join("tracked.txt"), "three\n").expect("modify tracked file");
        let turn_two_stop = serde_json::json!({
            "sessionId": session_id,
            "transcriptPath": transcript_path.to_string_lossy().to_string(),
        })
        .to_string();
        route_hook_command_to_lifecycle(
            repo.path(),
            AGENT_NAME_CODEX,
            CODEX_HOOK_STOP,
            &turn_two_stop,
        )?;
        Ok(())
    })?;

    with_process_state(Some(repo.path()), &[], || -> anyhow::Result<()> {
        let conn = open_events_duckdb(repo.path());
        let mut stmt = conn
            .prepare(
                "SELECT transcript_fragment FROM interaction_turns WHERE session_id = ?1 ORDER BY turn_number ASC",
            )
            .expect("prepare fragment query");
        let fragments = stmt
            .query_map([session_id], |row| row.get::<_, String>(0))
            .expect("query fragments")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect fragments");
        assert_eq!(fragments.len(), 2);
        assert!(fragments[0].contains("Prompt 1"));
        assert!(fragments[0].contains("Answer 1"));
        assert!(!fragments[0].contains("Prompt 2"));
        assert!(fragments[1].contains("Prompt 2"));
        assert!(fragments[1].contains("Answer 2"));
        assert!(
            !fragments[1].contains("Prompt 1"),
            "second turn fragment should exclude the first turn transcript"
        );
        Ok(())
    })?;

    Ok(())
}

// CLI-869
#[test]
fn test_handle_lifecycle_compaction_resets_transcript_offset() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    with_cwd(dir.path(), || {
        let adapter = ClaudeCodeLifecycleAdapter;
        let event = sample_event(LifecycleEventType::Compaction);

        handle_lifecycle_compaction(&adapter, &event)
            .expect("compaction should reset transcript offset and succeed");
    });
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
                pending: PendingCheckpointState {
                    files_touched: vec!["tracked.txt".to_string()],
                    checkpoint_transcript_start: 77,
                    ..Default::default()
                },
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
        assert_eq!(saved.pending.checkpoint_transcript_start, 0);
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

#[test]
fn test_handle_lifecycle_session_end_marks_session_ended() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    with_cwd(dir.path(), || {
        let backend = create_session_backend_or_local(dir.path());
        backend
            .save_session(&SessionState {
                session_id: "copilot-session-end".to_string(),
                phase: SessionPhase::Active,
                pending: PendingCheckpointState {
                    files_touched: vec!["file.txt".to_string()],
                    ..Default::default()
                },
                ..Default::default()
            })
            .unwrap();

        let adapter = CopilotCliLifecycleAdapter;
        let mut event = sample_event(LifecycleEventType::SessionEnd);
        event.session_id = "copilot-session-end".to_string();

        handle_lifecycle_session_end(&adapter, &event).expect("session end should persist state");

        let state = backend
            .load_session("copilot-session-end")
            .unwrap()
            .expect("session should exist");
        assert_eq!(state.phase, SessionPhase::Ended);
        assert!(state.ended_at.is_some());
        assert!(state.last_interaction_time.is_some());
    });
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
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    with_cwd(dir.path(), || {
        let adapter = ClaudeCodeLifecycleAdapter;

        let cases = vec![
            (
                "session start empty id",
                LifecycleEvent {
                    event_type: Some(LifecycleEventType::SessionStart),
                    session_id: String::new(),
                    ..sample_event(LifecycleEventType::SessionStart)
                },
                "session_id is required",
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
    });
}

// CLI-873
#[test]
fn test_parse_hook_event_session_start_claude() {
    let adapter = ClaudeCodeLifecycleAdapter;
    let mut stdin = Cursor::new(
        r#"{"session_id":"test-session-123","transcript_path":"/tmp/transcript.jsonl","model":"claude-opus-4-1"}"#,
    );

    let event = adapter
        .parse_hook_event(CLAUDE_HOOK_SESSION_START, &mut stdin)
        .unwrap()
        .expect("event should exist");

    assert_eq!(Some(LifecycleEventType::SessionStart), event.event_type);
    assert_eq!("test-session-123", event.session_id);
    assert_eq!("/tmp/transcript.jsonl", event.session_ref);
    assert_eq!("claude-opus-4-1", event.model);
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
fn test_parse_hook_event_post_todo_maps_todo_checkpoint_claude() {
    let adapter = ClaudeCodeLifecycleAdapter;
    let mut stdin = Cursor::new(
        r#"{"session_id":"todo-session","transcript_path":"/tmp/todo.jsonl","tool_use_id":"toolu_todo","tool_name":"TodoWrite","tool_input":{"todos":[{"content":"Ship feature","status":"completed"}]}}"#,
    );

    let event = adapter
        .parse_hook_event(CLAUDE_HOOK_POST_TODO, &mut stdin)
        .unwrap()
        .expect("event should exist");

    assert_eq!(Some(LifecycleEventType::TodoCheckpoint), event.event_type);
    assert_eq!("todo-session", event.session_id);
    assert_eq!("/tmp/todo.jsonl", event.session_ref);
    assert_eq!("toolu_todo", event.tool_use_id);
    assert_eq!("TodoWrite", event.tool_name);
    assert_eq!(
        Some(serde_json::json!({
            "todos": [{"content": "Ship feature", "status": "completed"}]
        })),
        event.tool_input
    );
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
            Some(LifecycleEventType::TodoCheckpoint),
            false,
            r#"{"session_id":"s7","transcript_path":"/t","tool_name":"TodoWrite","tool_use_id":"todo-7","tool_input":{"todos":[]}}"#,
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
    let mut stdin = Cursor::new(r#"{"sessionId":"copilot-session-1","initialPrompt":"Bootstrap"}"#);
    let event = adapter
        .parse_hook_event(COPILOT_HOOK_SESSION_START, &mut stdin)
        .unwrap()
        .expect("event should exist");

    assert_eq!(Some(LifecycleEventType::SessionStart), event.event_type);
    assert_eq!("copilot-session-1", event.session_id);
    assert_eq!("Bootstrap", event.prompt);
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
fn test_parse_hook_event_turn_start_codex() {
    let adapter = CodexLifecycleAdapter;
    let mut stdin = Cursor::new(
        r#"{"session_id":"codex-session-3","transcript_path":"/tmp/codex-3.jsonl","prompt":"Refactor tracked file","model":"gpt-5.4-codex"}"#,
    );
    let event = adapter
        .parse_hook_event(CODEX_HOOK_USER_PROMPT_SUBMIT, &mut stdin)
        .expect("parse")
        .expect("event");
    assert_eq!(event.event_type, Some(LifecycleEventType::TurnStart));
    assert_eq!(event.session_id, "codex-session-3");
    assert_eq!(event.session_ref, "/tmp/codex-3.jsonl");
    assert_eq!(event.prompt, "Refactor tracked file");
    assert_eq!(event.model, "gpt-5.4-codex");
}

#[test]
fn test_parse_hook_event_pre_tool_use_codex_returns_none() {
    let adapter = CodexLifecycleAdapter;
    let mut stdin = Cursor::new(
        r#"{"session_id":"codex-session-ptu","transcript_path":"/tmp/codex-ptu.jsonl","tool_name":"Bash","tool_use_id":"toolu_1","tool_input":{"command":"git status"}}"#,
    );
    let event = adapter
        .parse_hook_event(CODEX_HOOK_PRE_TOOL_USE, &mut stdin)
        .expect("parse");
    assert!(event.is_none());
}

#[test]
fn test_parse_hook_event_post_tool_use_codex_returns_none() {
    let adapter = CodexLifecycleAdapter;
    let mut stdin = Cursor::new(
        r#"{"session_id":"codex-session-post","transcript_path":"/tmp/codex-post.jsonl","tool_name":"Bash","tool_use_id":"toolu_2","tool_input":{"command":"git status"},"tool_response":"clean"}"#,
    );
    let event = adapter
        .parse_hook_event(CODEX_HOOK_POST_TOOL_USE, &mut stdin)
        .expect("parse");
    assert!(event.is_none());
}

#[test]
fn test_hook_names_codex() {
    let adapter = CodexLifecycleAdapter;
    let names = adapter.hook_names();
    assert_eq!(
        names,
        vec![
            CODEX_HOOK_SESSION_START,
            CODEX_HOOK_USER_PROMPT_SUBMIT,
            CODEX_HOOK_PRE_TOOL_USE,
            CODEX_HOOK_POST_TOOL_USE,
            CODEX_HOOK_STOP,
        ]
    );
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
fn test_subagent_start_handler_creates_marker_and_updates_session() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    with_cwd(dir.path(), || {
        let backend = create_session_backend_or_local(dir.path());
        backend
            .save_session(&SessionState {
                session_id: "session-123".to_string(),
                phase: SessionPhase::Active,
                ..Default::default()
            })
            .unwrap();

        let adapter = ClaudeCodeLifecycleAdapter;
        let mut event = sample_event(LifecycleEventType::SubagentStart);
        event.session_id = "session-123".to_string();
        event.tool_use_id = "toolu_123".to_string();
        event.subagent_id = "subagent-1".to_string();

        handle_lifecycle_subagent_start(&adapter, &event)
            .expect("subagent start should persist runtime marker state");

        assert!(backend.load_pre_task_marker("toolu_123").unwrap().is_some());
        let state = backend
            .load_session("session-123")
            .unwrap()
            .expect("session should exist");
        assert!(state.last_interaction_time.is_some());
    });
}

#[test]
fn test_subagent_end_handler_clears_marker_and_updates_session_without_repo_changes() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    with_cwd(dir.path(), || {
        let backend = create_session_backend_or_local(dir.path());
        backend
            .save_session(&SessionState {
                session_id: "session-123".to_string(),
                phase: SessionPhase::Active,
                ..Default::default()
            })
            .unwrap();

        let adapter = ClaudeCodeLifecycleAdapter;
        let mut start_event = sample_event(LifecycleEventType::SubagentStart);
        start_event.session_id = "session-123".to_string();
        start_event.tool_use_id = "toolu_123".to_string();
        start_event.subagent_id = "subagent-1".to_string();
        start_event.tool_input = Some(serde_json::json!({"prompt":"task start"}));
        handle_lifecycle_subagent_start(&adapter, &start_event)
            .expect("subagent start should create marker");

        let mut end_event = sample_event(LifecycleEventType::SubagentEnd);
        end_event.session_id = "session-123".to_string();
        end_event.tool_use_id = "toolu_123".to_string();
        end_event.subagent_id = "subagent-1".to_string();
        end_event.tool_input = Some(serde_json::json!({"prompt":"task done"}));

        handle_lifecycle_subagent_end(&adapter, &end_event)
            .expect("subagent end should complete cleanly without repository changes");

        assert!(backend.load_pre_task_marker("toolu_123").unwrap().is_none());
        let state = backend
            .load_session("session-123")
            .unwrap()
            .expect("session should exist");
        assert!(state.last_interaction_time.is_some());
    });
}

//! Tests for lifecycle orchestration: capture pre-prompt state, turn-end metadata extraction,
//! and token usage. These tests are written against the intended behavior and will fail until
//! the corresponding orchestration is implemented.

use super::adapters::GeminiCliLifecycleAdapter;
use super::{
    LifecycleEvent, LifecycleEventType, capture_pre_prompt_state, handle_lifecycle_turn_end,
};
use crate::engine::agent::gemini_cli::agent::GeminiCliAgent;
use crate::engine::session::backend::SessionBackend;
use crate::engine::session::local_backend::LocalFileBackend;
use crate::test_support::process_state::with_cwd;
use std::process::Command;

fn setup_git_repo(dir: &tempfile::TempDir) {
    let run = |args: &[&str]| {
        let out = Command::new("git")
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
}

/// Fails until capture_pre_prompt_state uses the agent's get_transcript_position and persists it.
#[test]
fn capture_pre_prompt_state_persists_transcript_position_from_agent() {
    let dir = tempfile::tempdir().expect("temp dir");
    setup_git_repo(&dir);

    with_cwd(dir.path(), || {
        let transcript = br#"{"messages":[
            {"type":"user","content":"first"},
            {"type":"gemini","content":"response"},
            {"type":"user","content":"second"}
        ]}"#;
        let transcript_path = dir.path().join("transcript.json");
        std::fs::write(&transcript_path, transcript).unwrap();

        let agent = GeminiCliAgent;
        let session_id = "test-session-123";
        let repo_root = dir.path();
        capture_pre_prompt_state(
            &agent,
            session_id,
            transcript_path.to_str().unwrap(),
            repo_root,
        )
        .expect("capture_pre_prompt_state should succeed");

        let backend = LocalFileBackend::new(repo_root);
        let state = backend
            .load_pre_prompt(session_id)
            .expect("load_pre_prompt should succeed")
            .expect("pre-prompt state should exist");

        assert_eq!(
            state.transcript_offset, 3,
            "capture_pre_prompt_state should persist transcript position from agent (3 messages); \
             currently stub saves 0"
        );
    });
}

/// Fails until handle_lifecycle_turn_end creates session metadata and writes prompt/summary from transcript.
#[test]
fn turn_end_writes_prompt_and_summary_to_session_metadata() {
    let dir = tempfile::tempdir().expect("temp dir");
    setup_git_repo(&dir);

    with_cwd(dir.path(), || {
        let transcript = br#"{"messages":[
            {"type":"user","content":"What is 2+2?"},
            {"type":"gemini","content":"2+2 equals 4."}
        ]}"#;
        let transcript_path = dir.path().join("transcript.json");
        std::fs::write(&transcript_path, transcript).unwrap();
        let transcript_path = transcript_path.canonicalize().unwrap_or(transcript_path);

        let adapter = GeminiCliLifecycleAdapter;
        let event = LifecycleEvent {
            event_type: Some(LifecycleEventType::TurnEnd),
            session_id: "turn-end-session".to_string(),
            session_ref: transcript_path.to_string_lossy().to_string(),
            ..LifecycleEvent::default()
        };

        handle_lifecycle_turn_end(&adapter, &event)
            .expect("handle_lifecycle_turn_end should succeed");

        let meta_dir = dir
            .path()
            .join(".bitloops")
            .join("metadata")
            .join("turn-end-session");
        let prompt_file = meta_dir.join("prompt.txt");
        let summary_file = meta_dir.join("summary.txt");
        assert!(
            prompt_file.exists(),
            "session metadata should contain prompt.txt"
        );
        assert!(
            summary_file.exists(),
            "session metadata should contain summary.txt"
        );

        let prompt_content = std::fs::read_to_string(&prompt_file).unwrap();
        let summary_content = std::fs::read_to_string(&summary_file).unwrap();
        assert!(
            prompt_content.contains("2+2"),
            "prompt.txt should contain user prompt from transcript"
        );
        assert!(
            summary_content.contains("4"),
            "summary.txt should contain last assistant message from transcript"
        );
    });
}

/// Fails until turn-end flow includes token usage from the agent in the saved step/session.
#[test]
fn turn_end_includes_token_usage_in_step() {
    let dir = tempfile::tempdir().expect("temp dir");
    setup_git_repo(&dir);

    with_cwd(dir.path(), || {
        let transcript = br#"{"messages":[
            {"type":"user","content":"hi"},
            {"type":"gemini","content":"hello","tokens":{"input":10,"output":20,"cached":5,"thoughts":0,"tool":0,"total":35}}
        ]}"#;
        let transcript_path = dir.path().join("transcript.json");
        std::fs::write(&transcript_path, transcript).unwrap();

        let adapter = GeminiCliLifecycleAdapter;
        let event = LifecycleEvent {
            event_type: Some(LifecycleEventType::TurnEnd),
            session_id: "token-session".to_string(),
            session_ref: transcript_path.to_string_lossy().to_string(),
            ..LifecycleEvent::default()
        };

        handle_lifecycle_turn_end(&adapter, &event)
            .expect("handle_lifecycle_turn_end should succeed");

        let backend = LocalFileBackend::new(dir.path());
        let state = backend
            .load_session("token-session")
            .expect("load_session should succeed");

        let state = state.expect("session state should exist after turn end");
        assert!(
            state.token_usage.is_some() && state.token_usage.as_ref().unwrap().api_call_count > 0,
            "session state should include token usage from transcript after turn end"
        );
    });
}

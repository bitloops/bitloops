use super::*;
use crate::adapters::agents::{
    AGENT_NAME_CLAUDE_CODE, AGENT_TYPE_CLAUDE_CODE, AGENT_TYPE_CODEX, AGENT_TYPE_CURSOR,
};
use crate::host::checkpoints::lifecycle::UNKNOWN_SESSION_ID;
use crate::host::checkpoints::session::create_session_backend_or_local;
use crate::host::checkpoints::session::local_backend::LocalFileBackend;
use crate::host::checkpoints::session::phase::SessionPhase;
use crate::host::checkpoints::session::state::PendingCheckpointState;
use crate::host::checkpoints::strategy::manual_commit::ManualCommitStrategy;
use crate::host::checkpoints::strategy::noop::NoOpStrategy;
use crate::host::checkpoints::strategy::registry;
use crate::host::checkpoints::strategy::{StepContext, TaskStepContext};
use crate::host::hooks::dispatcher::{
    CursorHookVerb, HooksAgent, current_hook_agent_name_for_tests, dispatch_cursor_hook,
    run_agent_hook_with_logging,
};
use crate::test_support::git_fixtures::ensure_test_store_backends;
use crate::test_support::logger_lock::with_logger_test_lock;
use crate::test_support::process_state::{
    git_command, isolated_git_command, with_cwd, with_process_state,
};
use anyhow::anyhow;
use serde_json::json;
use std::fs;
use std::path::Path;
use std::sync::Mutex;
use tempfile::TempDir;

fn setup() -> (TempDir, LocalFileBackend, NoOpStrategy) {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join(".git")).unwrap();
    ensure_test_store_backends(dir.path());
    let backend = LocalFileBackend::new(dir.path());
    (dir, backend, NoOpStrategy)
}

fn setup_git_repo(dir: &TempDir) {
    let run = |args: &[&str]| {
        let out = isolated_git_command(dir.path())
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {:?} failed", args);
    };
    run(&["init"]);
    run(&["config", "user.email", "t@t.com"]);
    run(&["config", "user.name", "Test"]);
    fs::write(dir.path().join("tracked.txt"), "one\n").unwrap();
    fs::write(dir.path().join(".gitignore"), "stores/\n").unwrap();
    ensure_test_store_backends(dir.path());
    run(&["add", "."]);
    run(&["commit", "-m", "initial"]);
}

fn run_git(dir: &Path, args: &[&str]) {
    let out = isolated_git_command(dir).args(args).output().unwrap();
    assert!(out.status.success(), "git {:?} failed", args);
}

fn open_events_duckdb(repo_root: &Path) -> duckdb::Connection {
    let backends = crate::config::resolve_store_backend_config_for_repo(repo_root)
        .expect("resolve store backends");
    let path = backends.events.resolve_duckdb_db_path_for_repo(repo_root);
    duckdb::Connection::open(path).expect("open events duckdb")
}

fn interaction_event_types(repo_root: &Path) -> Vec<String> {
    let conn = open_events_duckdb(repo_root);
    let mut stmt = conn
        .prepare("SELECT event_type FROM interaction_events ORDER BY event_time ASC, event_id ASC")
        .expect("prepare event type query");
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query event types");
    rows.collect::<Result<Vec<_>, _>>()
        .expect("collect event types")
}

fn assert_sorted_event_types(repo_root: &Path, mut expected: Vec<&str>) {
    let mut actual = interaction_event_types(repo_root);
    actual.sort();
    expected.sort();
    assert_eq!(
        actual,
        expected.into_iter().map(str::to_string).collect::<Vec<_>>()
    );
}

fn interaction_row_counts(repo_root: &Path) -> (i64, i64, i64) {
    let conn = open_events_duckdb(repo_root);
    let sessions = conn
        .query_row("SELECT COUNT(*) FROM interaction_sessions", [], |row| {
            row.get(0)
        })
        .expect("count interaction sessions");
    let turns = conn
        .query_row("SELECT COUNT(*) FROM interaction_turns", [], |row| {
            row.get(0)
        })
        .expect("count interaction turns");
    let events = conn
        .query_row("SELECT COUNT(*) FROM interaction_events", [], |row| {
            row.get(0)
        })
        .expect("count interaction events");
    (sessions, turns, events)
}

fn interaction_turn_fragment(repo_root: &Path) -> String {
    let conn = open_events_duckdb(repo_root);
    conn.query_row(
        "SELECT transcript_fragment FROM interaction_turns ORDER BY updated_at DESC LIMIT 1",
        [],
        |row| row.get(0),
    )
    .expect("read interaction turn transcript_fragment")
}

fn interaction_session_model(repo_root: &Path, session_id: &str) -> String {
    let conn = open_events_duckdb(repo_root);
    conn.query_row(
        "SELECT model FROM interaction_sessions WHERE session_id = ?1 ORDER BY updated_at DESC LIMIT 1",
        [session_id],
        |row| row.get(0),
    )
    .expect("read interaction session model")
}

fn interaction_turn_model(repo_root: &Path, session_id: &str) -> String {
    let conn = open_events_duckdb(repo_root);
    conn.query_row(
        "SELECT model FROM interaction_turns WHERE session_id = ?1 ORDER BY updated_at DESC LIMIT 1",
        [session_id],
        |row| row.get(0),
    )
    .expect("read interaction turn model")
}

fn interaction_turn_end_payload(repo_root: &Path) -> serde_json::Value {
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

fn write_claude_write_transcript(path: &Path, file_path: &str) {
    let line = json!({
        "type":"assistant",
        "uuid":"a1",
        "message":{
            "model":"claude-opus-4-1",
            "content":[
                {
                    "type":"tool_use",
                    "name":"Write",
                    "input":{"file_path":file_path}
                }
            ]
        }
    });
    fs::write(path, format!("{line}\n")).unwrap();
}

#[derive(Default)]
struct RecordingStrategy {
    step_calls: Mutex<Vec<StepContext>>,
    task_calls: Mutex<Vec<TaskStepContext>>,
}

impl Strategy for RecordingStrategy {
    fn name(&self) -> &str {
        "recording"
    }

    fn save_step(&self, ctx: &StepContext) -> Result<()> {
        self.step_calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(ctx.clone());
        Ok(())
    }

    fn save_task_step(&self, ctx: &TaskStepContext) -> Result<()> {
        self.task_calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(ctx.clone());
        Ok(())
    }

    fn prepare_commit_msg(&self, _commit_msg_file: &Path, _source: Option<&str>) -> Result<()> {
        Ok(())
    }

    fn commit_msg(&self, _commit_msg_file: &Path) -> Result<()> {
        Ok(())
    }

    fn post_commit(&self) -> Result<()> {
        Ok(())
    }

    fn pre_push(&self, _remote: &str, _stdin_lines: &[String]) -> Result<()> {
        Ok(())
    }
}

#[derive(Default)]
struct FailingStrategy;

impl Strategy for FailingStrategy {
    fn name(&self) -> &str {
        "failing"
    }

    fn save_step(&self, _ctx: &StepContext) -> Result<()> {
        Err(anyhow!("save_step failed"))
    }

    fn save_task_step(&self, _ctx: &TaskStepContext) -> Result<()> {
        Err(anyhow!("save_task_step failed"))
    }

    fn prepare_commit_msg(&self, _commit_msg_file: &Path, _source: Option<&str>) -> Result<()> {
        Ok(())
    }

    fn commit_msg(&self, _commit_msg_file: &Path) -> Result<()> {
        Ok(())
    }

    fn post_commit(&self) -> Result<()> {
        Ok(())
    }

    fn pre_push(&self, _remote: &str, _stdin_lines: &[String]) -> Result<()> {
        Ok(())
    }
}

#[test]
fn session_start_creates_session_state() {
    let (_dir, backend, _strat) = setup();

    let input = SessionInfoInput {
        session_id: "s1".to_string(),
        transcript_path: "/tmp/t.jsonl".to_string(),
    };
    handle_session_start(input, &backend, None).unwrap();

    let state = backend.load_session("s1").unwrap().unwrap();
    assert_eq!(state.session_id, "s1");
    assert_eq!(state.phase, SessionPhase::Idle);
}

#[test]
fn session_start_rejects_empty_session_id() {
    let (_dir, backend, _strat) = setup();
    let err = handle_session_start(
        SessionInfoInput {
            session_id: "   ".to_string(),
            transcript_path: "/tmp/t.jsonl".to_string(),
        },
        &backend,
        None,
    )
    .expect_err("expected validation error");
    assert!(
        err.to_string()
            .contains("session-start requires non-empty session_id"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn session_start_resets_ended_session() {
    let (_dir, backend, _strat) = setup();

    // Pre-create an ended session.
    let ended = SessionState {
        session_id: "s2".to_string(),
        phase: SessionPhase::Ended,
        ..Default::default()
    };
    backend.save_session(&ended).unwrap();

    let input = SessionInfoInput {
        session_id: "s2".to_string(),
        transcript_path: "/tmp/t.jsonl".to_string(),
    };
    handle_session_start(input, &backend, None).unwrap();

    let state = backend.load_session("s2").unwrap().unwrap();
    assert_eq!(state.phase, SessionPhase::Idle);
}

#[test]
fn user_prompt_submit_transitions_to_active() {
    with_process_state(None, &[], || {
        let (_dir, backend, _strat) = setup();

        let input = UserPromptSubmitInput {
            session_id: "s3".to_string(),
            transcript_path: "/tmp/t.jsonl".to_string(),
            prompt: "Fix the bug".to_string(),
        };
        handle_user_prompt_submit(input, &backend, None).unwrap();

        let state = backend.load_session("s3").unwrap().unwrap();
        assert_eq!(state.phase, SessionPhase::Active);
        assert_eq!(state.turn_id.len(), 12);
        assert!(
            state
                .turn_id
                .chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
            "turn_id should be 12 lowercase hex chars, got {}",
            state.turn_id
        );

        // Pre-prompt state should be saved.
        let pp = backend.load_pre_prompt("s3").unwrap();
        assert!(pp.is_some());
        assert_eq!(pp.unwrap().prompt, "Fix the bug");
    });
}

#[test]
fn user_prompt_submit_rejects_empty_session_id() {
    with_process_state(None, &[], || {
        let (_dir, backend, _strat) = setup();
        let err = handle_user_prompt_submit(
            UserPromptSubmitInput {
                session_id: " ".to_string(),
                transcript_path: "/tmp/t.jsonl".to_string(),
                prompt: "Fix the bug".to_string(),
            },
            &backend,
            None,
        )
        .expect_err("expected validation error");
        assert!(
            err.to_string()
                .contains("turn-start requires non-empty session_id"),
            "unexpected error: {err:#}"
        );
    });
}

#[test]
fn user_prompt_submit_records_first_prompt() {
    with_process_state(None, &[], || {
        let (_dir, backend, _strat) = setup();

        let long_prompt = "A".repeat(200);
        let input = UserPromptSubmitInput {
            session_id: "s4".to_string(),
            transcript_path: "/tmp/t.jsonl".to_string(),
            prompt: long_prompt,
        };
        handle_user_prompt_submit(input, &backend, None).unwrap();

        let state = backend.load_session("s4").unwrap().unwrap();
        assert_eq!(
            state.first_prompt.chars().count(),
            100,
            "first_prompt truncated to 100 chars"
        );
    });
}

#[test]
fn stop_transitions_to_idle_deletes_pre_prompt() {
    let (_dir, backend, strat) = setup();

    // Set up active session with pre-prompt state.
    let active = SessionState {
        session_id: "s5".to_string(),
        phase: SessionPhase::Active,
        ..Default::default()
    };
    backend.save_session(&active).unwrap();
    backend
        .save_pre_prompt(&PrePromptState {
            session_id: "s5".to_string(),
            prompt: "test".to_string(),
            transcript_path: "/tmp/t.jsonl".to_string(),
            ..Default::default()
        })
        .unwrap();

    let input = SessionInfoInput {
        session_id: "s5".to_string(),
        transcript_path: "/tmp/t.jsonl".to_string(),
    };
    handle_stop(input, &backend, &strat, None).unwrap();

    let state = backend.load_session("s5").unwrap().unwrap();
    assert_eq!(state.phase, SessionPhase::Idle);

    // Pre-prompt state should be deleted.
    assert!(backend.load_pre_prompt("s5").unwrap().is_none());
}

#[test]
fn session_end_sets_ended_phase() {
    let (_dir, backend, _strat) = setup();

    let state = SessionState {
        session_id: "s6".to_string(),
        phase: SessionPhase::Idle,
        ..Default::default()
    };
    backend.save_session(&state).unwrap();

    let input = SessionInfoInput {
        session_id: "s6".to_string(),
        transcript_path: "/tmp/t.jsonl".to_string(),
    };
    handle_session_end(input, &backend).unwrap();

    let loaded = backend.load_session("s6").unwrap().unwrap();
    assert_eq!(loaded.phase, SessionPhase::Ended);
    assert!(loaded.ended_at.is_some(), "ended_at should be set");
}

#[test]
fn claude_stop_persists_interactions_before_save_step_failure() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let backend = LocalFileBackend::new(dir.path());
    let strategy = FailingStrategy;

    handle_session_start_with_profile(
        SessionInfoInput {
            session_id: "claude-event-first".to_string(),
            transcript_path: "/tmp/claude-event-first.jsonl".to_string(),
        },
        &backend,
        Some(dir.path()),
        Some(CLAUDE_HOOK_AGENT_PROFILE),
    )
    .unwrap();

    handle_user_prompt_submit_with_strategy_and_profile(
        UserPromptSubmitInput {
            session_id: "claude-event-first".to_string(),
            transcript_path: "/tmp/claude-event-first.jsonl".to_string(),
            prompt: "Update tracked file".to_string(),
        },
        &backend,
        &NoOpStrategy,
        Some(dir.path()),
        CLAUDE_HOOK_AGENT_PROFILE,
    )
    .unwrap();

    fs::write(dir.path().join("tracked.txt"), "claude-event-first\n").unwrap();

    let err = handle_stop_with_profile(
        SessionInfoInput {
            session_id: "claude-event-first".to_string(),
            transcript_path: "/tmp/claude-event-first.jsonl".to_string(),
        },
        &backend,
        &strategy,
        Some(dir.path()),
        CLAUDE_HOOK_AGENT_PROFILE,
    )
    .expect_err("stop should fail after interaction capture");
    assert!(
        err.to_string().contains("save_step failed"),
        "unexpected error: {err:#}"
    );

    assert_eq!(interaction_row_counts(dir.path()), (1, 1, 3));
    assert_sorted_event_types(dir.path(), vec!["session_start", "turn_start", "turn_end"]);
}

#[test]
fn claude_stop_persists_transcript_fragment_in_event_store() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let backend = LocalFileBackend::new(dir.path());
    let transcript_dir = tempfile::tempdir().unwrap();
    let transcript_path = transcript_dir.path().join("claude-fragment.jsonl");

    handle_session_start_with_profile(
        SessionInfoInput {
            session_id: "claude-fragment".to_string(),
            transcript_path: transcript_path.to_string_lossy().to_string(),
        },
        &backend,
        Some(dir.path()),
        Some(CLAUDE_HOOK_AGENT_PROFILE),
    )
    .unwrap();

    handle_user_prompt_submit_with_strategy_and_profile(
        UserPromptSubmitInput {
            session_id: "claude-fragment".to_string(),
            transcript_path: transcript_path.to_string_lossy().to_string(),
            prompt: "Update tracked file".to_string(),
        },
        &backend,
        &NoOpStrategy,
        Some(dir.path()),
        CLAUDE_HOOK_AGENT_PROFILE,
    )
    .unwrap();

    fs::write(
        &transcript_path,
        "{\"type\":\"user\",\"message\":{\"content\":\"Update tracked file\"}}\n\
{\"type\":\"assistant\",\"message\":{\"model\":\"claude-opus-4-1\",\"content\":[{\"type\":\"text\",\"text\":\"Implemented the change\"}]}}\n",
    )
    .unwrap();

    handle_stop_with_profile(
        SessionInfoInput {
            session_id: "claude-fragment".to_string(),
            transcript_path: transcript_path.to_string_lossy().to_string(),
        },
        &backend,
        &NoOpStrategy,
        Some(dir.path()),
        CLAUDE_HOOK_AGENT_PROFILE,
    )
    .unwrap();

    let transcript_fragment = interaction_turn_fragment(dir.path());
    assert!(
        transcript_fragment.contains("Implemented the change"),
        "turn row should persist the completed transcript fragment"
    );

    let payload = interaction_turn_end_payload(dir.path());
    assert_eq!(
        payload["transcript_fragment"].as_str().unwrap_or_default(),
        transcript_fragment,
        "turn_end event payload should mirror the persisted transcript fragment"
    );
    assert_eq!(
        interaction_session_model(dir.path(), "claude-fragment"),
        "claude-opus-4-1"
    );
    assert_eq!(
        interaction_turn_model(dir.path(), "claude-fragment"),
        "claude-opus-4-1"
    );
}

#[test]
fn cursor_stop_persists_structured_transcript_fragment_in_event_store() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let backend = LocalFileBackend::new(dir.path());
    let transcript_dir = tempfile::tempdir().unwrap();
    let transcript_path = transcript_dir.path().join("cursor-fragment.json");
    let strategy = RecordingStrategy::default();

    dispatch_cursor_hook(
        &CursorHookVerb::BeforeSubmitPrompt,
        &format!(
            r#"{{"conversation_id":"cursor-fragment","transcript_path":"{}","prompt":"Update tracked file"}}"#,
            transcript_path.to_string_lossy()
        ),
        &backend,
        &strategy,
        dir.path(),
        "before-submit-prompt",
    )
    .unwrap();

    fs::write(
        &transcript_path,
        r#"{"messages":[{"type":"user","content":"Update tracked file"},{"type":"assistant","content":"Implemented the change"}]}"#,
    )
    .unwrap();

    dispatch_cursor_hook(
        &CursorHookVerb::Stop,
        &format!(
            r#"{{"conversation_id":"cursor-fragment","transcript_path":"{}"}}"#,
            transcript_path.to_string_lossy()
        ),
        &backend,
        &strategy,
        dir.path(),
        "stop",
    )
    .unwrap();

    let transcript_fragment = interaction_turn_fragment(dir.path());
    assert!(
        transcript_fragment.contains("Implemented the change"),
        "turn row should persist the completed structured transcript fragment"
    );

    let payload = interaction_turn_end_payload(dir.path());
    assert_eq!(
        payload["transcript_fragment"].as_str().unwrap_or_default(),
        transcript_fragment,
        "turn_end event payload should mirror the persisted structured transcript fragment"
    );
}

#[test]
fn cursor_session_start_creates_or_updates_session_state() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let backend = LocalFileBackend::new(dir.path());
    let strat = RecordingStrategy::default();

    dispatch_cursor_hook(
        &CursorHookVerb::SessionStart,
        r#"{"conversation_id":"cursor-s1","transcript_path":"/tmp/cursor-s1.jsonl"}"#,
        &backend,
        &strat,
        dir.path(),
        "session-start",
    )
    .unwrap();

    let state = backend.load_session("cursor-s1").unwrap().unwrap();
    assert_eq!(state.session_id, "cursor-s1");
    assert_eq!(state.phase, SessionPhase::Idle);
    assert_eq!(state.transcript_path, "/tmp/cursor-s1.jsonl");
}

#[test]
fn cursor_session_start_rejects_empty_conversation_id() {
    let (dir, backend, strat) = setup();
    let err = dispatch_cursor_hook(
        &CursorHookVerb::SessionStart,
        r#"{"conversation_id":"  ","transcript_path":"/tmp/cursor-empty.jsonl"}"#,
        &backend,
        &strat,
        dir.path(),
        "session-start",
    )
    .expect_err("expected validation error");
    assert!(
        err.to_string().contains("session_id is required"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn cursor_stop_persists_interactions_before_save_step_failure() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let backend = LocalFileBackend::new(dir.path());
    let strategy = FailingStrategy;

    dispatch_cursor_hook(
        &CursorHookVerb::SessionStart,
        r#"{"conversation_id":"cursor-event-first","transcript_path":"/tmp/cursor-event-first.jsonl"}"#,
        &backend,
        &strategy,
        dir.path(),
        "session-start",
    )
    .unwrap();

    dispatch_cursor_hook(
        &CursorHookVerb::BeforeSubmitPrompt,
        r#"{"conversation_id":"cursor-event-first","transcript_path":"/tmp/cursor-event-first.jsonl","prompt":"Update tracked file"}"#,
        &backend,
        &strategy,
        dir.path(),
        "before-submit-prompt",
    )
    .unwrap();

    fs::write(dir.path().join("tracked.txt"), "cursor-event-first\n").unwrap();

    let err = dispatch_cursor_hook(
        &CursorHookVerb::Stop,
        r#"{"conversation_id":"cursor-event-first","transcript_path":"/tmp/cursor-event-first.jsonl"}"#,
        &backend,
        &strategy,
        dir.path(),
        "stop",
    )
    .expect_err("stop should fail after interaction capture");
    assert!(
        err.to_string().contains("save_step failed"),
        "unexpected error: {err:#}"
    );

    assert_eq!(interaction_row_counts(dir.path()), (1, 1, 3));
    assert_sorted_event_types(dir.path(), vec!["session_start", "turn_start", "turn_end"]);
}

#[test]
fn cursor_before_submit_prompt_creates_pre_prompt_state() {
    let (dir, backend, strat) = setup();

    with_process_state(Some(dir.path()), &[], || {
        dispatch_cursor_hook(
            &CursorHookVerb::BeforeSubmitPrompt,
            r#"{"conversation_id":"cursor-s2","transcript_path":"/tmp/cursor-s2.jsonl","prompt":"Fix bug in parser"}"#,
            &backend,
            &strat,
            dir.path(),
            "before-submit-prompt",
        )
    })
    .unwrap();

    let state = backend.load_session("cursor-s2").unwrap().unwrap();
    assert_eq!(state.phase, SessionPhase::Active);
    assert_eq!(state.agent_type, AGENT_TYPE_CURSOR);
    let pre_prompt = backend.load_pre_prompt("cursor-s2").unwrap().unwrap();
    assert_eq!(pre_prompt.prompt, "Fix bug in parser");
    assert_eq!(pre_prompt.transcript_path, "/tmp/cursor-s2.jsonl");
}

#[test]
fn cursor_before_submit_prompt_persists_model_from_hook_payload() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let backend = create_session_backend_or_local(dir.path());
    let strategy = RecordingStrategy::default();

    with_process_state(Some(dir.path()), &[], || {
        dispatch_cursor_hook(
            &CursorHookVerb::BeforeSubmitPrompt,
            r#"{"conversation_id":"cursor-model-1","transcript_path":"/tmp/cursor-model-1.jsonl","prompt":"Fix bug in parser","modelSlug":"gpt-5.4-mini"}"#,
            backend.as_ref(),
            &strategy,
            dir.path(),
            "before-submit-prompt",
        )
    })
    .expect("before-submit-prompt should succeed");

    assert_eq!(
        interaction_session_model(dir.path(), "cursor-model-1"),
        "gpt-5.4-mini"
    );
    assert_eq!(
        interaction_turn_model(dir.path(), "cursor-model-1"),
        "gpt-5.4-mini"
    );
}

#[test]
fn cursor_before_submit_prompt_rejects_empty_conversation_id() {
    let (dir, backend, strat) = setup();
    let err = dispatch_cursor_hook(
        &CursorHookVerb::BeforeSubmitPrompt,
        r#"{"conversation_id":" ","transcript_path":"/tmp/cursor-s2.jsonl","prompt":"Fix bug in parser"}"#,
        &backend,
        &strat,
        dir.path(),
        "before-submit-prompt",
    )
    .expect_err("expected validation error");
    assert!(
        err.to_string().contains("session_id is required"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn cursor_before_shell_execution_creates_shell_fallback_pre_prompt() {
    let (dir, backend, strat) = setup();

    with_process_state(Some(dir.path()), &[], || {
        dispatch_cursor_hook(
            &CursorHookVerb::BeforeShellExecution,
            r#"{"conversation_id":"cursor-shell-1","transcript_path":"/tmp/cursor-shell-1.jsonl","command":"npm test"}"#,
            &backend,
            &strat,
            dir.path(),
            "before-shell-execution",
        )
    })
    .unwrap();

    let state = backend.load_session("cursor-shell-1").unwrap().unwrap();
    assert_eq!(state.phase, SessionPhase::Active);

    let pre_prompt = backend.load_pre_prompt("cursor-shell-1").unwrap().unwrap();
    assert_eq!(
        pre_prompt.source,
        crate::host::checkpoints::session::state::PRE_PROMPT_SOURCE_CURSOR_SHELL
    );
    assert_eq!(pre_prompt.prompt, "Run shell command: npm test");
}

#[test]
fn cursor_after_shell_execution_triggers_stop_for_shell_fallback() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let backend = LocalFileBackend::new(dir.path());
    let strat = RecordingStrategy::default();

    dispatch_cursor_hook(
        &CursorHookVerb::BeforeShellExecution,
        r#"{"conversation_id":"cursor-shell-stop","transcript_path":"","command":"npm test"}"#,
        &backend,
        &strat,
        dir.path(),
        "before-shell-execution",
    )
    .unwrap();

    fs::write(dir.path().join("tracked.txt"), "cursor-shell-change\n").unwrap();

    dispatch_cursor_hook(
        &CursorHookVerb::AfterShellExecution,
        r#"{"conversation_id":"cursor-shell-stop","transcript_path":"","command":"npm test"}"#,
        &backend,
        &strat,
        dir.path(),
        "after-shell-execution",
    )
    .unwrap();

    let calls = strat
        .step_calls
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(calls.len(), 1, "expected one save_step call");
    assert!(
        backend
            .load_pre_prompt("cursor-shell-stop")
            .unwrap()
            .is_none(),
        "shell fallback stop should consume pre-prompt state"
    );
}

#[test]
fn cursor_after_shell_execution_ignores_non_shell_pre_prompt() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let backend = LocalFileBackend::new(dir.path());
    let strat = RecordingStrategy::default();

    dispatch_cursor_hook(
        &CursorHookVerb::BeforeSubmitPrompt,
        r#"{"conversation_id":"cursor-shell-ignore","transcript_path":"","prompt":"Fix bug"}"#,
        &backend,
        &strat,
        dir.path(),
        "before-submit-prompt",
    )
    .unwrap();

    fs::write(dir.path().join("tracked.txt"), "cursor-shell-ignore\n").unwrap();

    dispatch_cursor_hook(
        &CursorHookVerb::AfterShellExecution,
        r#"{"conversation_id":"cursor-shell-ignore","transcript_path":"","command":"npm test"}"#,
        &backend,
        &strat,
        dir.path(),
        "after-shell-execution",
    )
    .unwrap();

    let calls = strat
        .step_calls
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(
        calls.len(),
        0,
        "after-shell fallback should ignore regular turn pre-prompt state"
    );
    assert!(
        backend
            .load_pre_prompt("cursor-shell-ignore")
            .unwrap()
            .is_some(),
        "regular pre-prompt state should remain untouched"
    );
}

#[test]
fn cursor_stop_with_file_changes_triggers_save_step() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let backend = LocalFileBackend::new(dir.path());
    let strat = RecordingStrategy::default();

    dispatch_cursor_hook(
        &CursorHookVerb::BeforeSubmitPrompt,
        r#"{"conversation_id":"cursor-stop","transcript_path":"","prompt":"Update tracked file"}"#,
        &backend,
        &strat,
        dir.path(),
        "before-submit-prompt",
    )
    .unwrap();

    fs::write(dir.path().join("tracked.txt"), "cursor-change\n").unwrap();

    dispatch_cursor_hook(
        &CursorHookVerb::Stop,
        r#"{"conversation_id":"cursor-stop","transcript_path":""}"#,
        &backend,
        &strat,
        dir.path(),
        "stop",
    )
    .unwrap();

    let calls = strat
        .step_calls
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(calls.len(), 1, "expected one save_step call");
    assert_eq!(calls[0].agent_type, "cursor");
    assert!(
        calls[0].modified_files.contains(&"tracked.txt".to_string()),
        "modified files: {:?}, new files: {:?}",
        calls[0].modified_files,
        calls[0].new_files
    );
}

#[test]
fn cursor_stop_with_empty_conversation_id_falls_back_to_unknown_session() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let backend = LocalFileBackend::new(dir.path());
    let strat = RecordingStrategy::default();

    fs::write(dir.path().join("tracked.txt"), "cursor-empty-stop\n").unwrap();

    dispatch_cursor_hook(
        &CursorHookVerb::Stop,
        r#"{"conversation_id":" ","transcript_path":""}"#,
        &backend,
        &strat,
        dir.path(),
        "stop",
    )
    .unwrap();

    let calls = strat
        .step_calls
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(calls.len(), 1, "expected one save_step call");
    assert_eq!(calls[0].session_id, UNKNOWN_SESSION_ID);
}

#[test]
fn cursor_stop_with_manual_strategy_persists_cursor_agent_type() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let backend = LocalFileBackend::new(dir.path());
    let strat = ManualCommitStrategy::new(dir.path());

    dispatch_cursor_hook(
        &CursorHookVerb::BeforeSubmitPrompt,
        r#"{"conversation_id":"cursor-manual","transcript_path":"","prompt":"Update tracked file"}"#,
        &backend,
        &strat,
        dir.path(),
        "before-submit-prompt",
    )
    .unwrap();

    fs::write(dir.path().join("tracked.txt"), "cursor-manual-change\n").unwrap();

    dispatch_cursor_hook(
        &CursorHookVerb::Stop,
        r#"{"conversation_id":"cursor-manual","transcript_path":""}"#,
        &backend,
        &strat,
        dir.path(),
        "stop",
    )
    .unwrap();

    let state = backend.load_session("cursor-manual").unwrap().unwrap();
    assert_eq!(state.agent_type, AGENT_TYPE_CURSOR);
}

#[test]
fn claude_before_submit_prompt_sets_claude_agent_type() {
    let (_dir, backend, strat) = setup();

    handle_user_prompt_submit_with_strategy(
        UserPromptSubmitInput {
            session_id: "claude-s2".to_string(),
            transcript_path: "/tmp/claude-s2.jsonl".to_string(),
            prompt: "Fix bug in parser".to_string(),
        },
        &backend,
        &strat,
        None,
    )
    .unwrap();

    let state = backend.load_session("claude-s2").unwrap().unwrap();
    assert_eq!(state.agent_type, AGENT_TYPE_CLAUDE_CODE);
}

#[test]
fn cursor_session_end_marks_session_ended() {
    let (dir, backend, strat) = setup();
    backend
        .save_session(&SessionState {
            session_id: "cursor-s3".to_string(),
            phase: SessionPhase::Active,
            ..Default::default()
        })
        .unwrap();

    dispatch_cursor_hook(
        &CursorHookVerb::SessionEnd,
        r#"{"conversation_id":"cursor-s3","transcript_path":"/tmp/cursor-s3.jsonl"}"#,
        &backend,
        &strat,
        dir.path(),
        "session-end",
    )
    .unwrap();

    let state = backend.load_session("cursor-s3").unwrap().unwrap();
    assert_eq!(state.phase, SessionPhase::Ended);
    assert!(state.ended_at.is_some());
}

#[test]
fn cursor_session_end_without_session_state_still_saves_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let backend = LocalFileBackend::new(dir.path());
    let strat = RecordingStrategy::default();

    fs::write(
        dir.path().join("tracked.txt"),
        "cursor-session-end-fallback\n",
    )
    .unwrap();

    dispatch_cursor_hook(
        &CursorHookVerb::SessionEnd,
        r#"{"conversation_id":"cursor-s3-fallback","transcript_path":""}"#,
        &backend,
        &strat,
        dir.path(),
        "session-end",
    )
    .unwrap();

    let calls = strat
        .step_calls
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(calls.len(), 1, "expected one save_step call");
    assert_eq!(calls[0].session_id, "cursor-s3-fallback");

    let state = backend.load_session("cursor-s3-fallback").unwrap().unwrap();
    assert_eq!(state.phase, SessionPhase::Ended);
    assert!(state.ended_at.is_some());
}

#[test]
fn cursor_session_end_with_idle_zero_steps_still_saves_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let backend = LocalFileBackend::new(dir.path());
    let strat = RecordingStrategy::default();

    backend
        .save_session(&SessionState {
            session_id: "cursor-s3-idle-zero".to_string(),
            phase: SessionPhase::Idle,
            pending: PendingCheckpointState::default(),
            ..Default::default()
        })
        .unwrap();

    fs::write(
        dir.path().join("tracked.txt"),
        "cursor-session-end-idle-zero\n",
    )
    .unwrap();

    dispatch_cursor_hook(
        &CursorHookVerb::SessionEnd,
        r#"{"conversation_id":"cursor-s3-idle-zero","transcript_path":""}"#,
        &backend,
        &strat,
        dir.path(),
        "session-end",
    )
    .unwrap();

    let calls = strat
        .step_calls
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(calls.len(), 1, "expected one save_step call");
    assert_eq!(calls[0].session_id, "cursor-s3-idle-zero");
}

#[test]
fn cursor_session_end_does_not_duplicate_turn_end_after_stop() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let backend = create_session_backend_or_local(dir.path());
    let strat = ManualCommitStrategy::new(dir.path());

    dispatch_cursor_hook(
        &CursorHookVerb::BeforeSubmitPrompt,
        r#"{"conversation_id":"cursor-s3-stop-first","transcript_path":"","prompt":"Fix bug"}"#,
        backend.as_ref(),
        &strat,
        dir.path(),
        "before-submit-prompt",
    )
    .unwrap();

    fs::write(dir.path().join("tracked.txt"), "cursor-stop-first\n").unwrap();

    dispatch_cursor_hook(
        &CursorHookVerb::Stop,
        r#"{"conversation_id":"cursor-s3-stop-first","transcript_path":""}"#,
        backend.as_ref(),
        &strat,
        dir.path(),
        "stop",
    )
    .unwrap();

    let state_before_session_end = backend
        .load_session("cursor-s3-stop-first")
        .unwrap()
        .unwrap();
    assert!(
        state_before_session_end.pending.step_count > 0,
        "manual strategy should record a step at stop"
    );

    dispatch_cursor_hook(
        &CursorHookVerb::SessionEnd,
        r#"{"conversation_id":"cursor-s3-stop-first","transcript_path":""}"#,
        backend.as_ref(),
        &strat,
        dir.path(),
        "session-end",
    )
    .unwrap();

    let state = backend
        .load_session("cursor-s3-stop-first")
        .unwrap()
        .unwrap();
    assert_eq!(
        state.pending.step_count, state_before_session_end.pending.step_count,
        "session-end should not re-run turn-end when stop already completed the turn"
    );
    assert_eq!(state.phase, SessionPhase::Ended);
}

#[test]
fn cursor_before_submit_prompt_resolves_transcript_when_missing() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    let transcript_root = tempfile::tempdir().unwrap();
    let conversation_id = "cursor-resolve";
    let transcript_path = transcript_root
        .path()
        .join(format!("{conversation_id}.jsonl"));
    fs::write(&transcript_path, "{\"type\":\"user\"}\n").unwrap();

    with_process_state(
        Some(dir.path()),
        &[(
            "BITLOOPS_TEST_CURSOR_PROJECT_DIR",
            Some(transcript_root.path().to_string_lossy().as_ref()),
        )],
        || {
            let backend = LocalFileBackend::new(dir.path());
            let strat = RecordingStrategy::default();
            let result = dispatch_cursor_hook(
                &CursorHookVerb::BeforeSubmitPrompt,
                r#"{"conversation_id":"cursor-resolve","transcript_path":null,"prompt":"hello"}"#,
                &backend,
                &strat,
                dir.path(),
                "before-submit-prompt",
            );
            result.unwrap();

            let pre_prompt = backend.load_pre_prompt(conversation_id).unwrap().unwrap();
            assert_eq!(
                pre_prompt.transcript_path,
                transcript_path.to_string_lossy().to_string()
            );
            assert_eq!(pre_prompt.transcript_offset, 1);
        },
    );
}

#[test]
fn pre_task_creates_marker() {
    let (_dir, backend, _strat) = setup();

    let state = SessionState {
        session_id: "s7".to_string(),
        phase: SessionPhase::Active,
        ..Default::default()
    };
    backend.save_session(&state).unwrap();

    let input = TaskHookInput {
        session_id: "s7".to_string(),
        transcript_path: "/tmp/t.jsonl".to_string(),
        tool_use_id: "tool-123".to_string(),
        tool_input: None,
    };
    handle_pre_task(input, &backend, None).unwrap();

    assert!(backend.load_pre_task_marker("tool-123").unwrap().is_some());
}

#[test]
fn claude_post_task_persists_subagent_events_before_save_task_step_failure() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let backend = LocalFileBackend::new(dir.path());
    let strategy = FailingStrategy;

    backend
        .save_session(&SessionState {
            session_id: "claude-subagent-events".to_string(),
            phase: SessionPhase::Active,
            turn_id: "turn-subagent".to_string(),
            agent_type: AGENT_TYPE_CLAUDE_CODE.to_string(),
            ..Default::default()
        })
        .unwrap();

    handle_pre_task_with_profile(
        TaskHookInput {
            session_id: "claude-subagent-events".to_string(),
            transcript_path: "/tmp/claude-subagent-events.jsonl".to_string(),
            tool_use_id: "tool-subagent".to_string(),
            tool_input: Some(json!({"subagent_type":"research","description":"inspect"})),
        },
        &backend,
        Some(dir.path()),
        CLAUDE_HOOK_AGENT_PROFILE,
    )
    .unwrap();

    fs::write(dir.path().join("tracked.txt"), "claude-subagent-events\n").unwrap();

    let err = handle_post_task_with_profile(
        PostTaskInput {
            session_id: "claude-subagent-events".to_string(),
            transcript_path: "/tmp/claude-subagent-events.jsonl".to_string(),
            tool_use_id: "tool-subagent".to_string(),
            tool_input: Some(json!({"subagent_type":"research","description":"inspect"})),
            tool_response: TaskToolResponse {
                agent_id: "agent-subagent".to_string(),
            },
        },
        &backend,
        &strategy,
        Some(dir.path()),
        CLAUDE_HOOK_AGENT_PROFILE,
    )
    .expect_err("post-task should fail after interaction capture");
    assert!(
        err.to_string().contains("save_task_step failed"),
        "unexpected error: {err:#}"
    );

    assert_eq!(interaction_row_counts(dir.path()), (0, 0, 2));
    assert_sorted_event_types(dir.path(), vec!["subagent_start", "subagent_end"]);
}

#[test]
fn post_task_deletes_marker_without_mutating_step_count() {
    let (_dir, backend, strat) = setup();

    let state = SessionState {
        session_id: "s8".to_string(),
        phase: SessionPhase::Active,
        pending: PendingCheckpointState {
            step_count: 2,
            ..Default::default()
        },
        ..Default::default()
    };
    backend.save_session(&state).unwrap();
    backend
        .create_pre_task_marker(&PreTaskState {
            tool_use_id: "tool-456".to_string(),
            session_id: "s8".to_string(),
            ..Default::default()
        })
        .unwrap();

    let input = PostTaskInput {
        session_id: "s8".to_string(),
        transcript_path: "/tmp/t.jsonl".to_string(),
        tool_use_id: "tool-456".to_string(),
        tool_input: None,
        tool_response: TaskToolResponse::default(),
    };
    handle_post_task(input, &backend, &strat, None).unwrap();

    assert!(
        backend.load_pre_task_marker("tool-456").unwrap().is_none(),
        "marker should be deleted"
    );

    let loaded = backend.load_session("s8").unwrap().unwrap();
    assert_eq!(
        loaded.pending.step_count, 2,
        "step_count should be unchanged"
    );
}

#[test]
fn post_todo_noop_when_no_changes_in_subagent() {
    let (_dir, backend, strat) = setup();

    let state = SessionState {
        session_id: "s9".to_string(),
        phase: SessionPhase::Active,
        pending: PendingCheckpointState::default(),
        ..Default::default()
    };
    backend.save_session(&state).unwrap();
    backend
        .create_pre_task_marker(&PreTaskState {
            tool_use_id: "task-789".to_string(),
            session_id: "s9".to_string(),
            ..Default::default()
        })
        .unwrap();

    let input = PostTodoInput {
        session_id: "s9".to_string(),
        transcript_path: "/tmp/t.jsonl".to_string(),
        tool_use_id: "todo-xyz".to_string(),
        tool_name: "TodoWrite".to_string(),
        tool_input: None,
    };
    handle_post_todo(input, &backend, &strat, None).unwrap();

    let loaded = backend.load_session("s9").unwrap().unwrap();
    assert_eq!(
        loaded.pending.step_count, 0,
        "step_count should be unchanged"
    );
}

#[test]
fn post_todo_noop_when_not_in_subagent() {
    let (_dir, backend, strat) = setup();

    let state = SessionState {
        session_id: "s10".to_string(),
        phase: SessionPhase::Active,
        pending: PendingCheckpointState {
            step_count: 5,
            ..Default::default()
        },
        ..Default::default()
    };
    backend.save_session(&state).unwrap();
    // No pre-task marker present.

    let input = PostTodoInput {
        session_id: "s10".to_string(),
        transcript_path: "/tmp/t.jsonl".to_string(),
        tool_use_id: "todo-abc".to_string(),
        tool_name: "TodoWrite".to_string(),
        tool_input: None,
    };
    handle_post_todo(input, &backend, &strat, None).unwrap();

    let loaded = backend.load_session("s10").unwrap().unwrap();
    assert_eq!(
        loaded.pending.step_count, 5,
        "step_count should be unchanged"
    );
}

#[test]
fn stop_without_existing_session_is_tolerant_and_saves_step_compat() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let backend = LocalFileBackend::new(dir.path());
    let strat = RecordingStrategy::default();

    fs::write(dir.path().join("tracked.txt"), "two\n").unwrap();

    handle_stop(
        SessionInfoInput {
            session_id: "stop-missing-state".to_string(),
            transcript_path: String::new(),
        },
        &backend,
        &strat,
        Some(dir.path()),
    )
    .unwrap();

    let calls = strat
        .step_calls
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(calls.len(), 1, "expected one save_step call");
    assert_eq!(calls[0].session_id, "stop-missing-state");

    let state = backend.load_session("stop-missing-state").unwrap().unwrap();
    assert_eq!(state.phase, SessionPhase::Idle);
}

#[test]
fn stop_with_codex_profile_persists_codex_agent_type() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let backend = LocalFileBackend::new(dir.path());
    let strat = RecordingStrategy::default();

    fs::write(dir.path().join("tracked.txt"), "codex-change\n").unwrap();

    handle_stop_with_profile(
        SessionInfoInput {
            session_id: "codex-stop".to_string(),
            transcript_path: String::new(),
        },
        &backend,
        &strat,
        Some(dir.path()),
        CODEX_HOOK_AGENT_PROFILE,
    )
    .unwrap();

    let calls = strat
        .step_calls
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].agent_type, "codex");

    let state = backend.load_session("codex-stop").unwrap().unwrap();
    assert_eq!(state.agent_type, AGENT_TYPE_CODEX);
    assert_eq!(state.phase, SessionPhase::Idle);
}

#[test]
fn stop_with_empty_session_id_falls_back_to_unknown_session() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let backend = LocalFileBackend::new(dir.path());
    let strat = RecordingStrategy::default();

    fs::write(dir.path().join("tracked.txt"), "three\n").unwrap();

    handle_stop(
        SessionInfoInput {
            session_id: String::new(),
            transcript_path: String::new(),
        },
        &backend,
        &strat,
        Some(dir.path()),
    )
    .unwrap();

    let calls = strat
        .step_calls
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(calls.len(), 1, "expected one save_step call");
    assert_eq!(calls[0].session_id, UNKNOWN_SESSION_ID);

    let state = backend.load_session(UNKNOWN_SESSION_ID).unwrap().unwrap();
    assert_eq!(state.phase, SessionPhase::Idle);
}

#[test]
fn stop_with_empty_session_id_does_not_conflate_existing_session_state() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let backend = LocalFileBackend::new(dir.path());
    let strat = RecordingStrategy::default();

    backend
        .save_session(&SessionState {
            session_id: "real-session".to_string(),
            phase: SessionPhase::Active,
            ..Default::default()
        })
        .unwrap();
    backend
        .save_pre_prompt(&PrePromptState {
            session_id: "real-session".to_string(),
            prompt: "real prompt".to_string(),
            transcript_path: String::new(),
            ..Default::default()
        })
        .unwrap();
    fs::write(dir.path().join("tracked.txt"), "unknown-fallback\n").unwrap();

    handle_stop(
        SessionInfoInput {
            session_id: String::new(),
            transcript_path: String::new(),
        },
        &backend,
        &strat,
        Some(dir.path()),
    )
    .unwrap();

    let calls = strat
        .step_calls
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(calls.len(), 1, "expected one save_step call");
    assert_eq!(calls[0].session_id, UNKNOWN_SESSION_ID);

    let real_state = backend.load_session("real-session").unwrap().unwrap();
    assert_eq!(real_state.phase, SessionPhase::Active);
    assert!(
        backend.load_pre_prompt("real-session").unwrap().is_some(),
        "fallback stop should not consume another session's pre-prompt state"
    );
}

#[test]
fn stop_saves_step_with_detected_changes() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let backend = LocalFileBackend::new(dir.path());
    let strat = RecordingStrategy::default();

    backend
        .save_session(&SessionState {
            session_id: "stop-ctx".to_string(),
            phase: SessionPhase::Active,
            ..Default::default()
        })
        .unwrap();
    backend
        .save_pre_prompt(&PrePromptState {
            session_id: "stop-ctx".to_string(),
            prompt: "Can you update tracked file?".to_string(),
            transcript_path: String::new(),
            untracked_files: vec![],
            ..Default::default()
        })
        .unwrap();
    fs::write(dir.path().join("tracked.txt"), "two\n").unwrap();

    handle_stop(
        SessionInfoInput {
            session_id: "stop-ctx".to_string(),
            transcript_path: String::new(),
        },
        &backend,
        &strat,
        Some(dir.path()),
    )
    .unwrap();

    let calls = strat
        .step_calls
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(calls.len(), 1, "expected one save_step call");
    let call = &calls[0];
    assert!(
        call.modified_files.contains(&"tracked.txt".to_string()),
        "modified files: {:?}, new files: {:?}",
        call.modified_files,
        call.new_files
    );
    assert_eq!(call.commit_message, "Update tracked file");
}

#[test]
fn stop_preserves_strategy_checkpoint_count_updates() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let backend = create_session_backend_or_local(dir.path());
    let strat = ManualCommitStrategy::new(dir.path());

    backend
        .save_session(&SessionState {
            session_id: "stop-manual".to_string(),
            phase: SessionPhase::Active,
            ..Default::default()
        })
        .unwrap();
    backend
        .save_pre_prompt(&PrePromptState {
            session_id: "stop-manual".to_string(),
            prompt: "Create file".to_string(),
            transcript_path: String::new(),
            untracked_files: vec![],
            ..Default::default()
        })
        .unwrap();
    fs::write(dir.path().join("new.txt"), "content\n").unwrap();

    handle_stop(
        SessionInfoInput {
            session_id: "stop-manual".to_string(),
            transcript_path: String::new(),
        },
        backend.as_ref(),
        &strat,
        Some(dir.path()),
    )
    .unwrap();

    let loaded = backend.load_session("stop-manual").unwrap().unwrap();
    assert_eq!(loaded.phase, SessionPhase::Idle);
    assert!(
        loaded.pending.step_count > 0,
        "stop should preserve checkpoint_count updates from strategy.save_step"
    );
}

#[test]
fn stop_merges_modified_files_with_transcript_primary_order_compat() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let backend = LocalFileBackend::new(dir.path());
    let strat = RecordingStrategy::default();

    let transcript_dir = tempfile::tempdir().unwrap();
    let transcript_path = transcript_dir.path().join("stop-order.jsonl");
    write_claude_write_transcript(&transcript_path, "z-transcript.txt");

    backend
        .save_session(&SessionState {
            session_id: "stop-order".to_string(),
            phase: SessionPhase::Active,
            ..Default::default()
        })
        .unwrap();
    backend
        .save_pre_prompt(&PrePromptState {
            session_id: "stop-order".to_string(),
            prompt: "Update files".to_string(),
            transcript_path: transcript_path.to_string_lossy().to_string(),
            untracked_files: vec![],
            ..Default::default()
        })
        .unwrap();

    fs::write(dir.path().join("tracked.txt"), "tracked-change\n").unwrap();

    handle_stop(
        SessionInfoInput {
            session_id: "stop-order".to_string(),
            transcript_path: transcript_path.to_string_lossy().to_string(),
        },
        &backend,
        &strat,
        Some(dir.path()),
    )
    .unwrap();

    let calls = strat
        .step_calls
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(calls.len(), 1, "expected one save_step call");
    assert_eq!(
        calls[0].modified_files,
        vec!["z-transcript.txt".to_string(), "tracked.txt".to_string()],
        "stop should preserve transcript-first merge order"
    );
}

#[test]
fn stop_filters_out_files_already_committed_mid_turn_compat() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let backend = LocalFileBackend::new(dir.path());
    let strat = RecordingStrategy::default();

    let transcript_dir = tempfile::tempdir().unwrap();
    let transcript_path = transcript_dir.path().join("stop-filter.jsonl");
    write_claude_write_transcript(&transcript_path, "tracked.txt");

    backend
        .save_session(&SessionState {
            session_id: "stop-filter".to_string(),
            phase: SessionPhase::Active,
            ..Default::default()
        })
        .unwrap();
    backend
        .save_pre_prompt(&PrePromptState {
            session_id: "stop-filter".to_string(),
            prompt: "Update tracked file".to_string(),
            transcript_path: transcript_path.to_string_lossy().to_string(),
            untracked_files: vec![],
            ..Default::default()
        })
        .unwrap();

    fs::write(dir.path().join("tracked.txt"), "committed-mid-turn\n").unwrap();
    run_git(dir.path(), &["add", "tracked.txt"]);
    run_git(dir.path(), &["commit", "-m", "mid-turn commit"]);

    handle_stop(
        SessionInfoInput {
            session_id: "stop-filter".to_string(),
            transcript_path: transcript_path.to_string_lossy().to_string(),
        },
        &backend,
        &strat,
        Some(dir.path()),
    )
    .unwrap();

    let calls = strat
        .step_calls
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(
        calls.len(),
        0,
        "already-committed transcript-only edits should not trigger save_step"
    );
}

#[test]
fn post_task_saves_task_step_when_changes_exist() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let backend = LocalFileBackend::new(dir.path());
    let strat = RecordingStrategy::default();

    backend
        .save_session(&SessionState {
            session_id: "post-task".to_string(),
            phase: SessionPhase::Active,
            pending: PendingCheckpointState {
                step_count: 7,
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap();
    backend
        .create_pre_task_marker(&PreTaskState {
            tool_use_id: "tool-post".to_string(),
            session_id: "post-task".to_string(),
            untracked_files: vec![],
            ..Default::default()
        })
        .unwrap();
    fs::write(dir.path().join("tracked.txt"), "three\n").unwrap();

    handle_post_task(
        PostTaskInput {
            session_id: "post-task".to_string(),
            transcript_path: "/tmp/transcript.jsonl".to_string(),
            tool_use_id: "tool-post".to_string(),
            tool_input: Some(json!({"subagent_type":"research","description":"inspect"})),
            tool_response: TaskToolResponse {
                agent_id: "agent-1".to_string(),
            },
        },
        &backend,
        &strat,
        Some(dir.path()),
    )
    .unwrap();

    assert!(backend.load_pre_task_marker("tool-post").unwrap().is_none());
    let calls = strat
        .task_calls
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(calls.len(), 1, "expected one save_task_step call");
    let call = &calls[0];
    assert_eq!(call.tool_use_id, "tool-post");
    assert_eq!(call.agent_id, "agent-1");
    assert_eq!(call.subagent_type, "research");
    assert_eq!(call.task_description, "inspect");
    assert!(
        call.modified_files.contains(&"tracked.txt".to_string()),
        "modified files: {:?}, new files: {:?}",
        call.modified_files,
        call.new_files
    );
    let loaded = backend.load_session("post-task").unwrap().unwrap();
    assert_eq!(
        loaded.pending.step_count, 7,
        "step_count should stay unchanged"
    );
}

#[test]
fn post_todo_saves_incremental_with_completed_todo() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    with_cwd(dir.path(), || {
        let checkout = git_command()
            .args(["checkout", "-b", "feature/test-post-todo"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(checkout.status.success(), "git checkout failed");
        let backend = LocalFileBackend::new(dir.path());
        let strat = RecordingStrategy::default();

        backend
            .save_session(&SessionState {
                session_id: "post-todo".to_string(),
                phase: SessionPhase::Active,
                ..Default::default()
            })
            .unwrap();
        backend
            .create_pre_task_marker(&PreTaskState {
                tool_use_id: "task-parent".to_string(),
                session_id: "post-todo".to_string(),
                ..Default::default()
            })
            .unwrap();
        fs::write(dir.path().join("tracked.txt"), "four\n").unwrap();

        handle_post_todo(
            PostTodoInput {
                session_id: "post-todo".to_string(),
                transcript_path: "/tmp/transcript.jsonl".to_string(),
                tool_use_id: "todo-ignored".to_string(),
                tool_name: "TodoWrite".to_string(),
                tool_input: Some(json!({
                    "todos": [
                        {"status":"pending","content":"A"},
                        {"status":"completed","content":"Ship feature"}
                    ]
                })),
            },
            &backend,
            &strat,
            Some(dir.path()),
        )
        .unwrap();

        let calls = strat
            .task_calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(calls.len(), 1, "expected one incremental task checkpoint");
        let call = &calls[0];
        assert!(call.is_incremental);
        assert_eq!(call.tool_use_id, "task-parent");
        assert_eq!(call.incremental_type, "TodoWrite");
        assert_eq!(call.todo_content, "Ship feature");
        assert!(
            call.modified_files.contains(&"tracked.txt".to_string()),
            "modified files: {:?}, new files: {:?}",
            call.modified_files,
            call.new_files
        );
    });
}

#[test]
fn parse_pre_task_hook_input_compat() {
    struct Case<'a> {
        input: &'a str,
        want_session_id: &'a str,
        want_transcript_path: &'a str,
        want_tool_use_id: &'a str,
        want_err: bool,
    }

    let cases = vec![
        Case {
            input: r#"{"session_id":"abc123","transcript_path":"/path/to/transcript.jsonl","tool_use_id":"tool_xyz"}"#,
            want_session_id: "abc123",
            want_transcript_path: "/path/to/transcript.jsonl",
            want_tool_use_id: "tool_xyz",
            want_err: false,
        },
        Case {
            input: "",
            want_session_id: "",
            want_transcript_path: "",
            want_tool_use_id: "",
            want_err: true,
        },
        Case {
            input: "not json",
            want_session_id: "",
            want_transcript_path: "",
            want_tool_use_id: "",
            want_err: true,
        },
        Case {
            input: r#"{"session_id":"abc123"}"#,
            want_session_id: "abc123",
            want_transcript_path: "",
            want_tool_use_id: "",
            want_err: false,
        },
    ];

    for case in cases {
        let got = parse_task_hook_input(case.input);
        assert_eq!(got.is_err(), case.want_err);
        if !case.want_err {
            let got = got.expect("parsed input");
            assert_eq!(got.session_id, case.want_session_id);
            assert_eq!(got.transcript_path, case.want_transcript_path);
            assert_eq!(got.tool_use_id, case.want_tool_use_id);
        }
    }
}

#[test]
fn parse_post_task_hook_input_compat() {
    struct Case<'a> {
        input: &'a str,
        want_session_id: &'a str,
        want_transcript_path: &'a str,
        want_tool_use_id: &'a str,
        want_agent_id: &'a str,
        want_err: bool,
    }

    let cases = vec![
        Case {
            input: r#"{
  "session_id":"abc123",
  "transcript_path":"/path/to/transcript.jsonl",
  "tool_use_id":"tool_xyz",
  "tool_input":{"prompt":"do something"},
  "tool_response":{"agentId":"agent_456"}
}"#,
            want_session_id: "abc123",
            want_transcript_path: "/path/to/transcript.jsonl",
            want_tool_use_id: "tool_xyz",
            want_agent_id: "agent_456",
            want_err: false,
        },
        Case {
            input: r#"{
  "session_id":"abc123",
  "transcript_path":"/path/to/transcript.jsonl",
  "tool_use_id":"tool_xyz",
  "tool_input":{},
  "tool_response":{}
}"#,
            want_session_id: "abc123",
            want_transcript_path: "/path/to/transcript.jsonl",
            want_tool_use_id: "tool_xyz",
            want_agent_id: "",
            want_err: false,
        },
        Case {
            input: "",
            want_session_id: "",
            want_transcript_path: "",
            want_tool_use_id: "",
            want_agent_id: "",
            want_err: true,
        },
        Case {
            input: "not json",
            want_session_id: "",
            want_transcript_path: "",
            want_tool_use_id: "",
            want_agent_id: "",
            want_err: true,
        },
    ];

    for case in cases {
        let got = parse_post_task_hook_input(case.input);
        assert_eq!(got.is_err(), case.want_err);
        if !case.want_err {
            let got = got.expect("parsed input");
            assert_eq!(got.session_id, case.want_session_id);
            assert_eq!(got.transcript_path, case.want_transcript_path);
            assert_eq!(got.tool_use_id, case.want_tool_use_id);
            assert_eq!(got.tool_response.agent_id, case.want_agent_id);
        }
    }
}

#[test]
fn log_pre_task_hook_context_compat() {
    let input = TaskHookInput {
        session_id: "test-session-123".to_string(),
        transcript_path: "/home/user/.claude/projects/myproject/transcript.jsonl".to_string(),
        tool_use_id: "toolu_abc123".to_string(),
        tool_input: None,
    };

    let mut buf = Vec::new();
    log_pre_task_hook_context(&mut buf, &input);
    let output = String::from_utf8(buf).unwrap();

    assert!(output.contains("[bitloops] PreToolUse[Task] hook invoked"));
    assert!(output.contains("Session ID: test-session-123"));
    assert!(output.contains("Tool Use ID: toolu_abc123"));
    assert!(output.contains("Transcript:"));
}

#[test]
fn parse_subagent_checkpoint_hook_input_compat() {
    struct Case<'a> {
        input: &'a str,
        want_session_id: &'a str,
        want_tool_name: &'a str,
        want_tool_use_id: &'a str,
        want_err: bool,
    }

    let cases = vec![
        Case {
            input: r#"{
  "session_id":"abc123",
  "tool_name":"TodoWrite",
  "tool_use_id":"toolu_xyz",
  "tool_input":{"todos":[{"content":"Task 1","status":"pending"}]},
  "tool_response":{"success":true}
}"#,
            want_session_id: "abc123",
            want_tool_name: "TodoWrite",
            want_tool_use_id: "toolu_xyz",
            want_err: false,
        },
        Case {
            input: "",
            want_session_id: "",
            want_tool_name: "",
            want_tool_use_id: "",
            want_err: true,
        },
        Case {
            input: "not json",
            want_session_id: "",
            want_tool_name: "",
            want_tool_use_id: "",
            want_err: true,
        },
        Case {
            input: r#"{
  "session_id":"def456",
  "tool_name":"Edit",
  "tool_use_id":"toolu_edit123",
  "tool_input":{"file_path":"/path/to/file","old_string":"foo","new_string":"bar"},
  "tool_response":{}
}"#,
            want_session_id: "def456",
            want_tool_name: "Edit",
            want_tool_use_id: "toolu_edit123",
            want_err: false,
        },
    ];

    for case in cases {
        let got = parse_subagent_checkpoint_hook_input(case.input);
        assert_eq!(got.is_err(), case.want_err);
        if !case.want_err {
            let got = got.expect("parsed input");
            assert_eq!(got.session_id, case.want_session_id);
            assert_eq!(got.transcript_path, "");
            assert_eq!(got.tool_name, case.want_tool_name);
            assert_eq!(got.tool_use_id, case.want_tool_use_id);
            assert!(got.tool_input.is_some());
            assert!(got.tool_response.is_some());
        }
    }
}

#[test]
fn parse_subagent_type_and_description_compat() {
    let cases = vec![
        (
            r#"{"subagent_type":"dev","description":"Implement user authentication","prompt":"Do the work"}"#,
            "dev",
            "Implement user authentication",
        ),
        (
            r#"{"subagent_type":"reviewer","prompt":"Review changes"}"#,
            "reviewer",
            "",
        ),
        (
            r#"{"description":"Fix the bug","prompt":"Fix it"}"#,
            "",
            "Fix the bug",
        ),
        (r#"{"prompt":"Do something"}"#, "", ""),
        ("", "", ""),
        ("not valid json", "", ""),
        ("null", "", ""),
    ];

    for (tool_input, want_agent_type, want_description) in cases {
        let parsed = serde_json::from_str::<Value>(tool_input).ok();
        let (got_agent_type, got_description) =
            parse_subagent_type_and_description(parsed.as_ref());
        assert_eq!(got_agent_type, want_agent_type);
        assert_eq!(got_description, want_description);
    }
}

#[test]
fn extract_todo_content_from_tool_input_compat() {
    let cases = vec![
        (
            r#"{"todos":[{"content":"First task","status":"completed"},{"content":"Second task","status":"in_progress"},{"content":"Third task","status":"pending"}]}"#,
            "Second task",
        ),
        (
            r#"{"todos":[{"content":"First task","status":"completed"},{"content":"Second task","status":"pending"},{"content":"Third task","status":"pending"}]}"#,
            "Second task",
        ),
        (
            r#"{"todos":[{"content":"First pending task","status":"pending","activeForm":"Doing first task"},{"content":"Second pending task","status":"pending","activeForm":"Doing second task"}]}"#,
            "First pending task",
        ),
        (
            r#"{"todos":[{"content":"First task","status":"completed"}]}"#,
            "First task",
        ),
        (r#"{"todos":[]}"#, ""),
        (r#"{"other_field":"value"}"#, ""),
        (r#"{"todos":null}"#, ""),
        ("", ""),
        ("not valid json", ""),
    ];

    for (tool_input, want) in cases {
        let parsed = serde_json::from_str::<Value>(tool_input).ok();
        let got = extract_todo_content_from_tool_input(parsed.as_ref());
        assert_eq!(got, want);
    }
}

#[test]
fn extract_last_completed_todo_from_tool_input_compat() {
    let cases = vec![
        (
            r#"{"todos":[{"content":"First task","status":"completed"},{"content":"Second task","status":"completed"},{"content":"Third task","status":"in_progress"}]}"#,
            "Second task",
        ),
        (
            r#"{"todos":[{"content":"First task","status":"in_progress"},{"content":"Second task","status":"pending"}]}"#,
            "",
        ),
        (r#"{"todos":[]}"#, ""),
        ("", ""),
    ];

    for (tool_input, want) in cases {
        let parsed = serde_json::from_str::<Value>(tool_input).ok();
        let got = extract_last_completed_todo_from_tool_input(parsed.as_ref());
        assert_eq!(got, want);
    }
}

#[test]
fn count_todos_from_tool_input_compat() {
    let cases = vec![
        (
            r#"{"todos":[{"content":"First task","status":"completed"},{"content":"Second task","status":"in_progress"},{"content":"Third task","status":"pending"}]}"#,
            3usize,
        ),
        (
            r#"{"todos":[{"content":"Task 1","status":"pending"},{"content":"Task 2","status":"pending"},{"content":"Task 3","status":"pending"},{"content":"Task 4","status":"pending"},{"content":"Task 5","status":"pending"},{"content":"Task 6","status":"in_progress"}]}"#,
            6,
        ),
        (r#"{"todos":[]}"#, 0),
        (r#"{"other_field":"value"}"#, 0),
        ("", 0),
        ("not valid json", 0),
    ];

    for (tool_input, want) in cases {
        let parsed = serde_json::from_str::<Value>(tool_input).ok();
        let got = count_todos_from_tool_input(parsed.as_ref());
        assert_eq!(got, want);
    }
}

#[test]
fn log_post_task_hook_context_compat() {
    let cases = vec![
        (
            PostTaskInput {
                session_id: "test-session-456".to_string(),
                transcript_path: "/path/to/transcript.jsonl".to_string(),
                tool_use_id: "toolu_xyz789".to_string(),
                tool_input: None,
                tool_response: TaskToolResponse {
                    agent_id: "agent_subagent_001".to_string(),
                },
            },
            "/path/to/agent-agent_subagent_001.jsonl",
            "Agent ID: agent_subagent_001",
            "Subagent Transcript: /path/to/agent-agent_subagent_001.jsonl",
        ),
        (
            PostTaskInput {
                session_id: "test-session-789".to_string(),
                transcript_path: "/path/to/transcript.jsonl".to_string(),
                tool_use_id: "toolu_def456".to_string(),
                tool_input: None,
                tool_response: TaskToolResponse {
                    agent_id: String::new(),
                },
            },
            "",
            "Agent ID: (none)",
            "Subagent Transcript: (none)",
        ),
    ];

    for (input, subagent_path, want_agent_id, want_subagent_path) in cases {
        let mut buf = Vec::new();
        log_post_task_hook_context(&mut buf, &input, subagent_path);
        let output = String::from_utf8(buf).unwrap();

        assert!(output.contains("[bitloops] PostToolUse[Task] hook invoked"));
        assert!(output.contains(want_agent_id), "output:\n{output}");
        assert!(output.contains(want_subagent_path), "output:\n{output}");
    }
}

#[test]
fn new_agent_hook_verb_cmd_logs_invocation() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let state_root_value = dir.path().join("state-root").display().to_string();

    with_process_state(
        Some(dir.path()),
        &[
            (
                "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
                Some(state_root_value.as_str()),
            ),
            (logging::LOG_LEVEL_ENV_VAR, Some("DEBUG")),
        ],
        || {
            with_logger_test_lock(|| {
                let backend = LocalFileBackend::new(dir.path());
                backend
                    .save_session(&SessionState {
                        session_id: "test-hook-session".to_string(),
                        started_at: "2026-01-01T00:00:00Z".to_string(),
                        last_interaction_time: Some("2026-01-01T00:00:00Z".to_string()),
                        phase: SessionPhase::Active,
                        ..Default::default()
                    })
                    .unwrap();

                logging::reset_logger_for_tests();

                run_agent_hook_with_logging(
                    dir.path(),
                    AGENT_NAME_CLAUDE_CODE,
                    "session-start",
                    registry::STRATEGY_NAME_MANUAL_COMMIT,
                    || Ok(()),
                )
                .unwrap();

                let log_file = logging::log_file_path();
                let content = fs::read_to_string(log_file).expect("log file should exist");

                let mut found_invocation = false;
                let mut found_completion = false;
                for line in content.lines() {
                    let entry: Value =
                        serde_json::from_str(line).expect("line should be valid JSON");
                    if entry.get("hook").and_then(Value::as_str) != Some("session-start") {
                        continue;
                    }
                    if entry.get("msg").and_then(Value::as_str) == Some("hook invoked") {
                        found_invocation = true;
                        assert_eq!(
                            entry.get("component").and_then(Value::as_str),
                            Some("hooks")
                        );
                    }
                    if entry.get("msg").and_then(Value::as_str) == Some("hook completed") {
                        found_completion = true;
                        assert!(entry.get("duration_ms").is_some());
                        assert_eq!(entry.get("success").and_then(Value::as_bool), Some(true));
                    }
                }

                assert!(found_invocation);
                assert!(found_completion);
            });
        },
    );
}

#[test]
fn claude_code_hooks_cmd_has_logging_hooks() {
    let hooks_cmd =
        <HooksAgent as clap::Subcommand>::augment_subcommands(clap::Command::new("hooks"));
    let has_claude_code = hooks_cmd
        .get_subcommands()
        .any(|cmd| cmd.get_name() == "claude-code");
    assert!(has_claude_code);
}

#[test]
fn codex_hooks_cmd_has_logging_hooks() {
    let hooks_cmd =
        <HooksAgent as clap::Subcommand>::augment_subcommands(clap::Command::new("hooks"));
    let has_codex = hooks_cmd
        .get_subcommands()
        .any(|cmd| cmd.get_name() == "codex");
    assert!(has_codex);
}

#[test]
fn gemini_hooks_cmd_has_logging_hooks() {
    let hooks_cmd =
        <HooksAgent as clap::Subcommand>::augment_subcommands(clap::Command::new("hooks"));
    let has_gemini = hooks_cmd
        .get_subcommands()
        .any(|cmd| cmd.get_name() == "gemini");
    assert!(has_gemini);
}

#[test]
fn cursor_hooks_cmd_has_logging_hooks() {
    let hooks_cmd =
        <HooksAgent as clap::Subcommand>::augment_subcommands(clap::Command::new("hooks"));
    let has_cursor = hooks_cmd
        .get_subcommands()
        .any(|cmd| cmd.get_name() == "cursor");
    assert!(has_cursor);
}

#[test]
fn copilot_hooks_cmd_has_logging_hooks() {
    let hooks_cmd =
        <HooksAgent as clap::Subcommand>::augment_subcommands(clap::Command::new("hooks"));
    let has_copilot = hooks_cmd
        .get_subcommands()
        .any(|cmd| cmd.get_name() == "copilot");
    assert!(has_copilot);
}

#[test]
fn opencode_hooks_cmd_has_logging_hooks() {
    let hooks_cmd =
        <HooksAgent as clap::Subcommand>::augment_subcommands(clap::Command::new("hooks"));
    let has_opencode = hooks_cmd
        .get_subcommands()
        .any(|cmd| cmd.get_name() == "opencode");
    assert!(has_opencode);
}

#[test]
fn hook_command_sets_current_hook_agent_name() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    with_process_state(
        Some(dir.path()),
        &[(logging::LOG_LEVEL_ENV_VAR, Some("DEBUG"))],
        || {
            with_logger_test_lock(|| {
                let inside = Mutex::new(String::new());
                run_agent_hook_with_logging(
                    dir.path(),
                    AGENT_NAME_CLAUDE_CODE,
                    "session-start",
                    registry::STRATEGY_NAME_MANUAL_COMMIT,
                    || {
                        *inside
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                            current_hook_agent_name_for_tests();
                        Ok(())
                    },
                )
                .unwrap();

                assert_eq!(
                    inside
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .as_str(),
                    AGENT_NAME_CLAUDE_CODE
                );
                assert_eq!(current_hook_agent_name_for_tests(), "");
            });
        },
    );
}

#[test]
fn pre_prompt_state_with_transcript_position() {
    with_process_state(None, &[], || {
        let (_dir, backend, _strat) = setup();
        let transcript_dir = tempfile::tempdir().unwrap();
        let transcript_path = transcript_dir.path().join("transcript.jsonl");
        fs::write(
            &transcript_path,
            "{\"type\":\"user\"}\n{\"type\":\"assistant\"}\n{\"type\":\"user\"}\n",
        )
        .unwrap();

        handle_user_prompt_submit(
            UserPromptSubmitInput {
                session_id: "test-session-123".to_string(),
                transcript_path: transcript_path.to_string_lossy().to_string(),
                prompt: "hello".to_string(),
            },
            &backend,
            None,
        )
        .unwrap();

        let state = backend
            .load_pre_prompt("test-session-123")
            .unwrap()
            .expect("pre-prompt state");
        assert_eq!(state.transcript_offset, 3);
    });
}

// Capture transcript position AND last transcript identifier at turn start.
#[test]
fn pre_prompt_state_captures_last_transcript_identifier() {
    with_process_state(None, &[], || {
        let (_dir, backend, _strat) = setup();
        let transcript_dir = tempfile::tempdir().unwrap();
        let transcript_path = transcript_dir.path().join("transcript.jsonl");
        fs::write(
            &transcript_path,
            r#"{"uuid":"user-1","type":"user","message":{"content":"first"}}
{"uuid":"assistant-1","type":"assistant","message":{"content":"ok"}}
{"uuid":"user-2","type":"user","message":{"content":"second"}}
"#,
        )
        .unwrap();

        handle_user_prompt_submit(
            UserPromptSubmitInput {
                session_id: "test-session-last-uuid".to_string(),
                transcript_path: transcript_path.to_string_lossy().to_string(),
                prompt: "hello".to_string(),
            },
            &backend,
            None,
        )
        .unwrap();

        let state = backend
            .load_pre_prompt("test-session-last-uuid")
            .unwrap()
            .expect("pre-prompt state");
        assert_eq!(state.transcript_offset, 3);
        assert_eq!(state.last_transcript_identifier, "user-2");
    });
}

#[test]
fn pre_prompt_state_with_empty_transcript_path() {
    with_process_state(None, &[], || {
        let (_dir, backend, _strat) = setup();
        handle_user_prompt_submit(
            UserPromptSubmitInput {
                session_id: "test-session-empty-transcript".to_string(),
                transcript_path: String::new(),
                prompt: "hello".to_string(),
            },
            &backend,
            None,
        )
        .unwrap();

        let state = backend
            .load_pre_prompt("test-session-empty-transcript")
            .unwrap()
            .expect("pre-prompt state");
        assert_eq!(state.transcript_offset, 0);
    });
}

#[test]
fn pre_prompt_state_with_summary_only_transcript() {
    with_process_state(None, &[], || {
        let (_dir, backend, _strat) = setup();
        let transcript_dir = tempfile::tempdir().unwrap();
        let transcript_path = transcript_dir.path().join("transcript-summary.jsonl");
        fs::write(
            &transcript_path,
            "{\"type\":\"summary\",\"leafUuid\":\"leaf-1\"}\n{\"type\":\"summary\",\"leafUuid\":\"leaf-2\"}\n",
        )
        .unwrap();

        handle_user_prompt_submit(
            UserPromptSubmitInput {
                session_id: "test-session-summary-only".to_string(),
                transcript_path: transcript_path.to_string_lossy().to_string(),
                prompt: "hello".to_string(),
            },
            &backend,
            None,
        )
        .unwrap();

        let state = backend
            .load_pre_prompt("test-session-summary-only")
            .unwrap()
            .expect("pre-prompt state");
        assert_eq!(state.transcript_offset, 2);
    });
}

#[test]
fn filter_and_normalize_paths_sibling_directories() {
    let files = vec![
        "/repo/api/src/lib/github.ts".to_string(),
        "/repo/api/src/types.ts".to_string(),
        "/repo/frontend/src/pages/api.ts".to_string(),
    ];
    let got = filter_and_normalize_paths(&files, "/repo");
    assert_eq!(
        got,
        vec![
            "api/src/lib/github.ts".to_string(),
            "api/src/types.ts".to_string(),
            "frontend/src/pages/api.ts".to_string(),
        ]
    );

    let files = vec![
        "/repo/api/src/lib/github.ts".to_string(),
        "/repo/frontend/src/pages/api.ts".to_string(),
    ];
    let got = filter_and_normalize_paths(&files, "/repo/frontend");
    assert_eq!(got, vec!["src/pages/api.ts".to_string()]);

    let files = vec!["src/file.ts".to_string(), "lib/util.rs".to_string()];
    let got = filter_and_normalize_paths(&files, "/repo");
    assert_eq!(
        got,
        vec!["src/file.ts".to_string(), "lib/util.rs".to_string()]
    );

    let files = vec![
        "/repo/src/file.ts".to_string(),
        "/repo/.bitloops/internal/sessions/session.json".to_string(),
    ];
    let got = filter_and_normalize_paths(&files, "/repo");
    assert_eq!(got, vec!["src/file.ts".to_string()]);
}

#[test]
fn detect_file_changes_deleted_files_with_nil_pre_state() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    fs::remove_file(dir.path().join("tracked.txt")).unwrap();

    let changes = detect_file_changes(Some(dir.path()), None);
    assert!(changes.new_files.is_empty());
    assert_eq!(changes.deleted, vec!["tracked.txt".to_string()]);
}

#[test]
fn detect_file_changes_new_and_deleted_files() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    fs::write(dir.path().join("tracked2.txt"), "content2\n").unwrap();
    let out = git_command()
        .args(["add", "tracked2.txt"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "git add tracked2.txt failed");
    let out = git_command()
        .args(["commit", "-m", "add tracked2"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "git commit tracked2.txt failed");

    fs::write(
        dir.path().join("pre-existing-untracked.txt"),
        "pre-existing\n",
    )
    .unwrap();
    fs::remove_file(dir.path().join("tracked.txt")).unwrap();
    fs::write(dir.path().join("new-file.txt"), "new content\n").unwrap();

    let pre = vec!["pre-existing-untracked.txt".to_string()];
    let changes = detect_file_changes(Some(dir.path()), Some(&pre));
    assert_eq!(changes.new_files, vec!["new-file.txt".to_string()]);
    assert_eq!(changes.deleted, vec!["tracked.txt".to_string()]);
}

#[test]
fn detect_file_changes_no_changes() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    let pre: Vec<String> = Vec::new();
    let changes = detect_file_changes(Some(dir.path()), Some(&pre));
    assert!(changes.modified.is_empty());
    assert!(changes.new_files.is_empty());
    assert!(changes.deleted.is_empty());
}

#[test]
fn detect_file_changes_nil_previously_untracked_returns_modified() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    fs::write(dir.path().join("tracked.txt"), "modified content\n").unwrap();
    fs::write(dir.path().join("untracked.txt"), "untracked\n").unwrap();

    let changes = detect_file_changes(Some(dir.path()), None);
    assert_eq!(changes.modified, vec!["tracked.txt".to_string()]);
    assert_eq!(changes.new_files, vec!["untracked.txt".to_string()]);
    assert!(changes.deleted.is_empty());
}

// DetectFileChanges returns untracked files, not parent directory placeholders.
#[test]
fn detect_file_changes_untracked_directory_returns_nested_file_paths() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/auth.rs"), "package auth\n").unwrap();

    let changes = detect_file_changes(Some(dir.path()), None);
    assert_eq!(changes.new_files, vec!["src/auth.rs".to_string()]);
    assert!(changes.modified.is_empty());
    assert!(changes.deleted.is_empty());
}

#[test]
fn mark_session_ended_sets_phase_ended() {
    let (_dir, backend, _strat) = setup();
    backend
        .save_session(&SessionState {
            session_id: "test-session-end-1".to_string(),
            phase: SessionPhase::Active,
            ..Default::default()
        })
        .unwrap();

    mark_session_ended("test-session-end-1", &backend).unwrap();

    let loaded = backend
        .load_session("test-session-end-1")
        .unwrap()
        .expect("state");
    assert_eq!(loaded.phase, SessionPhase::Ended);
    assert!(loaded.ended_at.is_some());
    assert!(loaded.last_interaction_time.is_some());
}

#[test]
fn mark_session_ended_idle_to_ended() {
    let (_dir, backend, _strat) = setup();
    backend
        .save_session(&SessionState {
            session_id: "test-session-end-idle".to_string(),
            phase: SessionPhase::Idle,
            ..Default::default()
        })
        .unwrap();

    mark_session_ended("test-session-end-idle", &backend).unwrap();
    let loaded = backend
        .load_session("test-session-end-idle")
        .unwrap()
        .expect("state");
    assert_eq!(loaded.phase, SessionPhase::Ended);
    assert!(loaded.ended_at.is_some());
}

#[test]
fn mark_session_ended_already_ended_is_noop() {
    let (_dir, backend, _strat) = setup();
    backend
        .save_session(&SessionState {
            session_id: "test-session-end-noop".to_string(),
            phase: SessionPhase::Ended,
            ended_at: Some("2026-01-01T00:00:00Z".to_string()),
            ..Default::default()
        })
        .unwrap();

    mark_session_ended("test-session-end-noop", &backend).unwrap();

    let loaded = backend
        .load_session("test-session-end-noop")
        .unwrap()
        .expect("state");
    assert_eq!(loaded.phase, SessionPhase::Ended);
    assert!(loaded.ended_at.is_some());
}

#[test]
fn mark_session_ended_empty_phase_backward_compat() {
    let (_dir, backend, _strat) = setup();
    let sessions_dir = backend.sessions_dir();
    fs::create_dir_all(&sessions_dir).unwrap();
    let state_file = sessions_dir.join("test-session-end-compat.json");
    fs::write(
        state_file,
        r#"{"session_id":"test-session-end-compat","base_commit":"abc123","phase":""}"#,
    )
    .unwrap();

    mark_session_ended("test-session-end-compat", &backend).unwrap();

    let loaded = backend
        .load_session("test-session-end-compat")
        .unwrap()
        .expect("state");
    assert_eq!(loaded.phase, SessionPhase::Ended);
}

#[test]
fn mark_session_ended_no_state() {
    let (_dir, backend, _strat) = setup();
    mark_session_ended("nonexistent-session", &backend).unwrap();
}

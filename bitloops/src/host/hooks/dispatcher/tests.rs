use anyhow::{Context, Result};

use super::*;
use crate::adapters::agents::claude_code::hooks_cmd::{
    SessionInfoInput, UserPromptSubmitInput, handle_session_end_with_profile_and_model,
    handle_session_start_with_profile_and_model, handle_stop_with_profile_and_model,
    handle_user_prompt_submit_with_strategy_and_profile_and_model,
};
use crate::adapters::agents::cursor::types::{
    CursorAfterShellExecutionRaw, CursorBeforeShellExecutionRaw, CursorBeforeSubmitPromptRaw,
    CursorSessionInfoRaw,
};
use crate::host::checkpoints::session::backend::SessionBackend;
use crate::host::checkpoints::session::phase::SessionPhase;
use crate::host::checkpoints::session::state::PRE_PROMPT_SOURCE_CURSOR_SHELL;
use crate::host::checkpoints::strategy::Strategy;
use crate::host::hooks::{BITLOOPS_SUPPRESS_AGENT_HOOKS_ENV, agent_hooks_suppressed_by_env};
use crate::test_support::process_state::enter_process_state;

pub(crate) fn dispatch_cursor_hook(
    verb: &CursorHookVerb,
    stdin: &str,
    backend: &dyn SessionBackend,
    strategy: &dyn Strategy,
    repo_root: &Path,
    hook_name: &str,
) -> Result<()> {
    match verb {
        CursorHookVerb::SessionStart => {
            let raw: CursorSessionInfoRaw =
                serde_json::from_str(stdin).context("parsing session-start input")?;
            let session_id = crate::host::checkpoints::lifecycle::apply_session_id_policy(
                &raw.conversation_id,
                crate::host::checkpoints::lifecycle::SessionIdPolicy::Strict,
            )?;
            let input = SessionInfoInput {
                session_id: session_id.clone(),
                transcript_path: crate::adapters::agents::cursor::lifecycle::resolve_transcript_ref(
                    &session_id,
                    raw.transcript_path.as_deref(),
                ),
            };
            handle_session_start_with_profile_and_model(
                input,
                backend,
                Some(repo_root),
                Some(crate::host::hooks::runtime::agent_runtime::CURSOR_HOOK_AGENT_PROFILE),
                &raw.model,
            )
        }
        CursorHookVerb::BeforeSubmitPrompt => {
            let raw: CursorBeforeSubmitPromptRaw =
                serde_json::from_str(stdin).context("parsing before-submit-prompt input")?;
            let session_id = crate::host::checkpoints::lifecycle::apply_session_id_policy(
                &raw.conversation_id,
                crate::host::checkpoints::lifecycle::SessionIdPolicy::Strict,
            )?;
            let input = UserPromptSubmitInput {
                session_id: session_id.clone(),
                transcript_path: crate::adapters::agents::cursor::lifecycle::resolve_transcript_ref(
                    &session_id,
                    raw.transcript_path.as_deref(),
                ),
                prompt: raw.prompt,
            };
            handle_user_prompt_submit_with_strategy_and_profile_and_model(
                input,
                backend,
                strategy,
                Some(repo_root),
                crate::host::hooks::runtime::agent_runtime::CURSOR_HOOK_AGENT_PROFILE,
                &raw.model,
            )
        }
        CursorHookVerb::BeforeShellExecution => {
            let raw: CursorBeforeShellExecutionRaw =
                serde_json::from_str(stdin).context("parsing before-shell-execution input")?;
            let session_id = crate::host::checkpoints::lifecycle::apply_session_id_policy(
                &raw.conversation_id,
                crate::host::checkpoints::lifecycle::SessionIdPolicy::Strict,
            )?;

            if backend.load_pre_prompt(&session_id)?.is_some() {
                return Ok(());
            }

            let input = UserPromptSubmitInput {
                session_id: session_id.clone(),
                transcript_path: crate::adapters::agents::cursor::lifecycle::resolve_transcript_ref(
                    &session_id,
                    raw.transcript_path.as_deref(),
                ),
                prompt: shell_command_to_prompt(&raw.command),
            };
            handle_user_prompt_submit_with_strategy_and_profile_and_model(
                input,
                backend,
                strategy,
                Some(repo_root),
                crate::host::hooks::runtime::agent_runtime::CURSOR_HOOK_AGENT_PROFILE,
                &raw.model,
            )?;

            if let Some(mut pre_prompt) = backend.load_pre_prompt(&session_id)? {
                pre_prompt.source = PRE_PROMPT_SOURCE_CURSOR_SHELL.to_string();
                backend.save_pre_prompt(&pre_prompt)?;
            }
            Ok(())
        }
        CursorHookVerb::AfterShellExecution => {
            let raw: CursorAfterShellExecutionRaw =
                serde_json::from_str(stdin).context("parsing after-shell-execution input")?;
            let session_id = crate::host::checkpoints::lifecycle::apply_session_id_policy(
                &raw.conversation_id,
                crate::host::checkpoints::lifecycle::SessionIdPolicy::PreserveEmpty,
            )?;

            let Some(pre_prompt) = backend.load_pre_prompt(&session_id)? else {
                return Ok(());
            };
            if pre_prompt.source != PRE_PROMPT_SOURCE_CURSOR_SHELL {
                return Ok(());
            }

            let input = SessionInfoInput {
                session_id: session_id.clone(),
                transcript_path: crate::adapters::agents::cursor::lifecycle::resolve_transcript_ref(
                    &session_id,
                    raw.transcript_path.as_deref(),
                ),
            };
            handle_stop_with_profile_and_model(
                input,
                backend,
                strategy,
                Some(repo_root),
                crate::host::hooks::runtime::agent_runtime::CURSOR_HOOK_AGENT_PROFILE,
                &raw.model,
            )
        }
        CursorHookVerb::Stop => {
            let raw: CursorSessionInfoRaw =
                serde_json::from_str(stdin).context("parsing stop input")?;
            let session_id = crate::host::checkpoints::lifecycle::apply_session_id_policy(
                &raw.conversation_id,
                crate::host::checkpoints::lifecycle::SessionIdPolicy::PreserveEmpty,
            )?;
            let input = SessionInfoInput {
                session_id: session_id.clone(),
                transcript_path: crate::adapters::agents::cursor::lifecycle::resolve_transcript_ref(
                    &session_id,
                    raw.transcript_path.as_deref(),
                ),
            };
            handle_stop_with_profile_and_model(
                input,
                backend,
                strategy,
                Some(repo_root),
                crate::host::hooks::runtime::agent_runtime::CURSOR_HOOK_AGENT_PROFILE,
                &raw.model,
            )
        }
        CursorHookVerb::SessionEnd => {
            let raw: CursorSessionInfoRaw =
                serde_json::from_str(stdin).context("parsing session-end input")?;
            let session_id = crate::host::checkpoints::lifecycle::apply_session_id_policy(
                &raw.conversation_id,
                crate::host::checkpoints::lifecycle::SessionIdPolicy::PreserveEmpty,
            )?;
            let transcript_path =
                crate::adapters::agents::cursor::lifecycle::resolve_transcript_ref(
                    &session_id,
                    raw.transcript_path.as_deref(),
                );

            let pre_prompt = backend.load_pre_prompt(&session_id)?;
            let session = backend.load_session(&session_id)?;
            let should_finalize_turn = !session_id.is_empty()
                && (pre_prompt.is_some()
                    || session.is_none()
                    || session.as_ref().is_some_and(|state| {
                        state.phase == SessionPhase::Active
                            || (state.phase == SessionPhase::Idle && state.pending.step_count == 0)
                    }));

            if should_finalize_turn {
                handle_stop_with_profile_and_model(
                    SessionInfoInput {
                        session_id: session_id.clone(),
                        transcript_path: transcript_path.clone(),
                    },
                    backend,
                    strategy,
                    Some(repo_root),
                    crate::host::hooks::runtime::agent_runtime::CURSOR_HOOK_AGENT_PROFILE,
                    &raw.model,
                )?;
            }

            let input = SessionInfoInput {
                session_id,
                transcript_path,
            };
            handle_session_end_with_profile_and_model(
                input,
                backend,
                Some(repo_root),
                Some(crate::host::hooks::runtime::agent_runtime::CURSOR_HOOK_AGENT_PROFILE),
                &raw.model,
            )
        }
        CursorHookVerb::PreCompact
        | CursorHookVerb::SubagentStart
        | CursorHookVerb::SubagentStop => {
            route_hook_command_to_lifecycle(repo_root, AGENT_NAME_CURSOR, hook_name, stdin)
                .map(|_| ())
        }
    }
}

fn shell_command_to_prompt(command: &str) -> String {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        "Run shell command".to_string()
    } else {
        format!("Run shell command: {trimmed}")
    }
}

#[test]
fn agent_hooks_suppressed_env_accepts_truthy_values() {
    let _guard = enter_process_state(None, &[(BITLOOPS_SUPPRESS_AGENT_HOOKS_ENV, Some("1"))]);
    assert!(agent_hooks_suppressed_by_env());
}

#[test]
fn agent_hooks_suppressed_env_rejects_false_values() {
    for value in ["", "0", "false", "no", "off"] {
        let _guard = enter_process_state(None, &[(BITLOOPS_SUPPRESS_AGENT_HOOKS_ENV, Some(value))]);
        assert!(
            !agent_hooks_suppressed_by_env(),
            "value `{value}` should not suppress hooks"
        );
    }
}

#[test]
fn lifecycle_hook_dispatch_modes_cover_async_target_hooks() {
    use LifecycleHookDispatchMode::*;

    let cases = [
        (
            AGENT_NAME_CLAUDE_CODE,
            CLAUDE_HOOK_SESSION_START,
            AsyncNoSnapshot,
        ),
        (
            AGENT_NAME_CLAUDE_CODE,
            crate::host::checkpoints::lifecycle::adapters::CLAUDE_HOOK_USER_PROMPT_SUBMIT,
            AsyncPreBoundarySnapshot,
        ),
        (
            AGENT_NAME_CLAUDE_CODE,
            CLAUDE_HOOK_STOP,
            AsyncWorkspaceSnapshot,
        ),
        (
            AGENT_NAME_CLAUDE_CODE,
            crate::host::checkpoints::lifecycle::adapters::CLAUDE_HOOK_SESSION_END,
            AsyncNoSnapshot,
        ),
        (
            AGENT_NAME_CLAUDE_CODE,
            crate::host::checkpoints::lifecycle::adapters::CLAUDE_HOOK_PRE_TASK,
            AsyncPreBoundarySnapshot,
        ),
        (
            AGENT_NAME_CLAUDE_CODE,
            crate::host::checkpoints::lifecycle::adapters::CLAUDE_HOOK_POST_TASK,
            AsyncWorkspaceSnapshot,
        ),
        (
            AGENT_NAME_CLAUDE_CODE,
            crate::host::checkpoints::lifecycle::adapters::CLAUDE_HOOK_POST_TODO,
            AsyncWorkspaceAndBranchSnapshot,
        ),
        (
            AGENT_NAME_CLAUDE_CODE,
            crate::host::checkpoints::lifecycle::adapters::CLAUDE_HOOK_PRE_TOOL_USE,
            AsyncNoSnapshot,
        ),
        (
            AGENT_NAME_CLAUDE_CODE,
            crate::host::checkpoints::lifecycle::adapters::CLAUDE_HOOK_POST_TOOL_USE,
            AsyncNoSnapshot,
        ),
        (
            AGENT_NAME_CODEX,
            crate::host::checkpoints::lifecycle::adapters::CODEX_HOOK_SESSION_START,
            AsyncNoSnapshot,
        ),
        (
            AGENT_NAME_CODEX,
            crate::host::checkpoints::lifecycle::adapters::CODEX_HOOK_USER_PROMPT_SUBMIT,
            AsyncPreBoundarySnapshot,
        ),
        (
            AGENT_NAME_CODEX,
            crate::host::checkpoints::lifecycle::adapters::CODEX_HOOK_PRE_TOOL_USE,
            AsyncPreBoundarySnapshot,
        ),
        (
            AGENT_NAME_CODEX,
            crate::host::checkpoints::lifecycle::adapters::CODEX_HOOK_POST_TOOL_USE,
            AsyncWorkspaceSnapshot,
        ),
        (AGENT_NAME_CODEX, CODEX_HOOK_STOP, AsyncWorkspaceSnapshot),
        (
            AGENT_NAME_GEMINI,
            crate::host::checkpoints::lifecycle::adapters::GEMINI_HOOK_SESSION_START,
            AsyncNoSnapshot,
        ),
        (
            AGENT_NAME_GEMINI,
            crate::host::checkpoints::lifecycle::adapters::GEMINI_HOOK_BEFORE_AGENT,
            AsyncPreBoundarySnapshot,
        ),
        (
            AGENT_NAME_GEMINI,
            crate::host::checkpoints::lifecycle::adapters::GEMINI_HOOK_AFTER_AGENT,
            AsyncWorkspaceSnapshot,
        ),
        (
            AGENT_NAME_GEMINI,
            crate::host::checkpoints::lifecycle::adapters::GEMINI_HOOK_SESSION_END,
            AsyncNoSnapshot,
        ),
        (
            AGENT_NAME_GEMINI,
            crate::host::checkpoints::lifecycle::adapters::GEMINI_HOOK_PRE_COMPRESS,
            AsyncNoSnapshot,
        ),
        (
            AGENT_NAME_CURSOR,
            crate::host::checkpoints::lifecycle::adapters::CURSOR_HOOK_SESSION_START,
            AsyncNoSnapshot,
        ),
        (
            AGENT_NAME_CURSOR,
            crate::host::checkpoints::lifecycle::adapters::CURSOR_HOOK_BEFORE_SUBMIT_PROMPT,
            AsyncPreBoundarySnapshot,
        ),
        (
            AGENT_NAME_CURSOR,
            crate::adapters::agents::cursor::lifecycle::HOOK_NAME_BEFORE_SHELL_EXECUTION,
            AsyncPreBoundarySnapshot,
        ),
        (
            AGENT_NAME_CURSOR,
            crate::adapters::agents::cursor::lifecycle::HOOK_NAME_AFTER_SHELL_EXECUTION,
            AsyncWorkspaceSnapshot,
        ),
        (
            AGENT_NAME_CURSOR,
            crate::host::checkpoints::lifecycle::adapters::CURSOR_HOOK_STOP,
            AsyncWorkspaceSnapshot,
        ),
        (
            AGENT_NAME_CURSOR,
            crate::host::checkpoints::lifecycle::adapters::CURSOR_HOOK_SESSION_END,
            AsyncWorkspaceSnapshot,
        ),
        (
            AGENT_NAME_CURSOR,
            crate::host::checkpoints::lifecycle::adapters::CURSOR_HOOK_PRE_COMPACT,
            AsyncNoSnapshot,
        ),
        (
            AGENT_NAME_CURSOR,
            crate::host::checkpoints::lifecycle::adapters::CURSOR_HOOK_SUBAGENT_START,
            AsyncPreBoundarySnapshot,
        ),
        (
            AGENT_NAME_CURSOR,
            crate::host::checkpoints::lifecycle::adapters::CURSOR_HOOK_SUBAGENT_STOP,
            AsyncWorkspaceSnapshot,
        ),
        (
            AGENT_NAME_COPILOT,
            crate::host::checkpoints::lifecycle::adapters::COPILOT_HOOK_SESSION_START,
            AsyncNoSnapshot,
        ),
        (
            AGENT_NAME_COPILOT,
            crate::host::checkpoints::lifecycle::adapters::COPILOT_HOOK_USER_PROMPT_SUBMITTED,
            AsyncPreBoundarySnapshot,
        ),
        (
            AGENT_NAME_COPILOT,
            crate::host::checkpoints::lifecycle::adapters::COPILOT_HOOK_AGENT_STOP,
            AsyncWorkspaceSnapshot,
        ),
        (
            AGENT_NAME_COPILOT,
            crate::host::checkpoints::lifecycle::adapters::COPILOT_HOOK_SESSION_END,
            AsyncNoSnapshot,
        ),
        (
            AGENT_NAME_COPILOT,
            crate::host::checkpoints::lifecycle::adapters::COPILOT_HOOK_SUBAGENT_STOP,
            AsyncWorkspaceSnapshot,
        ),
        (
            AGENT_NAME_OPEN_CODE,
            crate::host::checkpoints::lifecycle::adapters::OPENCODE_HOOK_SESSION_START,
            AsyncNoSnapshot,
        ),
        (
            AGENT_NAME_OPEN_CODE,
            crate::host::checkpoints::lifecycle::adapters::OPENCODE_HOOK_TURN_START,
            AsyncPreBoundarySnapshot,
        ),
        (
            AGENT_NAME_OPEN_CODE,
            crate::host::checkpoints::lifecycle::adapters::OPENCODE_HOOK_TURN_END,
            AsyncWorkspaceSnapshot,
        ),
        (
            AGENT_NAME_OPEN_CODE,
            crate::host::checkpoints::lifecycle::adapters::OPENCODE_HOOK_COMPACTION,
            AsyncNoSnapshot,
        ),
        (
            AGENT_NAME_OPEN_CODE,
            crate::host::checkpoints::lifecycle::adapters::OPENCODE_HOOK_SESSION_END,
            AsyncNoSnapshot,
        ),
    ];

    for (agent, hook, expected) in cases {
        assert_eq!(
            lifecycle_hook_dispatch_mode(agent, hook),
            expected,
            "unexpected lifecycle dispatch mode for agent={agent} hook={hook}"
        );
    }
}

#[test]
fn pass_through_hooks_remain_sync_noop() {
    use LifecycleHookDispatchMode::Sync;

    let cases = [
        (
            AGENT_NAME_COPILOT,
            crate::host::checkpoints::lifecycle::adapters::COPILOT_HOOK_PRE_TOOL_USE,
        ),
        (
            AGENT_NAME_COPILOT,
            crate::host::checkpoints::lifecycle::adapters::COPILOT_HOOK_POST_TOOL_USE,
        ),
        (
            AGENT_NAME_COPILOT,
            crate::host::checkpoints::lifecycle::adapters::COPILOT_HOOK_ERROR_OCCURRED,
        ),
        (
            AGENT_NAME_GEMINI,
            crate::host::checkpoints::lifecycle::adapters::GEMINI_HOOK_BEFORE_TOOL,
        ),
        (
            AGENT_NAME_GEMINI,
            crate::host::checkpoints::lifecycle::adapters::GEMINI_HOOK_AFTER_TOOL,
        ),
        (
            AGENT_NAME_GEMINI,
            crate::host::checkpoints::lifecycle::adapters::GEMINI_HOOK_BEFORE_MODEL,
        ),
        (
            AGENT_NAME_GEMINI,
            crate::host::checkpoints::lifecycle::adapters::GEMINI_HOOK_AFTER_MODEL,
        ),
        (
            AGENT_NAME_GEMINI,
            crate::host::checkpoints::lifecycle::adapters::GEMINI_HOOK_BEFORE_TOOL_SELECTION,
        ),
        (
            AGENT_NAME_GEMINI,
            crate::host::checkpoints::lifecycle::adapters::GEMINI_HOOK_NOTIFICATION,
        ),
    ];

    for (agent, hook) in cases {
        assert_eq!(
            lifecycle_hook_dispatch_mode(agent, hook),
            Sync,
            "unexpected async dispatch for pass-through agent={agent} hook={hook}"
        );
    }
}

#[test]
fn codex_stop_hook_enqueue_creates_spool_without_inline_turn() -> Result<()> {
    let repo = tempfile::tempdir()?;
    crate::test_support::git_fixtures::init_test_repo(
        repo.path(),
        "main",
        "Bitloops Test",
        "bitloops@example.com",
    );
    crate::test_support::git_fixtures::write_test_daemon_config(repo.path());

    enqueue_lifecycle_hook_from_hook(
        repo.path(),
        AGENT_NAME_CODEX,
        CODEX_HOOK_STOP,
        r#"{"session_id":"session-1","transcript_path":"/tmp/session.jsonl"}"#,
        LifecycleHookDispatchMode::AsyncWorkspaceSnapshot,
    )?;

    let conn = rusqlite::Connection::open(
        crate::config::resolve_bound_repo_runtime_db_path_for_repo(repo.path())?,
    )?;
    let (lifecycle_jobs, snapshots): (i64, i64) = conn.query_row(
        "SELECT COUNT(*), COUNT(workspace_snapshot) FROM agent_lifecycle_spool_jobs",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let turns_table_exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'interaction_turns'",
        [],
        |row| row.get(0),
    )?;
    let turns = if turns_table_exists == 0 {
        0
    } else {
        conn.query_row("SELECT COUNT(*) FROM interaction_turns", [], |row| {
            row.get(0)
        })?
    };

    assert_eq!(lifecycle_jobs, 1);
    assert_eq!(snapshots, 1);
    assert_eq!(turns, 0, "hook enqueue must not run turn-end inline");
    Ok(())
}

#[test]
fn pilot_non_turn_end_hook_enqueue_omits_workspace_snapshot() -> Result<()> {
    let repo = tempfile::tempdir()?;
    crate::test_support::git_fixtures::init_test_repo(
        repo.path(),
        "main",
        "Bitloops Test",
        "bitloops@example.com",
    );
    crate::test_support::git_fixtures::write_test_daemon_config(repo.path());

    enqueue_lifecycle_hook_from_hook(
        repo.path(),
        AGENT_NAME_GEMINI,
        crate::host::checkpoints::lifecycle::adapters::GEMINI_HOOK_PRE_COMPRESS,
        r#"{"session_id":"session-1","transcript_path":"/tmp/session.jsonl"}"#,
        LifecycleHookDispatchMode::AsyncNoSnapshot,
    )?;

    let conn = rusqlite::Connection::open(
        crate::config::resolve_bound_repo_runtime_db_path_for_repo(repo.path())?,
    )?;
    let (lifecycle_jobs, snapshots): (i64, i64) = conn.query_row(
        "SELECT COUNT(*), COUNT(workspace_snapshot) FROM agent_lifecycle_spool_jobs",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;

    assert_eq!(lifecycle_jobs, 1);
    assert_eq!(snapshots, 0);
    Ok(())
}

#[test]
fn observation_hook_enqueue_omits_workspace_snapshot() -> Result<()> {
    let repo = tempfile::tempdir()?;
    crate::test_support::git_fixtures::init_test_repo(
        repo.path(),
        "main",
        "Bitloops Test",
        "bitloops@example.com",
    );
    crate::test_support::git_fixtures::write_test_daemon_config(repo.path());

    enqueue_lifecycle_hook_from_hook(
        repo.path(),
        AGENT_NAME_CLAUDE_CODE,
        crate::host::checkpoints::lifecycle::adapters::CLAUDE_HOOK_PRE_TOOL_USE,
        r#"{"session_id":"session-1","transcript_path":"/tmp/session.jsonl","tool_use_id":"toolu_1","tool_name":"Bash","tool_input":{"command":"cargo check"}}"#,
        LifecycleHookDispatchMode::AsyncNoSnapshot,
    )?;

    let conn = rusqlite::Connection::open(
        crate::config::resolve_bound_repo_runtime_db_path_for_repo(repo.path())?,
    )?;
    let (lifecycle_jobs, snapshots): (i64, i64) = conn.query_row(
        "SELECT COUNT(*), COUNT(workspace_snapshot) FROM agent_lifecycle_spool_jobs",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;

    assert_eq!(lifecycle_jobs, 1);
    assert_eq!(snapshots, 0);
    Ok(())
}

#[test]
fn pre_boundary_hook_enqueue_persists_boundary_snapshot_without_workspace() -> Result<()> {
    let repo = tempfile::tempdir()?;
    crate::test_support::git_fixtures::init_test_repo(
        repo.path(),
        "main",
        "Bitloops Test",
        "bitloops@example.com",
    );
    crate::test_support::git_fixtures::write_test_daemon_config(repo.path());
    std::fs::write(repo.path().join("scratch.txt"), "before")?;

    enqueue_lifecycle_hook_from_hook(
        repo.path(),
        AGENT_NAME_CODEX,
        crate::host::checkpoints::lifecycle::adapters::CODEX_HOOK_USER_PROMPT_SUBMIT,
        r#"{"session_id":"session-1","transcript_path":"/tmp/session.jsonl","prompt":"hello"}"#,
        LifecycleHookDispatchMode::AsyncPreBoundarySnapshot,
    )?;

    let conn = rusqlite::Connection::open(
        crate::config::resolve_bound_repo_runtime_db_path_for_repo(repo.path())?,
    )?;
    let (workspace_snapshot, boundary_snapshot): (Option<String>, String) = conn.query_row(
        "SELECT workspace_snapshot, boundary_snapshot FROM agent_lifecycle_spool_jobs",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let snapshot: crate::host::checkpoints::lifecycle::spool::LifecycleBoundarySnapshot =
        serde_json::from_str(&boundary_snapshot)?;

    assert!(workspace_snapshot.is_none());
    assert_eq!(snapshot.pre_untracked_files, vec!["scratch.txt"]);
    assert!(snapshot.workspace.is_none());
    Ok(())
}

#[test]
fn pre_boundary_hook_enqueue_persists_transcript_offset_when_resolved() -> Result<()> {
    let repo = tempfile::tempdir()?;
    crate::test_support::git_fixtures::init_test_repo(
        repo.path(),
        "main",
        "Bitloops Test",
        "bitloops@example.com",
    );
    crate::test_support::git_fixtures::write_test_daemon_config(repo.path());
    let transcript_path = repo.path().join("codex-transcript.jsonl");
    std::fs::write(&transcript_path, "first line\nsecond line\n")?;

    enqueue_lifecycle_hook_from_hook(
        repo.path(),
        AGENT_NAME_CODEX,
        crate::host::checkpoints::lifecycle::adapters::CODEX_HOOK_USER_PROMPT_SUBMIT,
        &serde_json::json!({
            "session_id": "session-1",
            "transcript_path": transcript_path,
            "prompt": "hello"
        })
        .to_string(),
        LifecycleHookDispatchMode::AsyncPreBoundarySnapshot,
    )?;

    let conn = rusqlite::Connection::open(
        crate::config::resolve_bound_repo_runtime_db_path_for_repo(repo.path())?,
    )?;
    let boundary_snapshot: String = conn.query_row(
        "SELECT boundary_snapshot FROM agent_lifecycle_spool_jobs",
        [],
        |row| row.get(0),
    )?;
    let snapshot: crate::host::checkpoints::lifecycle::spool::LifecycleBoundarySnapshot =
        serde_json::from_str(&boundary_snapshot)?;

    assert_eq!(snapshot.transcript_offset, Some(2));
    Ok(())
}

#[test]
fn workspace_hook_enqueue_persists_boundary_workspace_snapshot() -> Result<()> {
    let repo = tempfile::tempdir()?;
    crate::test_support::git_fixtures::init_test_repo(
        repo.path(),
        "main",
        "Bitloops Test",
        "bitloops@example.com",
    );
    crate::test_support::git_fixtures::write_test_daemon_config(repo.path());
    std::fs::write(repo.path().join("README.md"), "new")?;

    enqueue_lifecycle_hook_from_hook(
        repo.path(),
        AGENT_NAME_CURSOR,
        crate::adapters::agents::cursor::lifecycle::HOOK_NAME_AFTER_SHELL_EXECUTION,
        r#"{"conversation_id":"session-1","transcript_path":"/tmp/session.jsonl"}"#,
        LifecycleHookDispatchMode::AsyncWorkspaceSnapshot,
    )?;

    let conn = rusqlite::Connection::open(
        crate::config::resolve_bound_repo_runtime_db_path_for_repo(repo.path())?,
    )?;
    let (workspace_snapshot, boundary_snapshot): (String, String) = conn.query_row(
        "SELECT workspace_snapshot, boundary_snapshot FROM agent_lifecycle_spool_jobs",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let legacy_workspace: crate::host::checkpoints::lifecycle::spool::LifecycleWorkspaceSnapshot =
        serde_json::from_str(&workspace_snapshot)?;
    let boundary: crate::host::checkpoints::lifecycle::spool::LifecycleBoundarySnapshot =
        serde_json::from_str(&boundary_snapshot)?;

    assert_eq!(legacy_workspace.new_files, vec!["README.md"]);
    assert_eq!(
        boundary
            .workspace
            .as_ref()
            .map(|snapshot| snapshot.new_files.clone()),
        Some(vec!["README.md".to_string()])
    );
    Ok(())
}

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
fn supported_agent_terminal_turn_end_hooks_use_lifecycle_spool() {
    assert!(should_spool_lifecycle_stop_hook(
        AGENT_NAME_CODEX,
        CODEX_HOOK_STOP
    ));
    assert!(should_spool_lifecycle_stop_hook(
        AGENT_NAME_CLAUDE_CODE,
        CLAUDE_HOOK_STOP
    ));
    assert!(should_spool_lifecycle_stop_hook(
        AGENT_NAME_GEMINI,
        crate::host::checkpoints::lifecycle::adapters::GEMINI_HOOK_AFTER_AGENT
    ));
    assert!(should_spool_lifecycle_stop_hook(
        AGENT_NAME_CURSOR,
        crate::host::checkpoints::lifecycle::adapters::CURSOR_HOOK_STOP
    ));
    assert!(should_spool_lifecycle_stop_hook(
        AGENT_NAME_COPILOT,
        crate::host::checkpoints::lifecycle::adapters::COPILOT_HOOK_AGENT_STOP
    ));
    assert!(should_spool_lifecycle_stop_hook(
        AGENT_NAME_OPEN_CODE,
        crate::host::checkpoints::lifecycle::adapters::OPENCODE_HOOK_TURN_END
    ));
    assert!(!should_spool_lifecycle_stop_hook(
        AGENT_NAME_CODEX,
        crate::host::checkpoints::lifecycle::adapters::CODEX_HOOK_USER_PROMPT_SUBMIT
    ));
}

#[test]
fn non_stop_hooks_remain_synchronous() {
    let cases = [
        (
            AGENT_NAME_CODEX,
            crate::host::checkpoints::lifecycle::adapters::CODEX_HOOK_SESSION_START,
        ),
        (
            AGENT_NAME_CODEX,
            crate::host::checkpoints::lifecycle::adapters::CODEX_HOOK_USER_PROMPT_SUBMIT,
        ),
        (
            AGENT_NAME_CODEX,
            crate::host::checkpoints::lifecycle::adapters::CODEX_HOOK_PRE_TOOL_USE,
        ),
        (
            AGENT_NAME_CODEX,
            crate::host::checkpoints::lifecycle::adapters::CODEX_HOOK_POST_TOOL_USE,
        ),
        (
            AGENT_NAME_CURSOR,
            crate::adapters::agents::cursor::lifecycle::HOOK_NAME_AFTER_SHELL_EXECUTION,
        ),
        (
            AGENT_NAME_CURSOR,
            crate::host::checkpoints::lifecycle::adapters::CURSOR_HOOK_SESSION_END,
        ),
        (
            AGENT_NAME_OPEN_CODE,
            crate::host::checkpoints::lifecycle::adapters::OPENCODE_HOOK_COMPACTION,
        ),
        (
            AGENT_NAME_GEMINI,
            crate::host::checkpoints::lifecycle::adapters::GEMINI_HOOK_SESSION_END,
        ),
        (
            AGENT_NAME_COPILOT,
            crate::host::checkpoints::lifecycle::adapters::COPILOT_HOOK_SESSION_END,
        ),
        (
            AGENT_NAME_CLAUDE_CODE,
            crate::host::checkpoints::lifecycle::adapters::CLAUDE_HOOK_SESSION_END,
        ),
    ];

    for (agent, hook) in cases {
        assert!(
            !should_spool_lifecycle_stop_hook(agent, hook),
            "unexpected spooling for agent={agent} hook={hook}"
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

    enqueue_lifecycle_stop_from_hook(
        repo.path(),
        AGENT_NAME_CODEX,
        CODEX_HOOK_STOP,
        r#"{"session_id":"session-1","transcript_path":"/tmp/session.jsonl"}"#,
    )?;

    let conn = rusqlite::Connection::open(
        crate::config::resolve_bound_repo_runtime_db_path_for_repo(repo.path())?,
    )?;
    let stop_jobs: i64 = conn.query_row(
        "SELECT COUNT(*) FROM agent_lifecycle_stop_spool_jobs",
        [],
        |row| row.get(0),
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

    assert_eq!(stop_jobs, 1);
    assert_eq!(turns, 0, "hook enqueue must not run turn-end inline");
    Ok(())
}

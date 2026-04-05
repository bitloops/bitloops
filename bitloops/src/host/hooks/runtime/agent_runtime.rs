//! Agent hook runtime shared by agent adapters.
//!
//! Shared top-level hook command routing lives in `crate::host::hooks::dispatcher`.

use std::collections::HashSet;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::process::Command;

#[cfg(test)]
use anyhow::bail;
use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::adapters::agents::{
    AGENT_NAME_CLAUDE_CODE, AGENT_NAME_CODEX, AGENT_NAME_CURSOR, AGENT_TYPE_CLAUDE_CODE,
    AGENT_TYPE_CODEX, AGENT_TYPE_CURSOR, TokenUsage,
};
use crate::config::settings;
use crate::git;
use crate::host::checkpoints::lifecycle::interaction::{
    flush_interaction_spool_best_effort, resolve_interaction_spool,
};
use crate::host::checkpoints::history::devql_prefetch;
use crate::host::checkpoints::transcript::commit_message;
use crate::host::checkpoints::transcript::utils::get_transcript_position;
#[cfg(test)]
use crate::telemetry::logging;
use crate::utils::paths;
use crate::utils::strings;

use crate::adapters::agents::claude_code::git_hooks;
use crate::adapters::agents::claude_code::hooks as claude_hooks;
use crate::adapters::agents::claude_code::transcript as claude_transcript;
use crate::host::checkpoints::session::backend::SessionBackend;
use crate::host::checkpoints::session::phase::{
    Event, NoOpActionHandler, TransitionContext, apply_transition, transition_with_context,
};
use crate::host::checkpoints::session::state::{PrePromptState, PreTaskState, SessionState};
use crate::host::checkpoints::strategy::manual_commit::TokenUsageMetadata;
use crate::host::checkpoints::strategy::noop::NoOpStrategy;
use crate::host::checkpoints::strategy::{StepContext, Strategy, TaskStepContext};
use crate::host::interactions::store::InteractionSpool;
use crate::host::interactions::types::{
    InteractionEvent, InteractionEventType, InteractionSession, InteractionTurn,
};

#[derive(Debug, Clone, Copy)]
pub struct HookAgentProfile {
    pub agent_name: &'static str,
    pub agent_type: &'static str,
}

pub const CLAUDE_HOOK_AGENT_PROFILE: HookAgentProfile = HookAgentProfile {
    agent_name: AGENT_NAME_CLAUDE_CODE,
    agent_type: AGENT_TYPE_CLAUDE_CODE,
};

pub const CURSOR_HOOK_AGENT_PROFILE: HookAgentProfile = HookAgentProfile {
    agent_name: AGENT_NAME_CURSOR,
    agent_type: AGENT_TYPE_CURSOR,
};

pub const CODEX_HOOK_AGENT_PROFILE: HookAgentProfile = HookAgentProfile {
    agent_name: AGENT_NAME_CODEX,
    agent_type: AGENT_TYPE_CODEX,
};

// ── Stdin JSON input types ────────────────────────────────────────────────────

/// Used by session-start, stop, session-end.
///
#[derive(Debug, Deserialize)]
pub struct SessionInfoInput {
    pub session_id: String,
    pub transcript_path: String,
}

/// Used by user-prompt-submit.
///
#[derive(Debug, Deserialize)]
pub struct UserPromptSubmitInput {
    pub session_id: String,
    pub transcript_path: String,
    pub prompt: String,
}

/// Used by pre-task.
///
#[derive(Debug, Deserialize)]
pub struct TaskHookInput {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub transcript_path: String,
    #[serde(default)]
    pub tool_use_id: String,
    #[serde(default)]
    pub tool_input: Option<Value>,
}

/// Used by post-task.
///
#[derive(Debug, Deserialize)]
pub struct PostTaskInput {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub transcript_path: String,
    #[serde(default)]
    pub tool_use_id: String,
    #[serde(default)]
    pub tool_input: Option<Value>,
    #[serde(default)]
    pub tool_response: TaskToolResponse,
}

#[derive(Debug, Deserialize, Default)]
pub struct TaskToolResponse {
    #[serde(default, rename = "agentId")]
    pub agent_id: String,
}

/// Used by post-todo.
///
#[derive(Debug, Deserialize)]
pub struct PostTodoInput {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub transcript_path: String,
    #[serde(default)]
    pub tool_use_id: String,
    #[serde(default)]
    pub tool_name: String,
    #[serde(default)]
    pub tool_input: Option<Value>,
}

/// Used by post-todo parsing tests.
#[cfg(test)]
#[derive(Debug, Deserialize)]
struct SubagentCheckpointHookInput {
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    transcript_path: String,
    #[serde(default)]
    tool_name: String,
    #[serde(default)]
    tool_use_id: String,
    #[serde(default)]
    tool_input: Option<Value>,
    #[serde(default)]
    tool_response: Option<Value>,
}

fn apply_session_transition(state: &mut SessionState, event: Event) {
    let result = transition_with_context(state.phase, event, TransitionContext::default());
    let mut handler = NoOpActionHandler;
    if let Err(err) = apply_transition(state, result, &mut handler) {
        eprintln!("[bitloops] Warning: session transition failed ({event}): {err}");
    }
}

fn interaction_agent_type(
    state: Option<&SessionState>,
    profile: Option<HookAgentProfile>,
) -> String {
    state
        .filter(|state| !state.agent_type.trim().is_empty())
        .map(|state| state.agent_type.clone())
        .or_else(|| profile.map(|profile| profile.agent_type.to_string()))
        .unwrap_or_default()
}

fn interaction_started_at(state: Option<&SessionState>, fallback: &str) -> String {
    state
        .filter(|state| !state.started_at.trim().is_empty())
        .map(|state| state.started_at.clone())
        .unwrap_or_else(|| fallback.to_string())
}

fn interaction_worktree_path(repo_root: &Path, state: Option<&SessionState>) -> String {
    state
        .filter(|state| !state.worktree_path.trim().is_empty())
        .map(|state| state.worktree_path.clone())
        .unwrap_or_else(|| repo_root.to_string_lossy().into_owned())
}

fn interaction_worktree_id(repo_root: &Path, state: Option<&SessionState>) -> String {
    state
        .filter(|state| !state.worktree_id.trim().is_empty())
        .map(|state| state.worktree_id.clone())
        .unwrap_or_else(|| paths::get_worktree_id(repo_root).unwrap_or_default())
}

fn interaction_event_id() -> String {
    Uuid::new_v4().simple().to_string()
}

fn token_usage_metadata(token_usage: Option<&TokenUsage>) -> Option<TokenUsageMetadata> {
    token_usage.map(|token_usage| TokenUsageMetadata {
        input_tokens: token_usage.input_tokens.max(0) as u64,
        cache_creation_tokens: token_usage.cache_creation_tokens.max(0) as u64,
        cache_read_tokens: token_usage.cache_read_tokens.max(0) as u64,
        output_tokens: token_usage.output_tokens.max(0) as u64,
        api_call_count: token_usage.api_call_count.max(0) as u64,
        subagent_tokens: None,
    })
}

fn with_interaction_spool<F>(repo_root: Option<&Path>, f: F)
where
    F: FnOnce(&crate::host::interactions::db_store::SqliteInteractionSpool),
{
    let Some(repo_root) = repo_root else {
        return;
    };
    let Some(spool) = resolve_interaction_spool(repo_root) else {
        return;
    };
    f(&spool);
    flush_interaction_spool_best_effort(repo_root);
}

fn record_session_start_interaction(
    repo_root: Option<&Path>,
    state: &SessionState,
    profile: Option<HookAgentProfile>,
) {
    let event_time = state
        .last_interaction_time
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(now_rfc3339);
    with_interaction_spool(repo_root, |spool| {
        let Some(repo_root) = repo_root else {
            return;
        };
        let session = InteractionSession {
            session_id: state.session_id.clone(),
            repo_id: spool.repo_id().to_string(),
            agent_type: interaction_agent_type(Some(state), profile),
            first_prompt: state.first_prompt.clone(),
            transcript_path: state.transcript_path.clone(),
            worktree_path: interaction_worktree_path(repo_root, Some(state)),
            worktree_id: interaction_worktree_id(repo_root, Some(state)),
            started_at: interaction_started_at(Some(state), &event_time),
            ended_at: state.ended_at.clone(),
            last_event_at: event_time.clone(),
            updated_at: event_time.clone(),
            ..Default::default()
        };
        if let Err(err) = spool.record_session(&session) {
            eprintln!("[bitloops] Warning: failed to spool interaction session: {err}");
        }
        if let Err(err) = spool.record_event(&InteractionEvent {
            event_id: interaction_event_id(),
            session_id: state.session_id.clone(),
            turn_id: None,
            repo_id: spool.repo_id().to_string(),
            event_type: InteractionEventType::SessionStart,
            event_time,
            agent_type: session.agent_type.clone(),
            model: String::new(),
            payload: serde_json::json!({
                "first_prompt": session.first_prompt,
                "transcript_path": session.transcript_path,
                "worktree_path": session.worktree_path,
                "worktree_id": session.worktree_id,
            }),
        }) {
            eprintln!("[bitloops] Warning: failed to spool session_start event: {err}");
        }
    });
}

fn record_turn_start_interaction(
    repo_root: Option<&Path>,
    state: &SessionState,
    prompt: &str,
    profile: HookAgentProfile,
) {
    let event_time = state
        .last_interaction_time
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(now_rfc3339);
    let prompt = truncate_prompt_for_storage(prompt);
    with_interaction_spool(repo_root, |spool| {
        let Some(repo_root) = repo_root else {
            return;
        };
        let session = InteractionSession {
            session_id: state.session_id.clone(),
            repo_id: spool.repo_id().to_string(),
            agent_type: interaction_agent_type(Some(state), Some(profile)),
            first_prompt: state.first_prompt.clone(),
            transcript_path: state.transcript_path.clone(),
            worktree_path: interaction_worktree_path(repo_root, Some(state)),
            worktree_id: interaction_worktree_id(repo_root, Some(state)),
            started_at: interaction_started_at(Some(state), &event_time),
            ended_at: state.ended_at.clone(),
            last_event_at: event_time.clone(),
            updated_at: event_time.clone(),
            ..Default::default()
        };
        if let Err(err) = spool.record_session(&session) {
            eprintln!("[bitloops] Warning: failed to spool interaction session: {err}");
        }
        let turn_number = state.step_count + 1;
        let turn = InteractionTurn {
            turn_id: state.turn_id.clone(),
            session_id: state.session_id.clone(),
            repo_id: spool.repo_id().to_string(),
            turn_number,
            prompt: prompt.clone(),
            agent_type: session.agent_type.clone(),
            started_at: event_time.clone(),
            updated_at: event_time.clone(),
            ..Default::default()
        };
        if let Err(err) = spool.record_turn(&turn) {
            eprintln!("[bitloops] Warning: failed to spool interaction turn start: {err}");
        }
        if let Err(err) = spool.record_event(&InteractionEvent {
            event_id: interaction_event_id(),
            session_id: state.session_id.clone(),
            turn_id: Some(state.turn_id.clone()),
            repo_id: spool.repo_id().to_string(),
            event_type: InteractionEventType::TurnStart,
            event_time,
            agent_type: session.agent_type.clone(),
            model: String::new(),
            payload: serde_json::json!({
                "prompt": turn.prompt,
                "turn_number": turn_number,
            }),
        }) {
            eprintln!("[bitloops] Warning: failed to spool turn_start event: {err}");
        }
    });
}

struct TurnEndInteraction<'a> {
    repo_root: Option<&'a Path>,
    session_id: &'a str,
    transcript_path: &'a str,
    state: Option<&'a SessionState>,
    prompt: &'a str,
    turn_started_at: Option<&'a str>,
    profile: HookAgentProfile,
    files_modified: &'a [String],
    token_usage: Option<&'a TokenUsage>,
}

fn record_turn_end_interaction(ctx: TurnEndInteraction<'_>) {
    let TurnEndInteraction {
        repo_root,
        session_id,
        transcript_path,
        state,
        prompt,
        turn_started_at,
        profile,
        files_modified,
        token_usage,
    } = ctx;
    let event_time = now_rfc3339();
    let token_usage = token_usage_metadata(token_usage);
    let prompt = truncate_prompt_for_storage(prompt);
    with_interaction_spool(repo_root, |spool| {
        let Some(repo_root) = repo_root else {
            return;
        };
        let agent_type = interaction_agent_type(state, Some(profile));
        let session = InteractionSession {
            session_id: session_id.to_string(),
            repo_id: spool.repo_id().to_string(),
            agent_type: agent_type.clone(),
            first_prompt: state
                .map(|state| state.first_prompt.clone())
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| prompt.clone()),
            transcript_path: if transcript_path.trim().is_empty() {
                state
                    .map(|state| state.transcript_path.clone())
                    .unwrap_or_default()
            } else {
                transcript_path.to_string()
            },
            worktree_path: interaction_worktree_path(repo_root, state),
            worktree_id: interaction_worktree_id(repo_root, state),
            started_at: interaction_started_at(state, &event_time),
            ended_at: state.and_then(|state| state.ended_at.clone()),
            last_event_at: event_time.clone(),
            updated_at: event_time.clone(),
            ..Default::default()
        };
        if let Err(err) = spool.record_session(&session) {
            eprintln!("[bitloops] Warning: failed to spool interaction session: {err}");
        }

        let turn_id = state
            .filter(|state| !state.turn_id.trim().is_empty())
            .map(|state| state.turn_id.clone())
            .unwrap_or_else(generate_turn_id);
        let turn = InteractionTurn {
            turn_id: turn_id.clone(),
            session_id: session_id.to_string(),
            repo_id: spool.repo_id().to_string(),
            turn_number: state.map_or(1, |state| state.step_count + 1),
            prompt: prompt.clone(),
            agent_type: agent_type.clone(),
            started_at: turn_started_at
                .filter(|value| !value.trim().is_empty())
                .map(str::to_string)
                .or_else(|| {
                    state.and_then(|state| {
                        state
                            .last_interaction_time
                            .clone()
                            .filter(|value| !value.trim().is_empty())
                    })
                })
                .unwrap_or_else(|| event_time.clone()),
            ended_at: Some(event_time.clone()),
            token_usage: token_usage.clone(),
            files_modified: files_modified.to_vec(),
            checkpoint_id: None,
            updated_at: event_time.clone(),
            ..Default::default()
        };
        if let Err(err) = spool.record_turn(&turn) {
            eprintln!("[bitloops] Warning: failed to spool interaction turn end: {err}");
        }
        if let Err(err) = spool.record_event(&InteractionEvent {
            event_id: interaction_event_id(),
            session_id: session_id.to_string(),
            turn_id: Some(turn_id),
            repo_id: spool.repo_id().to_string(),
            event_type: InteractionEventType::TurnEnd,
            event_time,
            agent_type,
            model: String::new(),
            payload: serde_json::json!({
                "files_modified": files_modified,
                "files_count": files_modified.len(),
                "token_usage": token_usage,
            }),
        }) {
            eprintln!("[bitloops] Warning: failed to spool turn_end event: {err}");
        }
    });
}

fn record_session_end_interaction(
    repo_root: Option<&Path>,
    session_id: &str,
    transcript_path: &str,
    state: Option<&SessionState>,
    profile: Option<HookAgentProfile>,
) {
    let ended_at = state
        .and_then(|state| state.ended_at.clone())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(now_rfc3339);
    with_interaction_spool(repo_root, |spool| {
        let Some(repo_root) = repo_root else {
            return;
        };
        let session = InteractionSession {
            session_id: session_id.to_string(),
            repo_id: spool.repo_id().to_string(),
            agent_type: interaction_agent_type(state, profile),
            first_prompt: state.map(|state| state.first_prompt.clone()).unwrap_or_default(),
            transcript_path: if transcript_path.trim().is_empty() {
                state
                    .map(|state| state.transcript_path.clone())
                    .unwrap_or_default()
            } else {
                transcript_path.to_string()
            },
            worktree_path: interaction_worktree_path(repo_root, state),
            worktree_id: interaction_worktree_id(repo_root, state),
            started_at: interaction_started_at(state, &ended_at),
            ended_at: Some(ended_at.clone()),
            last_event_at: ended_at.clone(),
            updated_at: ended_at.clone(),
            ..Default::default()
        };
        if let Err(err) = spool.record_session(&session) {
            eprintln!("[bitloops] Warning: failed to spool interaction session end: {err}");
        }
        if let Err(err) = spool.record_event(&InteractionEvent {
            event_id: interaction_event_id(),
            session_id: session_id.to_string(),
            turn_id: None,
            repo_id: spool.repo_id().to_string(),
            event_type: InteractionEventType::SessionEnd,
            event_time: ended_at,
            agent_type: session.agent_type.clone(),
            model: String::new(),
            payload: serde_json::Value::Object(Default::default()),
        }) {
            eprintln!("[bitloops] Warning: failed to spool session_end event: {err}");
        }
    });
}

fn record_subagent_interaction_event(
    repo_root: Option<&Path>,
    session_id: &str,
    state: Option<&SessionState>,
    profile: HookAgentProfile,
    event_type: InteractionEventType,
    payload: serde_json::Value,
) {
    let event_time = now_rfc3339();
    with_interaction_spool(repo_root, |spool| {
        if let Err(err) = spool.record_event(&InteractionEvent {
            event_id: interaction_event_id(),
            session_id: session_id.to_string(),
            turn_id: state
                .filter(|state| !state.turn_id.trim().is_empty())
                .map(|state| state.turn_id.clone()),
            repo_id: spool.repo_id().to_string(),
            event_type,
            event_time,
            agent_type: interaction_agent_type(state, Some(profile)),
            model: String::new(),
            payload,
        }) {
            eprintln!("[bitloops] Warning: failed to spool {event_type} event: {err}");
        }
    });
}

// ── Handler functions ─────────────────────────────────────────────────────────

/// `session-start`: create or reset session state, transition → Idle.
///
pub fn handle_session_start(
    input: SessionInfoInput,
    backend: &dyn SessionBackend,
    repo_root: Option<&std::path::Path>,
) -> Result<()> {
    handle_session_start_with_profile(input, backend, repo_root, None)
}

pub fn handle_session_start_with_profile(
    input: SessionInfoInput,
    backend: &dyn SessionBackend,
    repo_root: Option<&std::path::Path>,
    profile: Option<HookAgentProfile>,
) -> Result<()> {
    let session_id = crate::host::checkpoints::lifecycle::apply_session_id_policy(
        &input.session_id,
        crate::host::checkpoints::lifecycle::SessionIdPolicy::Strict,
    )
    .context("session-start requires non-empty session_id")?;

    let mut state = backend
        .load_session(&session_id)?
        .unwrap_or_else(|| SessionState {
            session_id: session_id.clone(),
            ..Default::default()
        });

    apply_session_transition(&mut state, Event::SessionStart);
    state.transcript_path = input.transcript_path;
    let now = now_rfc3339();
    if state.started_at.trim().is_empty() {
        state.started_at = now.clone();
    }
    state.last_interaction_time = Some(now);

    // Detect and record worktree information for shadow branch naming.
    if let Some(root) = repo_root {
        state.worktree_path = root.to_string_lossy().into_owned();
        state.worktree_id = paths::get_worktree_id(root)
            .with_context(|| format!("failed to resolve worktree id for {}", root.display()))?;
    }

    backend.save_session(&state)?;
    record_session_start_interaction(repo_root, &state, profile);
    Ok(())
}

/// `user-prompt-submit`: initialize session, transition → Active, save pre-prompt state.
///
pub fn handle_user_prompt_submit(
    input: UserPromptSubmitInput,
    backend: &dyn SessionBackend,
    repo_root: Option<&Path>,
) -> Result<()> {
    let fallback = NoOpStrategy;
    handle_user_prompt_submit_with_strategy(input, backend, &fallback, repo_root)
}

pub fn handle_user_prompt_submit_with_strategy(
    input: UserPromptSubmitInput,
    backend: &dyn SessionBackend,
    strategy: &dyn Strategy,
    repo_root: Option<&Path>,
) -> Result<()> {
    handle_user_prompt_submit_with_strategy_and_profile(
        input,
        backend,
        strategy,
        repo_root,
        CLAUDE_HOOK_AGENT_PROFILE,
    )
}

pub fn handle_user_prompt_submit_with_strategy_and_profile(
    input: UserPromptSubmitInput,
    backend: &dyn SessionBackend,
    strategy: &dyn Strategy,
    repo_root: Option<&Path>,
    profile: HookAgentProfile,
) -> Result<()> {
    let session_id = crate::host::checkpoints::lifecycle::apply_session_id_policy(
        &input.session_id,
        crate::host::checkpoints::lifecycle::SessionIdPolicy::Strict,
    )
    .context("turn-start requires non-empty session_id")?;

    if let Some(root) = repo_root {
        let _ = ensure_hook_setup(root);
    }

    // Capture pre-prompt state for use by `stop`.
    let transcript_position = get_transcript_position(&input.transcript_path).unwrap_or_default();
    let prompt_trunc = truncate_prompt_for_storage(&input.prompt);
    let mut pre_prompt = PrePromptState {
        session_id: session_id.clone(),
        timestamp: now_rfc3339(),
        source: String::new(),
        prompt: prompt_trunc,
        transcript_path: input.transcript_path.clone(),
        untracked_files: detect_untracked_files(repo_root),
        transcript_offset: transcript_position.line_count as i64,
        last_transcript_identifier: transcript_position.last_uuid,
        start_message_index: 0,
        step_transcript_start: 0,
        last_transcript_line_count: 0,
        devql_prefetch: None,
    };
    backend.save_pre_prompt(&pre_prompt)?;

    // InitializeSession is best-effort (warn, do not block hook).
    if let Err(err) = strategy.initialize_session(
        &session_id,
        profile.agent_type,
        &input.transcript_path,
        &input.prompt,
    ) {
        eprintln!("[bitloops] Warning: failed to initialize session state: {err}");
    }

    let mut state = backend
        .load_session(&session_id)?
        .unwrap_or_else(|| SessionState {
            session_id: session_id.clone(),
            started_at: now_rfc3339(),
            ..Default::default()
        });

    // Set first_prompt once (collapse whitespace + truncate runes).
    if state.first_prompt.is_empty() {
        state.first_prompt = truncate_prompt_for_storage(&input.prompt);
    }

    apply_session_transition(&mut state, Event::TurnStart);
    state.transcript_path = input.transcript_path.clone();
    state.last_interaction_time = Some(now_rfc3339());
    if state.turn_id.trim().is_empty() {
        state.turn_id = generate_turn_id();
    }
    state.turn_checkpoint_ids.clear();
    if state.agent_type.trim().is_empty() {
        state.agent_type = profile.agent_type.to_string();
    }

    if let Some(root) = repo_root {
        match devql_prefetch::prefetch_for_prompt(root, &session_id, &state.turn_id, &input.prompt)
        {
            Ok(Some(prefetch)) => {
                pre_prompt.devql_prefetch = Some(prefetch);
                if let Err(err) = backend.save_pre_prompt(&pre_prompt) {
                    eprintln!(
                        "[bitloops] Warning: failed to persist pre-hook DevQL prefetch: {err:#}"
                    );
                }
            }
            Ok(None) => {}
            Err(err) => {
                eprintln!("[bitloops] Warning: pre-hook DevQL history prefetch failed: {err:#}");
            }
        }
    }

    backend.save_session(&state)?;
    record_turn_start_interaction(repo_root, &state, &input.prompt, profile);
    Ok(())
}

/// Best-effort hook setup check done at turn start.
///
/// If hooks were overwritten/deleted by
/// third-party tools, reinstall them before the turn proceeds.
fn ensure_hook_setup(repo_root: &Path) -> Result<()> {
    if !claude_hooks::are_hooks_installed(repo_root) {
        let _ = claude_hooks::install_hooks(repo_root, false);
    }
    if !git_hooks::is_git_hook_installed(repo_root) {
        let policy_start = std::env::current_dir().unwrap_or_else(|_| repo_root.to_path_buf());
        let local_dev = settings::load_settings(&policy_start)
            .map(|s| s.local_dev)
            .unwrap_or(false);
        let _ = git_hooks::install_git_hooks(repo_root, local_dev);
    }
    Ok(())
}

fn generate_turn_id() -> String {
    let id = Uuid::new_v4().simple().to_string();
    id[..12].to_string()
}

/// `stop`: call strategy.save_step, transition → Idle, clean up pre-prompt state.
///
pub fn handle_stop(
    input: SessionInfoInput,
    backend: &dyn SessionBackend,
    strategy: &dyn Strategy,
    repo_root: Option<&Path>,
) -> Result<()> {
    handle_stop_with_profile(
        input,
        backend,
        strategy,
        repo_root,
        CLAUDE_HOOK_AGENT_PROFILE,
    )
}

pub fn handle_stop_with_profile(
    input: SessionInfoInput,
    backend: &dyn SessionBackend,
    strategy: &dyn Strategy,
    repo_root: Option<&Path>,
    profile: HookAgentProfile,
) -> Result<()> {
    // stop should remain tolerant when pre-turn/session state is missing.
    let session_id = crate::host::checkpoints::lifecycle::apply_session_id_policy(
        &input.session_id,
        crate::host::checkpoints::lifecycle::SessionIdPolicy::FallbackUnknown,
    )?;
    let mut state = backend.load_session(&session_id)?;

    let pre_prompt = backend.load_pre_prompt(&session_id)?;
    let transcript_start = pre_prompt
        .as_ref()
        .map(|p| p.transcript_offset)
        .unwrap_or(0);
    let pre_untracked = pre_prompt
        .as_ref()
        .map(|s| s.untracked_files.as_slice())
        .unwrap_or(&[]);
    let mut changes = detect_file_changes(repo_root, Some(pre_untracked));

    // Transcript parsing is the primary source for modified files, while
    // git status is merged in as fallback for unrecognized tools/transcript drift.
    let transcript_modified = detect_transcript_modified_files(
        &input.transcript_path,
        &session_id,
        transcript_start,
        repo_root,
    );
    changes.modified = merge_unique(transcript_modified, changes.modified);

    // Exclude files that are already committed to HEAD with identical content.
    changes.modified = filter_to_uncommitted_files(repo_root, changes.modified);

    // If no file changes, skip checkpoint creation but still transition + cleanup.
    let total_changes = changes.modified.len() + changes.new_files.len() + changes.deleted.len();
    let prompt = pre_prompt
        .as_ref()
        .map(|p| p.prompt.as_str())
        .unwrap_or_default();
    let turn_started_at = pre_prompt
        .as_ref()
        .map(|state| state.timestamp.as_str())
        .filter(|value| !value.trim().is_empty());
    let token_usage =
        calculate_stop_token_usage(&input.transcript_path, &session_id, transcript_start);
    let all_files: Vec<String> = changes
        .modified
        .iter()
        .chain(changes.new_files.iter())
        .chain(changes.deleted.iter())
        .cloned()
        .collect();
    record_turn_end_interaction(TurnEndInteraction {
        repo_root,
        session_id: &session_id,
        transcript_path: &input.transcript_path,
        state: state.as_ref(),
        prompt,
        turn_started_at,
        profile,
        files_modified: &all_files,
        token_usage: token_usage.as_ref(),
    });

    if total_changes > 0 {
        let metadata_dir = paths::session_metadata_dir_from_session_id(&session_id);
        let metadata_dir_abs = repo_root
            .map(|r| r.join(&metadata_dir).to_string_lossy().into_owned())
            .unwrap_or_else(|| metadata_dir.clone());

        strategy.save_step(&StepContext {
            session_id: session_id.clone(),
            modified_files: changes.modified,
            new_files: changes.new_files,
            deleted_files: changes.deleted,
            metadata_dir,
            metadata_dir_abs,
            commit_message: generate_commit_message(prompt),
            transcript_path: input.transcript_path.clone(),
            author_name: String::new(),
            author_email: String::new(),
            agent_type: profile.agent_name.to_string(),
            step_transcript_identifier: pre_prompt
                .as_ref()
                .map(|p| p.last_transcript_identifier.clone())
                .unwrap_or_default(),
            step_transcript_start: transcript_start,
            token_usage,
        })?;

        // Strategy may have persisted updated session counters/state; reload to avoid
        // clobbering fields (e.g. checkpoint_count) when saving phase transition.
        state = backend.load_session(&session_id)?;
    }

    // Preserve turn-end behavior even when no pre-existing session state was persisted.
    if state.is_none() && total_changes > 0 {
        state = Some(SessionState {
            session_id: session_id.clone(),
            started_at: now_rfc3339(),
            agent_type: profile.agent_type.to_string(),
            transcript_path: input.transcript_path.clone(),
            ..Default::default()
        });
    }

    if let Some(mut state) = state {
        apply_session_transition(&mut state, Event::TurnEnd);
        state.last_interaction_time = Some(now_rfc3339());
        if state.agent_type.trim().is_empty() {
            state.agent_type = profile.agent_type.to_string();
        }
        if !input.transcript_path.trim().is_empty() {
            state.transcript_path = input.transcript_path.clone();
        }

        // Strategy turn-end handling is best-effort and should not block stop.
        if let Err(err) = strategy.handle_turn_end(&mut state) {
            eprintln!("[bitloops] Warning: turn-end action dispatch failed: {err}");
        }

        backend.save_session(&state)?;
    }

    backend.delete_pre_prompt(&session_id)
}

/// `session-end`: transition → Ended, record ended_at.
///
pub fn handle_session_end(input: SessionInfoInput, backend: &dyn SessionBackend) -> Result<()> {
    handle_session_end_with_profile(input, backend, None, None)
}

pub fn handle_session_end_with_profile(
    input: SessionInfoInput,
    backend: &dyn SessionBackend,
    repo_root: Option<&Path>,
    profile: Option<HookAgentProfile>,
) -> Result<()> {
    let session_id = input.session_id;
    mark_session_ended(&session_id, backend)?;
    let state = backend.load_session(&session_id)?;
    record_session_end_interaction(repo_root, &session_id, &input.transcript_path, state.as_ref(), profile);
    Ok(())
}

pub fn mark_session_ended(session_id: &str, backend: &dyn SessionBackend) -> Result<()> {
    let mut state = match backend.load_session(session_id)? {
        Some(s) => s,
        None => return Ok(()), // no session — no-op
    };

    apply_session_transition(&mut state, Event::SessionStop);
    state.ended_at = Some(now_rfc3339());
    state.last_interaction_time = Some(now_rfc3339());

    backend.save_session(&state)
}

/// `pre-task`: create pre-task marker (signals we're inside a subagent turn).
///
pub fn handle_pre_task(
    input: TaskHookInput,
    backend: &dyn SessionBackend,
    repo_root: Option<&Path>,
) -> Result<()> {
    handle_pre_task_with_profile(input, backend, repo_root, CLAUDE_HOOK_AGENT_PROFILE)
}

pub fn handle_pre_task_with_profile(
    input: TaskHookInput,
    backend: &dyn SessionBackend,
    repo_root: Option<&Path>,
    profile: HookAgentProfile,
) -> Result<()> {
    log_pre_task_hook_context(&mut io::stderr(), &input);

    // Update session state interaction time.
    let mut session_state = backend.load_session(&input.session_id)?;
    if let Some(mut state) = session_state.clone() {
        state.last_interaction_time = Some(now_rfc3339());
        backend.save_session(&state)?;
        session_state = Some(state);
    }

    let marker = PreTaskState {
        tool_use_id: input.tool_use_id,
        session_id: input.session_id,
        timestamp: now_rfc3339(),
        untracked_files: detect_untracked_files(repo_root),
    };
    backend.create_pre_task_marker(&marker)?;
    record_subagent_interaction_event(
        repo_root,
        &marker.session_id,
        session_state.as_ref(),
        profile,
        InteractionEventType::SubagentStart,
        serde_json::json!({
            "tool_use_id": marker.tool_use_id,
        }),
    );
    Ok(())
}

/// `post-task`: call strategy.save_task_step when file changes exist, then delete marker.
///
pub fn handle_post_task(
    input: PostTaskInput,
    backend: &dyn SessionBackend,
    strategy: &dyn Strategy,
    repo_root: Option<&Path>,
) -> Result<()> {
    handle_post_task_with_profile(
        input,
        backend,
        strategy,
        repo_root,
        CLAUDE_HOOK_AGENT_PROFILE,
    )
}

pub fn handle_post_task_with_profile(
    input: PostTaskInput,
    backend: &dyn SessionBackend,
    strategy: &dyn Strategy,
    repo_root: Option<&Path>,
    profile: HookAgentProfile,
) -> Result<()> {
    let subagent_transcript_path = resolve_subagent_transcript_path(
        &input.transcript_path,
        &input.session_id,
        &input.tool_response.agent_id,
    );
    log_post_task_hook_context(&mut io::stderr(), &input, &subagent_transcript_path);

    let pre_untracked = backend
        .load_pre_task_marker(&input.tool_use_id)?
        .map(|s| s.untracked_files)
        .unwrap_or_default();
    let changes = detect_file_changes(repo_root, Some(&pre_untracked));
    let total_changes = changes.modified.len() + changes.new_files.len() + changes.deleted.len();
    let (subagent_type, task_description) =
        parse_subagent_type_and_description(input.tool_input.as_ref());
    let session_state = backend.load_session(&input.session_id)?;
    record_subagent_interaction_event(
        repo_root,
        &input.session_id,
        session_state.as_ref(),
        profile,
        InteractionEventType::SubagentEnd,
        serde_json::json!({
            "subagent_id": input.tool_response.agent_id.clone(),
            "tool_use_id": input.tool_use_id.clone(),
            "subagent_type": subagent_type.clone(),
            "task_description": task_description.clone(),
            "subagent_transcript_path": subagent_transcript_path.clone(),
        }),
    );

    if total_changes > 0 {
        strategy.save_task_step(&TaskStepContext {
            session_id: input.session_id.clone(),
            tool_use_id: input.tool_use_id.clone(),
            agent_id: input.tool_response.agent_id,
            modified_files: changes.modified,
            new_files: changes.new_files,
            deleted_files: changes.deleted,
            transcript_path: input.transcript_path,
            subagent_transcript_path,
            checkpoint_uuid: String::new(),
            author_name: String::new(),
            author_email: String::new(),
            subagent_type,
            task_description,
            agent_type: profile.agent_name.to_string(),
            is_incremental: false,
            incremental_sequence: 0,
            incremental_type: String::new(),
            incremental_data: String::new(),
            todo_content: String::new(),
            commit_message: String::new(),
        })?;
    }

    backend.delete_pre_task_marker(&input.tool_use_id)?;

    if let Some(mut state) = backend.load_session(&input.session_id)? {
        state.last_interaction_time = Some(now_rfc3339());
        backend.save_session(&state)?;
    }

    Ok(())
}

/// `post-todo`: if inside subagent context, create incremental checkpoint.
///
pub fn handle_post_todo(
    input: PostTodoInput,
    backend: &dyn SessionBackend,
    strategy: &dyn Strategy,
    repo_root: Option<&Path>,
) -> Result<()> {
    handle_post_todo_with_profile(
        input,
        backend,
        strategy,
        repo_root,
        CLAUDE_HOOK_AGENT_PROFILE,
    )
}

pub fn handle_post_todo_with_profile(
    input: PostTodoInput,
    backend: &dyn SessionBackend,
    strategy: &dyn Strategy,
    repo_root: Option<&Path>,
    profile: HookAgentProfile,
) -> Result<()> {
    // Only act when inside a subagent context.
    let active_task = backend.find_active_pre_task()?;
    let task_tool_use_id = match active_task {
        Some(id) => id,
        None => return Ok(()), // not in subagent context — no-op
    };

    // Skip on default branch to avoid polluting main/master history.
    let (skip, branch_name) = git::should_skip_on_default_branch();
    if skip {
        eprintln!("Bitloops: skipping incremental checkpoint on branch '{branch_name}'");
        return Ok(());
    }

    let changes = detect_file_changes(repo_root, None);
    let total_changes = changes.modified.len() + changes.new_files.len() + changes.deleted.len();
    if total_changes == 0 {
        return Ok(());
    }

    let mut todo_content = extract_last_completed_todo_from_tool_input(input.tool_input.as_ref());
    if todo_content.is_empty() {
        let todo_count = count_todos_from_tool_input(input.tool_input.as_ref());
        if todo_count > 0 {
            todo_content = format!("Planning: {todo_count} todos");
        }
    }

    strategy.save_task_step(&TaskStepContext {
        session_id: input.session_id.clone(),
        tool_use_id: task_tool_use_id.clone(),
        agent_id: String::new(),
        modified_files: changes.modified,
        new_files: changes.new_files,
        deleted_files: changes.deleted,
        transcript_path: input.transcript_path,
        subagent_transcript_path: String::new(),
        checkpoint_uuid: String::new(),
        author_name: String::new(),
        author_email: String::new(),
        subagent_type: String::new(),
        task_description: String::new(),
        agent_type: profile.agent_name.to_string(),
        is_incremental: true,
        incremental_sequence: next_incremental_sequence(
            repo_root,
            &input.session_id,
            &task_tool_use_id,
        ),
        incremental_type: input.tool_name.clone(),
        incremental_data: input
            .tool_input
            .as_ref()
            .map_or_else(String::new, |v| v.to_string()),
        todo_content,
        commit_message: String::new(),
    })?;

    if let Some(mut state) = backend.load_session(&input.session_id)? {
        state.last_interaction_time = Some(now_rfc3339());
        backend.save_session(&state)?;
    }

    Ok(())
}

mod helpers;
use self::helpers::*;

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "agent_runtime_tests.rs"]
mod tests;

//! Agent hook runtime shared by agent adapters.
//!
//! Shared top-level hook command routing lives in `crate::engine::hooks::dispatcher`.

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

use crate::engine::agent::{
    AGENT_NAME_CLAUDE_CODE, AGENT_NAME_CODEX, AGENT_NAME_CURSOR, AGENT_TYPE_CLAUDE_CODE,
    AGENT_TYPE_CODEX, AGENT_TYPE_CURSOR,
};
use crate::engine::git_operations;
use crate::engine::history::devql_prefetch;
#[cfg(test)]
use crate::engine::logging;
use crate::engine::paths;
use crate::engine::settings;
use crate::engine::stringutil;
use crate::engine::transcript::commit_message;
use crate::engine::transcript::utils::get_transcript_position;

use crate::engine::agent::claude_code::git_hooks;
use crate::engine::agent::claude_code::hooks as claude_hooks;
use crate::engine::agent::claude_code::transcript as claude_transcript;
use crate::engine::session::backend::SessionBackend;
use crate::engine::session::phase::{
    Event, NoOpActionHandler, TransitionContext, apply_transition, transition_with_context,
};
use crate::engine::session::state::{PrePromptState, PreTaskState, SessionState};
use crate::engine::strategy::noop::NoOpStrategy;
use crate::engine::strategy::{StepContext, Strategy, TaskStepContext};

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
#[allow(dead_code)] // fields reserved for future transcript handling
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
#[allow(dead_code)] // fields reserved for future transcript handling
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
#[allow(dead_code)] // fields reserved for future transcript handling
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

// ── Handler functions ─────────────────────────────────────────────────────────

/// `session-start`: create or reset session state, transition → Idle.
///
pub fn handle_session_start(
    input: SessionInfoInput,
    backend: &dyn SessionBackend,
    repo_root: Option<&std::path::Path>,
) -> Result<()> {
    let session_id = crate::engine::lifecycle::apply_session_id_policy(
        &input.session_id,
        crate::engine::lifecycle::SessionIdPolicy::Strict,
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
    state.last_interaction_time = Some(now_rfc3339());

    // Detect and record worktree information for shadow branch naming.
    if let Some(root) = repo_root {
        state.worktree_path = root.to_string_lossy().into_owned();
        state.worktree_id = paths::get_worktree_id(root)
            .with_context(|| format!("failed to resolve worktree id for {}", root.display()))?;
    }

    backend.save_session(&state)
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
    let session_id = crate::engine::lifecycle::apply_session_id_policy(
        &input.session_id,
        crate::engine::lifecycle::SessionIdPolicy::Strict,
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

    backend.save_session(&state)
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
        let local_dev = settings::load_settings(repo_root)
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
    let session_id = crate::engine::lifecycle::apply_session_id_policy(
        &input.session_id,
        crate::engine::lifecycle::SessionIdPolicy::FallbackUnknown,
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
    if total_changes > 0 {
        let metadata_dir = paths::session_metadata_dir_from_session_id(&session_id);
        let metadata_dir_abs = repo_root
            .map(|r| r.join(&metadata_dir).to_string_lossy().into_owned())
            .unwrap_or_else(|| metadata_dir.clone());
        let prompt = pre_prompt
            .as_ref()
            .map(|p| p.prompt.as_str())
            .unwrap_or_default();
        let token_usage =
            calculate_stop_token_usage(&input.transcript_path, &session_id, transcript_start);

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
    mark_session_ended(&input.session_id, backend)
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
    log_pre_task_hook_context(&mut io::stderr(), &input);

    // Update session state interaction time.
    if let Some(mut state) = backend.load_session(&input.session_id)? {
        state.last_interaction_time = Some(now_rfc3339());
        backend.save_session(&state)?;
    }

    let marker = PreTaskState {
        tool_use_id: input.tool_use_id,
        session_id: input.session_id,
        timestamp: now_rfc3339(),
        untracked_files: detect_untracked_files(repo_root),
    };
    backend.create_pre_task_marker(&marker)
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

    if total_changes > 0 {
        let (subagent_type, task_description) =
            parse_subagent_type_and_description(input.tool_input.as_ref());
        let subagent_transcript_path = resolve_subagent_transcript_path(
            &input.transcript_path,
            &input.session_id,
            &input.tool_response.agent_id,
        );
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
    let (skip, branch_name) = git_operations::should_skip_on_default_branch();
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

include!("agent_runtime/helpers.rs");

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "agent_runtime_tests.rs"]
mod tests;

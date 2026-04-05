use std::io;
use std::path::Path;

use anyhow::{Context, Result};

use super::helpers::*;
use super::interactions::{
    TurnEndInteraction, record_session_end_interaction, record_session_start_interaction,
    record_subagent_interaction_event, record_turn_end_interaction, record_turn_start_interaction,
};
use super::types::*;
use super::{
    CLAUDE_HOOK_AGENT_PROFILE, HookAgentProfile, apply_session_transition, generate_turn_id,
};
use crate::adapters::agents::claude_code::git_hooks;
use crate::adapters::agents::claude_code::hooks as claude_hooks;
use crate::config::settings;
use crate::git;
use crate::host::checkpoints::history::devql_prefetch;
use crate::host::checkpoints::session::backend::SessionBackend;
use crate::host::checkpoints::session::phase::Event;
use crate::host::checkpoints::session::state::{PrePromptState, PreTaskState, SessionState};
use crate::host::checkpoints::strategy::noop::NoOpStrategy;
use crate::host::checkpoints::strategy::{StepContext, Strategy, TaskStepContext};
use crate::host::checkpoints::transcript::utils::get_transcript_position;
use crate::host::interactions::types::InteractionEventType;
use crate::utils::paths;

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
        transcript_offset_start: transcript_start,
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
    record_session_end_interaction(
        repo_root,
        &session_id,
        &input.transcript_path,
        state.as_ref(),
        profile,
    );
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

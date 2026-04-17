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
use crate::config::settings;
use crate::git;
use crate::host::checkpoints::history::devql_prefetch;
use crate::host::checkpoints::session::backend::SessionBackend;
use crate::host::checkpoints::session::phase::Event;
use crate::host::checkpoints::session::state::{PrePromptState, PreTaskState, SessionState};
use crate::host::checkpoints::strategy::noop::NoOpStrategy;
use crate::host::checkpoints::strategy::{StepContext, Strategy, TaskStepContext};
use crate::host::checkpoints::transcript::metadata::{
    TaskCheckpointMetadataBundle, build_incremental_checkpoint_payload,
    build_session_metadata_bundle, build_task_checkpoint_payload,
    extract_prompts_from_transcript_bytes,
};
use crate::host::checkpoints::transcript::utils::get_transcript_position;
use crate::host::interactions::types::InteractionEventType;
use crate::host::runtime_store::{
    RepoSqliteRuntimeStore, RuntimeMetadataBlobType, SessionMetadataSnapshot,
    TaskCheckpointArtefact,
};
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
    handle_session_start_with_profile_and_model(input, backend, repo_root, profile, "")
}

pub fn handle_session_start_with_profile_and_model(
    input: SessionInfoInput,
    backend: &dyn SessionBackend,
    repo_root: Option<&std::path::Path>,
    profile: Option<HookAgentProfile>,
    model_hint: &str,
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
    record_session_start_interaction(repo_root, &state, profile, model_hint);
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
    handle_user_prompt_submit_with_strategy_and_profile_and_model(
        input, backend, strategy, repo_root, profile, "",
    )
}

pub fn handle_user_prompt_submit_with_strategy_and_profile_and_model(
    input: UserPromptSubmitInput,
    backend: &dyn SessionBackend,
    strategy: &dyn Strategy,
    repo_root: Option<&Path>,
    profile: HookAgentProfile,
    model_hint: &str,
) -> Result<()> {
    let session_id = crate::host::checkpoints::lifecycle::apply_session_id_policy(
        &input.session_id,
        crate::host::checkpoints::lifecycle::SessionIdPolicy::Strict,
    )
    .context("turn-start requires non-empty session_id")?;

    if let Some(root) = repo_root {
        let _ = ensure_hook_setup(root, profile.agent_type);
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
    record_turn_start_interaction(repo_root, &state, &input.prompt, profile, model_hint);
    Ok(())
}

/// Best-effort hook setup check done at turn start.
///
/// If hooks were overwritten/deleted by
/// third-party tools, reinstall them before the turn proceeds.
fn ensure_hook_setup(repo_root: &Path, agent_name: &str) -> Result<()> {
    let registry = crate::adapters::agents::AgentAdapterRegistry::builtin();
    let policy_start = std::env::current_dir().unwrap_or_else(|_| repo_root.to_path_buf());
    let local_dev = settings::load_settings(&policy_start)
        .map(|s| s.local_dev)
        .unwrap_or(false);

    if !registry
        .are_agent_hooks_installed(repo_root, agent_name)
        .unwrap_or(false)
    {
        let _ = registry.install_agent_hooks(
            repo_root,
            agent_name,
            local_dev,
            false,
            crate::adapters::agents::AgentHookInstallOptions::default(),
        );
    }
    if !git_hooks::is_git_hook_installed(repo_root) {
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
    handle_stop_with_profile_and_model(input, backend, strategy, repo_root, profile, "")
}

pub fn handle_stop_with_profile_and_model(
    input: SessionInfoInput,
    backend: &dyn SessionBackend,
    strategy: &dyn Strategy,
    repo_root: Option<&Path>,
    profile: HookAgentProfile,
    model_hint: &str,
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
    let metadata = std::fs::read(&input.transcript_path)
        .ok()
        .filter(|transcript| !transcript.is_empty())
        .and_then(|transcript| {
            let derived_prompts = extract_prompts_from_transcript_bytes(&transcript);
            let last_prompt = derived_prompts.last().cloned().unwrap_or_default();
            let commit_message = generate_commit_message(&last_prompt);
            build_session_metadata_bundle(&session_id, &commit_message, &transcript).ok()
        });
    if let (Some(root), Some(metadata)) = (repo_root, metadata.as_ref()) {
        let runtime_store = RepoSqliteRuntimeStore::open(root)
            .context("opening runtime store for stop metadata snapshot")?;
        let mut snapshot = SessionMetadataSnapshot::new(session_id.clone(), metadata.clone());
        snapshot.turn_id = state
            .as_ref()
            .map(|value| value.turn_id.clone())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(generate_turn_id);
        snapshot.transcript_identifier = pre_prompt
            .as_ref()
            .map(|value| value.last_transcript_identifier.clone())
            .unwrap_or_default();
        snapshot.transcript_path = input.transcript_path.clone();
        runtime_store
            .save_session_metadata_snapshot(&snapshot)
            .context("saving stop metadata snapshot")?;
    }
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
        model_hint,
        files_modified: &all_files,
        token_usage: token_usage.as_ref(),
    });

    if total_changes > 0 {
        let commit_message = metadata
            .as_ref()
            .and_then(|bundle| bundle.prompts.last().cloned())
            .map(|prompt| generate_commit_message(&prompt))
            .unwrap_or_else(|| generate_commit_message(prompt));

        strategy.save_step(&StepContext {
            session_id: session_id.clone(),
            modified_files: changes.modified,
            new_files: changes.new_files,
            deleted_files: changes.deleted,
            metadata: metadata.clone(),
            commit_message,
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
    handle_session_end_with_profile_and_model(input, backend, repo_root, profile, "")
}

pub fn handle_session_end_with_profile_and_model(
    input: SessionInfoInput,
    backend: &dyn SessionBackend,
    repo_root: Option<&Path>,
    profile: Option<HookAgentProfile>,
    model_hint: &str,
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
        model_hint,
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
    handle_pre_task_with_profile_and_model(input, backend, repo_root, profile, "")
}

pub fn handle_pre_task_with_profile_and_model(
    input: TaskHookInput,
    backend: &dyn SessionBackend,
    repo_root: Option<&Path>,
    profile: HookAgentProfile,
    model_hint: &str,
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
        model_hint,
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
    handle_post_task_with_profile_and_model(input, backend, strategy, repo_root, profile, "")
}

pub fn handle_post_task_with_profile_and_model(
    input: PostTaskInput,
    backend: &dyn SessionBackend,
    strategy: &dyn Strategy,
    repo_root: Option<&Path>,
    profile: HookAgentProfile,
    model_hint: &str,
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
        model_hint,
        serde_json::json!({
            "subagent_id": input.tool_response.agent_id.clone(),
            "tool_use_id": input.tool_use_id.clone(),
            "subagent_type": subagent_type.clone(),
            "task_description": task_description.clone(),
            "subagent_transcript_path": subagent_transcript_path.clone(),
        }),
    );

    if total_changes > 0 {
        let session_metadata = if let Some(root) = repo_root {
            std::fs::read(&input.transcript_path)
                .ok()
                .and_then(|transcript| {
                    let prompts = extract_prompts_from_transcript_bytes(&transcript);
                    let commit_message = generate_commit_message(
                        prompts.last().map(String::as_str).unwrap_or_default(),
                    );
                    let metadata = build_session_metadata_bundle(
                        &input.session_id,
                        &commit_message,
                        &transcript,
                    )
                    .ok()?;
                    let runtime_store = RepoSqliteRuntimeStore::open(root).ok()?;
                    let mut snapshot =
                        SessionMetadataSnapshot::new(input.session_id.clone(), metadata.clone());
                    snapshot.transcript_path = input.transcript_path.clone();
                    snapshot.transcript_identifier = input.session_id.clone();
                    runtime_store
                        .save_session_metadata_snapshot(&snapshot)
                        .ok()?;
                    Some(metadata)
                })
        } else {
            None
        };
        let checkpoint_json = build_task_checkpoint_payload(
            &input.session_id,
            &input.tool_use_id,
            "",
            &input.tool_response.agent_id,
        )?;
        let subagent_transcript = if subagent_transcript_path.trim().is_empty() {
            None
        } else {
            std::fs::read(&subagent_transcript_path).ok()
        };
        if let Some(root) = repo_root {
            let runtime_store = RepoSqliteRuntimeStore::open(root)
                .context("opening runtime store for task checkpoint artefacts")?;
            let mut checkpoint_artefact = TaskCheckpointArtefact::new(
                input.session_id.clone(),
                input.tool_use_id.clone(),
                RuntimeMetadataBlobType::TaskCheckpoint,
                checkpoint_json.clone(),
            );
            checkpoint_artefact.agent_id = input.tool_response.agent_id.clone();
            runtime_store.save_task_checkpoint_artefact(&checkpoint_artefact)?;
            if let Some(payload) = subagent_transcript.as_ref() {
                let mut transcript_artefact = TaskCheckpointArtefact::new(
                    input.session_id.clone(),
                    input.tool_use_id.clone(),
                    RuntimeMetadataBlobType::SubagentTranscript,
                    payload.clone(),
                );
                transcript_artefact.agent_id = input.tool_response.agent_id.clone();
                runtime_store.save_task_checkpoint_artefact(&transcript_artefact)?;
            }
        }

        strategy.save_task_step(&TaskStepContext {
            session_id: input.session_id.clone(),
            tool_use_id: input.tool_use_id.clone(),
            agent_id: input.tool_response.agent_id,
            modified_files: changes.modified,
            new_files: changes.new_files,
            deleted_files: changes.deleted,
            session_metadata,
            task_metadata: Some(TaskCheckpointMetadataBundle {
                checkpoint_json: Some(checkpoint_json),
                subagent_transcript,
                incremental_checkpoint: None,
                prompt: None,
            }),
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

    let incremental_sequence = match repo_root {
        Some(root) => RepoSqliteRuntimeStore::open(root)
            .and_then(|store| {
                store.next_task_incremental_sequence(&input.session_id, &task_tool_use_id)
            })
            .unwrap_or_else(|_| {
                next_incremental_sequence(repo_root, &input.session_id, &task_tool_use_id)
            }),
        None => next_incremental_sequence(repo_root, &input.session_id, &task_tool_use_id),
    };
    let session_metadata = if let Some(root) = repo_root {
        std::fs::read(&input.transcript_path)
            .ok()
            .and_then(|transcript| {
                let prompts = extract_prompts_from_transcript_bytes(&transcript);
                let commit_message =
                    generate_commit_message(prompts.last().map(String::as_str).unwrap_or_default());
                let metadata =
                    build_session_metadata_bundle(&input.session_id, &commit_message, &transcript)
                        .ok()?;
                let runtime_store = RepoSqliteRuntimeStore::open(root).ok()?;
                let mut snapshot =
                    SessionMetadataSnapshot::new(input.session_id.clone(), metadata.clone());
                snapshot.transcript_path = input.transcript_path.clone();
                snapshot.transcript_identifier = input.session_id.clone();
                runtime_store
                    .save_session_metadata_snapshot(&snapshot)
                    .ok()?;
                Some(metadata)
            })
    } else {
        None
    };
    let incremental_payload = build_incremental_checkpoint_payload(
        &task_tool_use_id,
        &input.tool_name,
        &now_rfc3339(),
        input
            .tool_input
            .as_ref()
            .unwrap_or(&serde_json::Value::Null),
    )?;
    if let Some(root) = repo_root {
        let runtime_store = RepoSqliteRuntimeStore::open(root)
            .context("opening runtime store for incremental checkpoint artefacts")?;
        let mut artefact = TaskCheckpointArtefact::new(
            input.session_id.clone(),
            task_tool_use_id.clone(),
            RuntimeMetadataBlobType::IncrementalCheckpoint,
            incremental_payload.clone(),
        );
        artefact.incremental_sequence = Some(incremental_sequence);
        artefact.incremental_type = input.tool_name.clone();
        artefact.is_incremental = true;
        runtime_store.save_task_checkpoint_artefact(&artefact)?;
    }

    strategy.save_task_step(&TaskStepContext {
        session_id: input.session_id.clone(),
        tool_use_id: task_tool_use_id.clone(),
        agent_id: String::new(),
        modified_files: changes.modified,
        new_files: changes.new_files,
        deleted_files: changes.deleted,
        session_metadata,
        task_metadata: Some(TaskCheckpointMetadataBundle {
            checkpoint_json: None,
            subagent_transcript: None,
            incremental_checkpoint: Some(incremental_payload),
            prompt: None,
        }),
        transcript_path: input.transcript_path,
        subagent_transcript_path: String::new(),
        checkpoint_uuid: String::new(),
        author_name: String::new(),
        author_email: String::new(),
        subagent_type: String::new(),
        task_description: String::new(),
        agent_type: profile.agent_name.to_string(),
        is_incremental: true,
        incremental_sequence,
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

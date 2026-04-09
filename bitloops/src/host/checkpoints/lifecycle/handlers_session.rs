use anyhow::{Result, anyhow};

use super::adapter::LifecycleAgentAdapter;
use super::canonical::build_phase3_canonical_request;
use super::git_workspace::collect_untracked_files_for_lifecycle;
use super::interaction::{flush_interaction_spool_best_effort, resolve_interaction_spool};
use super::time_and_ids::{
    generate_interaction_event_id, generate_lifecycle_turn_id, now_rfc3339,
    truncate_prompt_for_storage,
};
use super::types::{LifecycleEvent, SessionIdPolicy, apply_session_id_policy};
use crate::host::checkpoints::session::create_session_backend_or_local;
use crate::host::checkpoints::session::phase::{
    Event as SessionEvent, NoOpActionHandler as SessionNoOpActionHandler,
    TransitionContext as SessionTransitionContext, apply_transition as apply_session_transition,
    transition_with_context as transition_session_with_context,
};
use crate::host::checkpoints::session::state::PRE_PROMPT_SOURCE_CURSOR_SHELL;
use crate::host::interactions::model::resolve_interaction_model;
use crate::host::interactions::store::InteractionSpool;
use crate::host::interactions::types::{
    InteractionEvent, InteractionEventType, InteractionSession, InteractionTurn,
};

fn ensure_hook_setup(repo_root: &std::path::Path, agent_name: &str) -> Result<()> {
    let registry = crate::adapters::agents::AgentAdapterRegistry::builtin();
    let policy_start = std::env::current_dir().unwrap_or_else(|_| repo_root.to_path_buf());
    let local_dev = crate::config::settings::load_settings(&policy_start)
        .map(|s| s.local_dev)
        .unwrap_or(false);

    if !registry
        .are_agent_hooks_installed(repo_root, agent_name)
        .unwrap_or(false)
    {
        let _ = registry.install_agent_hooks(repo_root, agent_name, local_dev, false);
    }
    if !crate::adapters::agents::claude_code::git_hooks::is_git_hook_installed(repo_root) {
        let _ = crate::adapters::agents::claude_code::git_hooks::install_git_hooks(
            repo_root, local_dev,
        );
    }
    Ok(())
}

pub fn handle_lifecycle_session_start(
    agent: &dyn LifecycleAgentAdapter,
    event: &LifecycleEvent,
) -> Result<()> {
    let canonical_request = build_phase3_canonical_request(agent.agent_name(), event)?;
    let session_id = apply_session_id_policy(
        &canonical_request.session.session_id,
        SessionIdPolicy::Strict,
    )
    .map_err(|_| anyhow!("no session_id in SessionStart event"))?;
    let repo_root = crate::utils::paths::repo_root()?;
    let backend = create_session_backend_or_local(&repo_root);

    let mut state = backend.load_session(&session_id)?.unwrap_or_else(|| {
        crate::host::checkpoints::session::state::SessionState {
            session_id: session_id.clone(),
            ..Default::default()
        }
    });

    let transition = transition_session_with_context(
        state.phase,
        SessionEvent::SessionStart,
        SessionTransitionContext::default(),
    );
    apply_session_transition(&mut state, transition, &mut SessionNoOpActionHandler)?;
    let now = now_rfc3339();
    if state.started_at.trim().is_empty() {
        state.started_at = now.clone();
    }
    if let Some(session_ref) = canonical_request.session.session_ref.as_ref() {
        state.transcript_path = session_ref.clone();
    }
    state.last_interaction_time = Some(now.clone());
    state.worktree_path = repo_root.to_string_lossy().into_owned();
    state.worktree_id = crate::utils::paths::get_worktree_id(&repo_root)?;
    if state.agent_type.trim().is_empty() {
        state.agent_type = canonical_request.agent.agent_key.clone();
    }
    if state.first_prompt.is_empty()
        && let Some(prompt) = canonical_request.prompt.as_ref()
    {
        state.first_prompt = truncate_prompt_for_storage(prompt);
    }

    backend.save_session(&state)?;

    if let Some(spool) = resolve_interaction_spool(&repo_root) {
        let model = resolve_interaction_model(&event.model, &state.transcript_path);
        let session = InteractionSession {
            session_id: session_id.clone(),
            repo_id: spool.repo_id().to_string(),
            agent_type: state.agent_type.clone(),
            model: model.clone(),
            first_prompt: state.first_prompt.clone(),
            transcript_path: state.transcript_path.clone(),
            worktree_path: state.worktree_path.clone(),
            worktree_id: state.worktree_id.clone(),
            started_at: state.started_at.clone(),
            ended_at: None,
            last_event_at: now.clone(),
            updated_at: now.clone(),
        };
        if let Err(err) = spool.record_session(&session) {
            eprintln!("[bitloops] Warning: failed to spool interaction session: {err}");
        }
        if let Err(err) = spool.record_event(&InteractionEvent {
            event_id: generate_interaction_event_id(),
            session_id: session_id.clone(),
            turn_id: None,
            repo_id: spool.repo_id().to_string(),
            event_type: InteractionEventType::SessionStart,
            event_time: now.clone(),
            agent_type: state.agent_type.clone(),
            model,
            payload: serde_json::json!({
                "first_prompt": state.first_prompt,
                "transcript_path": state.transcript_path,
                "worktree_path": state.worktree_path,
                "worktree_id": state.worktree_id,
            }),
        }) {
            eprintln!("[bitloops] Warning: failed to spool session_start event: {err}");
        }
    }
    flush_interaction_spool_best_effort(&repo_root);

    Ok(())
}

pub fn handle_lifecycle_turn_start(
    agent: &dyn LifecycleAgentAdapter,
    event: &LifecycleEvent,
) -> Result<()> {
    let canonical_request = build_phase3_canonical_request(agent.agent_name(), event)?;
    let session_id = apply_session_id_policy(
        &canonical_request.session.session_id,
        SessionIdPolicy::Strict,
    )
    .map_err(|_| anyhow!("no session_id in TurnStart event"))?;
    let repo_root = crate::utils::paths::repo_root()?;
    let backend = create_session_backend_or_local(&repo_root);

    if event.source == PRE_PROMPT_SOURCE_CURSOR_SHELL
        && backend.load_pre_prompt(&session_id)?.is_some()
    {
        return Ok(());
    }

    let _ = ensure_hook_setup(&repo_root, agent.agent_name());

    let transcript_offset = agent
        .as_transcript_analyzer()
        .and_then(|analyzer| analyzer.get_transcript_position(&event.session_ref).ok())
        .unwrap_or(0);

    let pre_prompt = crate::host::checkpoints::session::state::PrePromptState {
        session_id: session_id.clone(),
        timestamp: now_rfc3339(),
        source: event.source.clone(),
        prompt: truncate_prompt_for_storage(
            canonical_request.prompt.as_deref().unwrap_or(&event.prompt),
        ),
        transcript_path: canonical_request
            .session
            .session_ref
            .clone()
            .unwrap_or_else(|| event.session_ref.clone()),
        untracked_files: collect_untracked_files_for_lifecycle(&repo_root),
        transcript_offset: transcript_offset as i64,
        ..crate::host::checkpoints::session::state::PrePromptState::default()
    };
    backend.save_pre_prompt(&pre_prompt)?;

    let strategy = super::resolve_configured_strategy(&repo_root)?;
    if let Err(err) = strategy.initialize_session(
        &session_id,
        agent.agent_name(),
        &event.session_ref,
        &event.prompt,
    ) {
        eprintln!("[bitloops] Warning: failed to initialize session state: {err}");
    }

    let mut state = backend.load_session(&session_id)?.unwrap_or_else(|| {
        crate::host::checkpoints::session::state::SessionState {
            session_id: session_id.clone(),
            started_at: now_rfc3339(),
            ..Default::default()
        }
    });
    let should_replace_bootstrap_prompt = state.pending.step_count == 0
        && state.turn_id.trim().is_empty()
        && state.phase == crate::host::checkpoints::session::phase::SessionPhase::Idle;
    if state.first_prompt.is_empty() || should_replace_bootstrap_prompt {
        state.first_prompt = truncate_prompt_for_storage(
            canonical_request.prompt.as_deref().unwrap_or(&event.prompt),
        );
    }

    let transition = transition_session_with_context(
        state.phase,
        SessionEvent::TurnStart,
        SessionTransitionContext::default(),
    );
    apply_session_transition(&mut state, transition, &mut SessionNoOpActionHandler)?;
    state.transcript_path = canonical_request
        .session
        .session_ref
        .clone()
        .unwrap_or_else(|| event.session_ref.clone());
    let now = now_rfc3339();
    if state.started_at.trim().is_empty() {
        state.started_at = now.clone();
    }
    state.last_interaction_time = Some(now.clone());
    if state.turn_id.trim().is_empty() {
        state.turn_id = generate_lifecycle_turn_id();
    }
    state.turn_checkpoint_ids.clear();
    if state.agent_type.trim().is_empty() {
        state.agent_type = canonical_request.agent.agent_key.clone();
    }

    backend.save_session(&state)?;

    let prompt_text =
        truncate_prompt_for_storage(canonical_request.prompt.as_deref().unwrap_or(&event.prompt));
    let turn_number = state.pending.step_count + 1;
    if let Some(spool) = resolve_interaction_spool(&repo_root) {
        let model = resolve_interaction_model(&event.model, &state.transcript_path);
        let session = InteractionSession {
            session_id: session_id.clone(),
            repo_id: spool.repo_id().to_string(),
            agent_type: state.agent_type.clone(),
            model: model.clone(),
            first_prompt: state.first_prompt.clone(),
            transcript_path: state.transcript_path.clone(),
            worktree_path: state.worktree_path.clone(),
            worktree_id: state.worktree_id.clone(),
            started_at: state.started_at.clone(),
            ended_at: state.ended_at.clone(),
            last_event_at: now.clone(),
            updated_at: now.clone(),
        };
        if let Err(err) = spool.record_session(&session) {
            eprintln!("[bitloops] Warning: failed to spool interaction session: {err}");
        }
        let turn = InteractionTurn {
            turn_id: state.turn_id.clone(),
            session_id: session_id.clone(),
            repo_id: spool.repo_id().to_string(),
            turn_number,
            prompt: prompt_text.clone(),
            agent_type: state.agent_type.clone(),
            model: model.clone(),
            started_at: now.clone(),
            prompt_count: 1,
            updated_at: now.clone(),
            ..Default::default()
        };
        if let Err(err) = spool.record_turn(&turn) {
            eprintln!("[bitloops] Warning: failed to spool interaction turn start: {err}");
        }
        if let Err(err) = spool.record_event(&InteractionEvent {
            event_id: generate_interaction_event_id(),
            session_id: session_id.clone(),
            turn_id: Some(state.turn_id.clone()),
            repo_id: spool.repo_id().to_string(),
            event_type: InteractionEventType::TurnStart,
            event_time: now.clone(),
            agent_type: state.agent_type.clone(),
            model,
            payload: serde_json::json!({
                "prompt": prompt_text,
                "turn_number": turn_number,
            }),
        }) {
            eprintln!("[bitloops] Warning: failed to spool turn_start event: {err}");
        }
    }
    flush_interaction_spool_best_effort(&repo_root);

    Ok(())
}

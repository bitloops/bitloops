use anyhow::{Result, anyhow};

use super::adapter::LifecycleAgentAdapter;
use super::canonical::build_phase3_canonical_request;
use super::git_workspace::collect_untracked_files_for_lifecycle;
use super::interaction::resolve_interaction_event_store;
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
use crate::host::interactions::store::InteractionEventStore;
use crate::host::interactions::types::{
    InteractionEvent, InteractionEventType, InteractionSession, InteractionTurn,
};

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
    if let Some(session_ref) = canonical_request.session.session_ref.as_ref() {
        state.transcript_path = session_ref.clone();
    }
    state.last_interaction_time = Some(now_rfc3339());
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

    // ── interaction event persistence ────────────────────────────────────────
    if let Some(store) = resolve_interaction_event_store(&repo_root) {
        let now = now_rfc3339();
        let is = InteractionSession {
            session_id: session_id.clone(),
            repo_id: store.repo_id().to_string(),
            agent_type: state.agent_type.clone(),
            model: event.model.clone(),
            first_prompt: state.first_prompt.clone(),
            transcript_path: state.transcript_path.clone(),
            worktree_path: state.worktree_path.clone(),
            worktree_id: state.worktree_id.clone(),
            started_at: now.clone(),
            ended_at: None,
        };
        if let Err(err) = store.record_session(&is) {
            eprintln!("[bitloops] Warning: failed to record interaction session: {err}");
        }
        if let Err(err) = store.record_event(&InteractionEvent {
            event_id: generate_interaction_event_id(),
            session_id: session_id.clone(),
            turn_id: None,
            repo_id: store.repo_id().to_string(),
            event_type: InteractionEventType::SessionStart,
            event_time: now,
            agent_type: state.agent_type.clone(),
            model: event.model.clone(),
            payload: serde_json::Value::Object(Default::default()),
        }) {
            eprintln!("[bitloops] Warning: failed to record session_start event: {err}");
        }
    }

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

    let transcript_offset = agent
        .as_transcript_analyzer()
        .and_then(|analyzer| analyzer.get_transcript_position(&event.session_ref).ok())
        .unwrap_or(0);

    let pre_prompt = crate::host::checkpoints::session::state::PrePromptState {
        session_id: session_id.clone(),
        timestamp: now_rfc3339(),
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

    let registry = crate::host::checkpoints::strategy::registry::StrategyRegistry::builtin();
    let strategy = registry.get(
        crate::host::checkpoints::strategy::registry::STRATEGY_NAME_MANUAL_COMMIT,
        &repo_root,
    )?;
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
    let should_replace_bootstrap_prompt = state.step_count == 0
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
    state.last_interaction_time = Some(now_rfc3339());
    if state.turn_id.trim().is_empty() {
        state.turn_id = generate_lifecycle_turn_id();
    }
    state.turn_checkpoint_ids.clear();
    if state.agent_type.trim().is_empty() {
        state.agent_type = canonical_request.agent.agent_key.clone();
    }

    backend.save_session(&state)?;

    // ── interaction event persistence ────────────────────────────────────────
    if let Some(store) = resolve_interaction_event_store(&repo_root) {
        let now = now_rfc3339();
        let prompt_text = truncate_prompt_for_storage(
            canonical_request.prompt.as_deref().unwrap_or(&event.prompt),
        );
        let turn = InteractionTurn {
            turn_id: state.turn_id.clone(),
            session_id: session_id.clone(),
            repo_id: store.repo_id().to_string(),
            turn_number: state.step_count + 1,
            prompt: prompt_text,
            agent_type: state.agent_type.clone(),
            model: event.model.clone(),
            started_at: now.clone(),
            ..Default::default()
        };
        if let Err(err) = store.record_turn_start(&turn) {
            eprintln!("[bitloops] Warning: failed to record interaction turn start: {err}");
        }
        if let Err(err) = store.record_event(&InteractionEvent {
            event_id: generate_interaction_event_id(),
            session_id: session_id.clone(),
            turn_id: Some(state.turn_id.clone()),
            repo_id: store.repo_id().to_string(),
            event_type: InteractionEventType::TurnStart,
            event_time: now,
            agent_type: state.agent_type.clone(),
            model: event.model.clone(),
            payload: serde_json::Value::Object(Default::default()),
        }) {
            eprintln!("[bitloops] Warning: failed to record turn_start event: {err}");
        }
    }

    Ok(())
}

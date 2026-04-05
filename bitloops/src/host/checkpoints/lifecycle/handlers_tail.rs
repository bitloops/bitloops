use anyhow::Result;

use super::adapter::LifecycleAgentAdapter;
use super::interaction::{flush_interaction_spool_best_effort, resolve_interaction_spool};
use super::time_and_ids::{generate_interaction_event_id, now_rfc3339};
use super::types::{LifecycleEvent, SessionIdPolicy, apply_session_id_policy};
use crate::host::checkpoints::session::create_session_backend_or_local;
use crate::host::checkpoints::session::phase::{
    Event as SessionEvent, NoOpActionHandler as SessionNoOpActionHandler,
    TransitionContext as SessionTransitionContext, apply_transition as apply_session_transition,
    transition_with_context as transition_session_with_context,
};
use crate::host::interactions::store::InteractionSpool;
use crate::host::interactions::types::{
    InteractionEvent, InteractionEventType, InteractionSession,
};

pub fn handle_lifecycle_compaction(
    _agent: &dyn LifecycleAgentAdapter,
    event: &LifecycleEvent,
) -> Result<()> {
    let session_id = apply_session_id_policy(&event.session_id, SessionIdPolicy::PreserveEmpty)?;
    if session_id.is_empty() {
        eprintln!("Context compaction: transcript offset reset");
        return Ok(());
    }

    let repo_root = match crate::utils::paths::repo_root() {
        Ok(root) => root,
        Err(err) => {
            eprintln!(
                "[bitloops] Warning: failed to resolve repository root for compaction: {err}"
            );
            eprintln!("Context compaction: transcript offset reset");
            return Ok(());
        }
    };

    let backend = create_session_backend_or_local(&repo_root);
    match backend.load_session(&session_id) {
        Ok(Some(mut state)) => {
            let context = SessionTransitionContext {
                has_files_touched: !state.pending.files_touched.is_empty(),
                is_rebase_in_progress: false,
            };
            let transition =
                transition_session_with_context(state.phase, SessionEvent::Compaction, context);
            let mut handler = SessionNoOpActionHandler;
            if let Err(err) = apply_session_transition(&mut state, transition, &mut handler) {
                eprintln!("[bitloops] Warning: compaction transition failed: {err}");
            }

            // Compaction resets transcript offset after transition.
            state.pending.checkpoint_transcript_start = 0;
            if let Err(err) = backend.save_session(&state) {
                eprintln!(
                    "[bitloops] Warning: failed to save session state after compaction: {err}"
                );
            }
        }
        Ok(None) => {}
        Err(err) => {
            eprintln!("[bitloops] Warning: failed to load session state for compaction: {err}");
        }
    }

    // ── interaction event persistence ────────────────────────────────────────
    if let Some(spool) = resolve_interaction_spool(&repo_root)
        && let Err(err) = spool.record_event(&InteractionEvent {
            event_id: generate_interaction_event_id(),
            session_id: session_id.clone(),
            turn_id: None,
            repo_id: spool.repo_id().to_string(),
            event_type: InteractionEventType::Compaction,
            event_time: now_rfc3339(),
            agent_type: String::new(),
            model: event.model.clone(),
            payload: serde_json::Value::Object(Default::default()),
        })
    {
        eprintln!("[bitloops] Warning: failed to spool compaction event: {err}");
    }
    flush_interaction_spool_best_effort(&repo_root);

    eprintln!("Context compaction: transcript offset reset");
    Ok(())
}

pub fn handle_lifecycle_session_end(
    _agent: &dyn LifecycleAgentAdapter,
    event: &LifecycleEvent,
) -> Result<()> {
    let session_id = apply_session_id_policy(&event.session_id, SessionIdPolicy::PreserveEmpty)?;
    if session_id.is_empty() {
        return Ok(());
    }

    let repo_root = crate::utils::paths::repo_root()?;
    let backend = create_session_backend_or_local(&repo_root);
    let ended_at = now_rfc3339();
    let maybe_state = backend.load_session(&session_id)?;
    if let Some(mut state) = maybe_state.clone() {
        let context = SessionTransitionContext {
            has_files_touched: !state.pending.files_touched.is_empty(),
            is_rebase_in_progress: false,
        };
        let transition =
            transition_session_with_context(state.phase, SessionEvent::SessionStop, context);
        apply_session_transition(&mut state, transition, &mut SessionNoOpActionHandler)?;
        state.ended_at = Some(ended_at.clone());
        state.last_interaction_time = Some(ended_at.clone());
        backend.save_session(&state)?;
    }
    if let Some(spool) = resolve_interaction_spool(&repo_root) {
        let session = maybe_state
            .map(|state| InteractionSession {
                session_id: session_id.clone(),
                repo_id: spool.repo_id().to_string(),
                agent_type: state.agent_type.clone(),
                model: event.model.clone(),
                first_prompt: state.first_prompt.clone(),
                transcript_path: state.transcript_path.clone(),
                worktree_path: state.worktree_path.clone(),
                worktree_id: state.worktree_id.clone(),
                started_at: state.started_at.clone(),
                ended_at: Some(ended_at.clone()),
                last_event_at: ended_at.clone(),
                updated_at: ended_at.clone(),
            })
            .unwrap_or(InteractionSession {
                session_id: session_id.clone(),
                repo_id: spool.repo_id().to_string(),
                model: event.model.clone(),
                ended_at: Some(ended_at.clone()),
                last_event_at: ended_at.clone(),
                updated_at: ended_at.clone(),
                ..Default::default()
            });
        if let Err(err) = spool.record_session(&session) {
            eprintln!("[bitloops] Warning: failed to spool interaction session end: {err}");
        }
        if let Err(err) = spool.record_event(&InteractionEvent {
            event_id: generate_interaction_event_id(),
            session_id: session_id.clone(),
            turn_id: None,
            repo_id: spool.repo_id().to_string(),
            event_type: InteractionEventType::SessionEnd,
            event_time: ended_at.clone(),
            agent_type: session.agent_type.clone(),
            model: event.model.clone(),
            payload: serde_json::Value::Object(Default::default()),
        }) {
            eprintln!("[bitloops] Warning: failed to spool session_end event: {err}");
        }
    }
    flush_interaction_spool_best_effort(&repo_root);
    Ok(())
}

pub fn handle_lifecycle_subagent_start(
    _agent: &dyn LifecycleAgentAdapter,
    event: &LifecycleEvent,
) -> Result<()> {
    if let Ok(repo_root) = crate::utils::paths::repo_root()
        && let Some(spool) = resolve_interaction_spool(&repo_root)
    {
        if let Err(err) = spool.record_event(&InteractionEvent {
            event_id: generate_interaction_event_id(),
            session_id: event.session_id.clone(),
            turn_id: None,
            repo_id: spool.repo_id().to_string(),
            event_type: InteractionEventType::SubagentStart,
            event_time: now_rfc3339(),
            agent_type: String::new(),
            model: event.model.clone(),
            payload: serde_json::json!({
                "subagent_id": event.subagent_id,
                "tool_use_id": event.tool_use_id,
            }),
        }) {
            eprintln!("[bitloops] Warning: failed to spool subagent_start event: {err}");
        }
        flush_interaction_spool_best_effort(&repo_root);
    }
    Ok(())
}

pub fn handle_lifecycle_subagent_end(
    _agent: &dyn LifecycleAgentAdapter,
    event: &LifecycleEvent,
) -> Result<()> {
    if let Ok(repo_root) = crate::utils::paths::repo_root()
        && let Some(spool) = resolve_interaction_spool(&repo_root)
    {
        if let Err(err) = spool.record_event(&InteractionEvent {
            event_id: generate_interaction_event_id(),
            session_id: event.session_id.clone(),
            turn_id: None,
            repo_id: spool.repo_id().to_string(),
            event_type: InteractionEventType::SubagentEnd,
            event_time: now_rfc3339(),
            agent_type: String::new(),
            model: event.model.clone(),
            payload: serde_json::json!({
                "subagent_id": event.subagent_id,
                "tool_use_id": event.tool_use_id,
            }),
        }) {
            eprintln!("[bitloops] Warning: failed to spool subagent_end event: {err}");
        }
        flush_interaction_spool_best_effort(&repo_root);
    }
    Ok(())
}

use anyhow::Result;

use super::adapter::LifecycleAgentAdapter;
use super::interaction::resolve_interaction_event_store;
use super::time_and_ids::{generate_interaction_event_id, now_rfc3339};
use super::types::{LifecycleEvent, SessionIdPolicy, apply_session_id_policy};
use crate::host::checkpoints::session::create_session_backend_or_local;
use crate::host::checkpoints::session::phase::{
    Event as SessionEvent, NoOpActionHandler as SessionNoOpActionHandler,
    TransitionContext as SessionTransitionContext, apply_transition as apply_session_transition,
    transition_with_context as transition_session_with_context,
};
use crate::host::interactions::store::InteractionEventStore;
use crate::host::interactions::types::{InteractionEvent, InteractionEventType};

pub fn handle_lifecycle_compaction(
    _agent: &dyn LifecycleAgentAdapter,
    event: &LifecycleEvent,
) -> Result<()> {
    if event.session_id.is_empty() {
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
    match backend.load_session(&event.session_id) {
        Ok(Some(mut state)) => {
            let context = SessionTransitionContext {
                has_files_touched: !state.files_touched.is_empty(),
                is_rebase_in_progress: false,
            };
            let transition =
                transition_session_with_context(state.phase, SessionEvent::Compaction, context);
            let mut handler = SessionNoOpActionHandler;
            if let Err(err) = apply_session_transition(&mut state, transition, &mut handler) {
                eprintln!("[bitloops] Warning: compaction transition failed: {err}");
            }

            // Compaction resets transcript offset after transition.
            state.checkpoint_transcript_start = 0;
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
    if let Some(store) = resolve_interaction_event_store(&repo_root)
        && let Err(err) = store.record_event(&InteractionEvent {
            event_id: generate_interaction_event_id(),
            session_id: event.session_id.clone(),
            turn_id: None,
            repo_id: store.repo_id().to_string(),
            event_type: InteractionEventType::Compaction,
            event_time: now_rfc3339(),
            agent_type: String::new(),
            model: event.model.clone(),
            payload: serde_json::Value::Object(Default::default()),
        })
    {
        eprintln!("[bitloops] Warning: failed to record compaction event: {err}");
    }

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
    if let Some(mut state) = backend.load_session(&session_id)? {
        let context = SessionTransitionContext {
            has_files_touched: !state.files_touched.is_empty(),
            is_rebase_in_progress: false,
        };
        let transition =
            transition_session_with_context(state.phase, SessionEvent::SessionStop, context);
        apply_session_transition(&mut state, transition, &mut SessionNoOpActionHandler)?;
        let ended_at = now_rfc3339();
        state.ended_at = Some(ended_at.clone());
        state.last_interaction_time = Some(ended_at.clone());
        backend.save_session(&state)?;

        // ── interaction event persistence ────────────────────────────────────
        if let Some(store) = resolve_interaction_event_store(&repo_root) {
            if let Err(err) = store.end_session(&session_id, &ended_at) {
                eprintln!("[bitloops] Warning: failed to end interaction session: {err}");
            }
            if let Err(err) = store.record_event(&InteractionEvent {
                event_id: generate_interaction_event_id(),
                session_id: session_id.clone(),
                turn_id: None,
                repo_id: store.repo_id().to_string(),
                event_type: InteractionEventType::SessionEnd,
                event_time: ended_at,
                agent_type: state.agent_type.clone(),
                model: event.model.clone(),
                payload: serde_json::Value::Object(Default::default()),
            }) {
                eprintln!("[bitloops] Warning: failed to record session_end event: {err}");
            }
        }
    }
    Ok(())
}

pub fn handle_lifecycle_subagent_start(
    _agent: &dyn LifecycleAgentAdapter,
    event: &LifecycleEvent,
) -> Result<()> {
    if let Ok(repo_root) = crate::utils::paths::repo_root()
        && let Some(store) = resolve_interaction_event_store(&repo_root)
        && let Err(err) = store.record_event(&InteractionEvent {
            event_id: generate_interaction_event_id(),
            session_id: event.session_id.clone(),
            turn_id: None,
            repo_id: store.repo_id().to_string(),
            event_type: InteractionEventType::SubagentStart,
            event_time: now_rfc3339(),
            agent_type: String::new(),
            model: event.model.clone(),
            payload: serde_json::json!({
                "subagent_id": event.subagent_id,
                "tool_use_id": event.tool_use_id,
            }),
        })
    {
        eprintln!("[bitloops] Warning: failed to record subagent_start event: {err}");
    }
    Ok(())
}

pub fn handle_lifecycle_subagent_end(
    _agent: &dyn LifecycleAgentAdapter,
    event: &LifecycleEvent,
) -> Result<()> {
    if let Ok(repo_root) = crate::utils::paths::repo_root()
        && let Some(store) = resolve_interaction_event_store(&repo_root)
        && let Err(err) = store.record_event(&InteractionEvent {
            event_id: generate_interaction_event_id(),
            session_id: event.session_id.clone(),
            turn_id: None,
            repo_id: store.repo_id().to_string(),
            event_type: InteractionEventType::SubagentEnd,
            event_time: now_rfc3339(),
            agent_type: String::new(),
            model: event.model.clone(),
            payload: serde_json::json!({
                "subagent_id": event.subagent_id,
                "tool_use_id": event.tool_use_id,
            }),
        })
    {
        eprintln!("[bitloops] Warning: failed to record subagent_end event: {err}");
    }
    Ok(())
}

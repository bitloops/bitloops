use std::path::Path;

use crate::adapters::agents::TokenUsage;
use crate::host::checkpoints::lifecycle::interaction::{
    flush_interaction_spool_best_effort, resolve_interaction_spool,
};
use crate::host::checkpoints::session::state::SessionState;
use crate::host::checkpoints::strategy::manual_commit::TokenUsageMetadata;
use crate::host::interactions::store::InteractionSpool;
use crate::host::interactions::types::{
    InteractionEvent, InteractionEventType, InteractionSession, InteractionTurn,
};
use uuid::Uuid;

use super::HookAgentProfile;
use super::helpers::{now_rfc3339, truncate_prompt_for_storage};

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
        .unwrap_or_else(|| crate::utils::paths::get_worktree_id(repo_root).unwrap_or_default())
}

fn interaction_event_id() -> String {
    Uuid::new_v4().simple().to_string()
}

pub(super) fn token_usage_metadata(token_usage: Option<&TokenUsage>) -> Option<TokenUsageMetadata> {
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

pub(super) fn record_session_start_interaction(
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

pub(super) fn record_turn_start_interaction(
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

pub(super) struct TurnEndInteraction<'a> {
    pub(super) repo_root: Option<&'a Path>,
    pub(super) session_id: &'a str,
    pub(super) transcript_path: &'a str,
    pub(super) state: Option<&'a SessionState>,
    pub(super) prompt: &'a str,
    pub(super) turn_started_at: Option<&'a str>,
    pub(super) profile: HookAgentProfile,
    pub(super) files_modified: &'a [String],
    pub(super) token_usage: Option<&'a TokenUsage>,
}

pub(super) fn record_turn_end_interaction(ctx: TurnEndInteraction<'_>) {
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
            .unwrap_or_else(super::generate_turn_id);
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

pub(super) fn record_session_end_interaction(
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
            first_prompt: state
                .map(|state| state.first_prompt.clone())
                .unwrap_or_default(),
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

pub(super) fn record_subagent_interaction_event(
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

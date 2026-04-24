use std::path::Path;

use crate::adapters::agents::{AgentRegistry, TokenUsage};
use crate::host::checkpoints::lifecycle::interaction::{
    flush_interaction_spool_best_effort, resolve_interaction_spool,
};
use crate::host::checkpoints::session::state::SessionState;
use crate::host::checkpoints::strategy::manual_commit::TokenUsageMetadata;
use crate::host::checkpoints::strategy::manual_commit::current_branch_name;
use crate::host::interactions::model::resolve_interaction_model;
use crate::host::interactions::store::InteractionSpool;
use crate::host::interactions::tool_events::{
    DerivedToolEventContext, INTERACTION_SOURCE_LIVE_HOOK, derive_tool_events_with_deriver,
    transcript_derived_turn_end_sequence,
};
use crate::host::interactions::transcript_fragment::read_transcript_fragment_from_path;
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

fn interaction_branch(repo_root: &Path) -> String {
    current_branch_name(repo_root)
}

fn interaction_actor_identity() -> (String, String, String, String) {
    let Some(session) = crate::daemon::load_workos_session_details_cached()
        .ok()
        .flatten()
    else {
        return (String::new(), String::new(), String::new(), String::new());
    };
    let actor_name = session.display_label();
    (
        session.user_id.unwrap_or_default(),
        actor_name,
        session.user_email.unwrap_or_default(),
        "workos".to_string(),
    )
}

fn read_transcript_fragment(transcript_path: &str, start_offset: i64) -> (String, Option<i64>) {
    read_transcript_fragment_from_path(transcript_path, start_offset)
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
    model_hint: &str,
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
        let branch = interaction_branch(repo_root);
        let (actor_id, actor_name, actor_email, actor_source) = interaction_actor_identity();
        let model = resolve_interaction_model(model_hint, &state.transcript_path);
        let session = InteractionSession {
            session_id: state.session_id.clone(),
            repo_id: spool.repo_id().to_string(),
            branch: branch.clone(),
            actor_id: actor_id.clone(),
            actor_name: actor_name.clone(),
            actor_email: actor_email.clone(),
            actor_source: actor_source.clone(),
            agent_type: interaction_agent_type(Some(state), profile),
            model: model.clone(),
            first_prompt: state.first_prompt.clone(),
            transcript_path: state.transcript_path.clone(),
            worktree_path: interaction_worktree_path(repo_root, Some(state)),
            worktree_id: interaction_worktree_id(repo_root, Some(state)),
            started_at: interaction_started_at(Some(state), &event_time),
            ended_at: state.ended_at.clone(),
            last_event_at: event_time.clone(),
            updated_at: event_time.clone(),
        };
        if let Err(err) = spool.record_session(&session) {
            eprintln!("[bitloops] Warning: failed to spool interaction session: {err}");
        }
        if let Err(err) = spool.record_event(&InteractionEvent {
            event_id: interaction_event_id(),
            session_id: state.session_id.clone(),
            turn_id: None,
            repo_id: spool.repo_id().to_string(),
            branch,
            actor_id,
            actor_name,
            actor_email,
            actor_source,
            event_type: InteractionEventType::SessionStart,
            event_time,
            agent_type: session.agent_type.clone(),
            model,
            payload: serde_json::json!({
                "first_prompt": session.first_prompt,
                "transcript_path": session.transcript_path,
                "worktree_path": session.worktree_path,
                "worktree_id": session.worktree_id,
            }),
            ..Default::default()
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
    model_hint: &str,
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
        let branch = interaction_branch(repo_root);
        let (actor_id, actor_name, actor_email, actor_source) = interaction_actor_identity();
        let model = resolve_interaction_model(model_hint, &state.transcript_path);
        let session = InteractionSession {
            session_id: state.session_id.clone(),
            repo_id: spool.repo_id().to_string(),
            branch: branch.clone(),
            actor_id: actor_id.clone(),
            actor_name: actor_name.clone(),
            actor_email: actor_email.clone(),
            actor_source: actor_source.clone(),
            agent_type: interaction_agent_type(Some(state), Some(profile)),
            model: model.clone(),
            first_prompt: state.first_prompt.clone(),
            transcript_path: state.transcript_path.clone(),
            worktree_path: interaction_worktree_path(repo_root, Some(state)),
            worktree_id: interaction_worktree_id(repo_root, Some(state)),
            started_at: interaction_started_at(Some(state), &event_time),
            ended_at: state.ended_at.clone(),
            last_event_at: event_time.clone(),
            updated_at: event_time.clone(),
        };
        if let Err(err) = spool.record_session(&session) {
            eprintln!("[bitloops] Warning: failed to spool interaction session: {err}");
        }
        let turn_number = state.pending.step_count + 1;
        let turn = InteractionTurn {
            turn_id: state.turn_id.clone(),
            session_id: state.session_id.clone(),
            repo_id: spool.repo_id().to_string(),
            branch: branch.clone(),
            actor_id: actor_id.clone(),
            actor_name: actor_name.clone(),
            actor_email: actor_email.clone(),
            actor_source: actor_source.clone(),
            turn_number,
            prompt: prompt.clone(),
            agent_type: session.agent_type.clone(),
            model: model.clone(),
            started_at: event_time.clone(),
            prompt_count: 1,
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
            branch,
            actor_id,
            actor_name,
            actor_email,
            actor_source,
            event_type: InteractionEventType::TurnStart,
            event_time,
            agent_type: session.agent_type.clone(),
            model,
            payload: serde_json::json!({
                "prompt": turn.prompt,
                "turn_number": turn_number,
            }),
            ..Default::default()
        }) {
            eprintln!("[bitloops] Warning: failed to spool turn_start event: {err}");
        }
    });
}

pub(super) struct TurnEndInteraction<'a> {
    pub(super) repo_root: Option<&'a Path>,
    pub(super) session_id: &'a str,
    pub(super) transcript_path: &'a str,
    pub(super) transcript_offset_start: i64,
    pub(super) state: Option<&'a SessionState>,
    pub(super) prompt: &'a str,
    pub(super) turn_started_at: Option<&'a str>,
    pub(super) profile: HookAgentProfile,
    pub(super) model_hint: &'a str,
    pub(super) files_modified: &'a [String],
    pub(super) token_usage: Option<&'a TokenUsage>,
}

pub(super) fn record_turn_end_interaction(ctx: TurnEndInteraction<'_>) {
    let TurnEndInteraction {
        repo_root,
        session_id,
        transcript_path,
        transcript_offset_start,
        state,
        prompt,
        turn_started_at,
        profile,
        model_hint,
        files_modified,
        token_usage,
    } = ctx;
    let event_time = now_rfc3339();
    let token_usage = token_usage_metadata(token_usage);
    let prompt = truncate_prompt_for_storage(prompt);
    let (transcript_fragment, transcript_offset_end) =
        read_transcript_fragment(transcript_path, transcript_offset_start);
    with_interaction_spool(repo_root, |spool| {
        let Some(repo_root) = repo_root else {
            return;
        };
        let branch = interaction_branch(repo_root);
        let (actor_id, actor_name, actor_email, actor_source) = interaction_actor_identity();
        let agent_type = interaction_agent_type(state, Some(profile));
        let transcript_path = if transcript_path.trim().is_empty() {
            state
                .map(|state| state.transcript_path.as_str())
                .unwrap_or_default()
        } else {
            transcript_path
        };
        let model = resolve_interaction_model(model_hint, transcript_path);
        let session = InteractionSession {
            session_id: session_id.to_string(),
            repo_id: spool.repo_id().to_string(),
            branch: branch.clone(),
            actor_id: actor_id.clone(),
            actor_name: actor_name.clone(),
            actor_email: actor_email.clone(),
            actor_source: actor_source.clone(),
            agent_type: agent_type.clone(),
            model: model.clone(),
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
            branch: branch.clone(),
            actor_id: actor_id.clone(),
            actor_name: actor_name.clone(),
            actor_email: actor_email.clone(),
            actor_source: actor_source.clone(),
            turn_number: state.map_or(1, |state| state.pending.step_count + 1),
            prompt: prompt.clone(),
            agent_type: agent_type.clone(),
            model: model.clone(),
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
            prompt_count: 1,
            transcript_offset_start: Some(transcript_offset_start),
            transcript_offset_end,
            transcript_fragment: transcript_fragment.clone(),
            files_modified: files_modified.to_vec(),
            checkpoint_id: None,
            updated_at: event_time.clone(),
            ..Default::default()
        };
        if let Err(err) = spool.record_turn(&turn) {
            eprintln!("[bitloops] Warning: failed to spool interaction turn end: {err}");
        }
        let agent_registry = AgentRegistry::builtin();
        let transcript_tool_event_deriver = agent_registry
            .get_by_agent_type(&agent_type)
            .ok()
            .and_then(|agent| agent.as_transcript_tool_event_deriver());
        let derived_tool_events = match derive_tool_events_with_deriver(
            transcript_tool_event_deriver,
            &DerivedToolEventContext {
                repo_id: spool.repo_id(),
                session_id,
                turn_id: &turn.turn_id,
                branch: &branch,
                actor_id: &actor_id,
                actor_name: &actor_name,
                actor_email: &actor_email,
                actor_source: &actor_source,
                event_time: &event_time,
                agent_type: &agent_type,
                model: &model,
                transcript_path,
            },
            &transcript_fragment,
        ) {
            Ok(events) => events,
            Err(err) => {
                eprintln!(
                    "[bitloops] Warning: failed to derive transcript tool events for interaction turn end: {err:#}"
                );
                Vec::new()
            }
        };
        for derived_event in &derived_tool_events {
            if let Err(err) = spool.record_event(derived_event) {
                eprintln!(
                    "[bitloops] Warning: failed to spool transcript-derived tool event: {err}"
                );
            }
        }
        if let Err(err) = spool.record_event(&InteractionEvent {
            event_id: interaction_event_id(),
            session_id: session_id.to_string(),
            turn_id: Some(turn_id),
            repo_id: spool.repo_id().to_string(),
            branch,
            actor_id,
            actor_name,
            actor_email,
            actor_source,
            event_type: InteractionEventType::TurnEnd,
            event_time,
            sequence_number: transcript_derived_turn_end_sequence(&derived_tool_events),
            agent_type,
            model,
            payload: serde_json::json!({
                "files_modified": files_modified,
                "files_count": files_modified.len(),
                "transcript_offset_start": transcript_offset_start,
                "transcript_offset_end": transcript_offset_end,
                "transcript_fragment": transcript_fragment,
                "token_usage": token_usage,
            }),
            ..Default::default()
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
    model_hint: &str,
) {
    let ended_at = state
        .and_then(|state| state.ended_at.clone())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(now_rfc3339);
    with_interaction_spool(repo_root, |spool| {
        let Some(repo_root) = repo_root else {
            return;
        };
        let branch = interaction_branch(repo_root);
        let (actor_id, actor_name, actor_email, actor_source) = interaction_actor_identity();
        let transcript_path = if transcript_path.trim().is_empty() {
            state
                .map(|state| state.transcript_path.as_str())
                .unwrap_or_default()
        } else {
            transcript_path
        };
        let model = resolve_interaction_model(model_hint, transcript_path);
        let session = InteractionSession {
            session_id: session_id.to_string(),
            repo_id: spool.repo_id().to_string(),
            branch: branch.clone(),
            actor_id: actor_id.clone(),
            actor_name: actor_name.clone(),
            actor_email: actor_email.clone(),
            actor_source: actor_source.clone(),
            agent_type: interaction_agent_type(state, profile),
            model: model.clone(),
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
        };
        if let Err(err) = spool.record_session(&session) {
            eprintln!("[bitloops] Warning: failed to spool interaction session end: {err}");
        }
        if let Err(err) = spool.record_event(&InteractionEvent {
            event_id: interaction_event_id(),
            session_id: session_id.to_string(),
            turn_id: None,
            repo_id: spool.repo_id().to_string(),
            branch,
            actor_id,
            actor_name,
            actor_email,
            actor_source,
            event_type: InteractionEventType::SessionEnd,
            event_time: ended_at,
            agent_type: session.agent_type.clone(),
            model,
            payload: serde_json::Value::Object(Default::default()),
            ..Default::default()
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
    model_hint: &str,
    payload: serde_json::Value,
) {
    let event_time = now_rfc3339();
    with_interaction_spool(repo_root, |spool| {
        let Some(repo_root) = repo_root else {
            return;
        };
        let branch = interaction_branch(repo_root);
        let (actor_id, actor_name, actor_email, actor_source) = interaction_actor_identity();
        let transcript_path = state
            .map(|state| state.transcript_path.as_str())
            .unwrap_or_default();
        let model = resolve_interaction_model(model_hint, transcript_path);
        let tool_use_id = payload
            .get("tool_use_id")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string();
        let tool_kind = payload
            .get("subagent_type")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string();
        let task_description = payload
            .get("task_description")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string();
        let subagent_id = payload
            .get("subagent_id")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string();
        if let Err(err) = spool.record_event(&InteractionEvent {
            event_id: interaction_event_id(),
            session_id: session_id.to_string(),
            turn_id: state
                .filter(|state| !state.turn_id.trim().is_empty())
                .map(|state| state.turn_id.clone()),
            repo_id: spool.repo_id().to_string(),
            branch,
            actor_id,
            actor_name,
            actor_email,
            actor_source,
            event_type,
            event_time,
            source: INTERACTION_SOURCE_LIVE_HOOK.to_string(),
            sequence_number: 0,
            agent_type: interaction_agent_type(state, Some(profile)),
            model,
            tool_use_id,
            tool_kind,
            task_description,
            subagent_id,
            payload,
        }) {
            eprintln!("[bitloops] Warning: failed to spool {event_type} event: {err}");
        }
    });
}

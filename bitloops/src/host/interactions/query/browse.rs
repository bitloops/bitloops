use std::cmp::Reverse;
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use anyhow::Result;
use rusqlite::OptionalExtension;

use super::filters::{
    event_matches_filter, session_matches_filter, session_sort_key, turn_matches_filter,
    turn_sort_key,
};
use super::state::load_state;
use super::types::{
    InteractionActorBucket, InteractionAgentBucket, InteractionBrowseFilter,
    InteractionChangeSnapshot, InteractionCommitAuthorBucket, InteractionKpis,
    InteractionSessionDetail, InteractionSessionSummary, InteractionTurnSummary,
};
use crate::host::checkpoints::lifecycle::interaction::resolve_interaction_spool;
use crate::host::checkpoints::strategy::manual_commit::TokenUsageMetadata;
use crate::host::interactions::types::InteractionEvent;

pub(crate) fn list_session_summaries(
    repo_root: &Path,
    filter: &InteractionBrowseFilter,
) -> Result<Vec<InteractionSessionSummary>> {
    let state = load_state(repo_root)?;
    let mut sessions = state
        .session_summaries
        .into_values()
        .filter(|session| session_matches_filter(session, filter))
        .collect::<Vec<_>>();
    sessions.sort_by_key(|session| Reverse(session_sort_key(session)));
    Ok(sessions)
}

pub(crate) fn list_turn_summaries(
    repo_root: &Path,
    filter: &InteractionBrowseFilter,
) -> Result<Vec<InteractionTurnSummary>> {
    let state = load_state(repo_root)?;
    let mut turns = state
        .turn_summaries
        .into_values()
        .filter(|turn| turn_matches_filter(turn, filter))
        .collect::<Vec<_>>();
    turns.sort_by_key(|turn| Reverse(turn_sort_key(turn)));
    Ok(turns)
}

pub(crate) fn list_events(
    repo_root: &Path,
    filter: &InteractionBrowseFilter,
) -> Result<Vec<InteractionEvent>> {
    let state = load_state(repo_root)?;
    let mut events = state
        .events
        .iter()
        .filter(|event| event_matches_filter(event, &state, filter))
        .cloned()
        .collect::<Vec<_>>();
    events.sort_by(|left, right| {
        right
            .event_time
            .cmp(&left.event_time)
            .then_with(|| right.sequence_number.cmp(&left.sequence_number))
            .then_with(|| right.event_id.cmp(&left.event_id))
    });
    Ok(events)
}

pub(crate) fn load_session_detail(
    repo_root: &Path,
    session_id: &str,
) -> Result<Option<InteractionSessionDetail>> {
    let state = load_state(repo_root)?;
    let summary = match state.session_summaries.get(session_id) {
        Some(summary) => summary.clone(),
        None => return Ok(None),
    };
    let turns = state
        .turn_summaries
        .values()
        .filter(|turn| turn.turn.session_id == session_id)
        .cloned()
        .collect::<Vec<_>>();
    let mut raw_events = state
        .events
        .into_iter()
        .filter(|event| event.session_id == session_id)
        .collect::<Vec<_>>();
    raw_events.sort_by(|left, right| {
        left.event_time
            .cmp(&right.event_time)
            .then_with(|| left.sequence_number.cmp(&right.sequence_number))
            .then_with(|| left.event_id.cmp(&right.event_id))
    });
    Ok(Some(InteractionSessionDetail {
        summary,
        turns,
        raw_events,
    }))
}

pub(crate) fn compute_kpis(
    repo_root: &Path,
    filter: &InteractionBrowseFilter,
) -> Result<InteractionKpis> {
    let sessions = list_session_summaries(repo_root, filter)?;
    let turns = list_turn_summaries(repo_root, filter)?;
    let mut actors = HashSet::new();
    let mut agents = HashSet::new();
    let mut checkpoint_ids = HashSet::new();
    let mut tool_use_ids = HashSet::new();
    let mut subagent_run_ids = HashSet::new();
    let mut totals = TokenUsageMetadata::default();

    for session in &sessions {
        if !session.session.actor_email.trim().is_empty()
            || !session.session.actor_id.trim().is_empty()
            || !session.session.actor_name.trim().is_empty()
        {
            actors.insert((
                session.session.actor_id.clone(),
                session.session.actor_email.clone(),
                session.session.actor_name.clone(),
            ));
        }
        if !session.session.agent_type.trim().is_empty() {
            agents.insert(session.session.agent_type.clone());
        }
        for checkpoint_id in &session.checkpoint_ids {
            checkpoint_ids.insert(checkpoint_id.clone());
        }
        for tool_use in &session.tool_uses {
            tool_use_ids.insert(tool_use.tool_invocation_id.clone());
        }
        for subagent_run in &session.subagent_runs {
            subagent_run_ids.insert(subagent_run.subagent_run_id.clone());
        }
        if let Some(token_usage) = session.token_usage.as_ref() {
            totals.input_tokens += token_usage.input_tokens;
            totals.output_tokens += token_usage.output_tokens;
            totals.cache_creation_tokens += token_usage.cache_creation_tokens;
            totals.cache_read_tokens += token_usage.cache_read_tokens;
            totals.api_call_count += token_usage.api_call_count;
        }
    }

    for turn in &turns {
        if !turn.turn.agent_type.trim().is_empty() {
            agents.insert(turn.turn.agent_type.clone());
        }
    }

    Ok(InteractionKpis {
        total_sessions: sessions.len(),
        total_turns: turns.len(),
        total_checkpoints: checkpoint_ids.len(),
        total_tool_uses: tool_use_ids.len(),
        total_subagent_runs: subagent_run_ids.len(),
        total_actors: actors.len(),
        total_agents: agents.len(),
        input_tokens: totals.input_tokens,
        output_tokens: totals.output_tokens,
        cache_creation_tokens: totals.cache_creation_tokens,
        cache_read_tokens: totals.cache_read_tokens,
        api_call_count: totals.api_call_count,
    })
}

pub(crate) fn interaction_change_snapshot(repo_root: &Path) -> Result<InteractionChangeSnapshot> {
    let repo_id = crate::host::devql::resolve_repo_identity(repo_root)
        .map(|identity| identity.repo_id)
        .unwrap_or_default();
    let Some(spool) = resolve_interaction_spool(repo_root) else {
        return Ok(InteractionChangeSnapshot {
            repo_id,
            ..Default::default()
        });
    };

    spool.with_connection(|conn| {
        let session_count = conn.query_row(
            "SELECT COUNT(*)
             FROM interaction_sessions
             WHERE repo_id = ?1",
            rusqlite::params![spool.repo_id()],
            |row| row.get::<_, i64>(0),
        )?;
        let latest_session = conn
            .query_row(
                "SELECT session_id, changed_at
                 FROM (
                    SELECT session_id,
                           COALESCE(
                               NULLIF(updated_at, ''),
                               NULLIF(last_event_at, ''),
                               NULLIF(started_at, ''),
                               ''
                           ) AS changed_at
                    FROM interaction_sessions
                    WHERE repo_id = ?1
                 )
                 ORDER BY changed_at DESC, session_id DESC
                 LIMIT 1",
                rusqlite::params![spool.repo_id()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;

        let turn_count = conn.query_row(
            "SELECT COUNT(*)
             FROM interaction_turns
             WHERE repo_id = ?1",
            rusqlite::params![spool.repo_id()],
            |row| row.get::<_, i64>(0),
        )?;
        let latest_turn = conn
            .query_row(
                "SELECT turn_id, changed_at
                 FROM (
                    SELECT turn_id,
                           COALESCE(
                               NULLIF(updated_at, ''),
                               NULLIF(ended_at, ''),
                               NULLIF(started_at, ''),
                               ''
                           ) AS changed_at
                    FROM interaction_turns
                    WHERE repo_id = ?1
                 )
                 ORDER BY changed_at DESC, turn_id DESC
                 LIMIT 1",
                rusqlite::params![spool.repo_id()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;

        Ok(InteractionChangeSnapshot {
            repo_id: spool.repo_id().to_string(),
            session_count: usize::try_from(session_count.max(0)).unwrap_or_default(),
            turn_count: usize::try_from(turn_count.max(0)).unwrap_or_default(),
            latest_session_id: latest_session
                .as_ref()
                .map(|(session_id, _)| session_id.clone()),
            latest_session_updated_at: latest_session
                .and_then(|(_, updated_at)| (!updated_at.is_empty()).then_some(updated_at)),
            latest_turn_id: latest_turn.as_ref().map(|(turn_id, _)| turn_id.clone()),
            latest_turn_updated_at: latest_turn
                .and_then(|(_, updated_at)| (!updated_at.is_empty()).then_some(updated_at)),
        })
    })
}

pub(crate) fn list_actor_buckets(
    repo_root: &Path,
    filter: &InteractionBrowseFilter,
) -> Result<Vec<InteractionActorBucket>> {
    let sessions = list_session_summaries(repo_root, filter)?;
    let turns = list_turn_summaries(repo_root, filter)?;
    let mut by_key: BTreeMap<(String, String, String, String), InteractionActorBucket> =
        BTreeMap::new();

    for session in sessions {
        let key = (
            session.session.actor_id.clone(),
            session.session.actor_name.clone(),
            session.session.actor_email.clone(),
            session.session.actor_source.clone(),
        );
        if key.0.is_empty() && key.1.is_empty() && key.2.is_empty() {
            continue;
        }
        let entry = by_key
            .entry(key.clone())
            .or_insert_with(|| InteractionActorBucket {
                actor_id: key.0.clone(),
                actor_name: key.1.clone(),
                actor_email: key.2.clone(),
                actor_source: key.3.clone(),
                ..Default::default()
            });
        entry.session_count += 1;
    }

    for turn in turns {
        let key = (
            turn.turn.actor_id.clone(),
            turn.turn.actor_name.clone(),
            turn.turn.actor_email.clone(),
            turn.turn.actor_source.clone(),
        );
        if key.0.is_empty() && key.1.is_empty() && key.2.is_empty() {
            continue;
        }
        let entry = by_key
            .entry(key.clone())
            .or_insert_with(|| InteractionActorBucket {
                actor_id: key.0.clone(),
                actor_name: key.1.clone(),
                actor_email: key.2.clone(),
                actor_source: key.3.clone(),
                ..Default::default()
            });
        entry.turn_count += 1;
    }

    Ok(by_key.into_values().collect())
}

pub(crate) fn list_commit_author_buckets(
    repo_root: &Path,
    filter: &InteractionBrowseFilter,
) -> Result<Vec<InteractionCommitAuthorBucket>> {
    let sessions = list_session_summaries(repo_root, filter)?;
    let turns = list_turn_summaries(repo_root, filter)?;
    let mut by_key: BTreeMap<(String, String), InteractionCommitAuthorBucket> = BTreeMap::new();

    for session in sessions {
        let mut seen = HashSet::new();
        for checkpoint in &session.linked_checkpoints {
            let key = (
                checkpoint.author_name.clone(),
                checkpoint.author_email.clone(),
            );
            if key.0.is_empty() && key.1.is_empty() {
                continue;
            }
            let entry =
                by_key
                    .entry(key.clone())
                    .or_insert_with(|| InteractionCommitAuthorBucket {
                        author_name: key.0.clone(),
                        author_email: key.1.clone(),
                        ..Default::default()
                    });
            entry.session_count += 1;
            if seen.insert(checkpoint.checkpoint_id.clone()) {
                entry.checkpoint_count += 1;
            }
        }
    }

    for turn in turns {
        let mut seen = HashSet::new();
        for checkpoint in &turn.linked_checkpoints {
            let key = (
                checkpoint.author_name.clone(),
                checkpoint.author_email.clone(),
            );
            if key.0.is_empty() && key.1.is_empty() {
                continue;
            }
            let entry =
                by_key
                    .entry(key.clone())
                    .or_insert_with(|| InteractionCommitAuthorBucket {
                        author_name: key.0.clone(),
                        author_email: key.1.clone(),
                        ..Default::default()
                    });
            entry.turn_count += 1;
            if seen.insert(checkpoint.checkpoint_id.clone()) {
                entry.checkpoint_count += 1;
            }
        }
    }

    Ok(by_key.into_values().collect())
}

pub(crate) fn list_agent_buckets(
    repo_root: &Path,
    filter: &InteractionBrowseFilter,
) -> Result<Vec<InteractionAgentBucket>> {
    let sessions = list_session_summaries(repo_root, filter)?;
    let turns = list_turn_summaries(repo_root, filter)?;
    let mut by_key: BTreeMap<String, InteractionAgentBucket> = BTreeMap::new();

    for session in sessions {
        if session.session.agent_type.trim().is_empty() {
            continue;
        }
        let entry = by_key
            .entry(session.session.agent_type.clone())
            .or_insert_with(|| InteractionAgentBucket {
                key: session.session.agent_type.clone(),
                ..Default::default()
            });
        entry.session_count += 1;
    }

    for turn in turns {
        if turn.turn.agent_type.trim().is_empty() {
            continue;
        }
        let entry = by_key
            .entry(turn.turn.agent_type.clone())
            .or_insert_with(|| InteractionAgentBucket {
                key: turn.turn.agent_type.clone(),
                ..Default::default()
            });
        entry.turn_count += 1;
    }

    Ok(by_key.into_values().collect())
}

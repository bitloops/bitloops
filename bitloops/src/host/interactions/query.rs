use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, FixedOffset};

use super::db_store::SqliteInteractionSpool;
use super::store::InteractionSpool;
use super::types::{InteractionEvent, InteractionSession, InteractionToolUse, InteractionTurn};
use crate::host::checkpoints::lifecycle::interaction::resolve_interaction_spool;
use crate::host::checkpoints::strategy::manual_commit::TokenUsageMetadata;
use crate::host::relational_store::{DefaultRelationalStore, RelationalStore};

const MAX_INTERACTION_ROWS: usize = 1_000_000;
const FIELD_PROMPT: &str = "prompt";
const FIELD_SUMMARY: &str = "summary";
const FIELD_TOOL: &str = "tool";
const FIELD_PATH: &str = "path";
const FIELD_TRANSCRIPT: &str = "transcript";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InteractionBrowseFilter {
    pub since: Option<String>,
    pub until: Option<String>,
    pub actor: Option<String>,
    pub actor_id: Option<String>,
    pub actor_email: Option<String>,
    pub commit_author: Option<String>,
    pub commit_author_email: Option<String>,
    pub agent: Option<String>,
    pub model: Option<String>,
    pub branch: Option<String>,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub checkpoint_id: Option<String>,
    pub tool_use_id: Option<String>,
    pub tool_kind: Option<String>,
    pub has_checkpoint: Option<bool>,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InteractionSearchInput {
    pub filter: InteractionBrowseFilter,
    pub query: String,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InteractionLinkedCheckpoint {
    pub checkpoint_id: String,
    pub commit_sha: String,
    pub author_name: String,
    pub author_email: String,
    pub committed_at: String,
}

#[derive(Debug, Clone)]
pub struct InteractionSessionSummary {
    pub session: InteractionSession,
    pub turn_count: usize,
    pub turn_ids: Vec<String>,
    pub checkpoint_count: usize,
    pub checkpoint_ids: Vec<String>,
    pub token_usage: Option<TokenUsageMetadata>,
    pub file_paths: Vec<String>,
    pub tool_uses: Vec<InteractionToolUse>,
    pub linked_checkpoints: Vec<InteractionLinkedCheckpoint>,
    pub latest_commit_author: Option<InteractionLinkedCheckpoint>,
}

#[derive(Debug, Clone)]
pub struct InteractionTurnSummary {
    pub turn: InteractionTurn,
    pub tool_uses: Vec<InteractionToolUse>,
    pub linked_checkpoints: Vec<InteractionLinkedCheckpoint>,
    pub latest_commit_author: Option<InteractionLinkedCheckpoint>,
}

#[derive(Debug, Clone)]
pub struct InteractionSessionDetail {
    pub summary: InteractionSessionSummary,
    pub turns: Vec<InteractionTurnSummary>,
    pub raw_events: Vec<InteractionEvent>,
}

#[derive(Debug, Clone)]
pub struct InteractionSessionSearchHit {
    pub session: InteractionSessionSummary,
    pub score: i64,
    pub matched_fields: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct InteractionTurnSearchHit {
    pub turn: InteractionTurnSummary,
    pub session: InteractionSessionSummary,
    pub score: i64,
    pub matched_fields: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InteractionKpis {
    pub total_sessions: usize,
    pub total_turns: usize,
    pub total_checkpoints: usize,
    pub total_tool_uses: usize,
    pub total_actors: usize,
    pub total_agents: usize,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
    pub api_call_count: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InteractionActorBucket {
    pub actor_id: String,
    pub actor_name: String,
    pub actor_email: String,
    pub actor_source: String,
    pub session_count: usize,
    pub turn_count: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InteractionCommitAuthorBucket {
    pub author_name: String,
    pub author_email: String,
    pub checkpoint_count: usize,
    pub session_count: usize,
    pub turn_count: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InteractionAgentBucket {
    pub key: String,
    pub session_count: usize,
    pub turn_count: usize,
}

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
            .then_with(|| left.event_id.cmp(&right.event_id))
    });
    Ok(Some(InteractionSessionDetail {
        summary,
        turns,
        raw_events,
    }))
}

pub(crate) fn search_session_summaries(
    repo_root: &Path,
    input: &InteractionSearchInput,
) -> Result<Vec<InteractionSessionSearchHit>> {
    let state = load_state(repo_root)?;
    let summaries = state
        .session_summaries
        .into_values()
        .filter(|summary| session_matches_filter(summary, &input.filter))
        .collect::<Vec<_>>();
    let allowed = summaries
        .iter()
        .map(|summary| summary.session.session_id.clone())
        .collect::<HashSet<_>>();
    let score_map = score_documents(
        state.spool.as_ref(),
        &state.repo_id,
        &input.query,
        &allowed,
        SearchDocumentKind::Session,
    )?;
    let mut hits = summaries
        .into_iter()
        .filter_map(|summary| {
            score_map
                .get(&summary.session.session_id)
                .map(|score| InteractionSessionSearchHit {
                    session: summary,
                    score: score.score,
                    matched_fields: score.matched_fields.clone(),
                })
        })
        .collect::<Vec<_>>();
    hits.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| session_sort_key(&right.session).cmp(&session_sort_key(&left.session)))
    });
    let limit = search_limit(input.limit);
    hits.truncate(limit);
    Ok(hits)
}

pub(crate) fn search_turn_summaries(
    repo_root: &Path,
    input: &InteractionSearchInput,
) -> Result<Vec<InteractionTurnSearchHit>> {
    let state = load_state(repo_root)?;
    let session_summaries = state.session_summaries.clone();
    let turns = state
        .turn_summaries
        .into_values()
        .filter(|turn| turn_matches_filter(turn, &input.filter))
        .collect::<Vec<_>>();
    let allowed = turns
        .iter()
        .map(|turn| turn.turn.turn_id.clone())
        .collect::<HashSet<_>>();
    let score_map = score_documents(
        state.spool.as_ref(),
        &state.repo_id,
        &input.query,
        &allowed,
        SearchDocumentKind::Turn,
    )?;
    let mut hits = turns
        .into_iter()
        .filter_map(|turn| {
            let score = score_map.get(&turn.turn.turn_id)?;
            let session = session_summaries.get(&turn.turn.session_id)?.clone();
            Some(InteractionTurnSearchHit {
                turn,
                session,
                score: score.score,
                matched_fields: score.matched_fields.clone(),
            })
        })
        .collect::<Vec<_>>();
    hits.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| turn_sort_key(&right.turn).cmp(&turn_sort_key(&left.turn)))
    });
    let limit = search_limit(input.limit);
    hits.truncate(limit);
    Ok(hits)
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
            tool_use_ids.insert(tool_use.tool_use_id.clone());
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
        total_actors: actors.len(),
        total_agents: agents.len(),
        input_tokens: totals.input_tokens,
        output_tokens: totals.output_tokens,
        cache_creation_tokens: totals.cache_creation_tokens,
        cache_read_tokens: totals.cache_read_tokens,
        api_call_count: totals.api_call_count,
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

struct InteractionQueryState {
    repo_id: String,
    spool: Option<SqliteInteractionSpool>,
    session_summaries: HashMap<String, InteractionSessionSummary>,
    turn_summaries: HashMap<String, InteractionTurnSummary>,
    events: Vec<InteractionEvent>,
}

fn load_state(repo_root: &Path) -> Result<InteractionQueryState> {
    let Some(spool) = resolve_interaction_spool(repo_root) else {
        return Ok(InteractionQueryState {
            repo_id: crate::host::devql::resolve_repo_identity(repo_root)
                .map(|identity| identity.repo_id)
                .unwrap_or_default(),
            spool: None,
            session_summaries: HashMap::new(),
            turn_summaries: HashMap::new(),
            events: Vec::new(),
        });
    };
    let repo_id = spool.repo_id().to_string();
    let sessions = spool.list_sessions(None, MAX_INTERACTION_ROWS)?;
    let mut turns = Vec::new();
    for session in &sessions {
        turns.extend(spool.list_turns_for_session(&session.session_id, MAX_INTERACTION_ROWS)?);
    }
    let events = spool.list_events(&Default::default(), MAX_INTERACTION_ROWS)?;
    let tool_uses = load_tool_uses(&spool)?;
    let checkpoint_links = load_checkpoint_links(repo_root, &repo_id)?;

    let mut session_summaries = HashMap::new();
    let mut turn_summaries = HashMap::new();
    let turns_by_session = turns.into_iter().fold(
        HashMap::<String, Vec<InteractionTurn>>::new(),
        |mut map, turn| {
            map.entry(turn.session_id.clone()).or_default().push(turn);
            map
        },
    );
    let tool_uses_by_session = group_tool_uses(tool_uses.iter().cloned(), |tool_use| {
        tool_use.session_id.clone()
    });
    let tool_uses_by_turn = group_tool_uses(tool_uses, |tool_use| tool_use.turn_id.clone());
    for session in sessions {
        let session_turns = turns_by_session
            .get(&session.session_id)
            .cloned()
            .unwrap_or_default();
        let session_tool_uses = tool_uses_by_session
            .get(&session.session_id)
            .cloned()
            .unwrap_or_default();
        let checkpoint_ids = unique_checkpoint_ids(session_turns.iter());
        let linked_checkpoints = checkpoint_ids
            .iter()
            .flat_map(|checkpoint_id| {
                checkpoint_links
                    .get(checkpoint_id)
                    .cloned()
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>();
        let latest_commit_author = linked_checkpoints.first().cloned();
        let summary = InteractionSessionSummary {
            session: session.clone(),
            turn_count: session_turns.len(),
            turn_ids: session_turns
                .iter()
                .map(|turn| turn.turn_id.clone())
                .collect(),
            checkpoint_count: checkpoint_ids.len(),
            checkpoint_ids,
            token_usage: aggregate_token_usage(
                session_turns
                    .iter()
                    .filter_map(|turn| turn.token_usage.as_ref()),
            ),
            file_paths: unique_paths(
                session_turns
                    .iter()
                    .flat_map(|turn| turn.files_modified.iter().cloned()),
            ),
            tool_uses: session_tool_uses,
            linked_checkpoints,
            latest_commit_author,
        };
        session_summaries.insert(session.session_id.clone(), summary);
        for turn in session_turns {
            let linked_checkpoints = turn
                .checkpoint_id
                .as_ref()
                .and_then(|checkpoint_id| checkpoint_links.get(checkpoint_id))
                .cloned()
                .unwrap_or_default();
            let latest_commit_author = linked_checkpoints.first().cloned();
            let summary = InteractionTurnSummary {
                turn: turn.clone(),
                tool_uses: tool_uses_by_turn
                    .get(&turn.turn_id)
                    .cloned()
                    .unwrap_or_default(),
                linked_checkpoints,
                latest_commit_author,
            };
            turn_summaries.insert(turn.turn_id.clone(), summary);
        }
    }

    Ok(InteractionQueryState {
        repo_id,
        spool: Some(spool),
        session_summaries,
        turn_summaries,
        events,
    })
}

fn load_tool_uses(spool: &SqliteInteractionSpool) -> Result<Vec<InteractionToolUse>> {
    spool.with_connection(|conn| {
        let mut stmt = conn.prepare(
            "SELECT tool_use_id, repo_id, session_id, turn_id, tool_kind, task_description,
                    subagent_id, transcript_path, started_at, ended_at, updated_at
             FROM interaction_tool_uses
             WHERE repo_id = ?1
             ORDER BY COALESCE(ended_at, started_at, updated_at) DESC, tool_use_id DESC",
        )?;
        let rows = stmt.query_map(rusqlite::params![spool.repo_id()], |row| {
            Ok(InteractionToolUse {
                tool_use_id: row.get(0)?,
                repo_id: row.get(1)?,
                session_id: row.get(2)?,
                turn_id: row.get(3)?,
                tool_kind: row.get(4)?,
                task_description: row.get(5)?,
                subagent_id: row.get(6)?,
                transcript_path: row.get(7)?,
                started_at: row.get(8)?,
                ended_at: row.get(9)?,
                updated_at: row.get(10)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("reading interaction tool uses")
    })
}

fn load_checkpoint_links(
    repo_root: &Path,
    repo_id: &str,
) -> Result<HashMap<String, Vec<InteractionLinkedCheckpoint>>> {
    let relational = match DefaultRelationalStore::open_local_for_repo_root(repo_root) {
        Ok(store) => store,
        Err(_) => return Ok(HashMap::new()),
    };
    let _ = relational.initialise_local_relational_checkpoint_schema();
    let _ = relational.initialise_local_devql_schema();
    let sqlite = match relational.local_sqlite_pool() {
        Ok(sqlite) => sqlite,
        Err(_) => return Ok(HashMap::new()),
    };
    sqlite.with_connection(|conn| {
        let mut stmt = conn.prepare(
            "SELECT cc.checkpoint_id, cc.commit_sha,
                    COALESCE(c.author_name, ''), COALESCE(c.author_email, ''),
                    COALESCE(c.committed_at, '')
             FROM commit_checkpoints cc
             LEFT JOIN commits c
               ON c.commit_sha = cc.commit_sha AND c.repo_id = cc.repo_id
             WHERE cc.repo_id = ?1
             ORDER BY COALESCE(c.committed_at, '') DESC, cc.checkpoint_id DESC, cc.commit_sha DESC",
        )?;
        let rows = stmt.query_map(rusqlite::params![repo_id], |row| {
            Ok(InteractionLinkedCheckpoint {
                checkpoint_id: row.get(0)?,
                commit_sha: row.get(1)?,
                author_name: row.get(2)?,
                author_email: row.get(3)?,
                committed_at: row.get(4)?,
            })
        })?;
        let mut out: HashMap<String, Vec<InteractionLinkedCheckpoint>> = HashMap::new();
        for row in rows {
            let checkpoint = row?;
            if checkpoint.checkpoint_id.trim().is_empty() {
                continue;
            }
            out.entry(checkpoint.checkpoint_id.clone())
                .or_default()
                .push(checkpoint);
        }
        Ok(out)
    })
}

fn group_tool_uses<I, F>(tool_uses: I, key_fn: F) -> HashMap<String, Vec<InteractionToolUse>>
where
    I: IntoIterator<Item = InteractionToolUse>,
    F: Fn(&InteractionToolUse) -> String,
{
    tool_uses
        .into_iter()
        .fold(HashMap::new(), |mut map, tool_use| {
            let key = key_fn(&tool_use);
            if !key.trim().is_empty() {
                map.entry(key).or_default().push(tool_use);
            }
            map
        })
}

fn unique_checkpoint_ids<'a, I>(turns: I) -> Vec<String>
where
    I: IntoIterator<Item = &'a InteractionTurn>,
{
    let mut seen = BTreeSet::new();
    for turn in turns {
        if let Some(checkpoint_id) = turn
            .checkpoint_id
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            seen.insert(checkpoint_id.to_string());
        }
    }
    seen.into_iter().collect()
}

fn unique_paths<I>(paths: I) -> Vec<String>
where
    I: IntoIterator<Item = String>,
{
    let mut seen = BTreeSet::new();
    for path in paths {
        let path = path.trim();
        if !path.is_empty() {
            seen.insert(path.to_string());
        }
    }
    seen.into_iter().collect()
}

fn aggregate_token_usage<'a, I>(token_usages: I) -> Option<TokenUsageMetadata>
where
    I: IntoIterator<Item = &'a TokenUsageMetadata>,
{
    let mut total = TokenUsageMetadata::default();
    let mut any = false;
    for usage in token_usages {
        any = true;
        total.input_tokens += usage.input_tokens;
        total.output_tokens += usage.output_tokens;
        total.cache_creation_tokens += usage.cache_creation_tokens;
        total.cache_read_tokens += usage.cache_read_tokens;
        total.api_call_count += usage.api_call_count;
    }
    any.then_some(total)
}

fn session_matches_filter(
    summary: &InteractionSessionSummary,
    filter: &InteractionBrowseFilter,
) -> bool {
    if !matches_time_window(
        session_timestamp(summary),
        filter.since.as_deref(),
        filter.until.as_deref(),
    ) {
        return false;
    }
    if !matches_optional_contains(
        [
            summary.session.actor_name.as_str(),
            summary.session.actor_email.as_str(),
            summary.session.actor_id.as_str(),
        ],
        filter.actor.as_deref(),
    ) {
        return false;
    }
    if !matches_optional_equals(
        summary.session.actor_id.as_str(),
        filter.actor_id.as_deref(),
    ) {
        return false;
    }
    if !matches_optional_equals(
        summary.session.actor_email.as_str(),
        filter.actor_email.as_deref(),
    ) {
        return false;
    }
    if !matches_optional_commit_author(
        summary,
        filter.commit_author.as_deref(),
        filter.commit_author_email.as_deref(),
    ) {
        return false;
    }
    if !matches_optional_equals(summary.session.agent_type.as_str(), filter.agent.as_deref()) {
        return false;
    }
    if !matches_optional_equals(summary.session.model.as_str(), filter.model.as_deref()) {
        return false;
    }
    if !matches_optional_equals(summary.session.branch.as_str(), filter.branch.as_deref()) {
        return false;
    }
    if !matches_optional_equals(
        summary.session.session_id.as_str(),
        filter.session_id.as_deref(),
    ) {
        return false;
    }
    if let Some(turn_id) = filter
        .turn_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        && !summary
            .turn_ids
            .iter()
            .any(|candidate| candidate == turn_id)
    {
        return false;
    }
    if let Some(checkpoint_id) = filter
        .checkpoint_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        && !summary.checkpoint_ids.iter().any(|id| id == checkpoint_id)
    {
        return false;
    }
    if let Some(tool_use_id) = filter
        .tool_use_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        && !summary
            .tool_uses
            .iter()
            .any(|tool_use| tool_use.tool_use_id == tool_use_id)
    {
        return false;
    }
    if let Some(tool_kind) = filter
        .tool_kind
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        && !summary
            .tool_uses
            .iter()
            .any(|tool_use| eq_ignore_ascii_case(tool_use.tool_kind.as_str(), tool_kind))
    {
        return false;
    }
    if let Some(has_checkpoint) = filter.has_checkpoint
        && has_checkpoint == summary.checkpoint_ids.is_empty()
    {
        return false;
    }
    if let Some(path) = filter
        .path
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        && !summary
            .file_paths
            .iter()
            .any(|candidate| contains_ignore_ascii_case(candidate, path))
    {
        return false;
    }
    true
}

fn turn_matches_filter(summary: &InteractionTurnSummary, filter: &InteractionBrowseFilter) -> bool {
    if !matches_time_window(
        turn_timestamp(summary),
        filter.since.as_deref(),
        filter.until.as_deref(),
    ) {
        return false;
    }
    if !matches_optional_contains(
        [
            summary.turn.actor_name.as_str(),
            summary.turn.actor_email.as_str(),
            summary.turn.actor_id.as_str(),
        ],
        filter.actor.as_deref(),
    ) {
        return false;
    }
    if !matches_optional_equals(summary.turn.actor_id.as_str(), filter.actor_id.as_deref()) {
        return false;
    }
    if !matches_optional_equals(
        summary.turn.actor_email.as_str(),
        filter.actor_email.as_deref(),
    ) {
        return false;
    }
    if !matches_optional_commit_author_turn(
        summary,
        filter.commit_author.as_deref(),
        filter.commit_author_email.as_deref(),
    ) {
        return false;
    }
    if !matches_optional_equals(summary.turn.agent_type.as_str(), filter.agent.as_deref()) {
        return false;
    }
    if !matches_optional_equals(summary.turn.model.as_str(), filter.model.as_deref()) {
        return false;
    }
    if !matches_optional_equals(summary.turn.branch.as_str(), filter.branch.as_deref()) {
        return false;
    }
    if !matches_optional_equals(
        summary.turn.session_id.as_str(),
        filter.session_id.as_deref(),
    ) {
        return false;
    }
    if !matches_optional_equals(summary.turn.turn_id.as_str(), filter.turn_id.as_deref()) {
        return false;
    }
    if let Some(checkpoint_id) = filter
        .checkpoint_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        && summary.turn.checkpoint_id.as_deref() != Some(checkpoint_id)
    {
        return false;
    }
    if let Some(tool_use_id) = filter
        .tool_use_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        && !summary
            .tool_uses
            .iter()
            .any(|tool_use| tool_use.tool_use_id == tool_use_id)
    {
        return false;
    }
    if let Some(tool_kind) = filter
        .tool_kind
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        && !summary
            .tool_uses
            .iter()
            .any(|tool_use| eq_ignore_ascii_case(tool_use.tool_kind.as_str(), tool_kind))
    {
        return false;
    }
    if let Some(has_checkpoint) = filter.has_checkpoint
        && has_checkpoint != summary.turn.checkpoint_id.is_some()
    {
        return false;
    }
    if let Some(path) = filter
        .path
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        && !summary
            .turn
            .files_modified
            .iter()
            .any(|candidate| contains_ignore_ascii_case(candidate, path))
    {
        return false;
    }
    true
}

fn event_matches_filter(
    event: &InteractionEvent,
    state: &InteractionQueryState,
    filter: &InteractionBrowseFilter,
) -> bool {
    if !matches_time_window(
        &event.event_time,
        filter.since.as_deref(),
        filter.until.as_deref(),
    ) {
        return false;
    }
    if !matches_optional_contains(
        [
            event.actor_name.as_str(),
            event.actor_email.as_str(),
            event.actor_id.as_str(),
        ],
        filter.actor.as_deref(),
    ) {
        return false;
    }
    if !matches_optional_equals(event.actor_id.as_str(), filter.actor_id.as_deref()) {
        return false;
    }
    if !matches_optional_equals(event.actor_email.as_str(), filter.actor_email.as_deref()) {
        return false;
    }
    if !matches_optional_equals(event.agent_type.as_str(), filter.agent.as_deref()) {
        return false;
    }
    if !matches_optional_equals(event.model.as_str(), filter.model.as_deref()) {
        return false;
    }
    if !matches_optional_equals(event.branch.as_str(), filter.branch.as_deref()) {
        return false;
    }
    if !matches_optional_equals(event.session_id.as_str(), filter.session_id.as_deref()) {
        return false;
    }
    if let Some(turn_id) = filter
        .turn_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        && event.turn_id.as_deref() != Some(turn_id)
    {
        return false;
    }
    if !matches_optional_equals(event.tool_use_id.as_str(), filter.tool_use_id.as_deref()) {
        return false;
    }
    if !matches_optional_equals(event.tool_kind.as_str(), filter.tool_kind.as_deref()) {
        return false;
    }
    if filter.commit_author.is_some()
        || filter.commit_author_email.is_some()
        || filter.checkpoint_id.is_some()
        || filter.has_checkpoint.is_some()
        || filter.path.is_some()
    {
        let Some(turn_id) = event.turn_id.as_deref() else {
            return false;
        };
        let Some(turn) = state.turn_summaries.get(turn_id) else {
            return false;
        };
        if !turn_matches_filter(turn, filter) {
            return false;
        }
    }
    true
}

fn matches_optional_commit_author(
    summary: &InteractionSessionSummary,
    commit_author: Option<&str>,
    commit_author_email: Option<&str>,
) -> bool {
    let author_matches = summary.linked_checkpoints.iter().any(|checkpoint| {
        matches_optional_contains(
            [
                checkpoint.author_name.as_str(),
                checkpoint.author_email.as_str(),
            ],
            commit_author,
        ) && matches_optional_equals(checkpoint.author_email.as_str(), commit_author_email)
    });
    commit_author.is_none() && commit_author_email.is_none() || author_matches
}

fn matches_optional_commit_author_turn(
    summary: &InteractionTurnSummary,
    commit_author: Option<&str>,
    commit_author_email: Option<&str>,
) -> bool {
    let author_matches = summary.linked_checkpoints.iter().any(|checkpoint| {
        matches_optional_contains(
            [
                checkpoint.author_name.as_str(),
                checkpoint.author_email.as_str(),
            ],
            commit_author,
        ) && matches_optional_equals(checkpoint.author_email.as_str(), commit_author_email)
    });
    commit_author.is_none() && commit_author_email.is_none() || author_matches
}

fn matches_time_window(value: &str, since: Option<&str>, until: Option<&str>) -> bool {
    let value_dt = parse_rfc3339(value);
    if let Some(since) = since.filter(|candidate| !candidate.trim().is_empty()) {
        match (value_dt, parse_rfc3339(since)) {
            (Some(value_dt), Some(since_dt)) if value_dt < since_dt => return false,
            _ if value < since => return false,
            _ => {}
        }
    }
    if let Some(until) = until.filter(|candidate| !candidate.trim().is_empty()) {
        match (value_dt, parse_rfc3339(until)) {
            (Some(value_dt), Some(until_dt)) if value_dt > until_dt => return false,
            _ if value > until => return false,
            _ => {}
        }
    }
    true
}

fn parse_rfc3339(value: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(value).ok()
}

fn matches_optional_equals(actual: &str, expected: Option<&str>) -> bool {
    let Some(expected) = expected.map(str::trim).filter(|value| !value.is_empty()) else {
        return true;
    };
    eq_ignore_ascii_case(actual, expected)
}

fn matches_optional_contains<'a, I>(values: I, expected: Option<&str>) -> bool
where
    I: IntoIterator<Item = &'a str>,
{
    let Some(expected) = expected.map(str::trim).filter(|value| !value.is_empty()) else {
        return true;
    };
    values
        .into_iter()
        .any(|value| contains_ignore_ascii_case(value, expected))
}

fn contains_ignore_ascii_case(left: &str, right: &str) -> bool {
    left.to_ascii_lowercase()
        .contains(&right.to_ascii_lowercase())
}

fn eq_ignore_ascii_case(left: &str, right: &str) -> bool {
    left.trim().eq_ignore_ascii_case(right.trim())
}

fn session_timestamp(summary: &InteractionSessionSummary) -> &str {
    if summary.session.last_event_at.trim().is_empty() {
        summary.session.started_at.as_str()
    } else {
        summary.session.last_event_at.as_str()
    }
}

fn turn_timestamp(summary: &InteractionTurnSummary) -> &str {
    summary
        .turn
        .ended_at
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(summary.turn.started_at.as_str())
}

fn session_sort_key(summary: &InteractionSessionSummary) -> (String, String) {
    (
        session_timestamp(summary).to_string(),
        summary.session.session_id.clone(),
    )
}

fn turn_sort_key(summary: &InteractionTurnSummary) -> (String, String) {
    (
        turn_timestamp(summary).to_string(),
        summary.turn.turn_id.clone(),
    )
}

fn search_limit(limit: usize) -> usize {
    limit.clamp(1, 200)
}

#[derive(Clone, Copy)]
enum SearchDocumentKind {
    Session,
    Turn,
}

struct SearchScore {
    score: i64,
    matched_fields: Vec<String>,
}

fn score_documents(
    spool: Option<&SqliteInteractionSpool>,
    repo_id: &str,
    query: &str,
    allowed_ids: &HashSet<String>,
    kind: SearchDocumentKind,
) -> Result<HashMap<String, SearchScore>> {
    let Some(spool) = spool else {
        return Ok(HashMap::new());
    };
    let terms = tokenise(query);
    if terms.is_empty() {
        return Ok(allowed_ids
            .iter()
            .map(|id| {
                (
                    id.clone(),
                    SearchScore {
                        score: 0,
                        matched_fields: Vec::new(),
                    },
                )
            })
            .collect());
    }
    spool.with_connection(|conn| {
        let id_column = match kind {
            SearchDocumentKind::Session => "session_id",
            SearchDocumentKind::Turn => "turn_id",
        };
        let table = match kind {
            SearchDocumentKind::Session => "interaction_session_search_terms",
            SearchDocumentKind::Turn => "interaction_turn_search_terms",
        };
        let placeholders = (0..terms.len())
            .map(|index| format!("?{}", index + 2))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT {id_column}, field, SUM(occurrences) AS occurrences
             FROM {table}
             WHERE repo_id = ?1 AND term IN ({placeholders})
             GROUP BY {id_column}, field"
        );
        let mut params: Vec<&dyn rusqlite::types::ToSql> = vec![&repo_id];
        for term in &terms {
            params.push(term);
        }
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params.as_slice(), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("reading interaction lexical search scores")?;
        let mut scores: HashMap<String, SearchScore> = HashMap::new();
        for (document_id, field, occurrences) in rows {
            if !allowed_ids.contains(&document_id) {
                continue;
            }
            let score = field_weight(field.as_str()) * occurrences;
            let entry = scores.entry(document_id).or_insert_with(|| SearchScore {
                score: 0,
                matched_fields: Vec::new(),
            });
            entry.score += score;
            if !entry
                .matched_fields
                .iter()
                .any(|existing| existing == &field)
            {
                entry.matched_fields.push(field);
            }
        }
        Ok(scores)
    })
}

fn field_weight(field: &str) -> i64 {
    match field {
        FIELD_PROMPT => 8,
        FIELD_SUMMARY => 5,
        FIELD_TOOL => 5,
        FIELD_PATH => 3,
        FIELD_TRANSCRIPT => 1,
        _ => 1,
    }
}

fn tokenise(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in input.chars().flat_map(|ch| ch.to_lowercase()) {
        if ch.is_alphanumeric() {
            current.push(ch);
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

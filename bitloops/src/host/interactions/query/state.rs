use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use anyhow::{Context, Result};

use super::types::{
    InteractionLinkedCheckpoint, InteractionSessionSummary, InteractionTurnSummary,
};
use crate::host::checkpoints::lifecycle::interaction::resolve_interaction_spool;
use crate::host::checkpoints::strategy::manual_commit::TokenUsageMetadata;
use crate::host::interactions::db_store::SqliteInteractionSpool;
use crate::host::interactions::store::InteractionSpool;
use crate::host::interactions::types::{InteractionEvent, InteractionToolUse, InteractionTurn};
use crate::host::relational_store::{DefaultRelationalStore, RelationalStore};

const MAX_INTERACTION_ROWS: usize = 1_000_000;

pub(super) struct InteractionQueryState {
    pub(super) repo_id: String,
    pub(super) spool: Option<SqliteInteractionSpool>,
    pub(super) session_summaries: HashMap<String, InteractionSessionSummary>,
    pub(super) turn_summaries: HashMap<String, InteractionTurnSummary>,
    pub(super) events: Vec<InteractionEvent>,
}

pub(super) fn load_state(repo_root: &Path) -> Result<InteractionQueryState> {
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

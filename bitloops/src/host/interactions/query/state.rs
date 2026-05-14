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
use crate::host::interactions::types::{
    InteractionEvent, InteractionSubagentRun, InteractionToolInvocation, InteractionTurn,
};
use crate::host::relational_store::{DefaultRelationalStore, RelationalStore};
use crate::storage::SqliteConnectionPool;

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
    let subagent_runs = load_subagent_runs(&spool)?;
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
    let subagent_runs_by_session =
        group_subagent_runs(subagent_runs.iter().cloned(), |run| run.session_id.clone());
    let subagent_runs_by_turn = group_subagent_runs(subagent_runs, |run| run.turn_id.clone());

    for session in sessions {
        let session_turns = turns_by_session
            .get(&session.session_id)
            .cloned()
            .unwrap_or_default();
        let session_tool_uses = tool_uses_by_session
            .get(&session.session_id)
            .cloned()
            .unwrap_or_default();
        let session_subagent_runs = subagent_runs_by_session
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
            subagent_runs: session_subagent_runs,
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
                subagent_runs: subagent_runs_by_turn
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

fn load_tool_uses(spool: &SqliteInteractionSpool) -> Result<Vec<InteractionToolInvocation>> {
    spool.with_connection(|conn| {
        let mut stmt = conn.prepare(
            "SELECT tool_invocation_id, repo_id, session_id, turn_id, tool_use_id, tool_name,
                    source, input_summary, output_summary, command, command_binary, command_argv,
                    transcript_path, started_at, ended_at, started_sequence_number,
                    ended_sequence_number, updated_at
             FROM interaction_tool_invocations
             WHERE repo_id = ?1
             ORDER BY COALESCE(started_sequence_number, ended_sequence_number, 0) DESC,
                      COALESCE(ended_at, started_at, updated_at) DESC,
                      tool_invocation_id DESC",
        )?;
        let rows = stmt.query_map(rusqlite::params![spool.repo_id()], |row| {
            let command_argv_raw: String = row.get(11)?;
            let command_argv =
                serde_json::from_str::<Vec<String>>(&command_argv_raw).map_err(|err| {
                    rusqlite::Error::FromSqlConversionFailure(
                        11,
                        rusqlite::types::Type::Text,
                        Box::new(err),
                    )
                })?;
            Ok(InteractionToolInvocation {
                tool_invocation_id: row.get(0)?,
                repo_id: row.get(1)?,
                session_id: row.get(2)?,
                turn_id: row.get(3)?,
                tool_use_id: row.get(4)?,
                tool_name: row.get(5)?,
                source: row.get(6)?,
                input_summary: row.get(7)?,
                output_summary: row.get(8)?,
                command: row.get(9)?,
                command_binary: row.get(10)?,
                command_argv,
                transcript_path: row.get(12)?,
                started_at: row.get(13)?,
                ended_at: row.get(14)?,
                started_sequence_number: row.get(15)?,
                ended_sequence_number: row.get(16)?,
                updated_at: row.get(17)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("reading interaction tool uses")
    })
}

fn load_subagent_runs(spool: &SqliteInteractionSpool) -> Result<Vec<InteractionSubagentRun>> {
    spool.with_connection(|conn| {
        let mut stmt = conn.prepare(
            "SELECT subagent_run_id, repo_id, session_id, turn_id, tool_use_id, subagent_id,
                    subagent_type, task_description, source, transcript_path, child_session_id,
                    started_at, ended_at, started_sequence_number, ended_sequence_number, updated_at
             FROM interaction_subagent_runs
             WHERE repo_id = ?1
             ORDER BY COALESCE(started_sequence_number, ended_sequence_number, 0) DESC,
                      COALESCE(ended_at, started_at, updated_at) DESC,
                      subagent_run_id DESC",
        )?;
        let rows = stmt.query_map(rusqlite::params![spool.repo_id()], |row| {
            Ok(InteractionSubagentRun {
                subagent_run_id: row.get(0)?,
                repo_id: row.get(1)?,
                session_id: row.get(2)?,
                turn_id: row.get(3)?,
                tool_use_id: row.get(4)?,
                subagent_id: row.get(5)?,
                subagent_type: row.get(6)?,
                task_description: row.get(7)?,
                source: row.get(8)?,
                transcript_path: row.get(9)?,
                child_session_id: row.get(10)?,
                started_at: row.get(11)?,
                ended_at: row.get(12)?,
                started_sequence_number: row.get(13)?,
                ended_sequence_number: row.get(14)?,
                updated_at: row.get(15)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("reading interaction subagent runs")
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
    match query_checkpoint_links(&relational, repo_id) {
        Ok(rows) => Ok(rows),
        Err(err) if checkpoint_link_bootstrap_needed(&err) => {
            bootstrap_checkpoint_link_schema(&relational)?;
            match query_checkpoint_links(&relational, repo_id) {
                Ok(rows) => Ok(rows),
                Err(retry_err) if checkpoint_link_bootstrap_needed(&retry_err) => {
                    Ok(HashMap::new())
                }
                Err(retry_err) => Err(retry_err),
            }
        }
        Err(err) => Err(err),
    }
}

fn query_checkpoint_links(
    relational: &DefaultRelationalStore,
    repo_id: &str,
) -> Result<HashMap<String, Vec<InteractionLinkedCheckpoint>>> {
    let sqlite = relational.local_sqlite_pool()?;
    query_checkpoint_links_sqlite(&sqlite, repo_id)
}

fn query_checkpoint_links_sqlite(
    sqlite: &SqliteConnectionPool,
    repo_id: &str,
) -> Result<HashMap<String, Vec<InteractionLinkedCheckpoint>>> {
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

fn bootstrap_checkpoint_link_schema(relational: &DefaultRelationalStore) -> Result<()> {
    relational
        .initialise_local_devql_schema()
        .context("initialising local DevQL schema for interaction checkpoint links")?;
    relational
        .initialise_local_relational_checkpoint_schema()
        .context("initialising local relational checkpoint schema for interaction checkpoint links")
}

fn checkpoint_link_bootstrap_needed(err: &anyhow::Error) -> bool {
    let message = err.to_string();
    message.contains("SQLite database file not found at")
        || message.contains("no such table: commit_checkpoints")
        || message.contains("no such table: commits")
}

fn group_tool_uses<I, F>(tool_uses: I, key_fn: F) -> HashMap<String, Vec<InteractionToolInvocation>>
where
    I: IntoIterator<Item = InteractionToolInvocation>,
    F: Fn(&InteractionToolInvocation) -> String,
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

fn group_subagent_runs<I, F>(
    subagent_runs: I,
    key_fn: F,
) -> HashMap<String, Vec<InteractionSubagentRun>>
where
    I: IntoIterator<Item = InteractionSubagentRun>,
    F: Fn(&InteractionSubagentRun) -> String,
{
    subagent_runs
        .into_iter()
        .fold(HashMap::new(), |mut map, run| {
            let key = key_fn(&run);
            if !key.trim().is_empty() {
                map.entry(key).or_default().push(run);
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

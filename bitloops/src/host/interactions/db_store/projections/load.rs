use anyhow::{Context, Result};
use rusqlite::OptionalExtension;

use super::super::row_mapping::{map_event_row, map_session_row, map_turn_row};
use crate::host::interactions::types::{
    InteractionEvent, InteractionSession, InteractionSubagentRun, InteractionToolInvocation,
    InteractionTurn,
};

pub(super) fn load_session_ids(conn: &rusqlite::Connection, repo_id: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT session_id
         FROM interaction_sessions
         WHERE repo_id = ?1
         ORDER BY COALESCE(NULLIF(last_event_at, ''), started_at) DESC, session_id DESC",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![repo_id], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("reading interaction session ids for projections")?;
    Ok(rows)
}

pub(super) fn load_turn_ids(conn: &rusqlite::Connection, repo_id: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT turn_id
         FROM interaction_turns
         WHERE repo_id = ?1
         ORDER BY session_id ASC, turn_number ASC, started_at ASC, turn_id ASC",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![repo_id], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("reading interaction turn ids for projections")?;
    Ok(rows)
}

pub(super) fn load_events_for_repo(
    conn: &rusqlite::Connection,
    repo_id: &str,
) -> Result<Vec<InteractionEvent>> {
    let mut stmt = conn.prepare(
        "SELECT event_id, session_id, turn_id, repo_id, branch, actor_id, actor_name, actor_email,
                actor_source, event_type, event_time, source, sequence_number, agent_type, model,
                tool_use_id, tool_kind, task_description, subagent_id, payload
         FROM interaction_events
         WHERE repo_id = ?1
         ORDER BY event_time ASC, sequence_number ASC, event_id ASC",
    )?;
    let rows = stmt.query_map(rusqlite::params![repo_id], map_event_row)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .context("reading interaction events for projections")
}

pub(super) fn load_session(
    conn: &rusqlite::Connection,
    repo_id: &str,
    session_id: &str,
) -> Result<Option<InteractionSession>> {
    let mut stmt = conn.prepare(
        "SELECT session_id, repo_id, branch, actor_id, actor_name, actor_email, actor_source,
                agent_type, model, first_prompt, transcript_path, worktree_path, worktree_id,
                started_at, ended_at, last_event_at, updated_at
         FROM interaction_sessions
         WHERE repo_id = ?1 AND session_id = ?2
         LIMIT 1",
    )?;
    stmt.query_row(rusqlite::params![repo_id, session_id], map_session_row)
        .optional()
        .map_err(anyhow::Error::from)
}

pub(super) fn load_turn(
    conn: &rusqlite::Connection,
    repo_id: &str,
    turn_id: &str,
) -> Result<Option<InteractionTurn>> {
    let mut stmt = conn.prepare(
        "SELECT turn_id, session_id, repo_id, branch, actor_id, actor_name, actor_email,
                actor_source, turn_number, prompt, agent_type, model, started_at, ended_at,
                has_token_usage, input_tokens, cache_creation_tokens, cache_read_tokens,
                output_tokens, api_call_count, summary, prompt_count, transcript_offset_start,
                transcript_offset_end, transcript_fragment, files_modified, checkpoint_id, updated_at
         FROM interaction_turns
         WHERE repo_id = ?1 AND turn_id = ?2
         LIMIT 1",
    )?;
    stmt.query_row(rusqlite::params![repo_id, turn_id], map_turn_row)
        .optional()
        .map_err(anyhow::Error::from)
}

pub(super) fn load_turns_for_session(
    conn: &rusqlite::Connection,
    repo_id: &str,
    session_id: &str,
) -> Result<Vec<InteractionTurn>> {
    let mut stmt = conn.prepare(
        "SELECT turn_id, session_id, repo_id, branch, actor_id, actor_name, actor_email,
                actor_source, turn_number, prompt, agent_type, model, started_at, ended_at,
                has_token_usage, input_tokens, cache_creation_tokens, cache_read_tokens,
                output_tokens, api_call_count, summary, prompt_count, transcript_offset_start,
                transcript_offset_end, transcript_fragment, files_modified, checkpoint_id, updated_at
         FROM interaction_turns
         WHERE repo_id = ?1 AND session_id = ?2
         ORDER BY turn_number ASC, started_at ASC, turn_id ASC",
    )?;
    let rows = stmt.query_map(rusqlite::params![repo_id, session_id], map_turn_row)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .context("reading interaction turns for session projections")
}

pub(super) fn load_tool_uses_for_turn(
    conn: &rusqlite::Connection,
    repo_id: &str,
    turn_id: &str,
) -> Result<Vec<InteractionToolInvocation>> {
    let mut stmt = conn.prepare(
        "SELECT tool_invocation_id, repo_id, session_id, turn_id, tool_use_id, tool_name, source,
                input_summary, output_summary, command, command_binary, command_argv,
                transcript_path, started_at, ended_at, started_sequence_number,
                ended_sequence_number, updated_at
         FROM interaction_tool_invocations
         WHERE repo_id = ?1 AND turn_id = ?2
         ORDER BY COALESCE(started_sequence_number, ended_sequence_number, 0) ASC,
                  COALESCE(ended_at, started_at, updated_at) ASC,
                  tool_invocation_id ASC",
    )?;
    let rows = stmt.query_map(rusqlite::params![repo_id, turn_id], map_tool_use_row)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .context("reading interaction tool uses for turn projections")
}

pub(super) fn load_tool_uses_for_session(
    conn: &rusqlite::Connection,
    repo_id: &str,
    session_id: &str,
) -> Result<Vec<InteractionToolInvocation>> {
    let mut stmt = conn.prepare(
        "SELECT tool_invocation_id, repo_id, session_id, turn_id, tool_use_id, tool_name, source,
                input_summary, output_summary, command, command_binary, command_argv,
                transcript_path, started_at, ended_at, started_sequence_number,
                ended_sequence_number, updated_at
         FROM interaction_tool_invocations
         WHERE repo_id = ?1 AND session_id = ?2
         ORDER BY COALESCE(started_sequence_number, ended_sequence_number, 0) ASC,
                  COALESCE(ended_at, started_at, updated_at) ASC,
                  tool_invocation_id ASC",
    )?;
    let rows = stmt.query_map(rusqlite::params![repo_id, session_id], map_tool_use_row)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .context("reading interaction tool uses for session projections")
}

pub(super) fn load_subagent_runs_for_turn(
    conn: &rusqlite::Connection,
    repo_id: &str,
    turn_id: &str,
) -> Result<Vec<InteractionSubagentRun>> {
    let mut stmt = conn.prepare(
        "SELECT subagent_run_id, repo_id, session_id, turn_id, tool_use_id, subagent_id,
                subagent_type, task_description, source, transcript_path, child_session_id,
                started_at, ended_at, started_sequence_number, ended_sequence_number, updated_at
         FROM interaction_subagent_runs
         WHERE repo_id = ?1 AND turn_id = ?2
         ORDER BY COALESCE(started_sequence_number, ended_sequence_number, 0) ASC,
                  COALESCE(ended_at, started_at, updated_at) ASC,
                  subagent_run_id ASC",
    )?;
    let rows = stmt.query_map(rusqlite::params![repo_id, turn_id], map_subagent_run_row)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .context("reading interaction subagent runs for turn projections")
}

pub(super) fn load_subagent_runs_for_session(
    conn: &rusqlite::Connection,
    repo_id: &str,
    session_id: &str,
) -> Result<Vec<InteractionSubagentRun>> {
    let mut stmt = conn.prepare(
        "SELECT subagent_run_id, repo_id, session_id, turn_id, tool_use_id, subagent_id,
                subagent_type, task_description, source, transcript_path, child_session_id,
                started_at, ended_at, started_sequence_number, ended_sequence_number, updated_at
         FROM interaction_subagent_runs
         WHERE repo_id = ?1 AND session_id = ?2
         ORDER BY COALESCE(started_sequence_number, ended_sequence_number, 0) ASC,
                  COALESCE(ended_at, started_at, updated_at) ASC,
                  subagent_run_id ASC",
    )?;
    let rows = stmt.query_map(rusqlite::params![repo_id, session_id], map_subagent_run_row)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .context("reading interaction subagent runs for session projections")
}

fn map_tool_use_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<InteractionToolInvocation> {
    let command_argv_raw: String = row.get(11)?;
    let command_argv = serde_json::from_str::<Vec<String>>(&command_argv_raw).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(11, rusqlite::types::Type::Text, Box::new(err))
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
}

fn map_subagent_run_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<InteractionSubagentRun> {
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
}

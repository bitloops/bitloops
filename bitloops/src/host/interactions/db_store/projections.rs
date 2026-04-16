use std::collections::{BTreeSet, HashMap};

use anyhow::{Context, Result};
use rusqlite::OptionalExtension;

use super::row_mapping::{map_event_row, map_session_row, map_turn_row};
use crate::host::interactions::types::{
    InteractionEvent, InteractionEventType, InteractionToolUse,
};

const MAX_PROMPT_TEXT_CHARS: usize = 4_000;
const MAX_SUMMARY_TEXT_CHARS: usize = 4_000;
const MAX_TRANSCRIPT_TEXT_CHARS: usize = 8_000;
const MAX_TOOL_TEXT_CHARS: usize = 4_000;
const MAX_PATH_TEXT_CHARS: usize = 4_000;

const FIELD_PROMPT: &str = "prompt";
const FIELD_SUMMARY: &str = "summary";
const FIELD_TRANSCRIPT: &str = "transcript";
const FIELD_TOOL: &str = "tool";
const FIELD_PATH: &str = "path";

pub(super) fn rebuild_all_projections(conn: &rusqlite::Connection, repo_id: &str) -> Result<()> {
    let events = load_events_for_repo(conn, repo_id)?;
    for event in &events {
        upsert_tool_use_from_event(conn, repo_id, event)?;
    }

    let session_ids = load_session_ids(conn, repo_id)?;
    let turn_ids = load_turn_ids(conn, repo_id)?;
    for turn_id in &turn_ids {
        refresh_turn_search_document(conn, repo_id, turn_id)?;
    }
    for session_id in &session_ids {
        refresh_session_search_document(conn, repo_id, session_id)?;
    }
    Ok(())
}

pub(super) fn refresh_session_after_upsert(
    conn: &rusqlite::Connection,
    repo_id: &str,
    session_id: &str,
) -> Result<()> {
    refresh_session_search_document(conn, repo_id, session_id)
}

pub(super) fn refresh_turn_after_upsert(
    conn: &rusqlite::Connection,
    repo_id: &str,
    session_id: &str,
    turn_id: &str,
) -> Result<()> {
    refresh_turn_search_document(conn, repo_id, turn_id)?;
    refresh_session_search_document(conn, repo_id, session_id)
}

pub(super) fn refresh_after_event(
    conn: &rusqlite::Connection,
    repo_id: &str,
    event: &InteractionEvent,
) -> Result<()> {
    upsert_tool_use_from_event(conn, repo_id, event)?;
    if let Some(turn_id) = event.turn_id.as_deref().filter(|value| !value.is_empty()) {
        refresh_turn_search_document(conn, repo_id, turn_id)?;
    }
    refresh_session_search_document(conn, repo_id, &event.session_id)
}

pub(super) fn refresh_after_checkpoint_assignment(
    conn: &rusqlite::Connection,
    repo_id: &str,
    session_ids: &[String],
) -> Result<()> {
    for session_id in session_ids {
        refresh_session_search_document(conn, repo_id, session_id)?;
    }
    Ok(())
}

fn load_session_ids(conn: &rusqlite::Connection, repo_id: &str) -> Result<Vec<String>> {
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

fn load_turn_ids(conn: &rusqlite::Connection, repo_id: &str) -> Result<Vec<String>> {
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

fn load_events_for_repo(
    conn: &rusqlite::Connection,
    repo_id: &str,
) -> Result<Vec<InteractionEvent>> {
    let mut stmt = conn.prepare(
        "SELECT event_id, session_id, turn_id, repo_id, branch, actor_id, actor_name, actor_email,
                actor_source, event_type, event_time, agent_type, model, tool_use_id, tool_kind,
                task_description, subagent_id, payload
         FROM interaction_events
         WHERE repo_id = ?1
         ORDER BY event_time ASC, event_id ASC",
    )?;
    let rows = stmt.query_map(rusqlite::params![repo_id], map_event_row)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .context("reading interaction events for projections")
}

fn load_session(
    conn: &rusqlite::Connection,
    repo_id: &str,
    session_id: &str,
) -> Result<Option<crate::host::interactions::types::InteractionSession>> {
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

fn load_turn(
    conn: &rusqlite::Connection,
    repo_id: &str,
    turn_id: &str,
) -> Result<Option<crate::host::interactions::types::InteractionTurn>> {
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

fn load_turns_for_session(
    conn: &rusqlite::Connection,
    repo_id: &str,
    session_id: &str,
) -> Result<Vec<crate::host::interactions::types::InteractionTurn>> {
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

fn load_tool_uses_for_turn(
    conn: &rusqlite::Connection,
    repo_id: &str,
    turn_id: &str,
) -> Result<Vec<InteractionToolUse>> {
    let mut stmt = conn.prepare(
        "SELECT tool_use_id, repo_id, session_id, turn_id, tool_kind, task_description,
                subagent_id, transcript_path, started_at, ended_at, updated_at
         FROM interaction_tool_uses
         WHERE repo_id = ?1 AND turn_id = ?2
         ORDER BY COALESCE(ended_at, started_at, updated_at) ASC, tool_use_id ASC",
    )?;
    let rows = stmt.query_map(rusqlite::params![repo_id, turn_id], map_tool_use_row)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .context("reading interaction tool uses for turn projections")
}

fn load_tool_uses_for_session(
    conn: &rusqlite::Connection,
    repo_id: &str,
    session_id: &str,
) -> Result<Vec<InteractionToolUse>> {
    let mut stmt = conn.prepare(
        "SELECT tool_use_id, repo_id, session_id, turn_id, tool_kind, task_description,
                subagent_id, transcript_path, started_at, ended_at, updated_at
         FROM interaction_tool_uses
         WHERE repo_id = ?1 AND session_id = ?2
         ORDER BY COALESCE(ended_at, started_at, updated_at) ASC, tool_use_id ASC",
    )?;
    let rows = stmt.query_map(rusqlite::params![repo_id, session_id], map_tool_use_row)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .context("reading interaction tool uses for session projections")
}

fn map_tool_use_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<InteractionToolUse> {
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
}

fn refresh_turn_search_document(
    conn: &rusqlite::Connection,
    repo_id: &str,
    turn_id: &str,
) -> Result<()> {
    let Some(turn) = load_turn(conn, repo_id, turn_id)? else {
        conn.execute(
            "DELETE FROM interaction_turn_search_documents WHERE repo_id = ?1 AND turn_id = ?2",
            rusqlite::params![repo_id, turn_id],
        )?;
        conn.execute(
            "DELETE FROM interaction_turn_search_terms WHERE repo_id = ?1 AND turn_id = ?2",
            rusqlite::params![repo_id, turn_id],
        )?;
        return Ok(());
    };
    let tool_uses = load_tool_uses_for_turn(conn, repo_id, turn_id)?;
    let prompt_text = bounded_text(&turn.prompt, MAX_PROMPT_TEXT_CHARS);
    let summary_text = bounded_text(&turn.summary, MAX_SUMMARY_TEXT_CHARS);
    let transcript_text = bounded_text(&turn.transcript_fragment, MAX_TRANSCRIPT_TEXT_CHARS);
    let tool_text = bounded_join(
        tool_uses.iter().map(tool_use_search_text),
        MAX_TOOL_TEXT_CHARS,
    );
    let paths_text = bounded_join(turn.files_modified.iter().cloned(), MAX_PATH_TEXT_CHARS);
    let combined_text = bounded_join(
        [
            prompt_text.clone(),
            summary_text.clone(),
            transcript_text.clone(),
            tool_text.clone(),
            paths_text.clone(),
        ],
        MAX_PROMPT_TEXT_CHARS
            + MAX_SUMMARY_TEXT_CHARS
            + MAX_TRANSCRIPT_TEXT_CHARS
            + MAX_TOOL_TEXT_CHARS
            + MAX_PATH_TEXT_CHARS,
    );

    conn.execute(
        "INSERT INTO interaction_turn_search_documents (
            turn_id, repo_id, session_id, started_at, updated_at, prompt_text, summary_text,
            transcript_text, tool_text, paths_text, combined_text
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT(repo_id, turn_id) DO UPDATE SET
            session_id = excluded.session_id,
            started_at = excluded.started_at,
            updated_at = excluded.updated_at,
            prompt_text = excluded.prompt_text,
            summary_text = excluded.summary_text,
            transcript_text = excluded.transcript_text,
            tool_text = excluded.tool_text,
            paths_text = excluded.paths_text,
            combined_text = excluded.combined_text",
        rusqlite::params![
            turn.turn_id,
            repo_id,
            turn.session_id,
            turn.started_at,
            turn.updated_at,
            prompt_text,
            summary_text,
            transcript_text,
            tool_text,
            paths_text,
            combined_text,
        ],
    )
    .context("upserting interaction turn search document")?;
    replace_turn_terms(
        conn,
        repo_id,
        &turn.turn_id,
        [
            (FIELD_PROMPT, turn.prompt.as_str()),
            (FIELD_SUMMARY, turn.summary.as_str()),
            (FIELD_TRANSCRIPT, turn.transcript_fragment.as_str()),
            (FIELD_TOOL, tool_text.as_str()),
            (FIELD_PATH, paths_text.as_str()),
        ],
    )?;
    Ok(())
}

fn refresh_session_search_document(
    conn: &rusqlite::Connection,
    repo_id: &str,
    session_id: &str,
) -> Result<()> {
    let Some(session) = load_session(conn, repo_id, session_id)? else {
        conn.execute(
            "DELETE FROM interaction_session_search_documents
             WHERE repo_id = ?1 AND session_id = ?2",
            rusqlite::params![repo_id, session_id],
        )?;
        conn.execute(
            "DELETE FROM interaction_session_search_terms
             WHERE repo_id = ?1 AND session_id = ?2",
            rusqlite::params![repo_id, session_id],
        )?;
        return Ok(());
    };
    let turns = load_turns_for_session(conn, repo_id, session_id)?;
    let tool_uses = load_tool_uses_for_session(conn, repo_id, session_id)?;
    let prompt_text = bounded_join(
        std::iter::once(session.first_prompt.clone())
            .chain(turns.iter().map(|turn| turn.prompt.clone())),
        MAX_PROMPT_TEXT_CHARS,
    );
    let summary_text = bounded_join(
        turns.iter().map(|turn| turn.summary.clone()),
        MAX_SUMMARY_TEXT_CHARS,
    );
    let transcript_text = bounded_join(
        turns.iter().map(|turn| turn.transcript_fragment.clone()),
        MAX_TRANSCRIPT_TEXT_CHARS,
    );
    let tool_text = bounded_join(
        tool_uses.iter().map(tool_use_search_text),
        MAX_TOOL_TEXT_CHARS,
    );
    let paths_text = bounded_join(
        unique_paths(
            turns
                .iter()
                .flat_map(|turn| turn.files_modified.iter().cloned()),
        ),
        MAX_PATH_TEXT_CHARS,
    );
    let combined_text = bounded_join(
        [
            prompt_text.clone(),
            summary_text.clone(),
            transcript_text.clone(),
            tool_text.clone(),
            paths_text.clone(),
        ],
        MAX_PROMPT_TEXT_CHARS
            + MAX_SUMMARY_TEXT_CHARS
            + MAX_TRANSCRIPT_TEXT_CHARS
            + MAX_TOOL_TEXT_CHARS
            + MAX_PATH_TEXT_CHARS,
    );

    conn.execute(
        "INSERT INTO interaction_session_search_documents (
            session_id, repo_id, started_at, updated_at, prompt_text, summary_text,
            transcript_text, tool_text, paths_text, combined_text
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(repo_id, session_id) DO UPDATE SET
            started_at = excluded.started_at,
            updated_at = excluded.updated_at,
            prompt_text = excluded.prompt_text,
            summary_text = excluded.summary_text,
            transcript_text = excluded.transcript_text,
            tool_text = excluded.tool_text,
            paths_text = excluded.paths_text,
            combined_text = excluded.combined_text",
        rusqlite::params![
            session.session_id,
            repo_id,
            session.started_at,
            session.updated_at,
            prompt_text,
            summary_text,
            transcript_text,
            tool_text,
            paths_text,
            combined_text,
        ],
    )
    .context("upserting interaction session search document")?;
    replace_session_terms(
        conn,
        repo_id,
        &session.session_id,
        [
            (FIELD_PROMPT, prompt_text.as_str()),
            (FIELD_SUMMARY, summary_text.as_str()),
            (FIELD_TRANSCRIPT, transcript_text.as_str()),
            (FIELD_TOOL, tool_text.as_str()),
            (FIELD_PATH, paths_text.as_str()),
        ],
    )?;
    Ok(())
}

fn replace_turn_terms<'a, I>(
    conn: &rusqlite::Connection,
    repo_id: &str,
    turn_id: &str,
    fields: I,
) -> Result<()>
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    conn.execute(
        "DELETE FROM interaction_turn_search_terms WHERE repo_id = ?1 AND turn_id = ?2",
        rusqlite::params![repo_id, turn_id],
    )?;
    insert_terms(
        conn,
        "interaction_turn_search_terms",
        repo_id,
        turn_id,
        fields,
    )
}

fn replace_session_terms<'a, I>(
    conn: &rusqlite::Connection,
    repo_id: &str,
    session_id: &str,
    fields: I,
) -> Result<()>
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    conn.execute(
        "DELETE FROM interaction_session_search_terms WHERE repo_id = ?1 AND session_id = ?2",
        rusqlite::params![repo_id, session_id],
    )?;
    insert_terms(
        conn,
        "interaction_session_search_terms",
        repo_id,
        session_id,
        fields,
    )
}

fn insert_terms<'a, I>(
    conn: &rusqlite::Connection,
    table: &str,
    repo_id: &str,
    document_id: &str,
    fields: I,
) -> Result<()>
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    let mut counts: HashMap<(String, String), i64> = HashMap::new();
    for (field, text) in fields {
        for term in tokenise(text) {
            *counts.entry((term, field.to_string())).or_insert(0) += 1;
        }
    }
    if counts.is_empty() {
        return Ok(());
    }
    let sql = format!(
        "INSERT INTO {table} (repo_id, {}, term, field, occurrences)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(repo_id, {}, term, field) DO UPDATE SET
            occurrences = excluded.occurrences",
        if table.contains("_session_") {
            "session_id"
        } else {
            "turn_id"
        },
        if table.contains("_session_") {
            "session_id"
        } else {
            "turn_id"
        },
    );
    let mut stmt = conn.prepare(&sql)?;
    for ((term, field), occurrences) in counts {
        stmt.execute(rusqlite::params![
            repo_id,
            document_id,
            term,
            field,
            occurrences
        ])?;
    }
    Ok(())
}

fn upsert_tool_use_from_event(
    conn: &rusqlite::Connection,
    repo_id: &str,
    event: &InteractionEvent,
) -> Result<()> {
    let tool_use_id = event_tool_use_id(event);
    if tool_use_id.is_empty() {
        return Ok(());
    }
    let tool_kind = event_tool_kind(event);
    let task_description = event_task_description(event);
    let subagent_id = event_subagent_id(event);
    let transcript_path = payload_string_field(&event.payload, "subagent_transcript_path");
    let started_at = matches!(event.event_type, InteractionEventType::SubagentStart)
        .then(|| event.event_time.clone());
    let ended_at = matches!(event.event_type, InteractionEventType::SubagentEnd)
        .then(|| event.event_time.clone());
    conn.execute(
        "INSERT INTO interaction_tool_uses (
            tool_use_id, repo_id, session_id, turn_id, tool_kind, task_description,
            subagent_id, transcript_path, started_at, ended_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT(repo_id, tool_use_id) DO UPDATE SET
            session_id = CASE
                WHEN excluded.session_id = '' THEN interaction_tool_uses.session_id
                ELSE excluded.session_id
            END,
            turn_id = CASE
                WHEN excluded.turn_id = '' THEN interaction_tool_uses.turn_id
                ELSE excluded.turn_id
            END,
            tool_kind = CASE
                WHEN excluded.tool_kind = '' THEN interaction_tool_uses.tool_kind
                ELSE excluded.tool_kind
            END,
            task_description = CASE
                WHEN excluded.task_description = '' THEN interaction_tool_uses.task_description
                ELSE excluded.task_description
            END,
            subagent_id = CASE
                WHEN excluded.subagent_id = '' THEN interaction_tool_uses.subagent_id
                ELSE excluded.subagent_id
            END,
            transcript_path = CASE
                WHEN excluded.transcript_path = '' THEN interaction_tool_uses.transcript_path
                ELSE excluded.transcript_path
            END,
            started_at = COALESCE(interaction_tool_uses.started_at, excluded.started_at),
            ended_at = COALESCE(excluded.ended_at, interaction_tool_uses.ended_at),
            updated_at = excluded.updated_at",
        rusqlite::params![
            tool_use_id,
            repo_id,
            event.session_id,
            event.turn_id.clone().unwrap_or_default(),
            tool_kind,
            task_description,
            subagent_id,
            transcript_path,
            started_at,
            ended_at,
            event.event_time,
        ],
    )
    .context("upserting interaction tool-use projection")?;
    Ok(())
}

fn event_tool_use_id(event: &InteractionEvent) -> String {
    if !event.tool_use_id.trim().is_empty() {
        return event.tool_use_id.clone();
    }
    payload_string_field(&event.payload, "tool_use_id")
}

fn event_tool_kind(event: &InteractionEvent) -> String {
    if !event.tool_kind.trim().is_empty() {
        return event.tool_kind.clone();
    }
    let payload_kind = payload_string_field(&event.payload, "subagent_type");
    if !payload_kind.is_empty() {
        return payload_kind;
    }
    match event.event_type {
        InteractionEventType::SubagentStart | InteractionEventType::SubagentEnd => {
            "subagent".to_string()
        }
        _ => String::new(),
    }
}

fn event_task_description(event: &InteractionEvent) -> String {
    if !event.task_description.trim().is_empty() {
        return event.task_description.clone();
    }
    payload_string_field(&event.payload, "task_description")
}

fn event_subagent_id(event: &InteractionEvent) -> String {
    if !event.subagent_id.trim().is_empty() {
        return event.subagent_id.clone();
    }
    payload_string_field(&event.payload, "subagent_id")
}

fn payload_string_field(payload: &serde_json::Value, key: &str) -> String {
    payload
        .get(key)
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn tool_use_search_text(tool_use: &InteractionToolUse) -> String {
    let text = [
        tool_use.tool_kind.as_str(),
        tool_use.task_description.as_str(),
    ]
    .into_iter()
    .filter(|value| !value.trim().is_empty())
    .collect::<Vec<_>>()
    .join(" ");
    normalise_search_text(&text)
}

fn bounded_text(input: &str, max_chars: usize) -> String {
    truncate_chars(normalise_search_text(input), max_chars)
}

fn bounded_join<I>(values: I, max_chars: usize) -> String
where
    I: IntoIterator<Item = String>,
{
    let mut out = String::new();
    for value in values {
        let value = normalise_search_text(&value);
        if value.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&value);
        if out.chars().count() >= max_chars {
            return truncate_chars(out, max_chars);
        }
    }
    truncate_chars(out, max_chars)
}

fn truncate_chars(input: String, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input;
    }
    input.chars().take(max_chars).collect()
}

fn normalise_search_text(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn unique_paths<I>(paths: I) -> impl IntoIterator<Item = String>
where
    I: IntoIterator<Item = String>,
{
    let mut unique = BTreeSet::new();
    for path in paths {
        let path = path.trim();
        if path.is_empty() {
            continue;
        }
        unique.insert(path.to_string());
    }
    unique
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

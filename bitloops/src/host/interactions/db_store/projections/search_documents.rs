use std::collections::HashMap;

use anyhow::{Context, Result};

use super::load::{
    load_session, load_subagent_runs_for_session, load_subagent_runs_for_turn,
    load_tool_uses_for_session, load_tool_uses_for_turn, load_turn, load_turns_for_session,
};
use super::text::{
    bounded_join, bounded_text, subagent_run_search_text, tokenise, tool_use_search_text,
    unique_paths,
};
use super::{
    FIELD_PATH, FIELD_PROMPT, FIELD_SUMMARY, FIELD_TOOL, FIELD_TRANSCRIPT, MAX_PATH_TEXT_CHARS,
    MAX_PROMPT_TEXT_CHARS, MAX_SUMMARY_TEXT_CHARS, MAX_TOOL_TEXT_CHARS, MAX_TRANSCRIPT_TEXT_CHARS,
};

pub(super) fn refresh_turn_search_document(
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
    let subagent_runs = load_subagent_runs_for_turn(conn, repo_id, turn_id)?;
    let prompt_text = bounded_text(&turn.prompt, MAX_PROMPT_TEXT_CHARS);
    let summary_text = bounded_text(&turn.summary, MAX_SUMMARY_TEXT_CHARS);
    let transcript_text = bounded_text(&turn.transcript_fragment, MAX_TRANSCRIPT_TEXT_CHARS);
    let tool_text = bounded_join(
        tool_uses
            .iter()
            .map(tool_use_search_text)
            .chain(subagent_runs.iter().map(subagent_run_search_text)),
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

pub(super) fn refresh_session_search_document(
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
    let subagent_runs = load_subagent_runs_for_session(conn, repo_id, session_id)?;
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
        tool_uses
            .iter()
            .map(tool_use_search_text)
            .chain(subagent_runs.iter().map(subagent_run_search_text)),
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

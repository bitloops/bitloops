use anyhow::Result;

use crate::host::interactions::types::InteractionEvent;

use super::event_projections::{upsert_subagent_run_from_event, upsert_tool_invocation_from_event};
use super::load::{load_events_for_repo, load_session_ids, load_turn_ids};
use super::search_documents::{refresh_session_search_document, refresh_turn_search_document};

pub(super) fn rebuild_all_projections(conn: &rusqlite::Connection, repo_id: &str) -> Result<()> {
    conn.execute_batch("BEGIN IMMEDIATE;")?;
    let result = (|| -> Result<()> {
        conn.execute(
            "DELETE FROM interaction_tool_invocations WHERE repo_id = ?1",
            rusqlite::params![repo_id],
        )?;
        conn.execute(
            "DELETE FROM interaction_subagent_runs WHERE repo_id = ?1",
            rusqlite::params![repo_id],
        )?;
        let events = load_events_for_repo(conn, repo_id)?;
        for event in &events {
            upsert_tool_invocation_from_event(conn, repo_id, event)?;
            upsert_subagent_run_from_event(conn, repo_id, event)?;
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
    })();

    match result {
        Ok(()) => {
            conn.execute_batch("COMMIT;")?;
            Ok(())
        }
        Err(err) => {
            let _ = conn.execute_batch("ROLLBACK;");
            Err(err)
        }
    }
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
    upsert_tool_invocation_from_event(conn, repo_id, event)?;
    upsert_subagent_run_from_event(conn, repo_id, event)?;
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

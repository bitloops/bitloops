mod event_projections;
mod load;
mod refresh;
mod search_documents;
mod text;

use anyhow::Result;

use crate::host::interactions::types::InteractionEvent;

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
    refresh::rebuild_all_projections(conn, repo_id)
}

pub(super) fn refresh_session_after_upsert(
    conn: &rusqlite::Connection,
    repo_id: &str,
    session_id: &str,
) -> Result<()> {
    refresh::refresh_session_after_upsert(conn, repo_id, session_id)
}

pub(super) fn refresh_turn_after_upsert(
    conn: &rusqlite::Connection,
    repo_id: &str,
    session_id: &str,
    turn_id: &str,
) -> Result<()> {
    refresh::refresh_turn_after_upsert(conn, repo_id, session_id, turn_id)
}

pub(super) fn refresh_after_event(
    conn: &rusqlite::Connection,
    repo_id: &str,
    event: &InteractionEvent,
) -> Result<()> {
    refresh::refresh_after_event(conn, repo_id, event)
}

pub(super) fn refresh_after_checkpoint_assignment(
    conn: &rusqlite::Connection,
    repo_id: &str,
    session_ids: &[String],
) -> Result<()> {
    refresh::refresh_after_checkpoint_assignment(conn, repo_id, session_ids)
}

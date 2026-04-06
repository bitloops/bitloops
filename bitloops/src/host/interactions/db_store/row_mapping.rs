use crate::host::checkpoints::strategy::manual_commit::TokenUsageMetadata;
use crate::host::interactions::types::{
    InteractionEvent, InteractionEventFilter, InteractionEventType, InteractionSession,
    InteractionTurn,
};

pub(super) fn map_session_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<InteractionSession> {
    Ok(InteractionSession {
        session_id: row.get(0)?,
        repo_id: row.get(1)?,
        agent_type: row.get(2)?,
        model: row.get(3)?,
        first_prompt: row.get(4)?,
        transcript_path: row.get(5)?,
        worktree_path: row.get(6)?,
        worktree_id: row.get(7)?,
        started_at: row.get(8)?,
        ended_at: row.get(9)?,
        last_event_at: row.get(10)?,
        updated_at: row.get(11)?,
    })
}

pub(super) fn map_turn_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<InteractionTurn> {
    let has_token_usage: i64 = row.get(9)?;
    let files_modified_json: String = row.get(20)?;
    let files_modified =
        serde_json::from_str::<Vec<String>>(&files_modified_json).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(
                20,
                rusqlite::types::Type::Text,
                Box::new(err),
            )
        })?;
    let checkpoint_id: String = row.get(21)?;
    Ok(InteractionTurn {
        turn_id: row.get(0)?,
        session_id: row.get(1)?,
        repo_id: row.get(2)?,
        turn_number: u32::try_from(row.get::<_, i64>(3)?).unwrap_or_default(),
        prompt: row.get(4)?,
        agent_type: row.get(5)?,
        model: row.get(6)?,
        started_at: row.get(7)?,
        ended_at: row.get(8)?,
        token_usage: (has_token_usage == 1).then(|| TokenUsageMetadata {
            input_tokens: row.get::<_, i64>(10).unwrap_or_default().max(0) as u64,
            cache_creation_tokens: row.get::<_, i64>(11).unwrap_or_default().max(0) as u64,
            cache_read_tokens: row.get::<_, i64>(12).unwrap_or_default().max(0) as u64,
            output_tokens: row.get::<_, i64>(13).unwrap_or_default().max(0) as u64,
            api_call_count: row.get::<_, i64>(14).unwrap_or_default().max(0) as u64,
            subagent_tokens: None,
        }),
        summary: row.get(15)?,
        prompt_count: u32::try_from(row.get::<_, i64>(16)?).unwrap_or_default(),
        transcript_offset_start: row.get(17)?,
        transcript_offset_end: row.get(18)?,
        transcript_fragment: row.get(19)?,
        files_modified,
        checkpoint_id: (!checkpoint_id.trim().is_empty()).then_some(checkpoint_id),
        updated_at: row.get(22)?,
    })
}

pub(super) fn map_event_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<InteractionEvent> {
    let payload_raw: String = row.get(8)?;
    let payload = serde_json::from_str::<serde_json::Value>(&payload_raw).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(8, rusqlite::types::Type::Text, Box::new(err))
    })?;
    let event_type_raw: String = row.get(4)?;
    let event_type = InteractionEventType::parse(&event_type_raw).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            4,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unknown interaction event type `{event_type_raw}`"),
            )),
        )
    })?;
    Ok(InteractionEvent {
        event_id: row.get(0)?,
        session_id: row.get(1)?,
        turn_id: row.get(2)?,
        repo_id: row.get(3)?,
        event_type,
        event_time: row.get(5)?,
        agent_type: row.get(6)?,
        model: row.get(7)?,
        payload,
    })
}

pub(super) fn append_event_filter_sql(
    sql: &mut String,
    values: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
    filter: &InteractionEventFilter,
) {
    if let Some(session_id) = filter
        .session_id
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        sql.push_str(&format!(" AND session_id = ?{}", values.len() + 1));
        values.push(Box::new(session_id.to_string()));
    }
    if let Some(turn_id) = filter.turn_id.as_deref().filter(|value| !value.is_empty()) {
        sql.push_str(&format!(" AND turn_id = ?{}", values.len() + 1));
        values.push(Box::new(turn_id.to_string()));
    }
    if let Some(event_type) = filter.event_type {
        sql.push_str(&format!(" AND event_type = ?{}", values.len() + 1));
        values.push(Box::new(event_type.as_str().to_string()));
    }
    if let Some(since) = filter.since.as_deref().filter(|value| !value.is_empty()) {
        sql.push_str(&format!(" AND event_time >= ?{}", values.len() + 1));
        values.push(Box::new(since.to_string()));
    }
}

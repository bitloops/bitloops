use serde_json::Value;

use crate::host::checkpoints::strategy::manual_commit::TokenUsageMetadata;
use crate::host::devql::esc_pg;
use crate::host::interactions::types::{
    InteractionEvent, InteractionEventFilter, InteractionEventType, InteractionSession,
    InteractionTurn,
};

pub(super) fn append_event_filter_sql(sql: &mut String, filter: &InteractionEventFilter) {
    if let Some(session_id) = filter
        .session_id
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        sql.push_str(&format!(" AND session_id = '{}'", esc_pg(session_id)));
    }
    if let Some(turn_id) = filter.turn_id.as_deref().filter(|value| !value.is_empty()) {
        sql.push_str(&format!(" AND turn_id = '{}'", esc_pg(turn_id)));
    }
    if let Some(event_type) = filter.event_type {
        sql.push_str(&format!(
            " AND event_type = '{}'",
            esc_pg(event_type.as_str())
        ));
    }
    if let Some(since) = filter.since.as_deref().filter(|value| !value.is_empty()) {
        sql.push_str(&format!(" AND event_time >= '{}'", esc_pg(since)));
    }
}

pub(super) fn map_session_row(row: &duckdb::Row<'_>) -> duckdb::Result<InteractionSession> {
    let ended_at: Option<String> = row.get(14)?;
    Ok(InteractionSession {
        session_id: row.get(0)?,
        repo_id: row.get(1)?,
        branch: row.get(2)?,
        actor_id: row.get(3)?,
        actor_name: row.get(4)?,
        actor_email: row.get(5)?,
        actor_source: row.get(6)?,
        agent_type: row.get(7)?,
        model: row.get(8)?,
        first_prompt: row.get(9)?,
        transcript_path: row.get(10)?,
        worktree_path: row.get(11)?,
        worktree_id: row.get(12)?,
        started_at: row.get(13)?,
        ended_at: ended_at.filter(|value| !value.trim().is_empty()),
        last_event_at: row.get(15)?,
        updated_at: row.get(16)?,
    })
}

pub(super) fn map_turn_row(row: &duckdb::Row<'_>) -> duckdb::Result<InteractionTurn> {
    let files_modified_raw: String = row.get(25)?;
    let files_modified =
        serde_json::from_str::<Vec<String>>(&files_modified_raw).map_err(|err| {
            duckdb::Error::FromSqlConversionFailure(25, duckdb::types::Type::Text, Box::new(err))
        })?;
    let checkpoint_id: String = row.get(26)?;
    let has_token_usage: i32 = row.get(14)?;
    Ok(InteractionTurn {
        turn_id: row.get(0)?,
        session_id: row.get(1)?,
        repo_id: row.get(2)?,
        branch: row.get(3)?,
        actor_id: row.get(4)?,
        actor_name: row.get(5)?,
        actor_email: row.get(6)?,
        actor_source: row.get(7)?,
        turn_number: u32::try_from(row.get::<_, i32>(8)?).unwrap_or_default(),
        prompt: row.get(9)?,
        agent_type: row.get(10)?,
        model: row.get(11)?,
        started_at: row.get(12)?,
        ended_at: row
            .get::<_, Option<String>>(13)?
            .filter(|value| !value.trim().is_empty()),
        token_usage: (has_token_usage == 1).then(|| TokenUsageMetadata {
            input_tokens: row.get::<_, i64>(15).unwrap_or_default().max(0) as u64,
            cache_creation_tokens: row.get::<_, i64>(16).unwrap_or_default().max(0) as u64,
            cache_read_tokens: row.get::<_, i64>(17).unwrap_or_default().max(0) as u64,
            output_tokens: row.get::<_, i64>(18).unwrap_or_default().max(0) as u64,
            api_call_count: row.get::<_, i64>(19).unwrap_or_default().max(0) as u64,
            subagent_tokens: None,
        }),
        summary: row.get(20)?,
        prompt_count: u32::try_from(row.get::<_, i32>(21)?).unwrap_or_default(),
        transcript_offset_start: row.get(22)?,
        transcript_offset_end: row.get(23)?,
        transcript_fragment: row.get(24)?,
        files_modified,
        checkpoint_id: (!checkpoint_id.trim().is_empty()).then_some(checkpoint_id),
        updated_at: row.get(27)?,
    })
}

pub(super) fn map_event_row(row: &duckdb::Row<'_>) -> duckdb::Result<InteractionEvent> {
    let event_type_raw: String = row.get(9)?;
    let payload_raw: String = row.get(17)?;
    let payload = serde_json::from_str::<Value>(&payload_raw).map_err(|err| {
        duckdb::Error::FromSqlConversionFailure(17, duckdb::types::Type::Text, Box::new(err))
    })?;
    let event_type = InteractionEventType::parse(&event_type_raw).ok_or_else(|| {
        duckdb::Error::FromSqlConversionFailure(
            9,
            duckdb::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unknown interaction event type `{event_type_raw}`"),
            )),
        )
    })?;
    let turn_id: String = row.get(2)?;
    Ok(InteractionEvent {
        event_id: row.get(0)?,
        session_id: row.get(1)?,
        turn_id: (!turn_id.trim().is_empty()).then_some(turn_id),
        repo_id: row.get(3)?,
        branch: row.get(4)?,
        actor_id: row.get(5)?,
        actor_name: row.get(6)?,
        actor_email: row.get(7)?,
        actor_source: row.get(8)?,
        event_type,
        event_time: row.get(10)?,
        agent_type: row.get(11)?,
        model: row.get(12)?,
        tool_use_id: row.get(13)?,
        tool_kind: row.get(14)?,
        task_description: row.get(15)?,
        subagent_id: row.get(16)?,
        payload,
    })
}

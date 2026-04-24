use anyhow::{Context, Result};

use crate::host::interactions::projection_ids::scope_tool_projection_id;
use crate::host::interactions::types::{InteractionEvent, InteractionEventType};

pub(super) fn upsert_tool_invocation_from_event(
    conn: &rusqlite::Connection,
    repo_id: &str,
    event: &InteractionEvent,
) -> Result<()> {
    let tool_invocation_id = event_tool_projection_id(event);
    if tool_invocation_id.is_empty() {
        return Ok(());
    }
    if !matches!(
        event.event_type,
        InteractionEventType::ToolInvocationObserved | InteractionEventType::ToolResultObserved
    ) {
        return Ok(());
    }

    let tool_use_id = event_tool_use_id(event);
    let tool_name = event_tool_name(event);
    let source = event_source(event);
    let input_summary = payload_string_field(&event.payload, "input_summary");
    let output_summary = payload_string_field(&event.payload, "output_summary");
    let command = payload_string_field(&event.payload, "command");
    let command_binary = payload_string_field(&event.payload, "command_binary");
    let command_argv = payload_string_vec_field(&event.payload, "command_argv");
    let transcript_path = payload_string_field(&event.payload, "transcript_path");
    let started_at = matches!(
        event.event_type,
        InteractionEventType::ToolInvocationObserved
    )
    .then(|| event.event_time.clone());
    let ended_at = matches!(event.event_type, InteractionEventType::ToolResultObserved)
        .then(|| event.event_time.clone());
    let started_sequence_number = matches!(
        event.event_type,
        InteractionEventType::ToolInvocationObserved
    )
    .then_some(event.sequence_number);
    let ended_sequence_number =
        matches!(event.event_type, InteractionEventType::ToolResultObserved)
            .then_some(event.sequence_number);
    let command_argv_json =
        serde_json::to_string(&command_argv).context("serialising command argv projection")?;

    conn.execute(
        "INSERT INTO interaction_tool_invocations (
            tool_invocation_id, repo_id, session_id, turn_id, tool_use_id, tool_name, source,
            input_summary, output_summary, command, command_binary, command_argv, transcript_path,
            started_at, ended_at, started_sequence_number, ended_sequence_number, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
         ON CONFLICT(repo_id, tool_invocation_id) DO UPDATE SET
            session_id = CASE
                WHEN excluded.session_id = '' THEN interaction_tool_invocations.session_id
                ELSE excluded.session_id
            END,
            turn_id = CASE
                WHEN excluded.turn_id = '' THEN interaction_tool_invocations.turn_id
                ELSE excluded.turn_id
            END,
            tool_use_id = CASE
                WHEN excluded.tool_use_id = '' THEN interaction_tool_invocations.tool_use_id
                ELSE excluded.tool_use_id
            END,
            tool_name = CASE
                WHEN excluded.tool_name = '' THEN interaction_tool_invocations.tool_name
                ELSE excluded.tool_name
            END,
            source = CASE
                WHEN excluded.source = '' THEN interaction_tool_invocations.source
                ELSE excluded.source
            END,
            input_summary = CASE
                WHEN excluded.input_summary = '' THEN interaction_tool_invocations.input_summary
                ELSE excluded.input_summary
            END,
            output_summary = CASE
                WHEN excluded.output_summary = '' THEN interaction_tool_invocations.output_summary
                ELSE excluded.output_summary
            END,
            command = CASE
                WHEN excluded.command = '' THEN interaction_tool_invocations.command
                ELSE excluded.command
            END,
            command_binary = CASE
                WHEN excluded.command_binary = '' THEN interaction_tool_invocations.command_binary
                ELSE excluded.command_binary
            END,
            command_argv = CASE
                WHEN excluded.command_argv = '[]' THEN interaction_tool_invocations.command_argv
                ELSE excluded.command_argv
            END,
            transcript_path = CASE
                WHEN excluded.transcript_path = '' THEN interaction_tool_invocations.transcript_path
                ELSE excluded.transcript_path
            END,
            started_at = COALESCE(interaction_tool_invocations.started_at, excluded.started_at),
            ended_at = COALESCE(excluded.ended_at, interaction_tool_invocations.ended_at),
            started_sequence_number = COALESCE(
                interaction_tool_invocations.started_sequence_number,
                excluded.started_sequence_number
            ),
            ended_sequence_number = COALESCE(
                excluded.ended_sequence_number,
                interaction_tool_invocations.ended_sequence_number
            ),
            updated_at = excluded.updated_at",
        rusqlite::params![
            tool_invocation_id,
            repo_id,
            event.session_id,
            event.turn_id.clone().unwrap_or_default(),
            tool_use_id,
            tool_name,
            source,
            input_summary,
            output_summary,
            command,
            command_binary,
            command_argv_json,
            transcript_path,
            started_at,
            ended_at,
            started_sequence_number,
            ended_sequence_number,
            event.event_time,
        ],
    )
    .context("upserting interaction tool-invocation projection")?;
    Ok(())
}

pub(super) fn upsert_subagent_run_from_event(
    conn: &rusqlite::Connection,
    repo_id: &str,
    event: &InteractionEvent,
) -> Result<()> {
    if !matches!(
        event.event_type,
        InteractionEventType::SubagentStart | InteractionEventType::SubagentEnd
    ) {
        return Ok(());
    }

    let subagent_run_id = event_subagent_run_id(event);
    if subagent_run_id.is_empty() {
        return Ok(());
    }

    let tool_use_id = event_tool_use_id(event);
    let subagent_id = event_subagent_id(event);
    let subagent_type = event_tool_name(event);
    let task_description = event_task_description(event);
    let source = event_source(event);
    let transcript_path = payload_string_field(&event.payload, "subagent_transcript_path");
    let child_session_id = payload_string_field(&event.payload, "child_session_id");
    let started_at = matches!(event.event_type, InteractionEventType::SubagentStart)
        .then(|| event.event_time.clone());
    let ended_at = matches!(event.event_type, InteractionEventType::SubagentEnd)
        .then(|| event.event_time.clone());
    let started_sequence_number = matches!(event.event_type, InteractionEventType::SubagentStart)
        .then_some(event.sequence_number);
    let ended_sequence_number = matches!(event.event_type, InteractionEventType::SubagentEnd)
        .then_some(event.sequence_number);

    conn.execute(
        "INSERT INTO interaction_subagent_runs (
            subagent_run_id, repo_id, session_id, turn_id, tool_use_id, subagent_id,
            subagent_type, task_description, source, transcript_path, child_session_id,
            started_at, ended_at, started_sequence_number, ended_sequence_number, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
         ON CONFLICT(repo_id, subagent_run_id) DO UPDATE SET
            session_id = CASE
                WHEN excluded.session_id = '' THEN interaction_subagent_runs.session_id
                ELSE excluded.session_id
            END,
            turn_id = CASE
                WHEN excluded.turn_id = '' THEN interaction_subagent_runs.turn_id
                ELSE excluded.turn_id
            END,
            tool_use_id = CASE
                WHEN excluded.tool_use_id = '' THEN interaction_subagent_runs.tool_use_id
                ELSE excluded.tool_use_id
            END,
            subagent_id = CASE
                WHEN excluded.subagent_id = '' THEN interaction_subagent_runs.subagent_id
                ELSE excluded.subagent_id
            END,
            subagent_type = CASE
                WHEN excluded.subagent_type = '' THEN interaction_subagent_runs.subagent_type
                ELSE excluded.subagent_type
            END,
            task_description = CASE
                WHEN excluded.task_description = '' THEN interaction_subagent_runs.task_description
                ELSE excluded.task_description
            END,
            source = CASE
                WHEN excluded.source = '' THEN interaction_subagent_runs.source
                ELSE excluded.source
            END,
            transcript_path = CASE
                WHEN excluded.transcript_path = '' THEN interaction_subagent_runs.transcript_path
                ELSE excluded.transcript_path
            END,
            child_session_id = CASE
                WHEN excluded.child_session_id = '' THEN interaction_subagent_runs.child_session_id
                ELSE excluded.child_session_id
            END,
            started_at = COALESCE(interaction_subagent_runs.started_at, excluded.started_at),
            ended_at = COALESCE(excluded.ended_at, interaction_subagent_runs.ended_at),
            started_sequence_number = COALESCE(
                interaction_subagent_runs.started_sequence_number,
                excluded.started_sequence_number
            ),
            ended_sequence_number = COALESCE(
                excluded.ended_sequence_number,
                interaction_subagent_runs.ended_sequence_number
            ),
            updated_at = excluded.updated_at",
        rusqlite::params![
            subagent_run_id,
            repo_id,
            event.session_id,
            event.turn_id.clone().unwrap_or_default(),
            tool_use_id,
            subagent_id,
            subagent_type,
            task_description,
            source,
            transcript_path,
            child_session_id,
            started_at,
            ended_at,
            started_sequence_number,
            ended_sequence_number,
            event.event_time,
        ],
    )
    .context("upserting interaction subagent-run projection")?;
    Ok(())
}

fn event_tool_projection_id(event: &InteractionEvent) -> String {
    let tool_use_id = event_tool_use_id(event);
    let tool_projection_id =
        scope_tool_projection_id(event.turn_id.as_deref(), &event.session_id, &tool_use_id);
    if !tool_projection_id.is_empty() {
        return tool_projection_id;
    }
    if !event.event_id.trim().is_empty() {
        return event.event_id.clone();
    }
    String::new()
}

fn event_subagent_run_id(event: &InteractionEvent) -> String {
    let tool_use_id = event_tool_use_id(event);
    if !tool_use_id.trim().is_empty() {
        return tool_use_id;
    }
    if !event.subagent_id.trim().is_empty() {
        return event.subagent_id.clone();
    }
    if !event.event_id.trim().is_empty() {
        return event.event_id.clone();
    }
    String::new()
}

fn event_tool_use_id(event: &InteractionEvent) -> String {
    if !event.tool_use_id.trim().is_empty() {
        return event.tool_use_id.clone();
    }
    payload_string_field(&event.payload, "tool_use_id")
}

fn event_tool_name(event: &InteractionEvent) -> String {
    if !event.tool_kind.trim().is_empty() {
        return event.tool_kind.clone();
    }
    let tool_name = payload_string_field(&event.payload, "tool_name");
    if !tool_name.is_empty() {
        return tool_name;
    }
    payload_string_field(&event.payload, "subagent_type")
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

fn event_source(event: &InteractionEvent) -> String {
    if !event.source.trim().is_empty() {
        return event.source.clone();
    }
    payload_string_field(&event.payload, "source")
}

fn payload_string_field(payload: &serde_json::Value, key: &str) -> String {
    payload
        .get(key)
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn payload_string_vec_field(payload: &serde_json::Value, key: &str) -> Vec<String> {
    payload
        .get(key)
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

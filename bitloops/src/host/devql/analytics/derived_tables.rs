use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::row_access::{optional_row_string, row_i64, row_string};
use super::types::AnalyticsDerivedTables;
use crate::host::interactions::types::{InteractionEvent, InteractionEventType};

pub(super) fn derive_interaction_tables(events: &[Value]) -> AnalyticsDerivedTables {
    let mut tool_rows = Vec::new();
    let mut subagent_rows = Vec::new();
    let mut tools_by_id = BTreeMap::<String, Value>::new();
    let mut subagents_by_id = BTreeMap::<String, Value>::new();

    for event_row in events {
        let Some(event) = interaction_event_from_row(event_row) else {
            continue;
        };

        match event.event_type {
            InteractionEventType::ToolInvocationObserved
            | InteractionEventType::ToolResultObserved => {
                let tool_invocation_id = event_tool_projection_id(&event);
                if tool_invocation_id.is_empty() {
                    continue;
                }
                let entry = tools_by_id.entry(tool_invocation_id.clone()).or_insert_with(|| {
                    json!({
                        "tool_invocation_id": tool_invocation_id,
                        "repo_id": event.repo_id,
                        "session_id": event.session_id,
                        "turn_id": event.turn_id.clone().unwrap_or_default(),
                        "tool_use_id": event_tool_use_id(&event),
                        "tool_name": event_tool_name(&event),
                        "source": event_source(&event),
                        "input_summary": payload_string_field(&event.payload, "input_summary"),
                        "output_summary": payload_string_field(&event.payload, "output_summary"),
                        "command": payload_string_field(&event.payload, "command"),
                        "command_binary": payload_string_field(&event.payload, "command_binary"),
                        "command_argv": serde_json::to_string(&payload_string_vec_field(&event.payload, "command_argv")).unwrap_or_else(|_| "[]".to_string()),
                        "transcript_path": payload_string_field(&event.payload, "transcript_path"),
                        "started_at": Value::Null,
                        "ended_at": Value::Null,
                        "started_sequence_number": Value::Null,
                        "ended_sequence_number": Value::Null,
                        "updated_at": event.event_time,
                    })
                });

                set_if_empty(entry, "session_id", event.session_id.clone());
                set_if_empty(entry, "turn_id", event.turn_id.clone().unwrap_or_default());
                set_if_empty(entry, "tool_use_id", event_tool_use_id(&event));
                set_if_empty(entry, "tool_name", event_tool_name(&event));
                set_if_empty(entry, "source", event_source(&event));
                set_if_empty(
                    entry,
                    "input_summary",
                    payload_string_field(&event.payload, "input_summary"),
                );
                set_if_empty(
                    entry,
                    "output_summary",
                    payload_string_field(&event.payload, "output_summary"),
                );
                set_if_empty(
                    entry,
                    "command",
                    payload_string_field(&event.payload, "command"),
                );
                set_if_empty(
                    entry,
                    "command_binary",
                    payload_string_field(&event.payload, "command_binary"),
                );
                set_if_empty(
                    entry,
                    "command_argv",
                    serde_json::to_string(&payload_string_vec_field(
                        &event.payload,
                        "command_argv",
                    ))
                    .unwrap_or_else(|_| "[]".to_string()),
                );
                set_if_empty(
                    entry,
                    "transcript_path",
                    payload_string_field(&event.payload, "transcript_path"),
                );
                set_row_string(entry, "updated_at", event.event_time.clone());

                if matches!(
                    event.event_type,
                    InteractionEventType::ToolInvocationObserved
                ) {
                    set_if_empty(entry, "started_at", event.event_time.clone());
                    set_if_null_number(entry, "started_sequence_number", event.sequence_number);
                } else {
                    set_if_empty(entry, "ended_at", event.event_time.clone());
                    set_number(entry, "ended_sequence_number", event.sequence_number);
                }
            }
            InteractionEventType::SubagentStart | InteractionEventType::SubagentEnd => {
                let subagent_run_id = event_subagent_run_id(&event);
                if subagent_run_id.is_empty() {
                    continue;
                }
                let entry = subagents_by_id
                    .entry(subagent_run_id.clone())
                    .or_insert_with(|| {
                        json!({
                            "subagent_run_id": subagent_run_id,
                            "repo_id": event.repo_id,
                            "session_id": event.session_id,
                            "turn_id": event.turn_id.clone().unwrap_or_default(),
                            "tool_use_id": event_tool_use_id(&event),
                            "subagent_id": event_subagent_id(&event),
                            "subagent_type": event_tool_name(&event),
                            "task_description": event_task_description(&event),
                            "source": event_source(&event),
                            "transcript_path": payload_string_field(&event.payload, "subagent_transcript_path"),
                            "child_session_id": payload_string_field(&event.payload, "child_session_id"),
                            "started_at": Value::Null,
                            "ended_at": Value::Null,
                            "started_sequence_number": Value::Null,
                            "ended_sequence_number": Value::Null,
                            "updated_at": event.event_time,
                        })
                    });

                set_if_empty(entry, "session_id", event.session_id.clone());
                set_if_empty(entry, "turn_id", event.turn_id.clone().unwrap_or_default());
                set_if_empty(entry, "tool_use_id", event_tool_use_id(&event));
                set_if_empty(entry, "subagent_id", event_subagent_id(&event));
                set_if_empty(entry, "subagent_type", event_tool_name(&event));
                set_if_empty(entry, "task_description", event_task_description(&event));
                set_if_empty(entry, "source", event_source(&event));
                set_if_empty(
                    entry,
                    "transcript_path",
                    payload_string_field(&event.payload, "subagent_transcript_path"),
                );
                set_if_empty(
                    entry,
                    "child_session_id",
                    payload_string_field(&event.payload, "child_session_id"),
                );
                set_row_string(entry, "updated_at", event.event_time.clone());

                if matches!(event.event_type, InteractionEventType::SubagentStart) {
                    set_if_empty(entry, "started_at", event.event_time.clone());
                    set_if_null_number(entry, "started_sequence_number", event.sequence_number);
                } else {
                    set_if_empty(entry, "ended_at", event.event_time.clone());
                    set_number(entry, "ended_sequence_number", event.sequence_number);
                }
            }
            _ => {}
        }
    }

    tool_rows.extend(tools_by_id.into_values());
    subagent_rows.extend(subagents_by_id.into_values());
    AnalyticsDerivedTables {
        interaction_tool_invocations: tool_rows,
        interaction_subagent_runs: subagent_rows,
    }
}

fn interaction_event_from_row(row: &Value) -> Option<InteractionEvent> {
    let event_type = row
        .get("event_type")
        .and_then(Value::as_str)
        .and_then(InteractionEventType::parse)?;
    let payload = match row.get("payload") {
        Some(Value::String(text)) => serde_json::from_str(text).unwrap_or_else(|_| json!({})),
        Some(other) => other.clone(),
        None => json!({}),
    };

    Some(InteractionEvent {
        event_id: row_string(row, "event_id"),
        session_id: row_string(row, "session_id"),
        turn_id: optional_row_string(row, "turn_id"),
        repo_id: row_string(row, "repo_id"),
        branch: row_string(row, "branch"),
        actor_id: row_string(row, "actor_id"),
        actor_name: row_string(row, "actor_name"),
        actor_email: row_string(row, "actor_email"),
        actor_source: row_string(row, "actor_source"),
        event_type,
        event_time: row_string(row, "event_time"),
        source: row_string(row, "source"),
        sequence_number: row_i64(row, "sequence_number"),
        agent_type: row_string(row, "agent_type"),
        model: row_string(row, "model"),
        tool_use_id: row_string(row, "tool_use_id"),
        tool_kind: row_string(row, "tool_kind"),
        task_description: row_string(row, "task_description"),
        subagent_id: row_string(row, "subagent_id"),
        payload,
    })
}

fn event_tool_projection_id(event: &InteractionEvent) -> String {
    let tool_use_id = event_tool_use_id(event);
    if !tool_use_id.trim().is_empty() {
        return tool_use_id;
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

fn payload_string_field(payload: &Value, key: &str) -> String {
    payload
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn payload_string_vec_field(payload: &Value, key: &str) -> Vec<String> {
    payload
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn set_if_empty(row: &mut Value, key: &str, value: String) {
    if value.trim().is_empty() {
        return;
    }
    if row_string(row, key).is_empty() {
        set_row_string(row, key, value);
    }
}

fn set_if_null_number(row: &mut Value, key: &str, value: i64) {
    if row.get(key).is_none_or(Value::is_null) {
        set_number(row, key, value);
    }
}

fn set_number(row: &mut Value, key: &str, value: i64) {
    if let Some(object) = row.as_object_mut() {
        object.insert(key.to_string(), Value::from(value));
    }
}

fn set_row_string(row: &mut Value, key: &str, value: String) {
    if let Some(object) = row.as_object_mut() {
        object.insert(key.to_string(), Value::String(value));
    }
}

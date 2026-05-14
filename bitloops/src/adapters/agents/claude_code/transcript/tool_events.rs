use std::collections::HashMap;

use anyhow::Result;
use serde_json::Value;

use crate::adapters::agents::{TranscriptEntryDeriver, TranscriptToolEventDeriver};
use crate::host::checkpoints::transcript::parse::parse_from_bytes;
use crate::host::checkpoints::transcript::types::{
    AssistantMessage, CONTENT_TYPE_TEXT, CONTENT_TYPE_TOOL_USE, TYPE_ASSISTANT, TYPE_USER,
    UserMessage,
};
use crate::host::interactions::tool_events::TranscriptToolEventObservation;
use crate::host::interactions::transcript_entry::{
    DerivationScope, TranscriptActor, TranscriptEntry, TranscriptSource, TranscriptVariant,
    make_derived_tool_use_id, make_entry_id,
};

use super::super::agent::ClaudeCodeAgent;
use super::message_types::{ToolResultContentBlock, ToolResultMessage};

#[derive(Debug, Clone, Default)]
struct PendingTool {
    tool_name: String,
    is_subagent_task: bool,
}

impl TranscriptToolEventDeriver for ClaudeCodeAgent {
    fn derive_transcript_tool_event_observations(
        &self,
        turn_id: &str,
        transcript_fragment: &str,
    ) -> Result<Vec<TranscriptToolEventObservation>> {
        derive_tool_event_observations(turn_id, transcript_fragment)
    }
}

pub fn derive_tool_event_observations(
    turn_id: &str,
    transcript_fragment: &str,
) -> Result<Vec<TranscriptToolEventObservation>> {
    let lines = parse_from_bytes(transcript_fragment.as_bytes())?;
    let mut tool_use_block_number = 1_i64;
    let mut pending_tools = HashMap::<String, PendingTool>::new();
    let mut observations = Vec::new();

    for line in lines {
        match line.r#type.as_str() {
            TYPE_ASSISTANT => {
                let Ok(message) = serde_json::from_value::<AssistantMessage>(line.message) else {
                    continue;
                };

                for block in message.content {
                    if block.r#type != CONTENT_TYPE_TOOL_USE {
                        continue;
                    }

                    let tool_name = block.name.trim().to_string();
                    if tool_name.is_empty() {
                        continue;
                    }

                    let is_subagent_task = tool_name.eq_ignore_ascii_case("task");
                    let fallback_tool_use_block_number = tool_use_block_number;
                    tool_use_block_number += 1;
                    let tool_use_id = if block.id.trim().is_empty() {
                        format!("{turn_id}:tool:{fallback_tool_use_block_number:04}")
                    } else {
                        block.id.trim().to_string()
                    };
                    pending_tools.insert(
                        tool_use_id.clone(),
                        PendingTool {
                            tool_name: tool_name.clone(),
                            is_subagent_task,
                        },
                    );

                    if is_subagent_task {
                        continue;
                    }

                    observations.push(TranscriptToolEventObservation::Invocation {
                        tool_use_id,
                        tool_name,
                        tool_input: block.input,
                    });
                }
            }
            TYPE_USER => {
                let Ok(message) = serde_json::from_value::<ToolResultMessage>(line.message) else {
                    continue;
                };
                let Ok(blocks) =
                    serde_json::from_value::<Vec<ToolResultContentBlock>>(message.content)
                else {
                    continue;
                };

                for block in blocks {
                    if block.kind != "tool_result" {
                        continue;
                    }

                    let tool_use_id = block.tool_use_id.trim();
                    if tool_use_id.is_empty() {
                        continue;
                    }

                    let pending = pending_tools.get(tool_use_id);
                    if pending.is_some_and(|tool| tool.is_subagent_task) {
                        continue;
                    }
                    if pending.is_none() && looks_like_subagent_result_content(&block.content) {
                        continue;
                    }

                    observations.push(TranscriptToolEventObservation::Result {
                        tool_use_id: tool_use_id.to_string(),
                        tool_name: pending
                            .map(|tool| tool.tool_name.clone())
                            .unwrap_or_default(),
                        tool_output: block.content,
                    });
                }
            }
            _ => {}
        }
    }

    Ok(observations)
}

impl TranscriptEntryDeriver for ClaudeCodeAgent {
    fn derive_transcript_entries(
        &self,
        session_id: &str,
        turn_id: Option<&str>,
        transcript: &str,
    ) -> Result<Vec<TranscriptEntry>> {
        derive_transcript_entries(session_id, turn_id, transcript)
    }
}

/// Normalize a Claude transcript (JSONL with `type`/`message` lines) into canonical
/// `TranscriptEntry` rows. Handles user/assistant/thinking/tool_use/tool_result content
/// blocks, preserves document order, and pairs tool_use with tool_result by id.
pub fn derive_transcript_entries(
    session_id: &str,
    turn_id: Option<&str>,
    transcript: &str,
) -> Result<Vec<TranscriptEntry>> {
    let lines = parse_from_bytes(transcript.as_bytes())?;
    let scope = match turn_id {
        Some(id) => DerivationScope::Turn(id),
        None => DerivationScope::Session,
    };
    let mut entries: Vec<TranscriptEntry> = Vec::new();
    let mut order: i32 = 0;
    let mut tool_call_index: i32 = 0;
    let mut tool_kinds_by_id: HashMap<String, String> = HashMap::new();

    for line in lines {
        match line.r#type.as_str() {
            TYPE_USER => {
                let Ok(message) = serde_json::from_value::<UserMessage>(line.message) else {
                    continue;
                };
                emit_user_entries(
                    session_id,
                    &scope,
                    &message.content,
                    &mut order,
                    &mut entries,
                    &tool_kinds_by_id,
                );
            }
            TYPE_ASSISTANT => {
                let Ok(message) = serde_json::from_value::<AssistantMessage>(line.message) else {
                    continue;
                };
                for block in message.content {
                    match block.r#type.as_str() {
                        CONTENT_TYPE_TEXT => {
                            let text = block.text.trim();
                            if text.is_empty() {
                                continue;
                            }
                            entries.push(canonical_chat_entry(
                                session_id,
                                &scope,
                                order,
                                TranscriptActor::Assistant,
                                TranscriptVariant::Chat,
                                text,
                            ));
                            order += 1;
                        }
                        "thinking" => {
                            let text = block.text.trim();
                            if text.is_empty() {
                                continue;
                            }
                            entries.push(canonical_chat_entry(
                                session_id,
                                &scope,
                                order,
                                TranscriptActor::Assistant,
                                TranscriptVariant::Thinking,
                                text,
                            ));
                            order += 1;
                        }
                        CONTENT_TYPE_TOOL_USE => {
                            let tool_kind = block.name.trim();
                            if tool_kind.is_empty() {
                                continue;
                            }
                            let tool_use_id = if block.id.trim().is_empty() {
                                make_derived_tool_use_id(session_id, &scope, tool_call_index)
                            } else {
                                block.id.trim().to_string()
                            };
                            tool_call_index += 1;

                            let tool_input_summary = if block.input.is_null() {
                                String::new()
                            } else {
                                serde_json::to_string(&block.input).unwrap_or_default()
                            };
                            let tool_use_text = if tool_input_summary.is_empty() {
                                format!("Tool: {tool_kind}")
                            } else {
                                format!("Tool: {tool_kind}\n{tool_input_summary}")
                            };

                            tool_kinds_by_id
                                .insert(tool_use_id.clone(), tool_kind.to_string());

                            entries.push(TranscriptEntry {
                                entry_id: make_entry_id(session_id, &scope, order),
                                session_id: session_id.to_string(),
                                turn_id: scope.turn_id().map(str::to_string),
                                order,
                                timestamp: None,
                                actor: TranscriptActor::System,
                                variant: TranscriptVariant::ToolUse,
                                source: TranscriptSource::Transcript,
                                text: tool_use_text,
                                tool_use_id: Some(tool_use_id),
                                tool_kind: Some(tool_kind.to_string()),
                                is_error: false,
                            });
                            order += 1;
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    Ok(entries)
}

fn emit_user_entries(
    session_id: &str,
    scope: &DerivationScope<'_>,
    content: &Value,
    order: &mut i32,
    entries: &mut Vec<TranscriptEntry>,
    tool_kinds_by_id: &HashMap<String, String>,
) {
    match content {
        Value::String(text) => {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                entries.push(canonical_chat_entry(
                    session_id,
                    scope,
                    *order,
                    TranscriptActor::User,
                    TranscriptVariant::Chat,
                    trimmed,
                ));
                *order += 1;
            }
        }
        Value::Array(blocks) => {
            for block in blocks {
                let kind = block
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                match kind {
                    "text" => {
                        let text = block
                            .get("text")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .trim();
                        if text.is_empty() {
                            continue;
                        }
                        entries.push(canonical_chat_entry(
                            session_id,
                            scope,
                            *order,
                            TranscriptActor::User,
                            TranscriptVariant::Chat,
                            text,
                        ));
                        *order += 1;
                    }
                    "tool_result" => {
                        let tool_use_id = block
                            .get("tool_use_id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .trim();
                        if tool_use_id.is_empty() {
                            continue;
                        }
                        let is_error = block
                            .get("is_error")
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                        let result_content = block.get("content").cloned().unwrap_or(Value::Null);
                        if looks_like_subagent_result_content(&result_content) {
                            continue;
                        }
                        let text = stringify_tool_result(&result_content);
                        let tool_kind = tool_kinds_by_id
                            .get(tool_use_id)
                            .cloned()
                            .unwrap_or_default();
                        entries.push(TranscriptEntry {
                            entry_id: make_entry_id(session_id, scope, *order),
                            session_id: session_id.to_string(),
                            turn_id: scope.turn_id().map(str::to_string),
                            order: *order,
                            timestamp: None,
                            actor: TranscriptActor::System,
                            variant: TranscriptVariant::ToolResult,
                            source: TranscriptSource::Transcript,
                            text,
                            tool_use_id: Some(tool_use_id.to_string()),
                            tool_kind: if tool_kind.is_empty() {
                                None
                            } else {
                                Some(tool_kind)
                            },
                            is_error,
                        });
                        *order += 1;
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

fn canonical_chat_entry(
    session_id: &str,
    scope: &DerivationScope<'_>,
    order: i32,
    actor: TranscriptActor,
    variant: TranscriptVariant,
    text: &str,
) -> TranscriptEntry {
    TranscriptEntry {
        entry_id: make_entry_id(session_id, scope, order),
        session_id: session_id.to_string(),
        turn_id: scope.turn_id().map(str::to_string),
        order,
        timestamp: None,
        actor,
        variant,
        source: TranscriptSource::Transcript,
        text: text.to_string(),
        tool_use_id: None,
        tool_kind: None,
        is_error: false,
    }
}

fn stringify_tool_result(content: &Value) -> String {
    match content {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        Value::Array(items) => {
            let mut parts = Vec::new();
            for item in items {
                if let Some(text) = item.as_str() {
                    parts.push(text.to_string());
                } else if let Some(text) = item.get("text").and_then(Value::as_str) {
                    parts.push(text.to_string());
                } else if let Ok(s) = serde_json::to_string(item) {
                    parts.push(s);
                }
            }
            parts.join("\n")
        }
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

fn looks_like_subagent_result_content(content: &Value) -> bool {
    match content {
        Value::String(text) => text.contains("agentId:"),
        Value::Array(items) => items.iter().any(|item| {
            item.as_str().is_some_and(|text| text.contains("agentId:"))
                || item
                    .as_object()
                    .and_then(|value| value.get("text"))
                    .and_then(Value::as_str)
                    .is_some_and(|text| text.contains("agentId:"))
        }),
        _ => content.to_string().contains("agentId:"),
    }
}

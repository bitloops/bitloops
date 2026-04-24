use std::collections::HashMap;

use anyhow::Result;
use serde_json::Value;

use crate::adapters::agents::TranscriptToolEventDeriver;
use crate::host::checkpoints::transcript::parse::parse_from_bytes;
use crate::host::checkpoints::transcript::types::{
    AssistantMessage, CONTENT_TYPE_TOOL_USE, TYPE_ASSISTANT, TYPE_USER,
};
use crate::host::interactions::tool_events::TranscriptToolEventObservation;

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

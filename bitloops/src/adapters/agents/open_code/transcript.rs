use std::collections::HashSet;
use std::fs;

use anyhow::Result;
use serde_json::{Map, Value};

use crate::adapters::agents::{TokenUsage, TranscriptToolEventDeriver};
use crate::host::interactions::tool_events::TranscriptToolEventObservation;

use super::agent_api::OpenCodeAgent;
use super::types::{FILE_MODIFICATION_TOOLS, Message, ROLE_ASSISTANT, ROLE_USER, ToolState};

pub fn parse_messages(data: &[u8]) -> Result<Vec<Message>> {
    if data.is_empty() {
        return Ok(Vec::new());
    }

    let mut messages = Vec::new();
    for line in data.split(|b| *b == b'\n') {
        if line.iter().all(|byte| byte.is_ascii_whitespace()) {
            continue;
        }

        if let Ok(message) = serde_json::from_slice::<Message>(line) {
            messages.push(message);
        }
    }

    Ok(messages)
}

pub fn parse_messages_from_file(path: &str) -> Result<Vec<Message>> {
    let data = fs::read(path)?;
    parse_messages(&data)
}

pub fn extract_modified_files(data: &[u8]) -> Result<Vec<String>> {
    let messages = parse_messages(data)?;
    let mut seen = HashSet::new();
    let mut files = Vec::new();

    for message in messages {
        if message.role != ROLE_ASSISTANT {
            continue;
        }

        for part in message.parts {
            let Some(state) = part.state else {
                continue;
            };
            if part.part_type != "tool" {
                continue;
            }
            if !FILE_MODIFICATION_TOOLS.contains(&part.tool.as_str()) {
                continue;
            }

            let file_path = extract_file_path_from_input(&state.input);
            if !file_path.is_empty() && seen.insert(file_path.clone()) {
                files.push(file_path);
            }
        }
    }

    Ok(files)
}

pub fn extract_all_user_prompts(data: &[u8]) -> Result<Vec<String>> {
    let messages = parse_messages(data)?;
    let mut prompts = Vec::new();
    for message in messages {
        if message.role == ROLE_USER && !message.content.is_empty() {
            prompts.push(message.content);
        }
    }
    Ok(prompts)
}

pub fn extract_file_path_from_input(input: &Map<String, Value>) -> String {
    for key in ["file_path", "path", "file", "filename"] {
        if let Some(Value::String(path)) = input.get(key)
            && !path.is_empty()
        {
            return path.clone();
        }
    }
    String::new()
}

pub fn calculate_token_usage_from_bytes(data: &[u8], start_message_index: usize) -> TokenUsage {
    let Ok(messages) = parse_messages(data) else {
        return TokenUsage::default();
    };

    let mut usage = TokenUsage::default();
    for message in messages.into_iter().skip(start_message_index) {
        let Some(tokens) = message.tokens else {
            continue;
        };
        if message.role != ROLE_ASSISTANT {
            continue;
        }

        usage.input_tokens += tokens.input;
        usage.output_tokens += tokens.output;
        usage.cache_read_tokens += tokens.cache.read;
        usage.cache_creation_tokens += tokens.cache.write;
        usage.api_call_count += 1;
    }

    usage
}

impl TranscriptToolEventDeriver for OpenCodeAgent {
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
    let messages = parse_messages(transcript_fragment.as_bytes())?;
    let mut observations = Vec::new();
    let mut tool_use_block_number = 1_i64;

    for message in messages {
        if message.role != ROLE_ASSISTANT {
            continue;
        }

        for part in message.parts {
            if part.part_type != "tool" {
                continue;
            }

            let tool_name = part.tool.trim().to_string();
            if tool_name.is_empty() {
                continue;
            }

            let fallback_tool_use_block_number = tool_use_block_number;
            tool_use_block_number += 1;
            let tool_use_id = if part.call_id.trim().is_empty() {
                format!("{turn_id}:tool:{fallback_tool_use_block_number:04}")
            } else {
                part.call_id.trim().to_string()
            };

            if tool_name.eq_ignore_ascii_case("task") {
                continue;
            }

            let state = part.state.unwrap_or_default();
            observations.push(TranscriptToolEventObservation::Invocation {
                tool_use_id: tool_use_id.clone(),
                tool_name: tool_name.clone(),
                tool_input: Value::Object(state.input.clone()),
            });

            if has_tool_result(&state) {
                observations.push(TranscriptToolEventObservation::Result {
                    tool_use_id,
                    tool_name,
                    tool_output: state.output,
                });
            }
        }
    }

    Ok(observations)
}

fn has_tool_result(state: &ToolState) -> bool {
    !state.status.trim().is_empty() || !state.output.is_null()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::derive_tool_event_observations;
    use crate::host::interactions::tool_events::TranscriptToolEventObservation;

    #[test]
    fn derives_opencode_tool_event_observations_from_transcript_fragment() {
        let fragment = concat!(
            "{\"id\":\"msg-1\",\"role\":\"assistant\",\"content\":\"\",\"parts\":[",
            "{\"type\":\"tool\",\"tool\":\"read\",\"callID\":\"call_read\",\"state\":{\"status\":\"completed\",\"input\":{\"filePath\":\"src/lib.rs\"},\"output\":\"loaded file\"}},",
            "{\"type\":\"tool\",\"tool\":\"bash\",\"callID\":\"call_bash\",\"state\":{\"status\":\"completed\",\"input\":{\"command\":\"rg interaction_events src\",\"description\":\"Search interaction events\"},\"output\":\"found matches\"}}",
            "]}\n"
        );

        let observations =
            derive_tool_event_observations("turn-1", fragment).expect("derive observations");

        assert_eq!(
            observations,
            vec![
                TranscriptToolEventObservation::Invocation {
                    tool_use_id: "call_read".to_string(),
                    tool_name: "read".to_string(),
                    tool_input: json!({"filePath":"src/lib.rs"}),
                },
                TranscriptToolEventObservation::Result {
                    tool_use_id: "call_read".to_string(),
                    tool_name: "read".to_string(),
                    tool_output: json!("loaded file"),
                },
                TranscriptToolEventObservation::Invocation {
                    tool_use_id: "call_bash".to_string(),
                    tool_name: "bash".to_string(),
                    tool_input: json!({"command":"rg interaction_events src","description":"Search interaction events"}),
                },
                TranscriptToolEventObservation::Result {
                    tool_use_id: "call_bash".to_string(),
                    tool_name: "bash".to_string(),
                    tool_output: json!("found matches"),
                },
            ]
        );
    }

    #[test]
    fn ignores_opencode_task_tool_parts_and_preserves_fallback_ids() {
        let fragment = concat!(
            "{\"id\":\"msg-1\",\"role\":\"assistant\",\"content\":\"\",\"parts\":[",
            "{\"type\":\"tool\",\"tool\":\"task\",\"state\":{\"status\":\"completed\",\"input\":{\"prompt\":\"delegate\"},\"output\":\"ignored\"}},",
            "{\"type\":\"tool\",\"tool\":\"edit\",\"state\":{\"status\":\"completed\",\"input\":{\"filePath\":\"src/lib.rs\"},\"output\":\"updated file\"}}",
            "]}\n"
        );

        let observations =
            derive_tool_event_observations("turn-1", fragment).expect("derive observations");

        assert_eq!(
            observations,
            vec![
                TranscriptToolEventObservation::Invocation {
                    tool_use_id: "turn-1:tool:0002".to_string(),
                    tool_name: "edit".to_string(),
                    tool_input: json!({"filePath":"src/lib.rs"}),
                },
                TranscriptToolEventObservation::Result {
                    tool_use_id: "turn-1:tool:0002".to_string(),
                    tool_name: "edit".to_string(),
                    tool_output: json!("updated file"),
                },
            ]
        );
    }

    #[test]
    fn derives_opencode_structured_tool_results_without_stringifying() {
        let fragment = concat!(
            "{\"id\":\"msg-1\",\"role\":\"assistant\",\"content\":\"\",\"parts\":[",
            "{\"type\":\"tool\",\"tool\":\"read\",\"callID\":\"call_read\",\"state\":{\"status\":\"completed\",\"input\":{\"filePath\":\"src/lib.rs\"},\"output\":{\"status\":\"ok\",\"lines\":10}}}",
            "]}\n"
        );

        let observations =
            derive_tool_event_observations("turn-1", fragment).expect("derive observations");

        assert_eq!(
            observations,
            vec![
                TranscriptToolEventObservation::Invocation {
                    tool_use_id: "call_read".to_string(),
                    tool_name: "read".to_string(),
                    tool_input: json!({"filePath":"src/lib.rs"}),
                },
                TranscriptToolEventObservation::Result {
                    tool_use_id: "call_read".to_string(),
                    tool_name: "read".to_string(),
                    tool_output: json!({"status":"ok","lines":10}),
                },
            ]
        );
    }
}

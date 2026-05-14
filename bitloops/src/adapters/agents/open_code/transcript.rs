use std::collections::HashSet;
use std::fs;

use anyhow::Result;
use serde_json::{Map, Value};

use crate::adapters::agents::{TokenUsage, TranscriptEntryDeriver, TranscriptToolEventDeriver};
use crate::host::interactions::tool_events::TranscriptToolEventObservation;
use crate::host::interactions::transcript_entry::{
    DerivationScope, TranscriptActor, TranscriptEntry, TranscriptSource, TranscriptVariant,
    make_derived_tool_use_id, make_entry_id,
};

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

impl TranscriptEntryDeriver for OpenCodeAgent {
    fn derive_transcript_entries(
        &self,
        session_id: &str,
        turn_id: Option<&str>,
        transcript: &str,
    ) -> Result<Vec<TranscriptEntry>> {
        derive_transcript_entries(session_id, turn_id, transcript)
    }
}

/// Normalize an OpenCode transcript fragment (or full transcript) into canonical
/// `TranscriptEntry` rows. Preserves message order; within an assistant message,
/// text parts and tool parts are emitted in document order, with tool result
/// rows following their matching tool use rows.
pub fn derive_transcript_entries(
    session_id: &str,
    turn_id: Option<&str>,
    transcript: &str,
) -> Result<Vec<TranscriptEntry>> {
    let messages = parse_messages(transcript.as_bytes())?;
    let scope = match turn_id {
        Some(id) => DerivationScope::Turn(id),
        None => DerivationScope::Session,
    };
    let mut entries: Vec<TranscriptEntry> = Vec::new();
    let mut order: i32 = 0;
    let mut tool_call_index: i32 = 0;

    for message in messages {
        let role = message.role.as_str();

        if role == ROLE_USER {
            let text = message.content.trim();
            if !text.is_empty() {
                entries.push(make_chat_entry(
                    session_id,
                    &scope,
                    &mut order,
                    &message,
                    TranscriptActor::User,
                    text,
                ));
            }
            continue;
        }

        if role != ROLE_ASSISTANT {
            continue;
        }

        let mut emitted_assistant_text = false;

        for part in &message.parts {
            match part.part_type.as_str() {
                "text" => {
                    let text = part.text.trim();
                    if text.is_empty() {
                        continue;
                    }
                    entries.push(make_chat_entry(
                        session_id,
                        &scope,
                        &mut order,
                        &message,
                        TranscriptActor::Assistant,
                        text,
                    ));
                    emitted_assistant_text = true;
                }
                "tool" => {
                    let tool_kind = part.tool.trim();
                    if tool_kind.is_empty() {
                        continue;
                    }
                    // Task delegation is handled separately by the tool-event path.
                    if tool_kind.eq_ignore_ascii_case("task") {
                        continue;
                    }

                    let tool_use_id = if part.call_id.trim().is_empty() {
                        make_derived_tool_use_id(session_id, &scope, tool_call_index)
                    } else {
                        part.call_id.trim().to_string()
                    };
                    tool_call_index += 1;

                    let state = part.state.clone().unwrap_or_default();
                    let tool_input_summary = stringify_tool_input(&state.input);
                    let tool_use_text = if tool_input_summary.is_empty() {
                        format!("Tool: {tool_kind}")
                    } else {
                        format!("Tool: {tool_kind}\n{tool_input_summary}")
                    };

                    entries.push(TranscriptEntry {
                        entry_id: make_entry_id(session_id, &scope, order),
                        session_id: session_id.to_string(),
                        turn_id: scope.turn_id().map(str::to_string),
                        order,
                        timestamp: timestamp_from_message(&message),
                        actor: TranscriptActor::System,
                        variant: TranscriptVariant::ToolUse,
                        source: TranscriptSource::Transcript,
                        text: tool_use_text,
                        tool_use_id: Some(tool_use_id.clone()),
                        tool_kind: Some(tool_kind.to_string()),
                        is_error: false,
                    });
                    order += 1;

                    if has_tool_result(&state) {
                        let (output_text, is_error) = stringify_tool_output(&state);
                        entries.push(TranscriptEntry {
                            entry_id: make_entry_id(session_id, &scope, order),
                            session_id: session_id.to_string(),
                            turn_id: scope.turn_id().map(str::to_string),
                            order,
                            timestamp: timestamp_from_message(&message),
                            actor: TranscriptActor::System,
                            variant: TranscriptVariant::ToolResult,
                            source: TranscriptSource::Transcript,
                            text: output_text,
                            tool_use_id: Some(tool_use_id),
                            tool_kind: Some(tool_kind.to_string()),
                            is_error,
                        });
                        order += 1;
                    }
                }
                _ => {}
            }
        }

        // Fall back to top-level `content` only if no text parts were emitted.
        if !emitted_assistant_text {
            let text = message.content.trim();
            if !text.is_empty() {
                entries.push(make_chat_entry(
                    session_id,
                    &scope,
                    &mut order,
                    &message,
                    TranscriptActor::Assistant,
                    text,
                ));
            }
        }
    }

    Ok(entries)
}

fn make_chat_entry(
    session_id: &str,
    scope: &DerivationScope<'_>,
    order: &mut i32,
    message: &Message,
    actor: TranscriptActor,
    text: &str,
) -> TranscriptEntry {
    let entry = TranscriptEntry {
        entry_id: make_entry_id(session_id, scope, *order),
        session_id: session_id.to_string(),
        turn_id: scope.turn_id().map(str::to_string),
        order: *order,
        timestamp: timestamp_from_message(message),
        actor,
        variant: TranscriptVariant::Chat,
        source: TranscriptSource::Transcript,
        text: text.to_string(),
        tool_use_id: None,
        tool_kind: None,
        is_error: false,
    };
    *order += 1;
    entry
}

fn timestamp_from_message(message: &Message) -> Option<String> {
    if message.time.created == 0 {
        None
    } else {
        Some(message.time.created.to_string())
    }
}

fn stringify_tool_input(input: &Map<String, Value>) -> String {
    if input.is_empty() {
        return String::new();
    }
    serde_json::to_string(&Value::Object(input.clone())).unwrap_or_default()
}

fn stringify_tool_output(state: &ToolState) -> (String, bool) {
    let is_error = state.status.eq_ignore_ascii_case("error");
    let text = match &state.output {
        Value::Null => state.status.clone(),
        Value::String(s) => s.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    };
    (text, is_error)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{derive_tool_event_observations, derive_transcript_entries};
    use crate::host::interactions::tool_events::TranscriptToolEventObservation;
    use crate::host::interactions::transcript_entry::{
        TranscriptActor, TranscriptSource, TranscriptVariant,
    };

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

    #[test]
    fn user_message_produces_user_chat_entry() {
        let fragment = "{\"id\":\"msg-1\",\"role\":\"user\",\"content\":\"hello world\"}\n";
        let entries = derive_transcript_entries("sess-1", Some("turn-1"), fragment)
            .expect("derive entries");
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.actor, TranscriptActor::User);
        assert_eq!(entry.variant, TranscriptVariant::Chat);
        assert_eq!(entry.text, "hello world");
        assert_eq!(entry.session_id, "sess-1");
        assert_eq!(entry.turn_id.as_deref(), Some("turn-1"));
        assert_eq!(entry.order, 0);
        assert_eq!(entry.source, TranscriptSource::Transcript);
        assert!(entry.tool_use_id.is_none());
        assert!(entry.tool_kind.is_none());
        assert!(!entry.is_error);
    }

    #[test]
    fn assistant_text_part_produces_assistant_chat_entry() {
        let fragment = concat!(
            "{\"id\":\"msg-2\",\"role\":\"assistant\",\"content\":\"\",\"parts\":[",
            "{\"type\":\"text\",\"text\":\"reasoning result\"}",
            "]}\n"
        );
        let entries = derive_transcript_entries("sess-1", Some("turn-1"), fragment)
            .expect("derive entries");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].actor, TranscriptActor::Assistant);
        assert_eq!(entries[0].variant, TranscriptVariant::Chat);
        assert_eq!(entries[0].text, "reasoning result");
    }

    #[test]
    fn tool_part_with_call_id_emits_paired_use_and_result() {
        let fragment = concat!(
            "{\"id\":\"msg-3\",\"role\":\"assistant\",\"content\":\"\",\"parts\":[",
            "{\"type\":\"tool\",\"tool\":\"bash\",\"callID\":\"call_bash_1\",\"state\":{\"status\":\"completed\",\"input\":{\"command\":\"ls\"},\"output\":\"a\\nb\"}}",
            "]}\n"
        );
        let entries = derive_transcript_entries("sess-1", Some("turn-1"), fragment)
            .expect("derive entries");
        assert_eq!(entries.len(), 2);
        let tool_use = &entries[0];
        let tool_result = &entries[1];
        assert_eq!(tool_use.variant, TranscriptVariant::ToolUse);
        assert_eq!(tool_use.tool_use_id.as_deref(), Some("call_bash_1"));
        assert_eq!(tool_use.tool_kind.as_deref(), Some("bash"));
        assert!(tool_use.text.starts_with("Tool: bash"));
        assert!(tool_use.text.contains("\"command\":\"ls\""));
        assert_eq!(tool_result.variant, TranscriptVariant::ToolResult);
        assert_eq!(tool_result.tool_use_id.as_deref(), Some("call_bash_1"));
        assert_eq!(tool_result.text, "a\nb");
        assert!(!tool_result.is_error);
    }

    #[test]
    fn tool_part_without_call_id_generates_derived_id() {
        let fragment = concat!(
            "{\"id\":\"msg-4\",\"role\":\"assistant\",\"content\":\"\",\"parts\":[",
            "{\"type\":\"tool\",\"tool\":\"read\",\"state\":{\"status\":\"completed\",\"input\":{\"filePath\":\"src/lib.rs\"},\"output\":\"loaded\"}}",
            "]}\n"
        );
        let entries = derive_transcript_entries("sess-1", Some("turn-2"), fragment)
            .expect("derive entries");
        assert_eq!(entries.len(), 2);
        let id = entries[0].tool_use_id.clone().expect("tool_use_id");
        assert_eq!(id, "derived:sess-1:turn-2:0");
        assert_eq!(entries[1].tool_use_id.as_deref(), Some(id.as_str()));
    }

    #[test]
    fn task_tool_part_is_skipped_in_canonical_entries() {
        let fragment = concat!(
            "{\"id\":\"msg-5\",\"role\":\"assistant\",\"content\":\"\",\"parts\":[",
            "{\"type\":\"tool\",\"tool\":\"task\",\"callID\":\"call_task\",\"state\":{\"status\":\"completed\",\"input\":{\"prompt\":\"delegate\"},\"output\":\"x\"}},",
            "{\"type\":\"tool\",\"tool\":\"edit\",\"callID\":\"call_edit\",\"state\":{\"status\":\"completed\",\"input\":{\"filePath\":\"src/lib.rs\"},\"output\":\"updated\"}}",
            "]}\n"
        );
        let entries = derive_transcript_entries("sess-1", Some("turn-1"), fragment)
            .expect("derive entries");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].tool_kind.as_deref(), Some("edit"));
        assert_eq!(entries[1].tool_kind.as_deref(), Some("edit"));
    }

    #[test]
    fn mixed_messages_preserve_order_and_pair_ids() {
        let fragment = concat!(
            "{\"id\":\"msg-1\",\"role\":\"user\",\"content\":\"please run tests\"}\n",
            "{\"id\":\"msg-2\",\"role\":\"assistant\",\"content\":\"\",\"parts\":[",
            "{\"type\":\"text\",\"text\":\"running now\"},",
            "{\"type\":\"tool\",\"tool\":\"bash\",\"callID\":\"call_bash_2\",\"state\":{\"status\":\"completed\",\"input\":{\"command\":\"cargo test\"},\"output\":\"ok\"}}",
            "]}\n"
        );
        let entries = derive_transcript_entries("sess-7", Some("turn-3"), fragment)
            .expect("derive entries");
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0].actor, TranscriptActor::User);
        assert_eq!(entries[1].actor, TranscriptActor::Assistant);
        assert_eq!(entries[1].variant, TranscriptVariant::Chat);
        assert_eq!(entries[2].variant, TranscriptVariant::ToolUse);
        assert_eq!(entries[3].variant, TranscriptVariant::ToolResult);
        assert_eq!(entries[2].tool_use_id, entries[3].tool_use_id);
        for (idx, entry) in entries.iter().enumerate() {
            assert_eq!(entry.order, idx as i32);
        }
    }

    #[test]
    fn top_level_content_used_as_fallback_when_no_text_parts() {
        let fragment = "{\"id\":\"msg-6\",\"role\":\"assistant\",\"content\":\"fallback assistant text\",\"parts\":[]}\n";
        let entries =
            derive_transcript_entries("sess-1", None, fragment).expect("derive entries");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].actor, TranscriptActor::Assistant);
        assert_eq!(entries[0].text, "fallback assistant text");
        assert!(entries[0].turn_id.is_none());
        assert!(entries[0].entry_id.contains(":session:"));
    }

    #[test]
    fn tool_error_status_sets_is_error_on_result_only() {
        let fragment = concat!(
            "{\"id\":\"msg-7\",\"role\":\"assistant\",\"content\":\"\",\"parts\":[",
            "{\"type\":\"tool\",\"tool\":\"bash\",\"callID\":\"call_err\",\"state\":{\"status\":\"error\",\"input\":{\"command\":\"nope\"},\"output\":\"command not found\"}}",
            "]}\n"
        );
        let entries = derive_transcript_entries("sess-1", Some("turn-1"), fragment)
            .expect("derive entries");
        assert_eq!(entries.len(), 2);
        assert!(!entries[0].is_error);
        assert!(entries[1].is_error);
    }
}

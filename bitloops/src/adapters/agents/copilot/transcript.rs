use std::collections::HashSet;
use std::io::{BufRead, BufReader, Cursor};

use anyhow::{Result, anyhow};
use serde::Deserialize;
use serde_json::Value;

use crate::adapters::agents::{TokenUsage, TranscriptEntryDeriver};
use crate::host::interactions::transcript_entry::{
    DerivationScope, TranscriptActor, TranscriptEntry, TranscriptSource, TranscriptVariant,
    make_derived_tool_use_id, make_entry_id,
};

use super::agent::CopilotCliAgent;

pub const EVENT_TYPE_USER_MESSAGE: &str = "user.message";
pub const EVENT_TYPE_ASSISTANT_MESSAGE: &str = "assistant.message";
pub const EVENT_TYPE_TOOL_EXECUTION_COMPLETE: &str = "tool.execution_complete";
pub const EVENT_TYPE_MODEL_CHANGE: &str = "session.model_change";
pub const EVENT_TYPE_SESSION_SHUTDOWN: &str = "session.shutdown";

#[derive(Debug, Clone, Deserialize, Default)]
pub struct CopilotEvent {
    #[serde(default, rename = "type")]
    pub event_type: String,
    #[serde(default)]
    pub data: Value,
    #[serde(default)]
    pub id: String,
    #[serde(default, rename = "parentId")]
    pub parent_id: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct UserMessageData {
    #[serde(default)]
    content: String,
    #[serde(default, rename = "transformedContent")]
    transformed_content: String,
}

impl UserMessageData {
    fn best_prompt(&self) -> &str {
        if !self.content.trim().is_empty() {
            &self.content
        } else {
            &self.transformed_content
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
struct AssistantMessageData {
    #[serde(default)]
    content: String,
    #[serde(default, rename = "outputTokens")]
    output_tokens: i32,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct ModelChangeData {
    #[serde(default, rename = "newModel")]
    new_model: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct ToolExecutionCompleteData {
    #[serde(default)]
    model: String,
    #[serde(default, rename = "toolTelemetry")]
    tool_telemetry: ToolTelemetry,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct ToolTelemetry {
    #[serde(default)]
    properties: ToolProperties,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct ToolProperties {
    #[serde(default, rename = "filePaths")]
    file_paths: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct SessionShutdownData {
    #[serde(default, rename = "modelMetrics")]
    model_metrics: Vec<ModelMetric>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct ModelMetric {
    #[serde(default)]
    requests: ModelRequests,
    #[serde(default)]
    usage: ModelUsage,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct ModelRequests {
    #[serde(default)]
    count: i32,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct ModelUsage {
    #[serde(default, rename = "inputTokens")]
    input_tokens: i32,
    #[serde(default, rename = "outputTokens")]
    output_tokens: i32,
    #[serde(default, rename = "cacheReadTokens")]
    cache_read_tokens: i32,
    #[serde(default, rename = "cacheWriteTokens")]
    cache_write_tokens: i32,
}

pub fn parse_events_from_offset(
    data: &[u8],
    start_offset: usize,
) -> Result<(Vec<CopilotEvent>, usize)> {
    let mut events = Vec::new();
    let mut line_count = 0usize;
    let reader = BufReader::new(Cursor::new(data));

    for line in reader.lines() {
        let line = line.map_err(|err| anyhow!("transcript scanner error: {err}"))?;
        line_count += 1;
        if line_count <= start_offset || line.trim().is_empty() {
            continue;
        }

        if let Ok(event) = serde_json::from_str::<CopilotEvent>(&line) {
            events.push(event);
        }
    }

    Ok((events, line_count))
}

pub fn get_transcript_position_from_bytes(data: &[u8]) -> Result<usize> {
    let (_, line_count) = parse_events_from_offset(data, 0)?;
    Ok(line_count)
}

pub fn extract_prompts_from_events(events: &[CopilotEvent]) -> Vec<String> {
    let mut prompts = Vec::new();

    for event in events {
        if event.event_type != EVENT_TYPE_USER_MESSAGE {
            continue;
        }

        let Ok(data) = serde_json::from_value::<UserMessageData>(event.data.clone()) else {
            continue;
        };
        let prompt = data.best_prompt().trim();
        if !prompt.is_empty() {
            prompts.push(prompt.to_string());
        }
    }

    prompts
}

pub fn extract_summary_from_events(events: &[CopilotEvent]) -> String {
    for event in events.iter().rev() {
        if event.event_type != EVENT_TYPE_ASSISTANT_MESSAGE {
            continue;
        }

        let Ok(data) = serde_json::from_value::<AssistantMessageData>(event.data.clone()) else {
            continue;
        };
        if !data.content.is_empty() {
            return data.content;
        }
    }

    String::new()
}

pub fn extract_modified_files_from_events(events: &[CopilotEvent]) -> Vec<String> {
    let mut files = Vec::new();
    let mut seen = HashSet::new();

    for event in events {
        if event.event_type != EVENT_TYPE_TOOL_EXECUTION_COMPLETE {
            continue;
        }

        let Ok(data) = serde_json::from_value::<ToolExecutionCompleteData>(event.data.clone())
        else {
            continue;
        };

        if data.tool_telemetry.properties.file_paths.is_empty() {
            continue;
        }

        let parsed_paths =
            serde_json::from_str::<Vec<String>>(&data.tool_telemetry.properties.file_paths);
        let Ok(paths) = parsed_paths else {
            continue;
        };

        for path in paths {
            let normalized = path.trim();
            if !normalized.is_empty() && seen.insert(normalized.to_string()) {
                files.push(normalized.to_string());
            }
        }
    }

    files
}

pub fn extract_model_from_events(events: &[CopilotEvent]) -> String {
    for event in events.iter().rev() {
        if event.event_type != EVENT_TYPE_MODEL_CHANGE {
            continue;
        }

        let Ok(data) = serde_json::from_value::<ModelChangeData>(event.data.clone()) else {
            continue;
        };
        if !data.new_model.is_empty() {
            return data.new_model;
        }
    }

    for event in events.iter().rev() {
        if event.event_type != EVENT_TYPE_TOOL_EXECUTION_COMPLETE {
            continue;
        }

        let Ok(data) = serde_json::from_value::<ToolExecutionCompleteData>(event.data.clone())
        else {
            continue;
        };
        if !data.model.is_empty() {
            return data.model;
        }
    }

    String::new()
}

pub fn calculate_token_usage_from_events(events: &[CopilotEvent]) -> TokenUsage {
    for event in events.iter().rev() {
        if event.event_type != EVENT_TYPE_SESSION_SHUTDOWN {
            continue;
        }

        let Ok(data) = serde_json::from_value::<SessionShutdownData>(event.data.clone()) else {
            continue;
        };

        let mut token_usage = TokenUsage::default();
        for metric in data.model_metrics {
            token_usage.input_tokens += metric.usage.input_tokens;
            token_usage.output_tokens += metric.usage.output_tokens;
            token_usage.cache_read_tokens += metric.usage.cache_read_tokens;
            token_usage.cache_creation_tokens += metric.usage.cache_write_tokens;
            token_usage.api_call_count += metric.requests.count;
        }
        return token_usage;
    }

    let mut fallback = TokenUsage::default();
    for event in events {
        if event.event_type != EVENT_TYPE_ASSISTANT_MESSAGE {
            continue;
        }

        if let Ok(data) = serde_json::from_value::<AssistantMessageData>(event.data.clone()) {
            fallback.output_tokens += data.output_tokens;
            if data.output_tokens > 0 {
                fallback.api_call_count += 1;
            }
        }
    }

    fallback
}

/// Best-effort extraction of the Copilot tool name from a
/// `tool.execution_complete` event's `data` payload. Copilot's schema doesn't
/// publish a single canonical field for this, so try the documented and
/// observed locations and return the first non-empty string.
fn extract_copilot_tool_name(data: &Value) -> Option<String> {
    // Real Copilot `tool.execution_complete` payloads observed in practice
    // populate `toolTelemetry.properties.command` (values like `view`, `edit`,
    // `report_intent`). The other field names below are kept as best-effort
    // fallbacks in case Copilot's schema shifts; `command` is the load-bearing
    // one for today's transcripts.
    let candidates: &[&[&str]] = &[
        &["name"],
        &["toolName"],
        &["tool_name"],
        &["tool"],
        &["kind"],
        &["toolTelemetry", "properties", "command"],
        &["toolTelemetry", "properties", "toolName"],
        &["toolTelemetry", "properties", "name"],
        &["toolTelemetry", "properties", "tool"],
        &["toolTelemetry", "properties", "kind"],
        &["toolTelemetry", "properties", "type"],
        &["toolTelemetry", "properties", "viewType"],
    ];
    for path in candidates {
        let mut current = data;
        let mut found = true;
        for key in *path {
            match current.get(*key) {
                Some(value) => current = value,
                None => {
                    found = false;
                    break;
                }
            }
        }
        if found && let Some(text) = current.as_str() {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

/// Best-effort extraction of a short input summary for a Copilot
/// `tool.execution_complete` event.
///
/// Copilot publishes file paths in one of two locations depending on the
/// tool: the `view` family uses `toolTelemetry.properties.filePaths`,
/// whereas the `edit` family uses `toolTelemetry.restrictedProperties.filePaths`
/// (because edit paths are privacy-classified). Probe both and return the
/// first non-empty hit. Returns the empty string when nothing useful is
/// available.
fn extract_copilot_tool_input_summary(data: &Value) -> String {
    let paths = [
        "/toolTelemetry/properties/filePaths",
        "/toolTelemetry/restrictedProperties/filePaths",
    ];
    for pointer in paths {
        if let Some(text) = data.pointer(pointer).and_then(Value::as_str) {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return format!("files: {trimmed}");
            }
        }
    }
    String::new()
}

/// Best-effort extraction of the human-readable tool result for a Copilot
/// `tool.execution_complete` event.
///
/// Copilot publishes the short, LLM-facing summary at `data.result.content`
/// (e.g. `"Intent logged"`, `"File README.md updated with changes."`). For
/// view-style tools that field can carry the full content of a viewed file
/// — fine for the dashboard, but it can be megabytes long. Cap it to a
/// reasonable display length. Falls back to the literal `"Tool completed"`
/// only when no result text is available.
fn extract_copilot_tool_result(data: &Value) -> String {
    const MAX_RESULT_CHARS: usize = 2000;
    if let Some(text) = data.pointer("/result/content").and_then(Value::as_str) {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            return truncate_with_ellipsis(trimmed, MAX_RESULT_CHARS);
        }
    }
    "Tool completed".to_string()
}

fn truncate_with_ellipsis(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max_chars).collect();
    out.push('…');
    out
}

impl TranscriptEntryDeriver for CopilotCliAgent {
    fn derive_transcript_entries(
        &self,
        session_id: &str,
        turn_id: Option<&str>,
        transcript: &str,
    ) -> Result<Vec<TranscriptEntry>> {
        derive_transcript_entries(session_id, turn_id, transcript)
    }
}

/// Normalize a Copilot transcript fragment (or full transcript) into canonical
/// `TranscriptEntry` rows. Preserves event order; `tool.execution_complete`
/// events emit a `TOOL_USE` followed immediately by a `TOOL_RESULT`, both keyed
/// by a derived `tool_use_id` since Copilot's transcript does not expose one.
pub fn derive_transcript_entries(
    session_id: &str,
    turn_id: Option<&str>,
    transcript: &str,
) -> Result<Vec<TranscriptEntry>> {
    let (events, _) = parse_events_from_offset(transcript.as_bytes(), 0)?;
    let scope = match turn_id {
        Some(id) => DerivationScope::Turn(id),
        None => DerivationScope::Session,
    };
    let mut entries: Vec<TranscriptEntry> = Vec::new();
    let mut order: i32 = 0;
    let mut tool_call_index: i32 = 0;

    for event in events {
        match event.event_type.as_str() {
            EVENT_TYPE_USER_MESSAGE => {
                let Ok(data) = serde_json::from_value::<UserMessageData>(event.data.clone()) else {
                    continue;
                };
                let text = data.best_prompt().trim();
                if text.is_empty() {
                    continue;
                }
                entries.push(TranscriptEntry {
                    entry_id: make_entry_id(session_id, &scope, order),
                    session_id: session_id.to_string(),
                    turn_id: scope.turn_id().map(str::to_string),
                    order,
                    timestamp: None,
                    actor: TranscriptActor::User,
                    variant: TranscriptVariant::Chat,
                    source: TranscriptSource::Transcript,
                    text: text.to_string(),
                    tool_use_id: None,
                    tool_kind: None,
                    is_error: false,
                });
                order += 1;
            }
            EVENT_TYPE_ASSISTANT_MESSAGE => {
                let Ok(data) = serde_json::from_value::<AssistantMessageData>(event.data.clone())
                else {
                    continue;
                };
                let text = data.content.trim();
                if text.is_empty() {
                    continue;
                }
                entries.push(TranscriptEntry {
                    entry_id: make_entry_id(session_id, &scope, order),
                    session_id: session_id.to_string(),
                    turn_id: scope.turn_id().map(str::to_string),
                    order,
                    timestamp: None,
                    actor: TranscriptActor::Assistant,
                    variant: TranscriptVariant::Chat,
                    source: TranscriptSource::Transcript,
                    text: text.to_string(),
                    tool_use_id: None,
                    tool_kind: None,
                    is_error: false,
                });
                order += 1;
            }
            EVENT_TYPE_TOOL_EXECUTION_COMPLETE => {
                // The previous implementation pulled `tool_kind` from
                // `data.model` (which is the LLM name like "gpt-5"), not the
                // tool's identifier. Look in the documented and likely
                // locations for the actual tool name instead.
                let tool_kind = extract_copilot_tool_name(&event.data);
                let input_summary = extract_copilot_tool_input_summary(&event.data);
                let result_text = extract_copilot_tool_result(&event.data);
                let is_error = event
                    .data
                    .get("success")
                    .and_then(Value::as_bool)
                    .map(|s| !s)
                    .unwrap_or(false);

                let tool_use_id = make_derived_tool_use_id(session_id, &scope, tool_call_index);
                tool_call_index += 1;

                let kind_label = tool_kind.as_deref().unwrap_or("tool");
                let tool_use_text = if input_summary.is_empty() {
                    format!("Tool: {kind_label}")
                } else {
                    format!("Tool: {kind_label}\n{input_summary}")
                };

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
                    tool_use_id: Some(tool_use_id.clone()),
                    tool_kind: tool_kind.clone(),
                    is_error: false,
                });
                order += 1;

                entries.push(TranscriptEntry {
                    entry_id: make_entry_id(session_id, &scope, order),
                    session_id: session_id.to_string(),
                    turn_id: scope.turn_id().map(str::to_string),
                    order,
                    timestamp: None,
                    actor: TranscriptActor::System,
                    variant: TranscriptVariant::ToolResult,
                    source: TranscriptSource::Transcript,
                    text: result_text,
                    tool_use_id: Some(tool_use_id),
                    tool_kind,
                    is_error,
                });
                order += 1;
            }
            _ => {}
        }
    }

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_data() -> Vec<u8> {
        br#"{"type":"user.message","data":{"content":"Create hello.txt"}}
{"type":"tool.execution_complete","data":{"model":"gpt-5","toolTelemetry":{"properties":{"filePaths":"[\"hello.txt\"]"}}}}
{"type":"assistant.message","data":{"content":"Created hello.txt","outputTokens":42}}
{"type":"session.model_change","data":{"newModel":"gpt-5"}}
{"type":"session.shutdown","data":{"modelMetrics":[{"requests":{"count":1},"usage":{"inputTokens":100,"outputTokens":42,"cacheReadTokens":3,"cacheWriteTokens":5}}]}}
"#
        .to_vec()
    }

    #[test]
    fn parse_events_counts_lines() {
        let (_, position) = parse_events_from_offset(&sample_data(), 0).expect("parse");
        assert_eq!(position, 5);
    }

    #[test]
    fn transcript_position_counts_last_line_without_trailing_newline() {
        let position = get_transcript_position_from_bytes(
            br#"{"type":"assistant.message","data":{"content":"done"}}"#,
        )
        .expect("position");
        assert_eq!(position, 1);
    }

    #[test]
    fn parse_events_skips_malformed_lines() {
        let data = br#"{"type":"user.message","data":{"content":"hello"}}
not-json
{"type":"assistant.message","data":{"content":"done"}}
"#;
        let (events, position) = parse_events_from_offset(data, 0).expect("parse");
        assert_eq!(position, 3);
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn extract_prompts_reads_user_messages() {
        let (events, _) = parse_events_from_offset(&sample_data(), 0).expect("parse");
        assert_eq!(
            extract_prompts_from_events(&events),
            vec!["Create hello.txt"]
        );
    }

    #[test]
    fn extract_prompts_uses_transformed_content_when_content_is_empty() {
        let data = br#"{"type":"user.message","data":{"content":"","transformedContent":"Refactor parser"}}
"#;
        let (events, _) = parse_events_from_offset(data, 0).expect("parse");
        assert_eq!(
            extract_prompts_from_events(&events),
            vec!["Refactor parser"]
        );
    }

    #[test]
    fn extract_prompts_preserves_multi_turn_order() {
        let data = br#"{"type":"user.message","data":{"content":"First prompt"}}
{"type":"assistant.message","data":{"content":"done"}}
{"type":"user.message","data":{"content":"Second prompt"}}
"#;
        let (events, _) = parse_events_from_offset(data, 0).expect("parse");
        assert_eq!(
            extract_prompts_from_events(&events),
            vec!["First prompt", "Second prompt"]
        );
    }

    #[test]
    fn extract_summary_reads_last_assistant_message() {
        let (events, _) = parse_events_from_offset(&sample_data(), 0).expect("parse");
        assert_eq!(extract_summary_from_events(&events), "Created hello.txt");
    }

    #[test]
    fn extract_modified_files_reads_file_paths() {
        let (events, _) = parse_events_from_offset(&sample_data(), 0).expect("parse");
        assert_eq!(
            extract_modified_files_from_events(&events),
            vec!["hello.txt"]
        );
    }

    #[test]
    fn extract_modified_files_deduplicates_and_trims() {
        let data = br#"{"type":"tool.execution_complete","data":{"toolTelemetry":{"properties":{"filePaths":"[\" hello.txt \",\"hello.txt\",\"world.txt\"]"}}}}
"#;
        let (events, _) = parse_events_from_offset(data, 0).expect("parse");
        assert_eq!(
            extract_modified_files_from_events(&events),
            vec!["hello.txt", "world.txt"]
        );
    }

    #[test]
    fn extract_modified_files_skips_malformed_file_paths() {
        let data =
            br#"{"type":"tool.execution_complete","data":{"toolTelemetry":{"properties":{"filePaths":"not-json"}}}}
"#;
        let (events, _) = parse_events_from_offset(data, 0).expect("parse");
        assert!(extract_modified_files_from_events(&events).is_empty());
    }

    #[test]
    fn extract_modified_files_skips_empty_entries() {
        let data = br#"{"type":"tool.execution_complete","data":{"toolTelemetry":{"properties":{"filePaths":"[\"\",\"  \",\"src/main.rs\"]"}}}}
"#;
        let (events, _) = parse_events_from_offset(data, 0).expect("parse");
        assert_eq!(
            extract_modified_files_from_events(&events),
            vec!["src/main.rs"]
        );
    }

    #[test]
    fn extract_summary_skips_empty_assistant_messages() {
        let data = br#"{"type":"assistant.message","data":{"content":"Earlier summary"}}
{"type":"assistant.message","data":{"content":"","outputTokens":10}}
"#;
        let (events, _) = parse_events_from_offset(data, 0).expect("parse");
        assert_eq!(extract_summary_from_events(&events), "Earlier summary");
    }

    #[test]
    fn extract_model_prefers_model_change() {
        let (events, _) = parse_events_from_offset(&sample_data(), 0).expect("parse");
        assert_eq!(extract_model_from_events(&events), "gpt-5");
    }

    #[test]
    fn extract_model_falls_back_to_tool_execution_complete() {
        let data = br#"{"type":"tool.execution_complete","data":{"model":"gpt-5.2","toolTelemetry":{"properties":{"filePaths":"[]"}}}}
"#;
        let (events, _) = parse_events_from_offset(data, 0).expect("parse");
        assert_eq!(extract_model_from_events(&events), "gpt-5.2");
    }

    #[test]
    fn calculate_token_usage_reads_session_shutdown() {
        let (events, _) = parse_events_from_offset(&sample_data(), 0).expect("parse");
        let usage = calculate_token_usage_from_events(&events);
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 42);
        assert_eq!(usage.cache_read_tokens, 3);
        assert_eq!(usage.cache_creation_tokens, 5);
        assert_eq!(usage.api_call_count, 1);
    }

    #[test]
    fn calculate_token_usage_falls_back_to_assistant_output_tokens() {
        let data = br#"{"type":"assistant.message","data":{"content":"done","outputTokens":9}}
"#;
        let (events, _) = parse_events_from_offset(data, 0).expect("parse");
        let usage = calculate_token_usage_from_events(&events);
        assert_eq!(usage.output_tokens, 9);
        assert_eq!(usage.api_call_count, 1);
    }

    #[test]
    fn user_message_event_produces_user_chat_entry() {
        let fragment = r#"{"type":"user.message","data":{"content":"hello copilot"}}
"#;
        let entries =
            derive_transcript_entries("sess-1", Some("turn-1"), fragment).expect("derive entries");
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.actor, TranscriptActor::User);
        assert_eq!(entry.variant, TranscriptVariant::Chat);
        assert_eq!(entry.text, "hello copilot");
        assert_eq!(entry.session_id, "sess-1");
        assert_eq!(entry.turn_id.as_deref(), Some("turn-1"));
        assert_eq!(entry.order, 0);
        assert_eq!(entry.source, TranscriptSource::Transcript);
        assert!(entry.tool_use_id.is_none());
        assert!(entry.tool_kind.is_none());
        assert!(!entry.is_error);
    }

    #[test]
    fn assistant_message_event_produces_assistant_chat_entry() {
        let fragment = r#"{"type":"assistant.message","data":{"content":"done","outputTokens":12}}
"#;
        let entries =
            derive_transcript_entries("sess-1", Some("turn-1"), fragment).expect("derive entries");
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.actor, TranscriptActor::Assistant);
        assert_eq!(entry.variant, TranscriptVariant::Chat);
        assert_eq!(entry.text, "done");
        assert!(entry.tool_use_id.is_none());
        assert!(entry.tool_kind.is_none());
    }

    #[test]
    fn tool_execution_complete_emits_paired_tool_use_and_tool_result() {
        // This fixture intentionally has neither a tool-name field nor a
        // `result.content`. The deriver should fall back to a generic tool
        // label and the legacy "Tool completed" result string, but still emit
        // the proper paired entries.
        let fragment = r#"{"type":"tool.execution_complete","data":{"model":"gpt-5","toolTelemetry":{"properties":{"filePaths":"[\"hello.txt\"]"}}}}
"#;
        let entries =
            derive_transcript_entries("sess-1", Some("turn-1"), fragment).expect("derive entries");
        assert_eq!(entries.len(), 2);
        let tool_use = &entries[0];
        let tool_result = &entries[1];

        assert_eq!(tool_use.variant, TranscriptVariant::ToolUse);
        assert_eq!(tool_use.actor, TranscriptActor::System);
        // No `command` / `toolName` / `name` field, so tool_kind is None.
        assert!(tool_use.tool_kind.is_none());
        assert_eq!(
            tool_use.tool_use_id.as_deref(),
            Some("derived:sess-1:turn-1:0")
        );
        // Falls back to "tool" with the file-paths summary appended.
        assert_eq!(tool_use.text, "Tool: tool\nfiles: [\"hello.txt\"]");
        assert!(!tool_use.is_error);

        assert_eq!(tool_result.variant, TranscriptVariant::ToolResult);
        assert_eq!(tool_result.actor, TranscriptActor::System);
        assert_eq!(tool_result.tool_use_id, tool_use.tool_use_id);
        // tool_kind on the result mirrors the tool_use (None here).
        assert!(tool_result.tool_kind.is_none());
        // No `data.result.content` → fall back to the legacy literal.
        assert_eq!(tool_result.text, "Tool completed");
        assert!(!tool_result.is_error);
    }

    #[test]
    fn tool_execution_complete_uses_command_field_for_tool_kind() {
        // Real Copilot payload (lightly trimmed): `toolTelemetry.properties.command`
        // is the canonical tool-name field, and `result.content` holds the
        // human-readable result string.
        let fragment = r#"{"type":"tool.execution_complete","data":{"toolCallId":"call_X","model":"gpt-5-mini","success":true,"result":{"content":"File README.md updated with changes.","detailedContent":"diff --git ..."},"toolTelemetry":{"properties":{"command":"edit","options":"{}","inputs":"[]","fileExtension":"[\".md\"]"},"restrictedProperties":{"filePaths":"[\"/repo/README.md\"]"}}}}
"#;
        let entries =
            derive_transcript_entries("sess-2", Some("turn-2"), fragment).expect("derive entries");
        assert_eq!(entries.len(), 2);
        let tool_use = &entries[0];
        let tool_result = &entries[1];

        assert_eq!(tool_use.tool_kind.as_deref(), Some("edit"));
        assert_eq!(tool_use.text, "Tool: edit\nfiles: [\"/repo/README.md\"]");

        assert_eq!(tool_result.tool_kind.as_deref(), Some("edit"));
        assert_eq!(tool_result.text, "File README.md updated with changes.");
        assert!(!tool_result.is_error);
    }

    #[test]
    fn tool_execution_complete_marks_is_error_when_success_is_false() {
        let fragment = r#"{"type":"tool.execution_complete","data":{"toolCallId":"call_E","success":false,"result":{"content":"Command failed: permission denied"},"toolTelemetry":{"properties":{"command":"bash"}}}}
"#;
        let entries =
            derive_transcript_entries("sess-3", Some("turn-3"), fragment).expect("derive entries");
        assert_eq!(entries.len(), 2);
        let tool_result = &entries[1];
        assert_eq!(tool_result.tool_kind.as_deref(), Some("bash"));
        assert_eq!(tool_result.text, "Command failed: permission denied");
        assert!(tool_result.is_error);
    }

    #[test]
    fn tool_execution_complete_truncates_long_result_content() {
        // A tool whose result.content exceeds the dashboard cap — e.g. `view`
        // returning a large file — should be capped to MAX_RESULT_CHARS + 1
        // (one trailing ellipsis char).
        let long = "x".repeat(5000);
        let fragment = format!(
            r#"{{"type":"tool.execution_complete","data":{{"toolCallId":"c1","result":{{"content":"{long}"}},"toolTelemetry":{{"properties":{{"command":"view"}}}}}}}}{}"#,
            "\n"
        );
        let entries =
            derive_transcript_entries("sess-4", Some("turn-4"), &fragment).expect("derive entries");
        let tool_result = &entries[1];
        // 2000 cap + 1 ellipsis char.
        assert_eq!(tool_result.text.chars().count(), 2001);
        assert!(tool_result.text.ends_with('…'));
    }

    #[test]
    fn mixed_events_preserve_order() {
        let fragment = r#"{"type":"user.message","data":{"content":"Create hello.txt"}}
{"type":"tool.execution_complete","data":{"model":"gpt-5","toolTelemetry":{"properties":{"filePaths":"[\"hello.txt\"]"}}}}
{"type":"assistant.message","data":{"content":"Created hello.txt","outputTokens":42}}
"#;
        let entries =
            derive_transcript_entries("sess-7", Some("turn-3"), fragment).expect("derive entries");
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0].actor, TranscriptActor::User);
        assert_eq!(entries[0].variant, TranscriptVariant::Chat);
        assert_eq!(entries[1].variant, TranscriptVariant::ToolUse);
        assert_eq!(entries[2].variant, TranscriptVariant::ToolResult);
        assert_eq!(entries[1].tool_use_id, entries[2].tool_use_id);
        assert_eq!(entries[3].actor, TranscriptActor::Assistant);
        assert_eq!(entries[3].variant, TranscriptVariant::Chat);
        for (idx, entry) in entries.iter().enumerate() {
            assert_eq!(entry.order, idx as i32);
        }
    }

    #[test]
    fn model_change_event_is_skipped() {
        let fragment = r#"{"type":"user.message","data":{"content":"hi"}}
{"type":"session.model_change","data":{"newModel":"gpt-5"}}
{"type":"session.shutdown","data":{"modelMetrics":[]}}
{"type":"assistant.message","data":{"content":"hello"}}
"#;
        let entries =
            derive_transcript_entries("sess-1", Some("turn-1"), fragment).expect("derive entries");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].actor, TranscriptActor::User);
        assert_eq!(entries[0].text, "hi");
        assert_eq!(entries[1].actor, TranscriptActor::Assistant);
        assert_eq!(entries[1].text, "hello");
        assert_eq!(entries[0].order, 0);
        assert_eq!(entries[1].order, 1);
    }
}

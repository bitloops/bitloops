use std::collections::HashSet;
use std::io::{BufRead, BufReader, Cursor};

use anyhow::{Result, anyhow};
use serde::Deserialize;
use serde_json::Value;

use crate::engine::agent::TokenUsage;

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
        let position =
            get_transcript_position_from_bytes(br#"{"type":"assistant.message","data":{"content":"done"}}"#)
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
        assert_eq!(extract_prompts_from_events(&events), vec!["Refactor parser"]);
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
        assert_eq!(extract_modified_files_from_events(&events), vec!["src/main.rs"]);
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
}

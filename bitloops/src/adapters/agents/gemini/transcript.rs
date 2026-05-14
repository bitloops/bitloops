use std::collections::{HashMap, HashSet};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::adapters::agents::{TokenUsage, TranscriptEntryDeriver};
use crate::host::interactions::transcript_entry::{
    DerivationScope, TranscriptActor, TranscriptEntry, TranscriptSource, TranscriptVariant,
    make_derived_tool_use_id, make_entry_id,
};

use super::agent::GeminiCliAgent;

pub const MESSAGE_TYPE_USER: &str = "user";
pub const MESSAGE_TYPE_GEMINI: &str = "gemini";

pub const TOOL_WRITE_FILE: &str = "write_file";
pub const TOOL_EDIT_FILE: &str = "edit_file";
pub const TOOL_SAVE_FILE: &str = "save_file";
pub const TOOL_REPLACE: &str = "replace";

pub const FILE_MODIFICATION_TOOLS: [&str; 4] = [
    TOOL_WRITE_FILE,
    TOOL_EDIT_FILE,
    TOOL_SAVE_FILE,
    TOOL_REPLACE,
];

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GeminiTranscript {
    #[serde(default)]
    pub messages: Vec<GeminiMessage>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GeminiToolCall {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub args: HashMap<String, Value>,
    #[serde(default)]
    pub status: String,
    #[serde(default, rename = "displayName")]
    pub display_name: String,
    #[serde(default)]
    pub description: String,
    /// Raw result payload as it appears in Gemini's JSONL. Typically an
    /// array of `{functionResponse: {response: {output: "..."}}}` entries.
    /// Use `result_output()` to extract a displayable string.
    #[serde(default)]
    pub result: Vec<Value>,
}

impl GeminiToolCall {
    /// Best-effort extraction of a displayable string from `result`.
    /// Walks the array, looking for `functionResponse.response.output`.
    pub fn result_output(&self) -> String {
        for entry in &self.result {
            if let Some(output) = entry
                .pointer("/functionResponse/response/output")
                .and_then(Value::as_str)
            {
                let trimmed = output.trim();
                if !trimmed.is_empty() {
                    return trimmed.to_string();
                }
            }
        }
        String::new()
    }

    pub fn is_error_status(&self) -> bool {
        let s = self.status.trim();
        s.eq_ignore_ascii_case("error") || s.eq_ignore_ascii_case("failed")
    }
}

/// A Gemini "thinking" block. Real transcripts attach an array of these to
/// each `gemini` message; the canonical entry stream emits them as
/// `ASSISTANT/THINKING` rows.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GeminiThought {
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub timestamp: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GeminiMessageTokens {
    #[serde(default)]
    pub input: i32,
    #[serde(default)]
    pub output: i32,
    #[serde(default)]
    pub cached: i32,
    #[serde(default)]
    pub thoughts: i32,
    #[serde(default)]
    pub tool: i32,
    #[serde(default)]
    pub total: i32,
}

impl Serialize for GeminiMessageTokens {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("input", &self.input)?;
        map.serialize_entry("output", &self.output)?;
        map.serialize_entry("cached", &self.cached)?;
        if self.thoughts != 0 {
            map.serialize_entry("thoughts", &self.thoughts)?;
        }
        if self.tool != 0 {
            map.serialize_entry("tool", &self.tool)?;
        }
        if self.total != 0 {
            map.serialize_entry("total", &self.total)?;
        }
        map.end()
    }
}

#[derive(Debug, Clone, Default)]
pub struct GeminiMessage {
    pub id: String,
    pub r#type: String,
    pub content: String,
    pub tool_calls: Vec<GeminiToolCall>,
    pub thoughts: Vec<GeminiThought>,
    pub tokens: Option<GeminiMessageTokens>,
}

impl<'de> Deserialize<'de> for GeminiMessage {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Debug, Deserialize, Default)]
        struct RawGeminiMessage {
            #[serde(default)]
            id: String,
            #[serde(rename = "type", default)]
            r#type: String,
            #[serde(default)]
            content: Value,
            #[serde(rename = "toolCalls", default)]
            tool_calls: Vec<GeminiToolCall>,
            #[serde(default)]
            thoughts: Vec<GeminiThought>,
            #[serde(default)]
            tokens: Option<GeminiMessageTokens>,
        }

        let raw = RawGeminiMessage::deserialize(deserializer)?;
        let content = normalize_content(&raw.content);

        Ok(Self {
            id: raw.id,
            r#type: raw.r#type,
            content,
            tool_calls: raw.tool_calls,
            thoughts: raw.thoughts,
            tokens: raw.tokens,
        })
    }
}

impl Serialize for GeminiMessage {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap;

        let mut map = serializer.serialize_map(None)?;
        if !self.id.is_empty() {
            map.serialize_entry("id", &self.id)?;
        }
        map.serialize_entry("type", &self.r#type)?;
        map.serialize_entry("content", &self.content)?;
        if !self.thoughts.is_empty() {
            map.serialize_entry("thoughts", &self.thoughts)?;
        }
        if let Some(tokens) = &self.tokens {
            map.serialize_entry("tokens", tokens)?;
        }
        if !self.tool_calls.is_empty() {
            map.serialize_entry("toolCalls", &self.tool_calls)?;
        }
        map.end()
    }
}

fn normalize_content(value: &Value) -> String {
    if let Some(text) = value.as_str() {
        return text.to_string();
    }

    if value.is_null() {
        return String::new();
    }

    if let Some(parts) = value.as_array() {
        let mut out = Vec::new();
        for part in parts {
            if let Some(text) = part.get("text").and_then(Value::as_str)
                && !text.is_empty()
            {
                out.push(text.to_string());
            }
        }
        return out.join("\n");
    }

    String::new()
}

/// Parse a Gemini transcript, transparently handling either the real-world
/// JSONL format (current Gemini CLI) or the legacy `{"messages": [...]}` JSON
/// document (older fixtures and tests).
///
/// Tries JSONL first because that's what real Gemini sessions write. If JSONL
/// parsing produces no messages, falls back to the JSON-document shape.
pub fn parse_transcript(data: &[u8]) -> Result<GeminiTranscript> {
    if data.is_empty() {
        return Ok(GeminiTranscript::default());
    }

    if let Ok(t) = parse_transcript_jsonl(data)
        && !t.messages.is_empty()
    {
        return Ok(t);
    }

    serde_json::from_slice::<GeminiTranscript>(data)
        .map_err(|err| anyhow!("failed to parse transcript: {err}"))
}

/// Parse a Gemini JSONL transcript (one JSON object per line). Filters out
/// `$set` state-update lines, the session-header line, and `type: "info"`
/// notices. Deduplicates messages by `id` keeping the latest write so the
/// upsert pattern Gemini uses (incrementally adding `toolCalls` to a message
/// originally written with only `thoughts`) collapses to one entry.
pub fn parse_transcript_jsonl(data: &[u8]) -> Result<GeminiTranscript> {
    let text = std::str::from_utf8(data)
        .map_err(|err| anyhow!("transcript is not valid UTF-8: {err}"))?;

    let mut messages: Vec<GeminiMessage> = Vec::new();
    let mut id_index: HashMap<String, usize> = HashMap::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let value: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Skip `$set` state updates.
        if value.get("$set").is_some() {
            continue;
        }
        // Skip session header (has no `type` field) and info notices.
        let msg_type = match value.get("type").and_then(Value::as_str) {
            Some(t) => t,
            None => continue,
        };
        if msg_type != MESSAGE_TYPE_USER && msg_type != MESSAGE_TYPE_GEMINI {
            continue;
        }

        let msg = match serde_json::from_value::<GeminiMessage>(value) {
            Ok(m) => m,
            Err(_) => continue,
        };

        // Upsert by id: latest write replaces earlier ones.
        if !msg.id.is_empty()
            && let Some(&idx) = id_index.get(&msg.id)
        {
            messages[idx] = msg;
            continue;
        }
        let idx = messages.len();
        if !msg.id.is_empty() {
            id_index.insert(msg.id.clone(), idx);
        }
        messages.push(msg);
    }

    Ok(GeminiTranscript { messages })
}

pub fn extract_modified_files(data: &[u8]) -> Result<Vec<String>> {
    let transcript = parse_transcript(data)?;
    Ok(extract_modified_files_from_transcript(&transcript))
}

pub fn extract_modified_files_from_transcript(transcript: &GeminiTranscript) -> Vec<String> {
    let mut files = Vec::new();
    let mut seen = HashSet::new();

    for message in &transcript.messages {
        if message.r#type != MESSAGE_TYPE_GEMINI {
            continue;
        }

        for tool_call in &message.tool_calls {
            if !FILE_MODIFICATION_TOOLS
                .iter()
                .any(|name| *name == tool_call.name)
            {
                continue;
            }

            let file = tool_call
                .args
                .get("file_path")
                .and_then(Value::as_str)
                .or_else(|| tool_call.args.get("path").and_then(Value::as_str))
                .or_else(|| tool_call.args.get("filename").and_then(Value::as_str))
                .unwrap_or_default();

            if file.is_empty() {
                continue;
            }

            if seen.insert(file.to_string()) {
                files.push(file.to_string());
            }
        }
    }

    files
}

pub fn extract_last_user_prompt(data: &[u8]) -> Result<String> {
    let transcript = parse_transcript(data)?;
    Ok(extract_last_user_prompt_from_transcript(&transcript))
}

pub fn extract_last_user_prompt_from_transcript(transcript: &GeminiTranscript) -> String {
    for message in transcript.messages.iter().rev() {
        if message.r#type == MESSAGE_TYPE_USER && !message.content.is_empty() {
            return message.content.clone();
        }
    }

    String::new()
}

pub fn extract_all_user_prompts(data: &[u8]) -> Result<Vec<String>> {
    let transcript = parse_transcript(data)?;
    Ok(extract_all_user_prompts_from_transcript(&transcript))
}

pub fn extract_all_user_prompts_from_transcript(transcript: &GeminiTranscript) -> Vec<String> {
    let mut prompts = Vec::new();

    for message in &transcript.messages {
        if message.r#type == MESSAGE_TYPE_USER && !message.content.is_empty() {
            prompts.push(message.content.clone());
        }
    }

    prompts
}

pub fn extract_last_assistant_message(data: &[u8]) -> Result<String> {
    let transcript = parse_transcript(data)?;

    for message in transcript.messages.iter().rev() {
        if message.r#type == MESSAGE_TYPE_GEMINI && !message.content.is_empty() {
            return Ok(message.content.clone());
        }
    }

    Ok(String::new())
}

pub fn get_last_message_id(data: &[u8]) -> Result<String> {
    let transcript = parse_transcript(data)?;
    Ok(get_last_message_id_from_transcript(&transcript))
}

pub fn get_last_message_id_from_transcript(transcript: &GeminiTranscript) -> String {
    transcript
        .messages
        .last()
        .map(|msg| msg.id.clone())
        .unwrap_or_default()
}

pub fn get_last_message_id_from_file(path: &str) -> Result<String> {
    if path.is_empty() {
        return Ok(String::new());
    }

    let data = match std::fs::read(path) {
        Ok(data) => data,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
        Err(err) => return Err(anyhow!("failed to read transcript: {err}")),
    };

    if data.is_empty() {
        return Ok(String::new());
    }

    get_last_message_id(&data)
}

pub fn slice_from_message(data: &[u8], start_message_index: usize) -> Option<Vec<u8>> {
    if data.is_empty() || start_message_index == 0 {
        return Some(data.to_vec());
    }

    let transcript = parse_transcript(data).ok()?;
    if start_message_index >= transcript.messages.len() {
        return None;
    }

    let scoped = GeminiTranscript {
        messages: transcript.messages[start_message_index..].to_vec(),
    };
    serde_json::to_vec(&scoped).ok()
}

pub fn calculate_token_usage(data: &[u8], start_message_index: usize) -> TokenUsage {
    let parsed = match parse_transcript(data) {
        Ok(t) => t,
        Err(_) => return TokenUsage::default(),
    };

    let mut usage = TokenUsage::default();

    for (idx, msg) in parsed.messages.iter().enumerate() {
        if idx < start_message_index {
            continue;
        }
        if msg.r#type != MESSAGE_TYPE_GEMINI {
            continue;
        }
        let Some(tokens) = &msg.tokens else {
            continue;
        };
        usage.api_call_count += 1;
        usage.input_tokens += tokens.input;
        usage.output_tokens += tokens.output;
        usage.cache_read_tokens += tokens.cached;
    }

    usage
}

pub fn calculate_token_usage_from_file(
    path: &str,
    start_message_index: usize,
) -> Result<TokenUsage> {
    if path.is_empty() {
        return Ok(TokenUsage::default());
    }

    let data = match std::fs::read(path) {
        Ok(data) => data,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(TokenUsage::default()),
        Err(err) => return Err(anyhow!("failed to read transcript: {err}")),
    };

    Ok(calculate_token_usage(&data, start_message_index))
}

impl TranscriptEntryDeriver for GeminiCliAgent {
    fn derive_transcript_entries(
        &self,
        session_id: &str,
        turn_id: Option<&str>,
        transcript: &str,
    ) -> Result<Vec<TranscriptEntry>> {
        derive_transcript_entries(session_id, turn_id, transcript)
    }
}

/// Normalize a Gemini transcript document into canonical `TranscriptEntry` rows.
///
/// Unlike JSONL-style agents, Gemini stores a single JSON object with a
/// `messages` array. Each message is emitted in document order; tool calls on
/// an assistant message are followed immediately by a synthesised tool-result
/// row (Gemini does not record separate tool-result events).
pub fn derive_transcript_entries(
    session_id: &str,
    turn_id: Option<&str>,
    transcript: &str,
) -> Result<Vec<TranscriptEntry>> {
    let parsed = match parse_transcript(transcript.as_bytes()) {
        Ok(transcript) => transcript,
        Err(_) => return Ok(Vec::new()),
    };

    let scope = match turn_id {
        Some(id) => DerivationScope::Turn(id),
        None => DerivationScope::Session,
    };
    let mut entries: Vec<TranscriptEntry> = Vec::new();
    let mut order: i32 = 0;
    let mut tool_call_index: i32 = 0;

    for message in parsed.messages {
        let msg_type = message.r#type.as_str();

        if msg_type == MESSAGE_TYPE_USER {
            let text = message.content.trim();
            if !text.is_empty() {
                entries.push(make_chat_entry(
                    session_id,
                    &scope,
                    &mut order,
                    TranscriptActor::User,
                    text,
                ));
            }
            continue;
        }

        if msg_type != MESSAGE_TYPE_GEMINI {
            continue;
        }

        // Emit one ASSISTANT/THINKING entry per recorded thought, before any
        // chat or tool entries from the same message.
        for thought in &message.thoughts {
            let formatted = format_thought(thought);
            if formatted.is_empty() {
                continue;
            }
            entries.push(TranscriptEntry {
                entry_id: make_entry_id(session_id, &scope, order),
                session_id: session_id.to_string(),
                turn_id: scope.turn_id().map(str::to_string),
                order,
                timestamp: None,
                actor: TranscriptActor::Assistant,
                variant: TranscriptVariant::Thinking,
                source: TranscriptSource::Transcript,
                text: formatted,
                tool_use_id: None,
                tool_kind: None,
                is_error: false,
            });
            order += 1;
        }

        let text = message.content.trim();
        if !text.is_empty() {
            entries.push(make_chat_entry(
                session_id,
                &scope,
                &mut order,
                TranscriptActor::Assistant,
                text,
            ));
        }

        for tool_call in &message.tool_calls {
            let tool_kind = tool_call.name.trim();
            if tool_kind.is_empty() {
                continue;
            }

            let tool_use_id = if tool_call.id.trim().is_empty() {
                make_derived_tool_use_id(session_id, &scope, tool_call_index)
            } else {
                tool_call.id.trim().to_string()
            };
            tool_call_index += 1;

            let display = tool_call.display_name.trim();
            let kind_label = if display.is_empty() {
                tool_kind
            } else {
                display
            };
            let description = tool_call.description.trim();
            let tool_input_summary = stringify_tool_args(&tool_call.args);

            let mut lines: Vec<String> = vec![format!("Tool: {kind_label}")];
            if !description.is_empty() {
                lines.push(description.to_string());
            }
            if !tool_input_summary.is_empty() {
                lines.push(tool_input_summary);
            }
            let tool_use_text = lines.join("\n");

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
                tool_kind: Some(tool_kind.to_string()),
                is_error: false,
            });
            order += 1;

            // Prefer the real tool output from `result[].functionResponse...`.
            // Fall back to the status string ("success"/"error") or
            // "completed" when nothing else is available.
            let result_output = tool_call.result_output();
            let status = tool_call.status.trim();
            let result_text = if !result_output.is_empty() {
                result_output
            } else if !status.is_empty() {
                status.to_string()
            } else {
                "completed".to_string()
            };

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
                tool_kind: Some(tool_kind.to_string()),
                is_error: tool_call.is_error_status(),
            });
            order += 1;
        }
    }

    Ok(entries)
}

fn format_thought(thought: &GeminiThought) -> String {
    let subject = thought.subject.trim();
    let description = thought.description.trim();
    match (subject.is_empty(), description.is_empty()) {
        (false, false) => format!("{subject}\n{description}"),
        (false, true) => subject.to_string(),
        (true, false) => description.to_string(),
        (true, true) => String::new(),
    }
}

fn make_chat_entry(
    session_id: &str,
    scope: &DerivationScope<'_>,
    order: &mut i32,
    actor: TranscriptActor,
    text: &str,
) -> TranscriptEntry {
    let entry = TranscriptEntry {
        entry_id: make_entry_id(session_id, scope, *order),
        session_id: session_id.to_string(),
        turn_id: scope.turn_id().map(str::to_string),
        order: *order,
        timestamp: None,
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

fn stringify_tool_args(args: &HashMap<String, Value>) -> String {
    if args.is_empty() {
        return String::new();
    }
    let mut map = serde_json::Map::with_capacity(args.len());
    for (k, v) in args {
        map.insert(k.clone(), v.clone());
    }
    serde_json::to_string(&Value::Object(map)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;
    use crate::adapters::agents::gemini::agent::GeminiCliAgent;

    #[test]
    #[allow(non_snake_case)]
    fn TestParseTranscript() {
        let data = br#"{
  "messages": [
    {"type": "user", "content": "hello"},
    {"type": "gemini", "content": "hi there"}
  ]
}"#;

        let transcript = parse_transcript(data).expect("parse should succeed");
        assert_eq!(transcript.messages.len(), 2);
        assert_eq!(transcript.messages[0].r#type, "user");
        assert_eq!(transcript.messages[1].r#type, "gemini");

        let empty = parse_transcript(br#"{"messages": []}"#).expect("parse should succeed");
        assert!(empty.messages.is_empty());

        let err = parse_transcript(b"not valid json").expect_err("invalid json should fail");
        assert!(err.to_string().contains("failed to parse transcript"));

        let array_content = br#"{
  "messages": [
    {"type": "user", "content": [{"text": "hello world"}]},
    {"type": "gemini", "content": "hi there"},
    {"type": "user", "content": [{"text": "do something"}]},
    {"type": "gemini", "content": "sure thing"}
  ]
}"#;
        let transcript = parse_transcript(array_content).expect("array content should parse");
        assert_eq!(transcript.messages[0].content, "hello world");
        assert_eq!(transcript.messages[2].content, "do something");
        assert_eq!(transcript.messages[1].content, "hi there");
        assert_eq!(transcript.messages[3].content, "sure thing");

        let multi_part = br#"{
  "messages": [
    {"type": "user", "content": [{"text": "part one"}, {"text": "part two"}]}
  ]
}"#;
        let transcript = parse_transcript(multi_part).expect("multi-part content should parse");
        assert_eq!(transcript.messages[0].content, "part one\npart two");

        let null_content = br#"{
  "messages": [
    {"type": "user", "content": null},
    {"type": "gemini", "content": "response"}
  ]
}"#;
        let transcript = parse_transcript(null_content).expect("null content should parse");
        assert_eq!(transcript.messages[0].content, "");
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestExtractModifiedFiles() {
        let data = br#"{
  "messages": [
    {"type": "user", "content": "create a file"},
    {"type": "gemini", "content": "", "toolCalls": [{"name": "write_file", "args": {"file_path": "foo.rs"}}]},
    {"type": "gemini", "content": "", "toolCalls": [{"name": "edit_file", "args": {"file_path": "bar.rs"}}]},
    {"type": "gemini", "content": "", "toolCalls": [{"name": "read_file", "args": {"file_path": "other.rs"}}]},
    {"type": "gemini", "content": "", "toolCalls": [{"name": "write_file", "args": {"file_path": "foo.rs"}}]}
  ]
}"#;

        let files = extract_modified_files(data).expect("extract modified files should work");
        assert_eq!(files.len(), 2);
        assert!(files.contains(&"foo.rs".to_string()));
        assert!(files.contains(&"bar.rs".to_string()));

        let alternative = br#"{
  "messages": [
    {"type": "gemini", "content": "", "toolCalls": [{"name": "write_file", "args": {"path": "via_path.rs"}}]},
    {"type": "gemini", "content": "", "toolCalls": [{"name": "save_file", "args": {"filename": "via_filename.rs"}}]}
  ]
}"#;
        let files =
            extract_modified_files(alternative).expect("extract modified files should work");
        assert_eq!(
            files,
            vec!["via_path.rs".to_string(), "via_filename.rs".to_string()]
        );

        let no_tools = br#"{
  "messages": [
    {"type": "user", "content": "hello"},
    {"type": "gemini", "content": "just text response"}
  ]
}"#;
        let files = extract_modified_files(no_tools).expect("extract modified files should work");
        assert!(files.is_empty());

        let replace_tool = br#"{
  "messages": [
    {"type": "user", "content": "make the output uppercase"},
    {"type": "gemini", "content": "", "toolCalls": [{"name": "read_file", "args": {"file_path": "random_letter.rb"}}]},
    {"type": "gemini", "content": "", "toolCalls": [{"name": "replace", "args": {"file_path": "/path/to/random_letter.rb", "old_string": "sample", "new_string": "sample.upcase"}}]},
    {"type": "gemini", "content": "Done!"}
  ]
}"#;
        let files =
            extract_modified_files(replace_tool).expect("extract modified files should work");
        assert_eq!(files, vec!["/path/to/random_letter.rb".to_string()]);

        let array_content = br#"{
  "messages": [
    {"type": "user", "content": [{"text": "create a file"}]},
    {"type": "gemini", "content": "", "toolCalls": [{"name": "write_file", "args": {"file_path": "foo.rs"}}]},
    {"type": "user", "content": [{"text": "edit the file"}]},
    {"type": "gemini", "content": "", "toolCalls": [{"name": "edit_file", "args": {"file_path": "bar.rs"}}]}
  ]
}"#;
        let files =
            extract_modified_files(array_content).expect("extract modified files should work");
        assert_eq!(files.len(), 2);
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestExtractLastUserPrompt() {
        let cases = vec![
            (
                "string content",
                br#"{"messages": [
                {"type": "user", "content": "first"},
                {"type": "gemini", "content": "response"},
                {"type": "user", "content": "second"}
            ]}"#
                .as_slice(),
                "second",
            ),
            (
                "array content",
                br#"{"messages": [
                {"type": "user", "content": [{"text": "first prompt"}]},
                {"type": "gemini", "content": "response"},
                {"type": "user", "content": [{"text": "second prompt"}]}
            ]}"#
                .as_slice(),
                "second prompt",
            ),
            (
                "only one user message",
                br#"{"messages": [{"type": "user", "content": "only message"}]}"#.as_slice(),
                "only message",
            ),
            (
                "no user messages",
                br#"{"messages": [{"type": "gemini", "content": "assistant only"}]}"#.as_slice(),
                "",
            ),
            ("empty messages", br#"{"messages": []}"#.as_slice(), ""),
        ];

        for (name, data, expected) in cases {
            let got = extract_last_user_prompt(data).expect("extract last user prompt should work");
            assert_eq!(got, expected, "{name}");
        }
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestGetLastMessageID() {
        let with_ids = br#"{"messages": [
                {"id": "msg-1", "type": "user", "content": "hello"},
                {"id": "msg-2", "type": "gemini", "content": "hi there"}
            ]}"#;
        assert_eq!(
            get_last_message_id(with_ids).expect("last message id should parse"),
            "msg-2"
        );

        assert_eq!(
            get_last_message_id(br#"{"messages": []}"#).expect("last message id should parse"),
            ""
        );

        assert_eq!(
            get_last_message_id(
                br#"{"messages": [
                {"type": "user", "content": "hello"},
                {"type": "gemini", "content": "hi"}
            ]}"#,
            )
            .expect("last message id should parse"),
            ""
        );

        assert_eq!(
            get_last_message_id(
                br#"{"messages": [
                {"id": "msg-1", "type": "user", "content": "hello"},
                {"type": "gemini", "content": "hi"}
            ]}"#,
            )
            .expect("mixed id transcript should parse"),
            ""
        );

        let err = get_last_message_id(b"not valid json").expect_err("invalid json should fail");
        assert!(err.to_string().contains("failed to parse transcript"));
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestGetLastMessageIDFromTranscript() {
        let cases = vec![
            (
                GeminiTranscript {
                    messages: vec![
                        GeminiMessage {
                            id: "msg-1".to_string(),
                            r#type: "user".to_string(),
                            content: "hello".to_string(),
                            tool_calls: Vec::new(),
                            ..Default::default()
                        },
                        GeminiMessage {
                            id: "msg-2".to_string(),
                            r#type: "gemini".to_string(),
                            content: "hi there".to_string(),
                            tool_calls: Vec::new(),
                            ..Default::default()
                        },
                    ],
                },
                "msg-2",
            ),
            (
                GeminiTranscript {
                    messages: Vec::new(),
                },
                "",
            ),
            (
                GeminiTranscript {
                    messages: vec![GeminiMessage {
                        id: String::new(),
                        r#type: "user".to_string(),
                        content: "hello".to_string(),
                        tool_calls: Vec::new(),
                        ..Default::default()
                    }],
                },
                "",
            ),
            (GeminiTranscript::default(), ""),
        ];

        for (transcript, expected) in cases {
            assert_eq!(get_last_message_id_from_transcript(&transcript), expected);
        }
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestGetLastMessageIDFromFile() {
        assert_eq!(
            get_last_message_id_from_file("").expect("empty path should not fail"),
            ""
        );

        assert_eq!(
            get_last_message_id_from_file("/nonexistent/path/transcript.json")
                .expect("non-existent path should not fail"),
            ""
        );

        let dir = tempdir().expect("failed to create temp dir");
        let empty_path = dir.path().join("empty.json");
        std::fs::write(&empty_path, b"").expect("failed to write empty file");
        assert_eq!(
            get_last_message_id_from_file(empty_path.to_string_lossy().as_ref())
                .expect("empty file should not fail"),
            ""
        );

        let valid_path = dir.path().join("transcript.json");
        std::fs::write(
            &valid_path,
            br#"{"messages": [{"id": "abc-123", "type": "user", "content": "hello"}]}"#,
        )
        .expect("failed to write transcript file");
        assert_eq!(
            get_last_message_id_from_file(valid_path.to_string_lossy().as_ref())
                .expect("valid file should not fail"),
            "abc-123"
        );

        let invalid_path = dir.path().join("invalid.json");
        std::fs::write(&invalid_path, b"not valid json").expect("failed to write invalid file");
        let err = get_last_message_id_from_file(invalid_path.to_string_lossy().as_ref())
            .expect_err("invalid file should fail");
        assert!(err.to_string().contains("failed to parse transcript"));
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestExtractAllUserPrompts() {
        let data = br#"{
  "messages": [
    {"type": "user", "content": [{"text": "first prompt"}]},
    {"type": "gemini", "content": "response 1"},
    {"type": "user", "content": [{"text": "second prompt"}]},
    {"type": "gemini", "content": "response 2"}
  ]
}"#;

        let prompts = extract_all_user_prompts(data).expect("extract all prompts should work");
        assert_eq!(prompts, vec!["first prompt", "second prompt"]);
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestExtractModifiedFilesFromTranscript() {
        let transcript = GeminiTranscript {
            messages: vec![
                GeminiMessage {
                    id: String::new(),
                    r#type: "user".to_string(),
                    content: "hello".to_string(),
                    tool_calls: Vec::new(),
                    ..Default::default()
                },
                GeminiMessage {
                    id: String::new(),
                    r#type: "gemini".to_string(),
                    content: String::new(),
                    tool_calls: vec![GeminiToolCall {
                        id: String::new(),
                        name: "write_file".to_string(),
                        args: HashMap::from([(
                            "file_path".to_string(),
                            Value::String("test.rs".to_string()),
                        )]),
                        status: String::new(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ],
        };

        let files = extract_modified_files_from_transcript(&transcript);
        assert_eq!(files, vec!["test.rs".to_string()]);
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestExtractLastUserPromptFromTranscript() {
        let transcript = GeminiTranscript {
            messages: vec![
                GeminiMessage {
                    id: String::new(),
                    r#type: "user".to_string(),
                    content: "first prompt".to_string(),
                    tool_calls: Vec::new(),
                    ..Default::default()
                },
                GeminiMessage {
                    id: String::new(),
                    r#type: "gemini".to_string(),
                    content: "response".to_string(),
                    tool_calls: Vec::new(),
                    ..Default::default()
                },
                GeminiMessage {
                    id: String::new(),
                    r#type: "user".to_string(),
                    content: "last prompt".to_string(),
                    tool_calls: Vec::new(),
                    ..Default::default()
                },
            ],
        };

        assert_eq!(
            extract_last_user_prompt_from_transcript(&transcript),
            "last prompt"
        );
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestCalculateTokenUsage() {
        let data = br#"{
  "messages": [
    {"id": "1", "type": "user", "content": "hello"},
    {"id": "2", "type": "gemini", "content": "hi there", "tokens": {"input": 10, "output": 20, "cached": 5, "thoughts": 0, "tool": 0, "total": 35}},
    {"id": "3", "type": "user", "content": "how are you?"},
    {"id": "4", "type": "gemini", "content": "I'm doing well", "tokens": {"input": 15, "output": 25, "cached": 3, "thoughts": 0, "tool": 0, "total": 43}}
  ]
}"#;
        let usage = calculate_token_usage(data, 0);
        assert_eq!(usage.api_call_count, 2);
        assert_eq!(usage.input_tokens, 25);
        assert_eq!(usage.output_tokens, 45);
        assert_eq!(usage.cache_read_tokens, 8);

        let usage = calculate_token_usage(data, 2);
        assert_eq!(usage.api_call_count, 1);
        assert_eq!(usage.input_tokens, 15);
        assert_eq!(usage.output_tokens, 25);
        assert_eq!(usage.cache_read_tokens, 3);

        let ignores_user = br#"{
  "messages": [
    {"id": "1", "type": "user", "content": "hello", "tokens": {"input": 100, "output": 100, "cached": 100, "total": 300}},
    {"id": "2", "type": "gemini", "content": "hi", "tokens": {"input": 10, "output": 20, "cached": 5, "total": 35}}
  ]
}"#;
        let usage = calculate_token_usage(ignores_user, 0);
        assert_eq!(usage.api_call_count, 1);
        assert_eq!(usage.input_tokens, 10);
        assert_eq!(usage.output_tokens, 20);

        let usage = calculate_token_usage(br#"{"messages": []}"#, 0);
        assert_eq!(usage.api_call_count, 0);
        assert_eq!(usage.input_tokens, 0);
        assert_eq!(usage.output_tokens, 0);
        assert_eq!(usage.cache_read_tokens, 0);

        let usage = calculate_token_usage(b"not valid json", 0);
        assert_eq!(usage.api_call_count, 0);

        let missing_tokens = br#"{
  "messages": [
    {"id": "1", "type": "user", "content": "hello"},
    {"id": "2", "type": "gemini", "content": "hi there"}
  ]
}"#;
        let usage = calculate_token_usage(missing_tokens, 0);
        assert_eq!(usage.api_call_count, 0);
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestGeminiCLIAgent() {
        let dir = tempdir().expect("failed to create temp dir");
        let file = dir.path().join("transcript.json");
        std::fs::write(
            &file,
            br#"{
  "messages": [
    {"type": "user", "content": "hello"},
    {"type": "gemini", "content": "hi"},
    {"type": "user", "content": "bye"}
  ]
}"#,
        )
        .expect("failed to write transcript");

        let agent = GeminiCliAgent;
        let position = agent
            .get_transcript_position(file.to_string_lossy().as_ref())
            .expect("get transcript position should work");
        assert_eq!(position, 3);

        let position = agent
            .get_transcript_position("")
            .expect("get transcript position should work");
        assert_eq!(position, 0);

        let position = agent
            .get_transcript_position("/nonexistent/file.json")
            .expect("get transcript position should work");
        assert_eq!(position, 0);
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestExtractModifiedFilesFromOffset() {
        let dir = tempdir().expect("failed to create temp dir");
        let file = dir.path().join("transcript.json");

        std::fs::write(
            &file,
            br#"{
  "messages": [
    {"type": "user", "content": "prompt"},
    {"type": "gemini", "content": "", "toolCalls": [{"name": "write_file", "args": {"file_path": "a.rs"}}]},
    {"type": "gemini", "content": "", "toolCalls": [{"name": "edit_file", "args": {"file_path": "b.rs"}}]}
  ]
}"#,
        )
        .expect("failed to write transcript");

        let agent = GeminiCliAgent;
        let (files, current_position) = agent
            .extract_modified_files_from_offset(file.to_string_lossy().as_ref(), 1)
            .expect("offset extraction should work");
        assert_eq!(files, vec!["a.rs".to_string(), "b.rs".to_string()]);
        assert_eq!(current_position, 3);
    }

    #[test]
    fn user_message_produces_user_chat_entry() {
        let data = br#"{
  "messages": [
    {"id": "msg-1", "type": "user", "content": "hello world"}
  ]
}"#;
        let entries =
            derive_transcript_entries("sess-1", Some("turn-1"), std::str::from_utf8(data).unwrap())
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
    fn gemini_message_produces_assistant_chat_entry() {
        let data = br#"{
  "messages": [
    {"id": "msg-1", "type": "gemini", "content": "reasoning result"}
  ]
}"#;
        let entries =
            derive_transcript_entries("sess-1", Some("turn-1"), std::str::from_utf8(data).unwrap())
                .expect("derive entries");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].actor, TranscriptActor::Assistant);
        assert_eq!(entries[0].variant, TranscriptVariant::Chat);
        assert_eq!(entries[0].text, "reasoning result");
    }

    #[test]
    fn tool_call_emits_use_and_synthesized_result() {
        let data = br#"{
  "messages": [
    {"id": "msg-1", "type": "gemini", "content": "", "toolCalls": [
      {"id": "call_1", "name": "write_file", "args": {"file_path": "a.rs"}, "status": "success"}
    ]}
  ]
}"#;
        let entries =
            derive_transcript_entries("sess-1", Some("turn-1"), std::str::from_utf8(data).unwrap())
                .expect("derive entries");
        assert_eq!(entries.len(), 2);
        let tool_use = &entries[0];
        let tool_result = &entries[1];
        assert_eq!(tool_use.variant, TranscriptVariant::ToolUse);
        assert_eq!(tool_use.tool_use_id.as_deref(), Some("call_1"));
        assert_eq!(tool_use.tool_kind.as_deref(), Some("write_file"));
        assert!(tool_use.text.starts_with("Tool: write_file"));
        assert!(tool_use.text.contains("\"file_path\":\"a.rs\""));
        assert!(!tool_use.is_error);
        assert_eq!(tool_result.variant, TranscriptVariant::ToolResult);
        assert_eq!(tool_result.tool_use_id.as_deref(), Some("call_1"));
        assert_eq!(tool_result.tool_kind.as_deref(), Some("write_file"));
        assert_eq!(tool_result.text, "success");
        assert!(!tool_result.is_error);
    }

    #[test]
    fn tool_call_error_status_sets_is_error_on_result() {
        let data = br#"{
  "messages": [
    {"id": "msg-1", "type": "gemini", "content": "", "toolCalls": [
      {"id": "call_err", "name": "bash", "args": {"command": "nope"}, "status": "error"}
    ]}
  ]
}"#;
        let entries =
            derive_transcript_entries("sess-1", Some("turn-1"), std::str::from_utf8(data).unwrap())
                .expect("derive entries");
        assert_eq!(entries.len(), 2);
        assert!(!entries[0].is_error);
        assert!(entries[1].is_error);
        assert_eq!(entries[1].text, "error");
    }

    #[test]
    fn tool_call_without_id_uses_derived_id() {
        let data = br#"{
  "messages": [
    {"id": "msg-1", "type": "gemini", "content": "", "toolCalls": [
      {"name": "read_file", "args": {"file_path": "src/lib.rs"}, "status": ""}
    ]}
  ]
}"#;
        let entries =
            derive_transcript_entries("sess-1", Some("turn-2"), std::str::from_utf8(data).unwrap())
                .expect("derive entries");
        assert_eq!(entries.len(), 2);
        let id = entries[0].tool_use_id.clone().expect("tool_use_id");
        assert_eq!(id, "derived:sess-1:turn-2:0");
        assert_eq!(entries[1].tool_use_id.as_deref(), Some(id.as_str()));
        assert_eq!(entries[1].text, "completed");
        assert!(!entries[1].is_error);
    }

    #[test]
    fn mixed_messages_preserve_order() {
        let data = br#"{
  "messages": [
    {"id": "msg-1", "type": "user", "content": "please run tests"},
    {"id": "msg-2", "type": "gemini", "content": "running now", "toolCalls": [
      {"id": "call_bash_2", "name": "bash", "args": {"command": "cargo test"}, "status": "success"}
    ]}
  ]
}"#;
        let entries =
            derive_transcript_entries("sess-7", Some("turn-3"), std::str::from_utf8(data).unwrap())
                .expect("derive entries");
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0].actor, TranscriptActor::User);
        assert_eq!(entries[0].variant, TranscriptVariant::Chat);
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
    fn invalid_json_returns_empty_entries() {
        let entries = derive_transcript_entries("sess-1", Some("turn-1"), "not valid json")
            .expect("derive entries");
        assert!(entries.is_empty());

        let entries =
            derive_transcript_entries("sess-1", None, "").expect("derive entries from empty input");
        assert!(entries.is_empty());
    }
}

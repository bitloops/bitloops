use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;

use crate::adapters::agents::TokenUsage;
use crate::host::checkpoints::transcript::parse::{parse_from_bytes, parse_from_file_at_line};
use crate::host::checkpoints::transcript::types::{Line, TYPE_ASSISTANT, TYPE_USER};
use crate::host::checkpoints::transcript::utils::{
    agent_transcript_path, extract_last_user_prompt as extract_last_user_prompt_shared,
    extract_modified_files as extract_modified_files_shared,
    find_checkpoint_uuid as find_checkpoint_uuid_shared,
    truncate_transcript_at_uuid as truncate_transcript_at_uuid_shared,
};

pub type TranscriptLine = Line;

#[derive(Debug, Clone, Deserialize, Default, Copy)]
struct MessageUsage {
    #[serde(default)]
    input_tokens: i32,
    #[serde(default)]
    cache_creation_input_tokens: i32,
    #[serde(default)]
    cache_read_input_tokens: i32,
    #[serde(default)]
    output_tokens: i32,
}

#[derive(Debug, Deserialize, Default)]
struct MessageWithUsage {
    #[serde(default)]
    id: String,
    #[serde(default)]
    usage: MessageUsage,
}

#[derive(Debug, Deserialize, Default)]
struct ToolResultMessage {
    #[serde(default)]
    content: Value,
}

#[derive(Debug, Deserialize, Default)]
struct ToolResultContentBlock {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    tool_use_id: String,
    #[serde(default)]
    content: Value,
}

#[derive(Debug, Deserialize, Default)]
struct TextContentBlock {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    text: String,
}

pub fn parse_transcript(content: &[u8]) -> Result<Vec<TranscriptLine>> {
    parse_from_bytes(content)
}

pub fn serialize_transcript(lines: &[TranscriptLine]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for line in lines {
        let data = serde_json::to_vec(line).context("failed to marshal line")?;
        out.extend_from_slice(&data);
        out.push(b'\n');
    }
    Ok(out)
}

pub fn extract_modified_files(lines: &[TranscriptLine]) -> Vec<String> {
    extract_modified_files_shared(lines)
}

pub fn extract_last_user_prompt(lines: &[TranscriptLine]) -> String {
    extract_last_user_prompt_shared(lines)
}

pub fn truncate_at_uuid(lines: &[TranscriptLine], uuid: &str) -> Vec<TranscriptLine> {
    truncate_transcript_at_uuid_shared(lines, uuid)
}

pub fn find_checkpoint_uuid(lines: &[TranscriptLine], tool_use_id: &str) -> (String, bool) {
    match find_checkpoint_uuid_shared(lines, tool_use_id) {
        Some(uuid) => (uuid, true),
        None => (String::new(), false),
    }
}

pub fn calculate_token_usage(transcript: &[TranscriptLine]) -> TokenUsage {
    let mut usage_by_message_id: HashMap<String, MessageUsage> = HashMap::new();

    for line in transcript {
        if line.r#type != TYPE_ASSISTANT {
            continue;
        }

        let Ok(msg) = serde_json::from_value::<MessageWithUsage>(line.message.clone()) else {
            continue;
        };

        if msg.id.is_empty() {
            continue;
        }

        let should_replace = match usage_by_message_id.get(&msg.id) {
            None => true,
            Some(current) => msg.usage.output_tokens > current.output_tokens,
        };
        if should_replace {
            usage_by_message_id.insert(msg.id, msg.usage);
        }
    }

    let mut usage = TokenUsage {
        api_call_count: usage_by_message_id.len() as i32,
        ..TokenUsage::default()
    };

    for current in usage_by_message_id.values() {
        usage.input_tokens += current.input_tokens;
        usage.cache_creation_tokens += current.cache_creation_input_tokens;
        usage.cache_read_tokens += current.cache_read_input_tokens;
        usage.output_tokens += current.output_tokens;
    }

    usage
}

pub fn calculate_token_usage_from_file(path: &str, start_line: usize) -> Result<TokenUsage> {
    if path.is_empty() {
        return Ok(TokenUsage::default());
    }

    let (lines, _) = parse_from_file_at_line(path, start_line)
        .with_context(|| format!("failed to parse transcript: {path}"))?;
    Ok(calculate_token_usage(&lines))
}

pub fn extract_spawned_agent_ids(transcript: &[TranscriptLine]) -> HashMap<String, String> {
    let mut agent_ids = HashMap::new();

    for line in transcript {
        if line.r#type != TYPE_USER {
            continue;
        }

        let Ok(msg) = serde_json::from_value::<ToolResultMessage>(line.message.clone()) else {
            continue;
        };

        let Ok(content_blocks) = serde_json::from_value::<Vec<ToolResultContentBlock>>(msg.content)
        else {
            continue;
        };

        for block in content_blocks {
            if block.kind != "tool_result" {
                continue;
            }

            let text_content = if let Ok(text_blocks) =
                serde_json::from_value::<Vec<TextContentBlock>>(block.content.clone())
            {
                let mut text = String::new();
                for item in text_blocks {
                    if item.kind == "text" {
                        text.push_str(&item.text);
                        text.push('\n');
                    }
                }
                text
            } else {
                block.content.as_str().unwrap_or_default().to_string()
            };

            let agent_id = extract_agent_id_from_text(&text_content);
            if !agent_id.is_empty() {
                agent_ids.insert(agent_id, block.tool_use_id);
            }
        }
    }

    agent_ids
}

pub(crate) fn extract_agent_id_from_text(text: &str) -> String {
    const PREFIX: &str = "agentId: ";
    let Some(start_idx) = text.find(PREFIX) else {
        return String::new();
    };

    let rest = &text[start_idx + PREFIX.len()..];
    let end = rest
        .char_indices()
        .find_map(|(idx, ch)| (!ch.is_ascii_alphanumeric()).then_some(idx))
        .unwrap_or(rest.len());

    if end == 0 {
        return String::new();
    }

    rest[..end].to_string()
}

pub fn calculate_total_token_usage(
    transcript_path: &str,
    start_line: usize,
    subagents_dir: &str,
) -> Result<TokenUsage> {
    if transcript_path.is_empty() {
        return Ok(TokenUsage::default());
    }

    let (parsed, _) = parse_from_file_at_line(transcript_path, start_line)
        .with_context(|| format!("failed to parse transcript: {transcript_path}"))?;
    let mut main_usage = calculate_token_usage(&parsed);

    let spawned_agents = extract_spawned_agent_ids(&parsed);
    if spawned_agents.is_empty() {
        return Ok(main_usage);
    }

    let mut subagent_usage = TokenUsage::default();
    let mut spawned_agent_ids: Vec<String> = spawned_agents.keys().cloned().collect();
    spawned_agent_ids.sort();

    for agent_id in spawned_agent_ids {
        let path = agent_transcript_path(subagents_dir, &agent_id);
        let Ok(usage) = calculate_token_usage_from_file(&path, 0) else {
            continue;
        };
        subagent_usage.input_tokens += usage.input_tokens;
        subagent_usage.cache_creation_tokens += usage.cache_creation_tokens;
        subagent_usage.cache_read_tokens += usage.cache_read_tokens;
        subagent_usage.output_tokens += usage.output_tokens;
        subagent_usage.api_call_count += usage.api_call_count;
    }

    if subagent_usage.api_call_count > 0 {
        main_usage.subagent_tokens = Some(Box::new(subagent_usage));
    }

    Ok(main_usage)
}

pub fn extract_all_modified_files(
    transcript_path: &str,
    start_line: usize,
    subagents_dir: &str,
) -> Result<Vec<String>> {
    if transcript_path.is_empty() {
        return Ok(Vec::new());
    }

    let (parsed, _) = parse_from_file_at_line(transcript_path, start_line)
        .with_context(|| format!("failed to parse transcript: {transcript_path}"))?;

    let mut seen_files = HashSet::new();
    let mut files = Vec::new();
    for path in extract_modified_files(&parsed) {
        if seen_files.insert(path.clone()) {
            files.push(path);
        }
    }

    let spawned_agents = extract_spawned_agent_ids(&parsed);
    let mut spawned_agent_ids: Vec<String> = spawned_agents.keys().cloned().collect();
    spawned_agent_ids.sort();
    for agent_id in spawned_agent_ids {
        let path = agent_transcript_path(subagents_dir, &agent_id);
        let Ok((agent_lines, _)) = parse_from_file_at_line(&path, 0) else {
            continue;
        };
        for file in extract_modified_files(&agent_lines) {
            if seen_files.insert(file.clone()) {
                files.push(file);
            }
        }
    }

    Ok(files)
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::fs;
    use std::path::Path;

    use serde_json::{Value, json};
    use tempfile::tempdir;

    use super::{
        TranscriptLine, calculate_token_usage, calculate_total_token_usage,
        extract_agent_id_from_text, extract_all_modified_files, extract_last_user_prompt,
        extract_modified_files, extract_spawned_agent_ids, find_checkpoint_uuid, parse_transcript,
        serialize_transcript, truncate_at_uuid,
    };

    fn parse_lines(data: &str) -> Vec<TranscriptLine> {
        parse_transcript(data.as_bytes()).expect("failed to parse transcript lines")
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestParseTranscript() {
        let data = r#"{"type":"user","uuid":"u1","message":{"content":"hello"}}
{"type":"assistant","uuid":"a1","message":{"content":[{"type":"text","text":"hi"}]}}
"#;

        let lines = parse_transcript(data.as_bytes()).expect("parse should succeed");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].r#type, "user");
        assert_eq!(lines[0].uuid, "u1");
        assert_eq!(lines[1].r#type, "assistant");
        assert_eq!(lines[1].uuid, "a1");
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestParseTranscript_SkipsMalformed() {
        let data = r#"{"type":"user","uuid":"u1","message":{"content":"hello"}}
not valid json
{"type":"assistant","uuid":"a1","message":{"content":[]}}
"#;

        let lines = parse_transcript(data.as_bytes()).expect("parse should succeed");
        assert_eq!(lines.len(), 2);
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestSerializeTranscript() {
        let lines = vec![
            TranscriptLine {
                r#type: "user".to_string(),
                uuid: "u1".to_string(),
                message: json!({}),
            },
            TranscriptLine {
                r#type: "assistant".to_string(),
                uuid: "a1".to_string(),
                message: json!({}),
            },
        ];

        let data = serialize_transcript(&lines).expect("serialize should succeed");
        let parsed = parse_transcript(&data).expect("roundtrip parse should succeed");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].uuid, "u1");
        assert_eq!(parsed[1].uuid, "a1");
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestExtractModifiedFiles() {
        let data = r#"{"type":"assistant","uuid":"a1","message":{"content":[{"type":"tool_use","name":"Write","input":{"file_path":"foo.rs"}}]}}
{"type":"assistant","uuid":"a2","message":{"content":[{"type":"tool_use","name":"Edit","input":{"file_path":"bar.rs"}}]}}
{"type":"assistant","uuid":"a3","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"ls"}}]}}
{"type":"assistant","uuid":"a4","message":{"content":[{"type":"tool_use","name":"Write","input":{"file_path":"foo.rs"}}]}}
"#;
        let lines = parse_lines(data);
        let files = extract_modified_files(&lines);
        assert_eq!(files.len(), 2);
        assert!(files.contains(&"foo.rs".to_string()));
        assert!(files.contains(&"bar.rs".to_string()));
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestExtractLastUserPrompt() {
        let tests = vec![
            (
                "string content",
                r#"{"type":"user","uuid":"u1","message":{"content":"first"}}
{"type":"assistant","uuid":"a1","message":{"content":[]}}
{"type":"user","uuid":"u2","message":{"content":"second"}}"#,
                "second",
            ),
            (
                "array content with text block",
                r#"{"type":"user","uuid":"u1","message":{"content":[{"type":"text","text":"hello world"}]}}"#,
                "hello world",
            ),
            ("empty transcript", "", ""),
        ];

        for (name, data, expected) in tests {
            let lines = parse_transcript(data.as_bytes()).expect("parse should succeed");
            let got = extract_last_user_prompt(&lines);
            assert_eq!(got, expected, "{name}");
        }
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestTruncateAtUUID() {
        let data = r#"{"type":"user","uuid":"u1","message":{}}
{"type":"assistant","uuid":"a1","message":{}}
{"type":"user","uuid":"u2","message":{}}
{"type":"assistant","uuid":"a2","message":{}}
"#;
        let lines = parse_lines(data);

        let tests = vec![
            ("truncate at u1", "u1", 1usize, "u1"),
            ("truncate at a1", "a1", 2, "a1"),
            ("truncate at u2", "u2", 3, "u2"),
            ("truncate at a2", "a2", 4, "a2"),
            ("empty uuid returns all", "", 4, "a2"),
            ("unknown uuid returns all", "unknown", 4, "a2"),
        ];

        for (name, uuid, expected_len, expected_last_uuid) in tests {
            let truncated = truncate_at_uuid(&lines, uuid);
            assert_eq!(truncated.len(), expected_len, "{name}");
            assert_eq!(
                truncated
                    .last()
                    .expect("truncated transcript should not be empty")
                    .uuid,
                expected_last_uuid,
                "{name}"
            );
        }
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestFindCheckpointUUID() {
        let data = r#"{"type":"assistant","uuid":"a1","message":{"content":[{"type":"tool_use","id":"tool1"}]}}
{"type":"user","uuid":"u1","message":{"content":[{"type":"tool_result","tool_use_id":"tool1"}]}}
{"type":"assistant","uuid":"a2","message":{"content":[{"type":"tool_use","id":"tool2"}]}}
{"type":"user","uuid":"u2","message":{"content":[{"type":"tool_result","tool_use_id":"tool2"}]}}
"#;
        let lines = parse_lines(data);

        let tests = vec![
            ("tool1", "u1", true),
            ("tool2", "u2", true),
            ("unknown", "", false),
        ];

        for (tool_use_id, expected_uuid, expected_found) in tests {
            let (uuid, found) = find_checkpoint_uuid(&lines, tool_use_id);
            assert_eq!(found, expected_found, "{tool_use_id}");
            assert_eq!(uuid, expected_uuid, "{tool_use_id}");
        }
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestCalculateTokenUsage_BasicMessages() {
        let transcript = vec![
            TranscriptLine {
                r#type: "assistant".to_string(),
                uuid: "asst-1".to_string(),
                message: json!({
                    "id":"msg_001",
                    "usage":{
                        "input_tokens":10,
                        "cache_creation_input_tokens":100,
                        "cache_read_input_tokens":50,
                        "output_tokens":20
                    }
                }),
            },
            TranscriptLine {
                r#type: "assistant".to_string(),
                uuid: "asst-2".to_string(),
                message: json!({
                    "id":"msg_002",
                    "usage":{
                        "input_tokens":5,
                        "cache_creation_input_tokens":200,
                        "cache_read_input_tokens":0,
                        "output_tokens":30
                    }
                }),
            },
        ];

        let usage = calculate_token_usage(&transcript);
        assert_eq!(usage.api_call_count, 2);
        assert_eq!(usage.input_tokens, 15);
        assert_eq!(usage.cache_creation_tokens, 300);
        assert_eq!(usage.cache_read_tokens, 50);
        assert_eq!(usage.output_tokens, 50);
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestCalculateTokenUsage_StreamingDeduplication() {
        let transcript = vec![
            TranscriptLine {
                r#type: "assistant".to_string(),
                uuid: "asst-1".to_string(),
                message: json!({
                    "id":"msg_001",
                    "usage":{
                        "input_tokens":10,
                        "cache_creation_input_tokens":100,
                        "cache_read_input_tokens":50,
                        "output_tokens":1
                    }
                }),
            },
            TranscriptLine {
                r#type: "assistant".to_string(),
                uuid: "asst-2".to_string(),
                message: json!({
                    "id":"msg_001",
                    "usage":{
                        "input_tokens":10,
                        "cache_creation_input_tokens":100,
                        "cache_read_input_tokens":50,
                        "output_tokens":5
                    }
                }),
            },
            TranscriptLine {
                r#type: "assistant".to_string(),
                uuid: "asst-3".to_string(),
                message: json!({
                    "id":"msg_001",
                    "usage":{
                        "input_tokens":10,
                        "cache_creation_input_tokens":100,
                        "cache_read_input_tokens":50,
                        "output_tokens":20
                    }
                }),
            },
        ];

        let usage = calculate_token_usage(&transcript);
        assert_eq!(usage.api_call_count, 1);
        assert_eq!(usage.output_tokens, 20);
        assert_eq!(usage.input_tokens, 10);
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestCalculateTokenUsage_IgnoresUserMessages() {
        let transcript = vec![
            TranscriptLine {
                r#type: "user".to_string(),
                uuid: "user-1".to_string(),
                message: json!({"content":"hello"}),
            },
            TranscriptLine {
                r#type: "assistant".to_string(),
                uuid: "asst-1".to_string(),
                message: json!({
                    "id":"msg_001",
                    "usage":{
                        "input_tokens":10,
                        "cache_creation_input_tokens":100,
                        "cache_read_input_tokens":0,
                        "output_tokens":20
                    }
                }),
            },
        ];

        let usage = calculate_token_usage(&transcript);
        assert_eq!(usage.api_call_count, 1);
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestCalculateTokenUsage_EmptyTranscript() {
        let usage = calculate_token_usage(&[]);
        assert_eq!(usage.api_call_count, 0);
        assert_eq!(usage.input_tokens, 0);
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestExtractSpawnedAgentIDs_FromToolResult() {
        let transcript = vec![TranscriptLine {
            r#type: "user".to_string(),
            uuid: "user-1".to_string(),
            message: json!({
                "content":[
                    {
                        "type":"tool_result",
                        "tool_use_id":"toolu_abc123",
                        "content":[{"type":"text","text":"Result from agent\n\nagentId: ac66d4b (for resuming)"}]
                    }
                ]
            }),
        }];

        let agent_ids = extract_spawned_agent_ids(&transcript);
        assert_eq!(agent_ids.len(), 1);
        assert_eq!(agent_ids.get("ac66d4b"), Some(&"toolu_abc123".to_string()));
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestExtractSpawnedAgentIDs_MultipleAgents() {
        let transcript = vec![
            TranscriptLine {
                r#type: "user".to_string(),
                uuid: "user-1".to_string(),
                message: json!({
                    "content":[
                        {
                            "type":"tool_result",
                            "tool_use_id":"toolu_001",
                            "content":[{"type":"text","text":"agentId: aaa1111"}]
                        }
                    ]
                }),
            },
            TranscriptLine {
                r#type: "user".to_string(),
                uuid: "user-2".to_string(),
                message: json!({
                    "content":[
                        {
                            "type":"tool_result",
                            "tool_use_id":"toolu_002",
                            "content":[{"type":"text","text":"agentId: bbb2222"}]
                        }
                    ]
                }),
            },
        ];

        let agent_ids = extract_spawned_agent_ids(&transcript);
        assert_eq!(agent_ids.len(), 2);
        assert_eq!(agent_ids.get("aaa1111"), Some(&"toolu_001".to_string()));
        assert_eq!(agent_ids.get("bbb2222"), Some(&"toolu_002".to_string()));
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestExtractSpawnedAgentIDs_NoAgentID() {
        let transcript = vec![TranscriptLine {
            r#type: "user".to_string(),
            uuid: "user-1".to_string(),
            message: json!({
                "content":[
                    {
                        "type":"tool_result",
                        "tool_use_id":"toolu_001",
                        "content":[{"type":"text","text":"Some result without agent ID"}]
                    }
                ]
            }),
        }];

        let agent_ids = extract_spawned_agent_ids(&transcript);
        assert!(agent_ids.is_empty());
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestExtractAgentIDFromText() {
        let tests = vec![
            (
                "standard format",
                "agentId: ac66d4b (for resuming)",
                "ac66d4b",
            ),
            (
                "at end of text",
                "Result text\n\nagentId: abc1234",
                "abc1234",
            ),
            ("no agent ID", "Some text without agent ID", ""),
            ("empty text", "", ""),
            (
                "agent ID with newline after",
                "agentId: xyz9999\nMore text",
                "xyz9999",
            ),
        ];

        for (name, text, expected) in tests {
            assert_eq!(extract_agent_id_from_text(text), expected, "{name}");
        }
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestCalculateTotalTokenUsage_PerCheckpoint() {
        let dir = tempdir().expect("failed to create temp dir");
        let transcript_path = dir.path().join("transcript.jsonl");

        let content = concat!(
            "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"content\":\"first prompt\"}}\n",
            "{\"type\":\"assistant\",\"uuid\":\"a1\",\"message\":{\"id\":\"m1\",\"usage\":{\"input_tokens\":100,\"output_tokens\":50}}}\n",
            "{\"type\":\"user\",\"uuid\":\"u2\",\"message\":{\"content\":\"second prompt\"}}\n",
            "{\"type\":\"assistant\",\"uuid\":\"a2\",\"message\":{\"id\":\"m2\",\"usage\":{\"input_tokens\":200,\"output_tokens\":100}}}\n",
            "{\"type\":\"user\",\"uuid\":\"u3\",\"message\":{\"content\":\"third prompt\"}}\n",
            "{\"type\":\"assistant\",\"uuid\":\"a3\",\"message\":{\"id\":\"m3\",\"usage\":{\"input_tokens\":300,\"output_tokens\":150}}}\n"
        );
        fs::write(&transcript_path, content).expect("failed to write transcript");

        let usage1 = calculate_total_token_usage(
            transcript_path.to_str().expect("path must be utf-8"),
            0,
            "",
        )
        .expect("usage calculation should succeed");
        assert_eq!(usage1.input_tokens, 600);
        assert_eq!(usage1.output_tokens, 300);
        assert_eq!(usage1.api_call_count, 3);

        let usage2 = calculate_total_token_usage(
            transcript_path.to_str().expect("path must be utf-8"),
            2,
            "",
        )
        .expect("usage calculation should succeed");
        assert_eq!(usage2.input_tokens, 500);
        assert_eq!(usage2.output_tokens, 250);
        assert_eq!(usage2.api_call_count, 2);

        let usage3 = calculate_total_token_usage(
            transcript_path.to_str().expect("path must be utf-8"),
            4,
            "",
        )
        .expect("usage calculation should succeed");
        assert_eq!(usage3.input_tokens, 300);
        assert_eq!(usage3.output_tokens, 150);
        assert_eq!(usage3.api_call_count, 1);
    }

    fn write_jsonl_file(path: &Path, lines: &[String]) {
        let mut body = String::new();
        for line in lines {
            body.push_str(line);
            body.push('\n');
        }
        fs::write(path, body).expect("failed to write jsonl file");
    }

    fn make_assistant_tool_line(uuid: &str, tool_id: &str, name: &str, input: Value) -> String {
        serde_json::to_string(&json!({
            "type":"assistant",
            "uuid":uuid,
            "message":{
                "content":[{
                    "type":"tool_use",
                    "id":tool_id,
                    "name":name,
                    "input":input
                }]
            }
        }))
        .expect("assistant line must serialize")
    }

    fn make_write_tool_line(uuid: &str, file_path: &str) -> String {
        make_assistant_tool_line(
            uuid,
            &format!("toolu_{uuid}"),
            "Write",
            json!({"file_path": file_path}),
        )
    }

    fn make_edit_tool_line(uuid: &str, file_path: &str) -> String {
        make_assistant_tool_line(
            uuid,
            &format!("toolu_{uuid}"),
            "Edit",
            json!({"file_path": file_path}),
        )
    }

    fn make_task_tool_use_line(uuid: &str, tool_use_id: &str) -> String {
        make_assistant_tool_line(uuid, tool_use_id, "Task", json!({"prompt":"do something"}))
    }

    fn make_task_result_line(uuid: &str, tool_use_id: &str, agent_id: &str) -> String {
        serde_json::to_string(&json!({
            "type":"user",
            "uuid":uuid,
            "message":{
                "content":[{
                    "type":"tool_result",
                    "tool_use_id":tool_use_id,
                    "content":format!("agentId: {agent_id}")
                }]
            }
        }))
        .expect("task result line must serialize")
    }

    fn contains_all(paths: &[String], expected: &[&str]) -> bool {
        let set = paths.iter().cloned().collect::<HashSet<_>>();
        expected.iter().all(|path| set.contains(*path))
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestExtractAllModifiedFiles_IncludesSubagentFiles() {
        let dir = tempdir().expect("failed to create temp dir");
        let transcript_path = dir.path().join("transcript.jsonl");
        let subagents_dir = dir.path().join("tasks").join("toolu_task1");
        fs::create_dir_all(&subagents_dir).expect("failed to create subagent dir");

        write_jsonl_file(
            &transcript_path,
            &[
                make_write_tool_line("a1", "/repo/main.rs"),
                make_task_tool_use_line("a2", "toolu_task1"),
                make_task_result_line("u1", "toolu_task1", "sub1"),
            ],
        );

        write_jsonl_file(
            &subagents_dir.join("agent-sub1.jsonl"),
            &[
                make_write_tool_line("sa1", "/repo/helper.rs"),
                make_edit_tool_line("sa2", "/repo/utils.rs"),
            ],
        );

        let files = extract_all_modified_files(
            transcript_path.to_str().expect("path must be utf-8"),
            0,
            subagents_dir.to_str().expect("path must be utf-8"),
        )
        .expect("extract_all_modified_files should succeed");

        assert_eq!(files.len(), 3, "files: {files:?}");
        assert!(contains_all(
            &files,
            &["/repo/main.rs", "/repo/helper.rs", "/repo/utils.rs"]
        ));
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestExtractAllModifiedFiles_DeduplicatesAcrossAgents() {
        let dir = tempdir().expect("failed to create temp dir");
        let transcript_path = dir.path().join("transcript.jsonl");
        let subagents_dir = dir.path().join("tasks").join("toolu_task1");
        fs::create_dir_all(&subagents_dir).expect("failed to create subagent dir");

        write_jsonl_file(
            &transcript_path,
            &[
                make_write_tool_line("a1", "/repo/shared.rs"),
                make_task_tool_use_line("a2", "toolu_task1"),
                make_task_result_line("u1", "toolu_task1", "sub1"),
            ],
        );

        write_jsonl_file(
            &subagents_dir.join("agent-sub1.jsonl"),
            &[make_edit_tool_line("sa1", "/repo/shared.rs")],
        );

        let files = extract_all_modified_files(
            transcript_path.to_str().expect("path must be utf-8"),
            0,
            subagents_dir.to_str().expect("path must be utf-8"),
        )
        .expect("extract_all_modified_files should succeed");

        assert_eq!(files.len(), 1, "files: {files:?}");
        assert_eq!(files[0], "/repo/shared.rs");
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestExtractAllModifiedFiles_NoSubagents() {
        let dir = tempdir().expect("failed to create temp dir");
        let transcript_path = dir.path().join("transcript.jsonl");

        write_jsonl_file(
            &transcript_path,
            &[make_write_tool_line("a1", "/repo/solo.rs")],
        );

        let files = extract_all_modified_files(
            transcript_path.to_str().expect("path must be utf-8"),
            0,
            dir.path()
                .join("nonexistent")
                .to_str()
                .expect("path must be utf-8"),
        )
        .expect("extract_all_modified_files should succeed");

        assert_eq!(files.len(), 1, "files: {files:?}");
        assert_eq!(files[0], "/repo/solo.rs");
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestExtractAllModifiedFiles_SubagentOnlyChanges() {
        let dir = tempdir().expect("failed to create temp dir");
        let transcript_path = dir.path().join("transcript.jsonl");
        let subagents_dir = dir.path().join("tasks").join("toolu_task1");
        fs::create_dir_all(&subagents_dir).expect("failed to create subagent dir");

        write_jsonl_file(
            &transcript_path,
            &[
                make_task_tool_use_line("a1", "toolu_task1"),
                make_task_result_line("u1", "toolu_task1", "sub1"),
            ],
        );

        write_jsonl_file(
            &subagents_dir.join("agent-sub1.jsonl"),
            &[
                make_write_tool_line("sa1", "/repo/subagent_file1.rs"),
                make_write_tool_line("sa2", "/repo/subagent_file2.rs"),
            ],
        );

        let files = extract_all_modified_files(
            transcript_path.to_str().expect("path must be utf-8"),
            0,
            subagents_dir.to_str().expect("path must be utf-8"),
        )
        .expect("extract_all_modified_files should succeed");

        assert_eq!(files.len(), 2, "files: {files:?}");
        assert!(contains_all(
            &files,
            &["/repo/subagent_file1.rs", "/repo/subagent_file2.rs"]
        ));
    }

    #[test]
    fn extract_spawned_agent_ids_maps_agent_id_to_tool_use_id() {
        let transcript = vec![TranscriptLine {
            r#type: "user".to_string(),
            uuid: "u1".to_string(),
            message: json!({
                "content":[
                    {
                        "type":"tool_result",
                        "tool_use_id":"toolu_xyz",
                        "content":"agentId: abc1234"
                    }
                ]
            }),
        }];

        let got = extract_spawned_agent_ids(&transcript);
        let mut expected = HashMap::new();
        expected.insert("abc1234".to_string(), "toolu_xyz".to_string());
        assert_eq!(got, expected);
    }
}

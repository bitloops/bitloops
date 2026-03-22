use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, ErrorKind};
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;

use super::parse::extract_user_content;
use super::types::{
    AssistantMessage, CONTENT_TYPE_TEXT, CONTENT_TYPE_TOOL_USE, Line, TYPE_ASSISTANT, TYPE_USER,
    ToolInput, UserMessage,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptResponsePair {
    pub prompt: String,
    pub responses: Vec<String>,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TranscriptPosition {
    pub last_uuid: String,
    pub line_count: usize,
}

const FILE_MODIFICATION_TOOLS: [&str; 5] = [
    "Write",
    "Edit",
    "NotebookEdit",
    "mcp__acp__Write",
    "mcp__acp__Edit",
];

#[derive(Debug, Deserialize)]
struct ToolResultBlock {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    tool_use_id: String,
}

#[derive(Debug, Deserialize)]
struct UserMessageWithToolResults {
    #[serde(default)]
    content: Vec<ToolResultBlock>,
}

pub fn extract_all_prompt_responses(transcript: &[Line]) -> Vec<PromptResponsePair> {
    let mut pairs = Vec::new();

    let mut user_indices = Vec::new();
    for (idx, line) in transcript.iter().enumerate() {
        if line.r#type != TYPE_USER {
            continue;
        }

        let Ok(msg) = serde_json::from_value::<UserMessage>(line.message.clone()) else {
            continue;
        };

        if msg.content.is_string() {
            user_indices.push(idx);
            continue;
        }

        let has_text_block = msg.content.as_array().is_some_and(|arr| {
            arr.iter().any(|item| {
                item.as_object()
                    .and_then(|obj| obj.get("type"))
                    .and_then(Value::as_str)
                    == Some(CONTENT_TYPE_TEXT)
            })
        });
        if has_text_block {
            user_indices.push(idx);
        }
    }

    for (idx, user_idx) in user_indices.iter().enumerate() {
        let end_idx = if idx < user_indices.len() - 1 {
            user_indices[idx + 1]
        } else {
            transcript.len()
        };

        let prompt = extract_user_prompt_at(transcript, *user_idx);
        if prompt.is_empty() {
            continue;
        }

        let slice = &transcript[*user_idx..end_idx];
        let responses = extract_assistant_responses(slice);
        let files = extract_modified_files(slice);

        pairs.push(PromptResponsePair {
            prompt,
            responses,
            files,
        });
    }

    pairs
}

pub fn extract_last_user_prompt(transcript: &[Line]) -> String {
    let prompts = extract_user_prompts(transcript);
    prompts.last().cloned().unwrap_or_default()
}

pub fn extract_modified_files(transcript: &[Line]) -> Vec<String> {
    let mut file_set = HashSet::new();
    let mut files = Vec::new();

    for line in transcript {
        if line.r#type != TYPE_ASSISTANT {
            continue;
        }

        let Ok(msg) = serde_json::from_value::<AssistantMessage>(line.message.clone()) else {
            continue;
        };

        for block in msg.content {
            if block.r#type != CONTENT_TYPE_TOOL_USE
                || !FILE_MODIFICATION_TOOLS.contains(&block.name.as_str())
            {
                continue;
            }

            let Ok(input) = serde_json::from_value::<ToolInput>(block.input) else {
                continue;
            };

            let file = if input.file_path.is_empty() {
                input.notebook_path
            } else {
                input.file_path
            };

            if !file.is_empty() && file_set.insert(file.clone()) {
                files.push(file);
            }
        }
    }

    files
}

pub fn find_last_user_uuid(transcript: &[Line]) -> String {
    for line in transcript.iter().rev() {
        if line.r#type != TYPE_USER {
            continue;
        }

        let Ok(msg) = serde_json::from_value::<UserMessage>(line.message.clone()) else {
            continue;
        };

        if msg.content.is_string() {
            return line.uuid.clone();
        }
    }

    String::new()
}

pub fn filter_transcript_after_uuid(transcript: &[Line], uuid: &str) -> Vec<Line> {
    if uuid.is_empty() {
        return transcript.to_vec();
    }

    let found_index = transcript.iter().position(|line| line.uuid == uuid);

    match found_index {
        None => transcript.to_vec(),
        Some(idx) if idx == transcript.len().saturating_sub(1) => transcript.to_vec(),
        Some(idx) => transcript[idx + 1..].to_vec(),
    }
}

pub fn agent_transcript_path(transcript_dir: &str, agent_id: &str) -> String {
    PathBuf::from(transcript_dir)
        .join(format!("agent-{agent_id}.jsonl"))
        .to_string_lossy()
        .to_string()
}

pub fn find_checkpoint_uuid(transcript: &[Line], tool_use_id: &str) -> Option<String> {
    for line in transcript {
        if line.r#type != TYPE_USER {
            continue;
        }

        let Ok(msg) = serde_json::from_value::<UserMessageWithToolResults>(line.message.clone())
        else {
            continue;
        };

        for block in msg.content {
            if block.kind == "tool_result" && block.tool_use_id == tool_use_id {
                return Some(line.uuid.clone());
            }
        }
    }

    None
}

pub fn truncate_transcript_at_uuid(transcript: &[Line], uuid: &str) -> Vec<Line> {
    if uuid.is_empty() {
        return transcript.to_vec();
    }

    for (idx, line) in transcript.iter().enumerate() {
        if line.uuid == uuid {
            return transcript[..=idx].to_vec();
        }
    }

    transcript.to_vec()
}

pub fn get_transcript_position(path: &str) -> Result<TranscriptPosition> {
    if path.is_empty() {
        return Ok(TranscriptPosition::default());
    }

    let file = match File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(TranscriptPosition::default()),
        Err(err) => return Err(err).with_context(|| format!("failed to open transcript: {path}")),
    };

    let mut pos = TranscriptPosition::default();
    let mut reader = std::io::BufReader::new(file);
    let mut line_bytes = Vec::new();

    loop {
        line_bytes.clear();
        let read = reader
            .read_until(b'\n', &mut line_bytes)
            .with_context(|| format!("failed to read transcript: {path}"))?;
        if read == 0 {
            break;
        }

        if line_bytes.is_empty() {
            continue;
        }

        pos.line_count += 1;

        if let Ok(line) = serde_json::from_slice::<Value>(&line_bytes)
            && let Some(uuid) = line.get("uuid").and_then(Value::as_str)
            && !uuid.is_empty()
        {
            pos.last_uuid = uuid.to_string();
        }
    }

    Ok(pos)
}

fn extract_user_prompt_at(lines: &[Line], idx: usize) -> String {
    if idx >= lines.len() || lines[idx].r#type != TYPE_USER {
        return String::new();
    }

    extract_user_prompt_from_line(&lines[idx])
}

fn extract_assistant_responses(transcript: &[Line]) -> Vec<String> {
    let mut texts = Vec::new();

    for line in transcript {
        if line.r#type != TYPE_ASSISTANT {
            continue;
        }

        let Ok(msg) = serde_json::from_value::<AssistantMessage>(line.message.clone()) else {
            continue;
        };

        for block in msg.content {
            if block.r#type == CONTENT_TYPE_TEXT && !block.text.is_empty() {
                texts.push(block.text);
            }
        }
    }

    texts
}

fn extract_user_prompts(transcript: &[Line]) -> Vec<String> {
    transcript
        .iter()
        .filter(|line| line.r#type == TYPE_USER)
        .map(extract_user_prompt_from_line)
        .filter(|prompt| !prompt.is_empty())
        .collect()
}

fn extract_user_prompt_from_line(line: &Line) -> String {
    match serde_json::to_vec(&line.message) {
        Ok(raw) => extract_user_content(&raw),
        Err(_) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use tempfile::tempdir;

    use super::{
        agent_transcript_path, extract_all_prompt_responses, extract_last_user_prompt,
        extract_modified_files, filter_transcript_after_uuid, find_checkpoint_uuid,
        find_last_user_uuid, get_transcript_position, truncate_transcript_at_uuid,
    };
    use crate::host::transcript::types::Line;

    fn line(line_type: &str, uuid: &str, message_json: &str) -> Line {
        Line {
            r#type: line_type.to_string(),
            uuid: uuid.to_string(),
            message: serde_json::from_str(message_json).expect("invalid message json"),
        }
    }

    #[test]
    fn test_extract_last_user_prompt_string_content() {
        let transcript = vec![
            line("user", "u1", r#"{"content":"First prompt"}"#),
            line(
                "assistant",
                "a1",
                r#"{"content":[{"type":"text","text":"Response 1"}]}"#,
            ),
            line("user", "u2", r#"{"content":"Second prompt"}"#),
            line(
                "assistant",
                "a2",
                r#"{"content":[{"type":"text","text":"Response 2"}]}"#,
            ),
            line("user", "u3", r#"{"content":"Last prompt"}"#),
        ];

        let prompt = extract_last_user_prompt(&transcript);
        assert_eq!(prompt, "Last prompt");
    }

    #[test]
    fn test_extract_last_user_prompt_array_content() {
        let transcript = vec![
            line("user", "u1", r#"{"content":"First prompt"}"#),
            line(
                "assistant",
                "a1",
                r#"{"content":[{"type":"text","text":"Response"}]}"#,
            ),
            line(
                "user",
                "u2",
                r#"{"content":[{"type":"text","text":"Last part 1"},{"type":"text","text":"Last part 2"}]}"#,
            ),
        ];

        let prompt = extract_last_user_prompt(&transcript);
        assert_eq!(prompt, "Last part 1\n\nLast part 2");
    }

    #[test]
    fn test_extract_last_user_prompt_skips_tool_results() {
        let transcript = vec![
            line("user", "u1", r#"{"content":"Real user prompt"}"#),
            line(
                "assistant",
                "a1",
                r#"{"content":[{"type":"text","text":"Response"}]}"#,
            ),
            line(
                "user",
                "u2",
                r#"{"content":[{"type":"tool_result","tool_use_id":"123","content":"tool output"}]}"#,
            ),
        ];

        let prompt = extract_last_user_prompt(&transcript);
        assert_eq!(prompt, "Real user prompt");
    }

    #[test]
    fn test_extract_last_user_prompt_empty_transcript() {
        let transcript = vec![];
        let prompt = extract_last_user_prompt(&transcript);
        assert_eq!(prompt, "");
    }

    #[test]
    fn test_extract_last_user_prompt_no_user_messages() {
        let transcript = vec![line(
            "assistant",
            "a1",
            r#"{"content":[{"type":"text","text":"Response"}]}"#,
        )];
        let prompt = extract_last_user_prompt(&transcript);
        assert_eq!(prompt, "");
    }

    #[test]
    fn test_extract_all_prompt_responses() {
        let transcript = vec![
            line("user", "u1", r#"{"content":"First prompt"}"#),
            line(
                "assistant",
                "a1",
                r#"{"content":[{"type":"text","text":"Response 1"}]}"#,
            ),
            line(
                "assistant",
                "a2",
                r#"{"content":[{"type":"tool_use","name":"Write","input":{"file_path":"/path/first.rs"}},{"type":"text","text":"Response 2"}]}"#,
            ),
            line(
                "user",
                "u2",
                r#"{"content":[{"type":"text","text":"Second prompt part 1"},{"type":"text","text":"Second prompt part 2"}]}"#,
            ),
            line(
                "assistant",
                "a3",
                r#"{"content":[{"type":"text","text":"Final response"}]}"#,
            ),
            line(
                "user",
                "u3",
                r#"{"content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"tool output"}]}"#,
            ),
        ];

        let pairs = extract_all_prompt_responses(&transcript);
        assert_eq!(pairs.len(), 2, "expected 2 prompt/response pairs");

        assert_eq!(pairs[0].prompt, "First prompt");
        assert_eq!(
            pairs[0].responses,
            vec!["Response 1".to_string(), "Response 2".to_string()]
        );
        assert_eq!(pairs[0].files, vec!["/path/first.rs".to_string()]);

        assert_eq!(
            pairs[1].prompt,
            "Second prompt part 1\n\nSecond prompt part 2"
        );
        assert_eq!(pairs[1].responses, vec!["Final response".to_string()]);
        assert!(
            pairs[1].files.is_empty(),
            "expected no files for second pair"
        );
    }

    #[test]
    fn test_extract_modified_files_all_tool_types() {
        let transcript = vec![
            line(
                "assistant",
                "a1",
                r#"{"content":[{"type":"tool_use","name":"Write","input":{"file_path":"/path/write.rs"}}]}"#,
            ),
            line(
                "assistant",
                "a2",
                r#"{"content":[{"type":"tool_use","name":"Edit","input":{"file_path":"/path/edit.rs"}}]}"#,
            ),
            line(
                "assistant",
                "a3",
                r#"{"content":[{"type":"tool_use","name":"mcp__acp__Write","input":{"file_path":"/path/mcp_write.rs"}}]}"#,
            ),
            line(
                "assistant",
                "a4",
                r#"{"content":[{"type":"tool_use","name":"mcp__acp__Edit","input":{"file_path":"/path/mcp_edit.rs"}}]}"#,
            ),
            line(
                "assistant",
                "a5",
                r#"{"content":[{"type":"tool_use","name":"NotebookEdit","input":{"notebook_path":"/path/notebook.ipynb"}}]}"#,
            ),
            line(
                "assistant",
                "a6",
                r#"{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/path/read.rs"}}]}"#,
            ),
        ];

        let files = extract_modified_files(&transcript);
        let expected = [
            "/path/write.rs",
            "/path/edit.rs",
            "/path/mcp_write.rs",
            "/path/mcp_edit.rs",
            "/path/notebook.ipynb",
        ];

        assert_eq!(files.len(), expected.len(), "files: {files:?}");
        for (idx, exp) in expected.iter().enumerate() {
            assert_eq!(files[idx], *exp, "file index {idx}");
        }
    }

    #[test]
    fn test_extract_modified_files_deduplicates() {
        let transcript = vec![
            line(
                "assistant",
                "a1",
                r#"{"content":[{"type":"tool_use","name":"Write","input":{"file_path":"/path/file.rs"}}]}"#,
            ),
            line(
                "assistant",
                "a2",
                r#"{"content":[{"type":"tool_use","name":"Edit","input":{"file_path":"/path/file.rs"}}]}"#,
            ),
            line(
                "assistant",
                "a3",
                r#"{"content":[{"type":"tool_use","name":"Edit","input":{"file_path":"/path/file.rs"}}]}"#,
            ),
        ];

        let files = extract_modified_files(&transcript);
        assert_eq!(files.len(), 1, "files: {files:?}");
        assert_eq!(files[0], "/path/file.rs");
    }

    #[test]
    fn test_find_last_user_uuid_and_filter_transcript() {
        let transcript = vec![
            line("user", "u1", r#"{"content":"First prompt"}"#),
            line(
                "assistant",
                "a1",
                r#"{"content":[{"type":"text","text":"Response 1"}]}"#,
            ),
            line("user", "u2", r#"{"content":"Second prompt"}"#),
            line(
                "assistant",
                "a2",
                r#"{"content":[{"type":"text","text":"Response 2"}]}"#,
            ),
        ];

        let last_uuid = find_last_user_uuid(&transcript);
        assert_eq!(last_uuid, "u2");

        let filtered = filter_transcript_after_uuid(&transcript, &last_uuid);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].uuid, "a2");
    }

    #[test]
    fn test_agent_transcript_path() {
        let cases = [
            (
                "standard path",
                "/home/user/.claude/projects/myproject",
                "agent_abc123",
                "/home/user/.claude/projects/myproject/agent-agent_abc123.jsonl",
            ),
            (
                "empty agent ID",
                "/path/to/transcripts",
                "",
                "/path/to/transcripts/agent-.jsonl",
            ),
        ];

        for (name, transcript_dir, agent_id, expected) in cases {
            let got = agent_transcript_path(transcript_dir, agent_id);
            assert_eq!(got, expected, "{name}");
        }
    }

    #[test]
    fn test_find_checkpoint_uuid() {
        let transcript = vec![
            line("user", "u1", r#"{"content":"First prompt"}"#),
            line(
                "assistant",
                "a1",
                r#"{"content":[{"type":"tool_use","id":"toolu_task1","name":"Task","input":{}}]}"#,
            ),
            line(
                "user",
                "u2",
                r#"{"content":[{"type":"tool_result","tool_use_id":"toolu_task1","content":"Task completed"}]}"#,
            ),
            line(
                "assistant",
                "a2",
                r#"{"content":[{"type":"text","text":"Done"}]}"#,
            ),
            line(
                "assistant",
                "a3",
                r#"{"content":[{"type":"tool_use","id":"toolu_task2","name":"Task","input":{}}]}"#,
            ),
            line(
                "user",
                "u3",
                r#"{"content":[{"type":"tool_result","tool_use_id":"toolu_task2","content":"Second task done"}]}"#,
            ),
        ];

        let cases = [
            ("find first task result", "toolu_task1", Some("u2")),
            ("find second task result", "toolu_task2", Some("u3")),
            ("non-existent tool use ID", "toolu_nonexistent", None),
        ];

        for (name, tool_use_id, expected_uuid) in cases {
            let got = find_checkpoint_uuid(&transcript, tool_use_id);
            assert_eq!(got.as_deref(), expected_uuid, "{name}");
        }
    }

    #[test]
    fn test_truncate_transcript_at_uuid() {
        let transcript = vec![
            line("user", "u1", r#"{"content":"First"}"#),
            line("assistant", "a1", r#"{"content":[]}"#),
            line("user", "u2", r#"{"content":"Second"}"#),
            line("assistant", "a2", r#"{"content":[]}"#),
            line("user", "u3", r#"{"content":"Third"}"#),
        ];

        let cases = [
            ("truncate at u2", "u2", 3usize, "u2"),
            ("truncate at first", "u1", 1, "u1"),
            ("truncate at last", "u3", 5, "u3"),
            ("uuid not found - return all", "nonexistent", 5, "u3"),
            ("empty uuid - return all", "", 5, "u3"),
        ];

        for (name, uuid, expected_len, expected_last) in cases {
            let result = truncate_transcript_at_uuid(&transcript, uuid);
            assert_eq!(result.len(), expected_len, "{name}");
            if let Some(last) = result.last() {
                assert_eq!(last.uuid, expected_last, "{name}");
            }
        }
    }

    #[test]
    fn test_extract_last_user_prompt_strips_ide_tags() {
        let transcript = vec![line(
            "user",
            "u1",
            r#"{"content":[{"type":"text","text":"<ide_opened_file>The user opened /path/file.md</ide_opened_file>"},{"type":"text","text":"make the returned number red"}]}"#,
        )];

        let prompt = extract_last_user_prompt(&transcript);
        assert_eq!(prompt, "make the returned number red");
    }

    #[test]
    fn test_extract_last_user_prompt_strips_ide_tags_from_string_content() {
        let transcript = vec![line(
            "user",
            "u1",
            r#"{"content":"<ide_selection>some code</ide_selection>\n\nfix this bug"}"#,
        )];

        let prompt = extract_last_user_prompt(&transcript);
        assert_eq!(prompt, "fix this bug");
    }

    fn create_temp_transcript(content: &str) -> String {
        let dir = tempdir().expect("failed to create temp dir");
        let path = dir.path().join("transcript.jsonl");
        fs::write(&path, content).expect("failed to write transcript");
        // Keep temp dir alive by leaking it during test.
        let _ = dir.keep();
        path.to_string_lossy().to_string()
    }

    #[test]
    fn test_get_transcript_position_basic_messages() {
        let content = "{\"type\":\"user\",\"uuid\":\"user-1\",\"message\":{\"content\":\"Hello\"}}\n\
{\"type\":\"assistant\",\"uuid\":\"asst-1\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"Hi\"}]}}\n\
{\"type\":\"user\",\"uuid\":\"user-2\",\"message\":{\"content\":\"Bye\"}}";
        let tmp_file = create_temp_transcript(content);

        let pos = get_transcript_position(&tmp_file).expect("unexpected error");
        assert_eq!(pos.line_count, 3);
        assert_eq!(pos.last_uuid, "user-2");
    }

    #[test]
    fn test_get_transcript_position_with_summary_rows() {
        let content = "{\"type\":\"summary\",\"leafUuid\":\"leaf-1\",\"summary\":\"Previous context\"}\n\
{\"type\":\"summary\",\"leafUuid\":\"leaf-2\",\"summary\":\"More context\"}\n\
{\"type\":\"user\",\"uuid\":\"user-1\",\"message\":{\"content\":\"Hello\"}}\n\
{\"type\":\"assistant\",\"uuid\":\"asst-1\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"Hi\"}]}}";
        let tmp_file = create_temp_transcript(content);

        let pos = get_transcript_position(&tmp_file).expect("unexpected error");
        assert_eq!(pos.line_count, 4);
        assert_eq!(pos.last_uuid, "asst-1");
    }

    #[test]
    fn test_get_transcript_position_empty_file() {
        let tmp_file = create_temp_transcript("");
        let pos = get_transcript_position(&tmp_file).expect("unexpected error");

        assert_eq!(pos.line_count, 0);
        assert_eq!(pos.last_uuid, "");
    }

    #[test]
    fn test_get_transcript_position_non_existent_file() {
        let pos = get_transcript_position("/nonexistent/path/transcript.jsonl")
            .expect("unexpected error for non-existent file");

        assert_eq!(pos.line_count, 0);
        assert_eq!(pos.last_uuid, "");
    }

    #[test]
    fn test_get_transcript_position_empty_path() {
        let pos = get_transcript_position("").expect("unexpected error for empty path");
        assert_eq!(pos.line_count, 0);
        assert_eq!(pos.last_uuid, "");
    }

    #[test]
    fn test_get_transcript_position_only_summary_rows() {
        let content = "{\"type\":\"summary\",\"leafUuid\":\"leaf-1\",\"summary\":\"Context 1\"}\n\
{\"type\":\"summary\",\"leafUuid\":\"leaf-2\",\"summary\":\"Context 2\"}";
        let tmp_file = create_temp_transcript(content);

        let pos = get_transcript_position(&tmp_file).expect("unexpected error");
        assert_eq!(pos.line_count, 2);
        assert_eq!(pos.last_uuid, "");
    }

    #[test]
    fn test_get_transcript_position_mixed_with_malformed_lines() {
        let content = "{\"type\":\"user\",\"uuid\":\"user-1\",\"message\":{\"content\":\"Hello\"}}\n\
not valid json\n\
{\"type\":\"assistant\",\"uuid\":\"asst-1\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"Hi\"}]}}\n\
{broken json\n\
{\"type\":\"user\",\"uuid\":\"user-2\",\"message\":{\"content\":\"Final\"}}";
        let tmp_file = create_temp_transcript(content);

        let pos = get_transcript_position(&tmp_file).expect("unexpected error");
        assert_eq!(pos.line_count, 5);
        assert_eq!(pos.last_uuid, "user-2");
    }

    #[test]
    fn temp_transcript_helper_creates_file() {
        let tmp_file = create_temp_transcript("{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{}}");
        assert!(Path::new(&tmp_file).exists());
    }
}

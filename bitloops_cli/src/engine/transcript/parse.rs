use anyhow::{Context, Result};
use serde_json::Value;
use std::fs::File;
use std::io::{BufRead, BufReader};

use super::types::{CONTENT_TYPE_TEXT, Line, TYPE_ASSISTANT, TYPE_USER, UserMessage};
use crate::utils::text::strip_ide_context_tags;

pub fn parse_from_bytes(content: &[u8]) -> Result<Vec<Line>> {
    let mut lines = Vec::new();

    for raw_line in content.split(|b| *b == b'\n') {
        if raw_line.is_empty() {
            continue;
        }

        if let Some(parsed) = parse_line(raw_line) {
            lines.push(parsed);
        }
    }

    Ok(lines)
}

pub fn extract_user_content(raw: &[u8]) -> String {
    let msg = match serde_json::from_slice::<UserMessage>(raw) {
        Ok(msg) => msg,
        Err(_) => return String::new(),
    };

    if let Some(text) = msg.content.as_str() {
        return strip_ide_context_tags(text);
    }

    if let Some(items) = msg.content.as_array() {
        let text_blocks = items
            .iter()
            .filter_map(|item| item.as_object())
            .filter(|obj| obj.get("type").and_then(Value::as_str) == Some(CONTENT_TYPE_TEXT))
            .filter_map(|obj| obj.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>();

        if !text_blocks.is_empty() {
            return strip_ide_context_tags(&text_blocks.join("\n\n"));
        }
    }

    String::new()
}

pub fn slice_from_line(content: &[u8], from_line: usize) -> Vec<u8> {
    if content.is_empty() || from_line == 0 {
        return content.to_vec();
    }

    let mut line_count = 0usize;
    let mut offset = None;
    for (idx, byte) in content.iter().enumerate() {
        if *byte == b'\n' {
            line_count += 1;
            if line_count == from_line {
                offset = Some(idx + 1);
                break;
            }
        }
    }

    if line_count < from_line {
        return Vec::new();
    }

    let offset = match offset {
        Some(value) => value,
        None => return Vec::new(),
    };

    if offset >= content.len() {
        return Vec::new();
    }

    content[offset..].to_vec()
}

pub fn parse_from_file_at_line(path: &str, from_line: usize) -> Result<(Vec<Line>, usize)> {
    let file = File::open(path).with_context(|| format!("failed to open transcript: {path}"))?;
    let mut reader = BufReader::new(file);
    let mut lines = Vec::new();
    let mut total_lines = 0usize;
    let mut line_bytes = Vec::new();

    loop {
        line_bytes.clear();
        let read = reader
            .read_until(b'\n', &mut line_bytes)
            .with_context(|| format!("failed to read transcript: {path}"))?;

        if read == 0 {
            break;
        }

        if total_lines >= from_line
            && let Some(parsed) = parse_line(&line_bytes)
        {
            lines.push(parsed);
        }
        total_lines += 1;
    }

    Ok((lines, total_lines))
}

fn parse_line(raw_line: &[u8]) -> Option<Line> {
    if let Ok(parsed) = serde_json::from_slice::<Line>(raw_line) {
        return Some(parsed);
    }

    let value = serde_json::from_slice::<Value>(raw_line).ok()?;
    let role = value
        .get("role")
        .or_else(|| value.get("type"))
        .and_then(Value::as_str)?;
    if role != TYPE_USER && role != TYPE_ASSISTANT {
        return None;
    }

    let uuid = value
        .get("uuid")
        .or_else(|| value.get("id"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    let message = if let Some(message) = value.get("message") {
        message.clone()
    } else if let Some(content) = value.get("content") {
        serde_json::json!({ "content": content })
    } else {
        return None;
    };

    Some(Line {
        r#type: role.to_string(),
        uuid,
        message,
    })
}

#[cfg(test)]
mod tests {
    use super::{extract_user_content, parse_from_bytes, parse_from_file_at_line, slice_from_line};
    use crate::engine::transcript::types::{TYPE_ASSISTANT, TYPE_USER};
    use serde_json::json;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_parse_from_bytes_valid_jsonl() {
        let content = br#"{"type":"user","uuid":"u1","message":{"content":"hello"}}
{"type":"assistant","uuid":"a1","message":{"content":[{"type":"text","text":"hi"}]}}
"#;

        let lines = parse_from_bytes(content).expect("unexpected error");

        assert_eq!(lines.len(), 2, "expected 2 lines");
        assert_eq!(lines[0].r#type, TYPE_USER, "first line type mismatch");
        assert_eq!(lines[0].uuid, "u1", "first line uuid mismatch");
        assert_eq!(lines[1].r#type, "assistant", "second line type mismatch");
        assert_eq!(lines[1].uuid, "a1", "second line uuid mismatch");
    }

    #[test]
    fn test_parse_from_bytes_empty_content() {
        let lines = parse_from_bytes(&[]).expect("unexpected error");
        assert_eq!(lines.len(), 0, "expected zero lines for empty content");
    }

    #[test]
    fn test_parse_from_bytes_malformed_lines_skipped() {
        let content = br#"{"type":"user","uuid":"u1","message":{"content":"hello"}}
not valid json
{"type":"assistant","uuid":"a1","message":{"content":[{"type":"text","text":"hi"}]}}
"#;

        let lines = parse_from_bytes(content).expect("unexpected error");

        assert_eq!(
            lines.len(),
            2,
            "expected malformed lines to be skipped and 2 valid lines retained",
        );
    }

    #[test]
    fn test_parse_from_bytes_no_trailing_newline() {
        let content = br#"{"type":"user","uuid":"u1","message":{"content":"hello"}}"#;

        let lines = parse_from_bytes(content).expect("unexpected error");

        assert_eq!(lines.len(), 1, "expected one line without trailing newline");
    }

    #[test]
    fn test_parse_from_bytes_cursor_role_format() {
        let content = br#"{"role":"user","content":"hello"}
{"role":"assistant","content":"hi"}
"#;
        let lines = parse_from_bytes(content).expect("unexpected error");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].r#type, TYPE_USER);
        assert_eq!(lines[1].r#type, TYPE_ASSISTANT);
    }

    #[test]
    fn test_extract_user_content_string_content() {
        let raw =
            serde_json::to_vec(&json!({"content":"Hello, world!"})).expect("failed to marshal");
        let content = extract_user_content(&raw);

        assert_eq!(content, "Hello, world!");
    }

    #[test]
    fn test_extract_user_content_array_content() {
        let raw = br#"{"content":[{"type":"text","text":"First part"},{"type":"text","text":"Second part"}]}"#;
        let content = extract_user_content(raw);

        assert_eq!(content, "First part\n\nSecond part");
    }

    #[test]
    fn test_extract_user_content_empty_message() {
        let content = extract_user_content(br#"{}"#);
        assert_eq!(content, "", "expected empty string");
    }

    #[test]
    fn test_extract_user_content_invalid_json() {
        let content = extract_user_content(b"not valid json");
        assert_eq!(content, "", "expected empty string for invalid JSON");
    }

    #[test]
    fn test_extract_user_content_strips_ide_tags() {
        let raw = serde_json::to_vec(
            &json!({"content":"<ide_opened_file>file.rs</ide_opened_file>Hello, world!"}),
        )
        .expect("failed to marshal");

        let content = extract_user_content(&raw);
        assert_eq!(content, "Hello, world!");
    }

    #[test]
    fn test_extract_user_content_tool_results_ignored() {
        let raw = br#"{"content":[{"type":"tool_result","tool_use_id":"123","content":"result"}]}"#;
        let content = extract_user_content(raw);
        assert_eq!(content, "", "expected tool results to be ignored");
    }

    #[test]
    fn test_slice_from_line_skips_first_n_lines() {
        let content = br#"{"type":"user","uuid":"u1","message":{"content":"prompt 1"}}
{"type":"assistant","uuid":"a1","message":{"content":[{"type":"text","text":"response 1"}]}}
{"type":"user","uuid":"u2","message":{"content":"prompt 2"}}
{"type":"assistant","uuid":"a2","message":{"content":[{"type":"text","text":"response 2"}]}}
{"type":"user","uuid":"u3","message":{"content":"prompt 3"}}
"#;

        let sliced = slice_from_line(content, 2);
        let lines = parse_from_bytes(&sliced).expect("unexpected error");

        assert_eq!(lines.len(), 3, "expected 3 lines after skipping 2");
        assert_eq!(lines[0].uuid, "u2", "expected first UUID to be u2");
        assert_eq!(lines[1].uuid, "a2", "expected second UUID to be a2");
        assert_eq!(lines[2].uuid, "u3", "expected third UUID to be u3");
    }

    #[test]
    fn test_slice_from_line_zero_returns_all() {
        let content = br#"{"type":"user","uuid":"u1","message":{"content":"prompt 1"}}
{"type":"user","uuid":"u2","message":{"content":"prompt 2"}}
"#;

        let sliced = slice_from_line(content, 0);
        let lines = parse_from_bytes(&sliced).expect("unexpected error");

        assert_eq!(lines.len(), 2, "expected all lines when offset is zero");
    }

    #[test]
    fn test_slice_from_line_skip_more_than_exists() {
        let content = br#"{"type":"user","uuid":"u1","message":{"content":"prompt 1"}}
"#;

        let sliced = slice_from_line(content, 10);
        assert_eq!(
            sliced.len(),
            0,
            "expected empty result when skipping beyond end of content",
        );
    }

    #[test]
    fn test_slice_from_line_empty_content() {
        let sliced = slice_from_line(&[], 5);
        assert_eq!(sliced.len(), 0, "expected empty output for empty content");
    }

    #[test]
    fn test_slice_from_line_no_trailing_newline() {
        let content = br#"{"type":"user","uuid":"u1","message":{"content":"prompt 1"}}
{"type":"user","uuid":"u2","message":{"content":"prompt 2"}}"#;

        let sliced = slice_from_line(content, 1);
        let lines = parse_from_bytes(&sliced).expect("unexpected error");

        assert_eq!(lines.len(), 1, "expected one line after skipping one");
        assert_eq!(lines[0].uuid, "u2", "expected UUID u2");
    }

    fn create_temp_transcript(content: &str) -> NamedTempFile {
        let mut file = NamedTempFile::new().expect("failed to create temp file");
        file.write_all(content.as_bytes())
            .expect("failed to write transcript content");
        file.flush().expect("failed to flush transcript content");
        file
    }

    #[test]
    fn test_parse_from_file_at_line_valid_mixed_messages() {
        let content = r#"{"type":"user","uuid":"user-1","message":{"content":"Hello"}}
{"type":"assistant","uuid":"asst-1","message":{"content":[{"type":"text","text":"Hi there"}]}}
{"type":"user","uuid":"user-2","message":{"content":"Thanks"}}"#;

        let tmp_file = create_temp_transcript(content);
        let (lines, total_lines) =
            parse_from_file_at_line(tmp_file.path().to_string_lossy().as_ref(), 0)
                .expect("unexpected error");

        assert_eq!(total_lines, 3, "total lines mismatch");
        assert_eq!(lines.len(), 3, "parsed lines mismatch");
        assert_eq!(lines[0].r#type, TYPE_USER, "first line type mismatch");
        assert_eq!(lines[0].uuid, "user-1", "first line uuid mismatch");
        assert_eq!(lines[1].r#type, TYPE_ASSISTANT, "second line type mismatch");
        assert_eq!(lines[1].uuid, "asst-1", "second line uuid mismatch");
        assert_eq!(lines[2].r#type, TYPE_USER, "third line type mismatch");
        assert_eq!(lines[2].uuid, "user-2", "third line uuid mismatch");
    }

    #[test]
    fn test_parse_from_file_at_line_skips_malformed_lines() {
        let content = r#"{"type":"user","uuid":"user-1","message":{"content":"Hello"}}
this is not valid json
{"type":"assistant","uuid":"asst-1","message":{"content":[{"type":"text","text":"Hi"}]}}
{"broken json
{"type":"user","uuid":"user-2","message":{"content":"Bye"}}"#;

        let tmp_file = create_temp_transcript(content);
        let (lines, _) = parse_from_file_at_line(tmp_file.path().to_string_lossy().as_ref(), 0)
            .expect("unexpected error");

        assert_eq!(lines.len(), 3, "expected only 3 valid lines");
    }

    #[test]
    fn test_parse_from_file_at_line_large_lines() {
        let large_content = "x".repeat(100 * 1024);
        let content = format!(
            r#"{{"type":"user","uuid":"user-1","message":{{"content":"{}"}}}}"#,
            large_content
        );

        let tmp_file = create_temp_transcript(&content);
        let (lines, _) = parse_from_file_at_line(tmp_file.path().to_string_lossy().as_ref(), 0)
            .expect("unexpected error parsing large line");

        assert_eq!(lines.len(), 1, "expected one parsed line");
    }

    #[test]
    fn test_parse_from_file_at_line_line_exceeds_scanner_buffer() {
        let large_content = "x".repeat(11 * 1024 * 1024);
        let content = format!(
            r#"{{"type":"user","uuid":"user-1","message":{{"content":"{}"}}}}"#,
            large_content
        );

        let tmp_file = create_temp_transcript(&content);
        let (lines, _) = parse_from_file_at_line(tmp_file.path().to_string_lossy().as_ref(), 0)
            .expect("unexpected error parsing line exceeding buffer");

        assert_eq!(lines.len(), 1, "expected one parsed line");
    }

    #[test]
    fn test_parse_from_file_at_line_line_exceeds_scanner_buffer_with_offset() {
        let large_content = "x".repeat(11 * 1024 * 1024);
        let content = format!(
            r#"{{"type":"user","uuid":"user-1","message":{{"content":"{}"}}}}"#,
            large_content
        );

        let tmp_file = create_temp_transcript(&content);
        let (lines, total_lines) =
            parse_from_file_at_line(tmp_file.path().to_string_lossy().as_ref(), 0)
                .expect("unexpected error parsing line exceeding buffer");

        assert_eq!(total_lines, 1, "total lines mismatch");
        assert_eq!(lines.len(), 1, "parsed lines mismatch");
    }

    #[test]
    fn test_parse_from_file_at_line_bitloops_file() {
        let content = r#"{"type":"user","uuid":"user-1","message":{"content":"Hello"}}
{"type":"assistant","uuid":"asst-1","message":{"content":[{"type":"text","text":"Hi"}]}}
{"type":"user","uuid":"user-2","message":{"content":"Bye"}}"#;

        let tmp_file = create_temp_transcript(content);
        let (lines, total_lines) =
            parse_from_file_at_line(tmp_file.path().to_string_lossy().as_ref(), 0)
                .expect("unexpected error");

        assert_eq!(total_lines, 3, "total lines mismatch");
        assert_eq!(lines.len(), 3, "expected full file to be parsed");
    }

    #[test]
    fn test_parse_from_file_at_line_offset() {
        let content = r#"{"type":"user","uuid":"user-1","message":{"content":"Line1"}}
{"type":"assistant","uuid":"asst-1","message":{"content":[{"type":"text","text":"Line2"}]}}
{"type":"user","uuid":"user-2","message":{"content":"Line3"}}
{"type":"assistant","uuid":"asst-2","message":{"content":[{"type":"text","text":"Line4"}]}}"#;

        let tmp_file = create_temp_transcript(content);
        let (lines, total_lines) =
            parse_from_file_at_line(tmp_file.path().to_string_lossy().as_ref(), 2)
                .expect("unexpected error");

        assert_eq!(total_lines, 4, "total lines mismatch");
        assert_eq!(lines.len(), 2, "expected two lines after offset");
        if !lines.is_empty() {
            assert_eq!(lines[0].uuid, "user-2", "first offset line UUID mismatch");
        }
        if lines.len() >= 2 {
            assert_eq!(lines[1].uuid, "asst-2", "second offset line UUID mismatch");
        }
    }

    #[test]
    fn test_parse_from_file_at_line_offset_beyond_end() {
        let content = r#"{"type":"user","uuid":"user-1","message":{"content":"Hello"}}
{"type":"assistant","uuid":"asst-1","message":{"content":[{"type":"text","text":"Hi"}]}}"#;

        let tmp_file = create_temp_transcript(content);
        let (lines, total_lines) =
            parse_from_file_at_line(tmp_file.path().to_string_lossy().as_ref(), 10)
                .expect("unexpected error");

        assert_eq!(total_lines, 2, "total lines mismatch");
        assert_eq!(lines.len(), 0, "expected no lines beyond end");
    }

    #[test]
    fn test_parse_from_file_at_line_skips_malformed_lines_with_offset() {
        let content = r#"{"type":"user","uuid":"user-1","message":{"content":"Hello"}}
invalid json line
{"type":"assistant","uuid":"asst-1","message":{"content":[{"type":"text","text":"Hi"}]}}
{"type":"user","uuid":"user-2","message":{"content":"Bye"}}"#;

        let tmp_file = create_temp_transcript(content);
        let (lines, total_lines) =
            parse_from_file_at_line(tmp_file.path().to_string_lossy().as_ref(), 1)
                .expect("unexpected error");

        assert_eq!(total_lines, 4, "total lines should include malformed line");
        assert_eq!(
            lines.len(),
            2,
            "expected only valid lines after offset to be returned",
        );
    }
}

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
    let files = extract_modified_files(alternative).expect("extract modified files should work");
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
    let files = extract_modified_files(replace_tool).expect("extract modified files should work");
    assert_eq!(files, vec!["/path/to/random_letter.rb".to_string()]);

    let array_content = br#"{
  "messages": [
{"type": "user", "content": [{"text": "create a file"}]},
{"type": "gemini", "content": "", "toolCalls": [{"name": "write_file", "args": {"file_path": "foo.rs"}}]},
{"type": "user", "content": [{"text": "edit the file"}]},
{"type": "gemini", "content": "", "toolCalls": [{"name": "edit_file", "args": {"file_path": "bar.rs"}}]}
  ]
}"#;
    let files = extract_modified_files(array_content).expect("extract modified files should work");
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

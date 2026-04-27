use serde_json::json;

use super::*;

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

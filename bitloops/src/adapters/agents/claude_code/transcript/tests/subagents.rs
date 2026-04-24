use std::collections::HashMap;

use serde_json::json;
use tempfile::tempdir;

use super::*;

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

#[test]
#[allow(non_snake_case)]
fn TestExtractAllModifiedFiles_IncludesSubagentFiles() {
    let dir = tempdir().expect("failed to create temp dir");
    let transcript_path = dir.path().join("transcript.jsonl");
    let subagents_dir = dir.path().join("tasks").join("toolu_task1");
    std::fs::create_dir_all(&subagents_dir).expect("failed to create subagent dir");

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
    std::fs::create_dir_all(&subagents_dir).expect("failed to create subagent dir");

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
    std::fs::create_dir_all(&subagents_dir).expect("failed to create subagent dir");

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

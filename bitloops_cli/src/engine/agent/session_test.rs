use super::*;
use serde_json::json;
use std::time::SystemTime;

#[test]
#[allow(non_snake_case)]
fn TestAgentSessionStructure() {
    let session = AgentSession {
        session_id: "test-session-123".to_string(),
        agent_name: AGENT_NAME_CLAUDE_CODE.to_string(),
        repo_path: "/path/to/repo".to_string(),
        session_ref: "/path/to/session/file".to_string(),
        start_time: SystemTime::now(),
        entries: Vec::new(),
        modified_files: vec!["file1.rs".to_string()],
        new_files: vec!["file2.rs".to_string()],
        deleted_files: vec!["file3.rs".to_string()],
        ..AgentSession::default()
    };

    assert_eq!(session.session_id, "test-session-123");
    assert_eq!(session.agent_name, "claude-code");
}

#[test]
#[allow(non_snake_case)]
fn TestSessionEntryStructure() {
    let entry = SessionEntry {
        uuid: "entry-uuid-123".to_string(),
        entry_type: EntryType::Tool,
        timestamp: SystemTime::now(),
        content: "Tool output".to_string(),
        tool_name: "Write".to_string(),
        tool_input: json!({"file_path": "test.rs"}),
        tool_output: json!("file written"),
        files_affected: vec!["test.rs".to_string()],
    };

    assert_eq!(entry.uuid, "entry-uuid-123");
    assert_eq!(entry.entry_type, EntryType::Tool);
    assert_eq!(entry.entry_type.as_str(), "tool");
}

#[test]
#[allow(non_snake_case)]
fn TestGetLastUserPrompt() {
    let cases = vec![
        ("empty session", Vec::<SessionEntry>::new(), ""),
        (
            "single user entry",
            vec![SessionEntry {
                entry_type: EntryType::User,
                content: "hello".to_string(),
                ..SessionEntry::default()
            }],
            "hello",
        ),
        (
            "multiple entries, user last",
            vec![
                SessionEntry {
                    entry_type: EntryType::User,
                    content: "first".to_string(),
                    ..SessionEntry::default()
                },
                SessionEntry {
                    entry_type: EntryType::Assistant,
                    content: "response".to_string(),
                    ..SessionEntry::default()
                },
                SessionEntry {
                    entry_type: EntryType::User,
                    content: "second".to_string(),
                    ..SessionEntry::default()
                },
            ],
            "second",
        ),
        (
            "multiple entries, assistant last",
            vec![
                SessionEntry {
                    entry_type: EntryType::User,
                    content: "question".to_string(),
                    ..SessionEntry::default()
                },
                SessionEntry {
                    entry_type: EntryType::Assistant,
                    content: "answer".to_string(),
                    ..SessionEntry::default()
                },
            ],
            "question",
        ),
        (
            "no user entries",
            vec![
                SessionEntry {
                    entry_type: EntryType::System,
                    content: "system message".to_string(),
                    ..SessionEntry::default()
                },
                SessionEntry {
                    entry_type: EntryType::Assistant,
                    content: "greeting".to_string(),
                    ..SessionEntry::default()
                },
            ],
            "",
        ),
    ];

    for (name, entries, expected) in cases {
        let session = AgentSession {
            entries,
            ..AgentSession::default()
        };
        let result = session.get_last_user_prompt();
        assert_eq!(result, expected, "case {name} mismatch");
    }
}

#[test]
#[allow(non_snake_case)]
fn TestGetLastAssistantResponse() {
    let cases = vec![
        ("empty session", Vec::<SessionEntry>::new(), ""),
        (
            "single assistant entry",
            vec![SessionEntry {
                entry_type: EntryType::Assistant,
                content: "hello".to_string(),
                ..SessionEntry::default()
            }],
            "hello",
        ),
        (
            "multiple entries, assistant last",
            vec![
                SessionEntry {
                    entry_type: EntryType::User,
                    content: "question".to_string(),
                    ..SessionEntry::default()
                },
                SessionEntry {
                    entry_type: EntryType::Assistant,
                    content: "answer".to_string(),
                    ..SessionEntry::default()
                },
            ],
            "answer",
        ),
        (
            "multiple assistant entries",
            vec![
                SessionEntry {
                    entry_type: EntryType::Assistant,
                    content: "first response".to_string(),
                    ..SessionEntry::default()
                },
                SessionEntry {
                    entry_type: EntryType::User,
                    content: "follow up".to_string(),
                    ..SessionEntry::default()
                },
                SessionEntry {
                    entry_type: EntryType::Assistant,
                    content: "second response".to_string(),
                    ..SessionEntry::default()
                },
            ],
            "second response",
        ),
        (
            "no assistant entries",
            vec![
                SessionEntry {
                    entry_type: EntryType::User,
                    content: "question".to_string(),
                    ..SessionEntry::default()
                },
                SessionEntry {
                    entry_type: EntryType::Tool,
                    content: "tool output".to_string(),
                    ..SessionEntry::default()
                },
            ],
            "",
        ),
    ];

    for (name, entries, expected) in cases {
        let session = AgentSession {
            entries,
            ..AgentSession::default()
        };
        let result = session.get_last_assistant_response();
        assert_eq!(result, expected, "case {name} mismatch");
    }
}

#[test]
#[allow(non_snake_case)]
fn TestTruncateAtUUID() {
    {
        let session = AgentSession {
            session_id: "test".to_string(),
            entries: vec![
                SessionEntry {
                    uuid: "1".to_string(),
                    content: "first".to_string(),
                    ..SessionEntry::default()
                },
                SessionEntry {
                    uuid: "2".to_string(),
                    content: "second".to_string(),
                    ..SessionEntry::default()
                },
            ],
            ..AgentSession::default()
        };

        let result = session.truncate_at_uuid("");
        assert_eq!(
            result.session_id, session.session_id,
            "empty uuid should preserve session id"
        );
        assert_eq!(
            result.entries.len(),
            session.entries.len(),
            "empty uuid should keep all entries"
        );
        assert_eq!(
            result.entries[0].uuid, session.entries[0].uuid,
            "empty uuid should preserve first entry"
        );
        assert_eq!(
            result.entries[1].uuid, session.entries[1].uuid,
            "empty uuid should preserve second entry"
        );
    }

    {
        let session = AgentSession {
            session_id: "test".to_string(),
            agent_name: "claude-code".to_string(),
            repo_path: "/repo".to_string(),
            entries: vec![
                SessionEntry {
                    uuid: "1".to_string(),
                    content: "first".to_string(),
                    files_affected: vec!["a.rs".to_string()],
                    ..SessionEntry::default()
                },
                SessionEntry {
                    uuid: "2".to_string(),
                    content: "second".to_string(),
                    files_affected: vec!["b.rs".to_string()],
                    ..SessionEntry::default()
                },
                SessionEntry {
                    uuid: "3".to_string(),
                    content: "third".to_string(),
                    files_affected: vec!["c.rs".to_string()],
                    ..SessionEntry::default()
                },
            ],
            ..AgentSession::default()
        };

        let result = session.truncate_at_uuid("2");
        assert_eq!(result.entries.len(), 2, "expected truncation at uuid");
        assert_eq!(result.session_id, "test", "session metadata should persist");
    }

    {
        let session = AgentSession {
            session_id: "test".to_string(),
            entries: vec![
                SessionEntry {
                    uuid: "1".to_string(),
                    content: "first".to_string(),
                    ..SessionEntry::default()
                },
                SessionEntry {
                    uuid: "2".to_string(),
                    content: "second".to_string(),
                    ..SessionEntry::default()
                },
            ],
            ..AgentSession::default()
        };

        let result = session.truncate_at_uuid("nonexistent");
        assert_eq!(
            result.entries.len(),
            2,
            "missing uuid should return all entries"
        );
    }
}

#[test]
#[allow(non_snake_case)]
fn TestFindToolResultUUID() {
    let session = AgentSession {
        entries: vec![
            SessionEntry {
                uuid: "user-1".to_string(),
                entry_type: EntryType::User,
                ..SessionEntry::default()
            },
            SessionEntry {
                uuid: "tool-1".to_string(),
                entry_type: EntryType::Tool,
                ..SessionEntry::default()
            },
            SessionEntry {
                uuid: "assistant-1".to_string(),
                entry_type: EntryType::Assistant,
                ..SessionEntry::default()
            },
            SessionEntry {
                uuid: "tool-2".to_string(),
                entry_type: EntryType::Tool,
                ..SessionEntry::default()
            },
        ],
        ..AgentSession::default()
    };

    {
        let result = session.find_tool_result_uuid("tool-1");
        assert_eq!(result, Some("tool-1".to_string()));
    }

    {
        let result = session.find_tool_result_uuid("user-1");
        assert!(
            result.is_none(),
            "non-tool uuid should not be found as tool"
        );
    }

    {
        let result = session.find_tool_result_uuid("nonexistent");
        assert!(result.is_none(), "missing uuid should not be found");
    }
}

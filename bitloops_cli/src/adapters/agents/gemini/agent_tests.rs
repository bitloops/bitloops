use std::collections::HashMap;
use std::path::Path;

use serde_json::{Map, Value};
use tempfile::tempdir;

use super::*;
use crate::adapters::agents::gemini::transcript::GeminiToolCall;
use crate::adapters::agents::{
    AGENT_NAME_GEMINI, AGENT_TYPE_GEMINI, Agent, AgentSession, HookInput, HookSupport,
    MAX_CHUNK_SIZE,
};
use crate::test_support::process_state::{with_cwd, with_env_var};

#[test]
#[allow(non_snake_case)]
fn TestNewGeminiCLIAgent() {
    let agent = new_gemini_agent();
    assert_eq!(agent.name(), AGENT_NAME_GEMINI);
    assert_eq!(agent.agent_type(), AGENT_TYPE_GEMINI);
}

#[test]
#[allow(non_snake_case)]
fn TestName() {
    let agent = GeminiCliAgent;
    assert_eq!(agent.name(), AGENT_NAME_GEMINI);
    assert_eq!(agent.agent_type(), AGENT_TYPE_GEMINI);
}

#[test]
#[allow(non_snake_case)]
fn TestDescription() {
    let agent = GeminiCliAgent;
    assert_eq!(agent.description(), "Gemini - Google's AI coding assistant");
    assert!(agent.is_preview());
}

#[test]
#[allow(non_snake_case)]
fn TestDetectPresence() {
    let dir = tempdir().expect("failed to create temp dir");
    with_cwd(dir.path(), || {
        let agent = GeminiCliAgent;
        let present = agent
            .detect_presence()
            .expect("detect presence should not error");
        assert!(!present);

        std::fs::create_dir_all(dir.path().join(".gemini")).expect("failed to create .gemini");
        let present = agent
            .detect_presence()
            .expect("detect presence should not error");
        assert!(present);
    });
}

#[test]
#[allow(non_snake_case)]
fn TestGetSessionID() {
    let agent = GeminiCliAgent;
    let input = HookInput {
        session_id: "test-session-123".to_string(),
        ..HookInput::default()
    };

    assert_eq!(agent.get_session_id(&input), "test-session-123");
}

#[test]
#[allow(non_snake_case)]
fn TestResolveSessionFile() {
    let dir = tempdir().expect("failed to create temp dir");
    let agent = GeminiCliAgent;

    let existing = dir
        .path()
        .join("session-2026-02-10T09-19-0544a0f5.json")
        .to_string_lossy()
        .to_string();
    std::fs::write(&existing, b"{}").expect("failed to write test session file");

    let resolved = agent.resolve_session_file(
        dir.path().to_string_lossy().as_ref(),
        "0544a0f5-46a6-41b3-a89c-e7804df731b8",
    );
    assert_eq!(resolved, existing);

    let fallback = agent.resolve_session_file(dir.path().to_string_lossy().as_ref(), "abc123");
    let fallback_name = Path::new(&fallback)
        .file_name()
        .expect("fallback should have file name")
        .to_string_lossy()
        .to_string();
    assert!(fallback_name.starts_with("session-"));
    assert!(fallback_name.ends_with("-abc123.json"));
    assert_eq!(
        Path::new(&fallback)
            .parent()
            .expect("fallback should have parent"),
        dir.path()
    );
}

#[test]
#[allow(non_snake_case)]
fn TestProtectedDirs() {
    let agent = GeminiCliAgent;
    assert_eq!(agent.protected_dirs(), vec![".gemini".to_string()]);
}

#[test]
#[allow(non_snake_case)]
fn TestGetSessionDir() {
    let agent = GeminiCliAgent;

    with_env_var(
        "BITLOOPS_TEST_GEMINI_PROJECT_DIR",
        Some("/test/override"),
        || {
            let session_dir = agent
                .get_session_dir("/some/repo")
                .expect("get session dir should not fail with override");
            assert_eq!(session_dir, "/test/override");
        },
    );
    let session_dir = agent
        .get_session_dir("/some/repo")
        .expect("get session dir should not fail");
    assert!(Path::new(&session_dir).is_absolute());
}

#[test]
#[allow(non_snake_case)]
fn TestFormatResumeCommand() {
    let agent = GeminiCliAgent;
    assert_eq!(
        agent.format_resume_command("abc123"),
        "gemini --resume abc123"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestReadSession() {
    let dir = tempdir().expect("failed to create temp dir");
    let transcript_path = dir.path().join("transcript.json");
    std::fs::write(
        &transcript_path,
        br#"{"messages": [{"type": "user", "content": "hello"}]}"#,
    )
    .expect("failed to write transcript");

    let agent = GeminiCliAgent;
    let session = agent
        .read_session(&HookInput {
            session_id: "test-session".to_string(),
            session_ref: transcript_path.to_string_lossy().to_string(),
            ..HookInput::default()
        })
        .expect("read_session should succeed")
        .expect("read_session should return a session");

    assert_eq!(session.session_id, "test-session");
    assert_eq!(session.agent_name, AGENT_NAME_GEMINI);
    assert!(!session.native_data.is_empty());

    let err = agent
        .read_session(&HookInput {
            session_id: "test-session".to_string(),
            ..HookInput::default()
        })
        .expect_err("expected read_session without session ref to error");
    assert!(
        err.to_string()
            .contains("session reference (transcript path) is required")
    );
}

#[test]
#[allow(non_snake_case)]
fn TestWriteSession() {
    let dir = tempdir().expect("failed to create temp dir");
    let transcript_path = dir.path().join("transcript.json");

    let agent = GeminiCliAgent;
    let session = AgentSession {
        session_id: "test-session".to_string(),
        agent_name: AGENT_NAME_GEMINI.to_string(),
        session_ref: transcript_path.to_string_lossy().to_string(),
        native_data: br#"{"messages": []}"#.to_vec(),
        ..AgentSession::default()
    };

    agent
        .write_session(&session)
        .expect("write_session should succeed");
    let data = std::fs::read(&transcript_path).expect("failed to read transcript");
    assert_eq!(data, br#"{"messages": []}"#);

    let err = agent
        .write_session(&AgentSession {
            agent_name: "claude-code".to_string(),
            session_ref: "/tmp/nope.json".to_string(),
            native_data: b"{}".to_vec(),
            ..AgentSession::default()
        })
        .expect_err("write_session should fail for wrong agent");
    assert!(
        err.to_string()
            .contains("session belongs to agent \"claude-code\", not \"gemini\"")
    );

    let err = agent
        .write_session(&AgentSession {
            agent_name: AGENT_NAME_GEMINI.to_string(),
            native_data: b"{}".to_vec(),
            ..AgentSession::default()
        })
        .expect_err("write_session should fail with empty session ref");
    assert!(
        err.to_string()
            .contains("session reference (transcript path) is required")
    );

    let err = agent
        .write_session(&AgentSession {
            agent_name: AGENT_NAME_GEMINI.to_string(),
            session_ref: "/tmp/nope.json".to_string(),
            ..AgentSession::default()
        })
        .expect_err("write_session should fail without native data");
    assert!(
        err.to_string()
            .contains("session has no native data to write")
    );
}

#[test]
#[allow(non_snake_case)]
fn TestGetProjectHash() {
    let h1 = GeminiCliAgent::get_project_hash("/Users/test/project");
    let h2 = GeminiCliAgent::get_project_hash("/Users/test/project");
    assert_eq!(h1, h2);
    assert_eq!(h1.len(), 64);

    let h3 = GeminiCliAgent::get_project_hash("/Users/test/other");
    assert_ne!(h1, h3);
}

#[test]
#[allow(non_snake_case)]
fn TestChunkTranscript() {
    let agent = GeminiCliAgent;

    let small = br#"{"messages":[{"type":"user","content":"hello"},{"type":"gemini","content":"hi there"}]}"#;
    let chunks = agent
        .chunk_transcript(small, MAX_CHUNK_SIZE)
        .expect("chunk small transcript should work");
    assert_eq!(chunks.len(), 1);

    let mut messages = Vec::new();
    for idx in 0..100 {
        messages.push(GeminiMessage {
            id: String::new(),
            r#type: "user".to_string(),
            content: format!(
                "message {idx} with some content to make it larger: {}",
                "x".repeat(500)
            ),
            tool_calls: Vec::new(),
        });
    }
    let large =
        serde_json::to_vec(&GeminiTranscript { messages }).expect("marshal large transcript");
    let chunks = agent
        .chunk_transcript(&large, 5000)
        .expect("chunk large transcript should work");
    assert!(chunks.len() >= 2);
    for chunk in &chunks {
        let parsed: GeminiTranscript =
            serde_json::from_slice(chunk).expect("chunk should be valid Gemini JSON");
        assert!(!parsed.messages.is_empty());
    }
    let reassembled = agent
        .reassemble_transcript(&chunks)
        .expect("reassemble should work");
    let parsed: GeminiTranscript =
        serde_json::from_slice(&reassembled).expect("reassembled should parse");
    assert_eq!(parsed.messages.len(), 100);

    let empty = br#"{"messages":[]}"#;
    let chunks = agent
        .chunk_transcript(empty, MAX_CHUNK_SIZE)
        .expect("chunk empty transcript should work");
    assert_eq!(chunks, vec![empty.to_vec()]);

    let jsonl = br#"{"type":"user","content":"hello"}
{"type":"gemini","content":"hi"}"#;
    let chunks = agent
        .chunk_transcript(jsonl, MAX_CHUNK_SIZE)
        .expect("chunk jsonl fallback should work");
    assert_eq!(chunks.len(), 1);

    let original = GeminiTranscript {
        messages: vec![
            GeminiMessage {
                id: String::new(),
                r#type: "user".to_string(),
                content: "Write a hello world program".to_string(),
                tool_calls: Vec::new(),
            },
            GeminiMessage {
                id: String::new(),
                r#type: "gemini".to_string(),
                content: "Sure, here's a hello world program:".to_string(),
                tool_calls: vec![GeminiToolCall {
                    id: "1".to_string(),
                    name: "write_file".to_string(),
                    args: HashMap::from([(
                        "path".to_string(),
                        Value::String("main.rs".to_string()),
                    )]),
                    status: String::new(),
                }],
            },
            GeminiMessage {
                id: String::new(),
                r#type: "user".to_string(),
                content: "Now add a function".to_string(),
                tool_calls: Vec::new(),
            },
            GeminiMessage {
                id: String::new(),
                r#type: "gemini".to_string(),
                content: "I'll add a greet function:".to_string(),
                tool_calls: vec![GeminiToolCall {
                    id: "2".to_string(),
                    name: "edit_file".to_string(),
                    args: HashMap::from([(
                        "path".to_string(),
                        Value::String("main.rs".to_string()),
                    )]),
                    status: String::new(),
                }],
            },
        ],
    };
    let content = serde_json::to_vec(&original).expect("marshal original transcript");
    let chunks = agent
        .chunk_transcript(&content, 200)
        .expect("chunk roundtrip transcript should work");
    let reassembled = agent
        .reassemble_transcript(&chunks)
        .expect("reassemble roundtrip transcript should work");
    let result: GeminiTranscript =
        serde_json::from_slice(&reassembled).expect("roundtrip transcript should parse");
    assert_eq!(result.messages.len(), original.messages.len());
    for (idx, msg) in result.messages.iter().enumerate() {
        assert_eq!(msg.r#type, original.messages[idx].r#type);
        assert_eq!(msg.content, original.messages[idx].content);
        assert_eq!(
            msg.tool_calls.len(),
            original.messages[idx].tool_calls.len()
        );
    }

    let oversized = GeminiTranscript {
        messages: vec![GeminiMessage {
            id: String::new(),
            r#type: "user".to_string(),
            content: "x".repeat(1000),
            tool_calls: Vec::new(),
        }],
    };
    let oversized = serde_json::to_vec(&oversized).expect("marshal oversized transcript");
    let chunks = agent
        .chunk_transcript(&oversized, 100)
        .expect("chunk oversized transcript should work");
    assert_eq!(chunks.len(), 1);
    let parsed: GeminiTranscript =
        serde_json::from_slice(&chunks[0]).expect("oversized chunk should parse");
    assert_eq!(parsed.messages.len(), 1);

    let boundary = GeminiTranscript {
        messages: vec![
            GeminiMessage {
                id: String::new(),
                r#type: "user".to_string(),
                content: "msg1".to_string(),
                tool_calls: Vec::new(),
            },
            GeminiMessage {
                id: String::new(),
                r#type: "gemini".to_string(),
                content: "msg2".to_string(),
                tool_calls: Vec::new(),
            },
            GeminiMessage {
                id: String::new(),
                r#type: "user".to_string(),
                content: "msg3".to_string(),
                tool_calls: Vec::new(),
            },
            GeminiMessage {
                id: String::new(),
                r#type: "gemini".to_string(),
                content: "msg4".to_string(),
                tool_calls: Vec::new(),
            },
        ],
    };
    let boundary = serde_json::to_vec(&boundary).expect("marshal boundary transcript");
    let chunks = agent
        .chunk_transcript(&boundary, 100)
        .expect("chunk boundary transcript should work");
    let mut total = 0;
    for chunk in &chunks {
        let parsed: GeminiTranscript =
            serde_json::from_slice(chunk).expect("boundary chunk should parse");
        total += parsed.messages.len();
    }
    assert_eq!(total, 4);

    let mut ordered_messages = Vec::new();
    for idx in 0..20 {
        ordered_messages.push(GeminiMessage {
            id: String::new(),
            r#type: "user".to_string(),
            content: format!("message-{idx:03}"),
            tool_calls: Vec::new(),
        });
    }
    let ordered = serde_json::to_vec(&GeminiTranscript {
        messages: ordered_messages,
    })
    .expect("marshal ordered transcript");
    let chunks = agent
        .chunk_transcript(&ordered, 200)
        .expect("chunk ordered transcript should work");
    let reassembled = agent
        .reassemble_transcript(&chunks)
        .expect("reassemble ordered transcript should work");
    let result: GeminiTranscript =
        serde_json::from_slice(&reassembled).expect("ordered transcript should parse");
    for (idx, msg) in result.messages.iter().enumerate() {
        assert_eq!(msg.content, format!("message-{idx:03}"));
    }
}

#[test]
#[allow(non_snake_case)]
fn TestReassembleTranscript() {
    let agent = GeminiCliAgent;

    let single = vec![br#"{"messages":[{"type":"user","content":"hello"}]}"#.to_vec()];
    let reassembled = agent
        .reassemble_transcript(&single)
        .expect("reassemble single chunk should work");
    let parsed: GeminiTranscript =
        serde_json::from_slice(&reassembled).expect("reassembled chunk should be valid json");
    assert_eq!(parsed.messages.len(), 1);

    let multiple = vec![
        br#"{"messages":[{"type":"user","content":"hello"}]}"#.to_vec(),
        br#"{"messages":[{"type":"gemini","content":"hi"}]}"#.to_vec(),
    ];
    let reassembled = agent
        .reassemble_transcript(&multiple)
        .expect("reassemble multiple chunks should work");
    let parsed: GeminiTranscript =
        serde_json::from_slice(&reassembled).expect("reassembled chunk should be valid json");
    assert_eq!(parsed.messages.len(), 2);

    let err = agent
        .reassemble_transcript(&[
            br#"{"messages":[{"type":"user","content":"hello"}]}"#.to_vec(),
            b"not valid json".to_vec(),
        ])
        .expect_err("invalid chunk should fail");
    assert!(err.to_string().contains("failed to unmarshal chunk"));

    let reassembled = agent
        .reassemble_transcript(&[])
        .expect("empty chunks should return empty transcript");
    let parsed: GeminiTranscript =
        serde_json::from_slice(&reassembled).expect("reassembled chunk should be valid json");
    assert!(parsed.messages.is_empty());
}

#[test]
#[allow(non_snake_case)]
fn TestInstallHooks() {
    let agent = GeminiCliAgent;
    // Fresh install + idempotent + force + command verification.
    let dir = tempdir().expect("failed to create temp dir");
    with_cwd(dir.path(), || {
        let count = agent
            .install_hooks(false, false)
            .expect("fresh hook install should work");
        assert_eq!(count, 12);

        let settings = read_gemini_settings(dir.path());
        assert!(settings.hooks_config.enabled);
        assert_eq!(settings.hooks.session_start.len(), 1);
        assert_eq!(settings.hooks.session_end.len(), 2);
        assert_eq!(settings.hooks.before_agent.len(), 1);
        assert_eq!(settings.hooks.after_agent.len(), 1);
        assert_eq!(settings.hooks.before_model.len(), 1);
        assert_eq!(settings.hooks.after_model.len(), 1);
        assert_eq!(settings.hooks.before_tool_selection.len(), 1);
        assert_eq!(settings.hooks.before_tool.len(), 1);
        assert_eq!(settings.hooks.after_tool.len(), 1);
        assert_eq!(settings.hooks.pre_compress.len(), 1);
        assert_eq!(settings.hooks.notification.len(), 1);
        verify_hook_command(
            &settings.hooks.session_start,
            "",
            "bitloops hooks gemini session-start",
        );
        verify_hook_command(
            &settings.hooks.session_end,
            "exit",
            "bitloops hooks gemini session-end",
        );
        verify_hook_command(
            &settings.hooks.session_end,
            "logout",
            "bitloops hooks gemini session-end",
        );
        verify_hook_command(
            &settings.hooks.before_agent,
            "",
            "bitloops hooks gemini before-agent",
        );
        verify_hook_command(
            &settings.hooks.after_agent,
            "",
            "bitloops hooks gemini after-agent",
        );
        verify_hook_command(
            &settings.hooks.before_model,
            "",
            "bitloops hooks gemini before-model",
        );
        verify_hook_command(
            &settings.hooks.after_model,
            "",
            "bitloops hooks gemini after-model",
        );
        verify_hook_command(
            &settings.hooks.before_tool_selection,
            "",
            "bitloops hooks gemini before-tool-selection",
        );
        verify_hook_command(
            &settings.hooks.before_tool,
            "*",
            "bitloops hooks gemini before-tool",
        );
        verify_hook_command(
            &settings.hooks.after_tool,
            "*",
            "bitloops hooks gemini after-tool",
        );
        verify_hook_command(
            &settings.hooks.pre_compress,
            "",
            "bitloops hooks gemini pre-compress",
        );
        verify_hook_command(
            &settings.hooks.notification,
            "",
            "bitloops hooks gemini notification",
        );

        let count = agent
            .install_hooks(false, false)
            .expect("idempotent hook install should work");
        assert_eq!(count, 0);

        let count = agent
            .install_hooks(false, true)
            .expect("force hook install should work");
        assert_eq!(count, 12);
    });

    // Local dev command prefix.
    let local_dir = tempdir().expect("failed to create temp dir");
    with_cwd(local_dir.path(), || {
        let count = agent
            .install_hooks(true, false)
            .expect("local dev hook install should work");
        assert_eq!(count, 12);
        let settings = read_gemini_settings(local_dir.path());
        verify_hook_command(
            &settings.hooks.session_start,
            "",
            "cargo run -- hooks gemini session-start",
        );
    });

    // Preserve user hooks.
    let user_dir = tempdir().expect("failed to create temp dir");
    with_cwd(user_dir.path(), || {
        write_gemini_settings(
            user_dir.path(),
            r#"{
  "hooks": {
"SessionStart": [
  {
    "matcher": "startup",
    "hooks": [{"name": "my-hook", "type": "command", "command": "echo hello"}]
  }
]
  }
}"#,
        );
        agent
            .install_hooks(false, false)
            .expect("install hooks should preserve user hooks");
        let settings = read_gemini_settings(user_dir.path());
        assert_eq!(settings.hooks.session_start.len(), 2);
        let found_user_hook = settings.hooks.session_start.iter().any(|matcher| {
            matcher.matcher == "startup"
                && matcher
                    .hooks
                    .iter()
                    .any(|hook| hook.name == "my-hook" && hook.command == "echo hello")
        });
        assert!(found_user_hook);
    });

    // Preserve unknown hook types.
    let unknown_hook_dir = tempdir().expect("failed to create temp dir");
    with_cwd(unknown_hook_dir.path(), || {
        write_gemini_settings(
            unknown_hook_dir.path(),
            r#"{
  "hooks": {
"FutureHook": [
  {
    "matcher": "",
    "hooks": [{"name": "future-hook", "type": "command", "command": "echo future"}]
  }
],
"AnotherNewHook": [
  {
    "matcher": "pattern",
    "hooks": [{"name": "another-hook", "type": "command", "command": "echo another"}]
  }
]
  }
}"#,
        );
        agent
            .install_hooks(false, false)
            .expect("install hooks should preserve unknown hook types");
        let raw_hooks = read_raw_hooks(unknown_hook_dir.path());
        assert!(raw_hooks.contains_key("FutureHook"));
        assert!(raw_hooks.contains_key("AnotherNewHook"));
        assert!(raw_hooks.contains_key("SessionStart"));
        let future_matchers: Vec<GeminiHookMatcher> =
            serde_json::from_value(raw_hooks["FutureHook"].clone())
                .expect("failed to parse FutureHook");
        assert_eq!(future_matchers.len(), 1);
        assert_eq!(future_matchers[0].hooks[0].command, "echo future");
        let another_matchers: Vec<GeminiHookMatcher> =
            serde_json::from_value(raw_hooks["AnotherNewHook"].clone())
                .expect("failed to parse AnotherNewHook");
        assert_eq!(another_matchers[0].matcher, "pattern");
    });

    // Preserve unknown top-level settings fields.
    let unknown_fields_dir = tempdir().expect("failed to create temp dir");
    with_cwd(unknown_fields_dir.path(), || {
        write_gemini_settings(
            unknown_fields_dir.path(),
            r#"{
  "someOtherField": "value",
  "customConfig": {"nested": true}
}"#,
        );
        agent
            .install_hooks(false, false)
            .expect("install hooks should preserve unknown settings fields");
        let raw_settings = read_raw_settings(unknown_fields_dir.path());
        assert!(raw_settings.contains_key("someOtherField"));
        assert!(raw_settings.contains_key("customConfig"));
    });
}

#[test]
#[allow(non_snake_case)]
fn TestUninstallHooks() {
    let agent = GeminiCliAgent;
    let dir = tempdir().expect("failed to create temp dir");
    with_cwd(dir.path(), || {
        agent
            .install_hooks(false, false)
            .expect("hook install should work");
        assert!(agent.are_hooks_installed());

        agent.uninstall_hooks().expect("hook uninstall should work");
        assert!(!agent.are_hooks_installed());
    });

    // No settings file should not error.
    let no_settings_dir = tempdir().expect("failed to create temp dir");
    with_cwd(no_settings_dir.path(), || {
        agent
            .uninstall_hooks()
            .expect("uninstall should not fail without settings");
    });

    // Preserve user hooks.
    let user_dir = tempdir().expect("failed to create temp dir");
    with_cwd(user_dir.path(), || {
        write_gemini_settings(
            user_dir.path(),
            r#"{
  "hooks": {
"SessionStart": [
  {
    "matcher": "startup",
    "hooks": [{"name": "my-hook", "type": "command", "command": "echo hello"}]
  },
  {
    "hooks": [{"name": "bitloops-session-start", "type": "command", "command": "bitloops hooks gemini session-start"}]
  }
]
  }
}"#,
        );
        agent
            .uninstall_hooks()
            .expect("uninstall should preserve user hooks");
        let settings = read_gemini_settings(user_dir.path());
        assert_eq!(settings.hooks.session_start.len(), 1);
        assert_eq!(settings.hooks.session_start[0].matcher, "startup");
    });

    // Preserve unknown hook types.
    let unknown_type_dir = tempdir().expect("failed to create temp dir");
    with_cwd(unknown_type_dir.path(), || {
        write_gemini_settings(
            unknown_type_dir.path(),
            r#"{
  "hooks": {
"SessionStart": [
  {
    "hooks": [{"name": "bitloops-session-start", "type": "command", "command": "bitloops hooks gemini session-start"}]
  }
],
"FutureHook": [
  {
    "matcher": "",
    "hooks": [{"name": "future-hook", "type": "command", "command": "echo future"}]
  }
]
  }
}"#,
        );
        agent
            .uninstall_hooks()
            .expect("uninstall should preserve unknown hook types");
        let raw_hooks = read_raw_hooks(unknown_type_dir.path());
        assert!(raw_hooks.contains_key("FutureHook"));
        if let Some(session_start) = raw_hooks.get("SessionStart")
            && let Ok(matchers) =
                serde_json::from_value::<Vec<GeminiHookMatcher>>(session_start.clone())
        {
            assert!(matchers.is_empty());
        }
    });
}

#[test]
#[allow(non_snake_case)]
fn TestAreHooksInstalled() {
    let dir = tempdir().expect("failed to create temp dir");
    with_cwd(dir.path(), || {
        let agent = GeminiCliAgent;
        assert!(!agent.are_hooks_installed());

        agent
            .install_hooks(false, false)
            .expect("hook install should work");
        assert!(agent.are_hooks_installed());
    });
}

#[test]
#[allow(non_snake_case)]
fn TestHookNames() {
    let agent = GeminiCliAgent;
    let names = agent.hook_names();
    let expected = vec![
        HOOK_NAME_SESSION_START,
        HOOK_NAME_SESSION_END,
        HOOK_NAME_BEFORE_AGENT,
        HOOK_NAME_AFTER_AGENT,
        HOOK_NAME_BEFORE_MODEL,
        HOOK_NAME_AFTER_MODEL,
        HOOK_NAME_BEFORE_TOOL_SELECTION,
        HOOK_NAME_BEFORE_TOOL,
        HOOK_NAME_AFTER_TOOL,
        HOOK_NAME_PRE_COMPRESS,
        HOOK_NAME_NOTIFICATION,
    ];
    assert_eq!(names.len(), expected.len());
    for (idx, expected_name) in expected.iter().enumerate() {
        assert_eq!(names[idx], *expected_name);
    }
}

#[test]
#[allow(non_snake_case)]
fn TestParseHookEvent_SessionStart() {
    let agent = GeminiCliAgent;
    let mut stdin = std::io::Cursor::new(
        r#"{"session_id":"gemini-session-123","transcript_path":"/tmp/gemini.json"}"#,
    );
    let event = agent
        .parse_hook_event(HOOK_NAME_SESSION_START, &mut stdin)
        .expect("parse should succeed");
    assert!(event.is_some(), "expected event for session-start");
}

#[test]
#[allow(non_snake_case)]
fn TestParseHookEvent_TurnStart() {
    let agent = GeminiCliAgent;
    let mut stdin = std::io::Cursor::new(
        r#"{"session_id":"sess-456","transcript_path":"/tmp/t.json","prompt":"Hello Gemini"}"#,
    );
    let event = agent
        .parse_hook_event(HOOK_NAME_BEFORE_AGENT, &mut stdin)
        .expect("parse should succeed");
    assert!(
        event.is_some(),
        "expected event for before-agent (TurnStart)"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestParseHookEvent_TurnEnd() {
    let agent = GeminiCliAgent;
    let mut stdin =
        std::io::Cursor::new(r#"{"session_id":"sess-789","transcript_path":"/tmp/after.json"}"#);
    let event = agent
        .parse_hook_event(HOOK_NAME_AFTER_AGENT, &mut stdin)
        .expect("parse should succeed");
    assert!(event.is_some(), "expected event for after-agent (TurnEnd)");
}

#[test]
#[allow(non_snake_case)]
fn TestParseHookEvent_SessionEnd() {
    let agent = GeminiCliAgent;
    let mut stdin = std::io::Cursor::new(
        r#"{"session_id":"ending-session","transcript_path":"/tmp/end.json"}"#,
    );
    let event = agent
        .parse_hook_event(HOOK_NAME_SESSION_END, &mut stdin)
        .expect("parse should succeed");
    assert!(event.is_some(), "expected event for session-end");
}

#[test]
#[allow(non_snake_case)]
fn TestParseHookEvent_Compaction() {
    let agent = GeminiCliAgent;
    let mut stdin = std::io::Cursor::new(
        r#"{"session_id":"compress-session","transcript_path":"/tmp/compress.json"}"#,
    );
    let event = agent
        .parse_hook_event(HOOK_NAME_PRE_COMPRESS, &mut stdin)
        .expect("parse should succeed");
    assert!(
        event.is_some(),
        "expected event for pre-compress (Compaction)"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestParseHookEvent_PassThroughHooks_ReturnNil() {
    let agent = GeminiCliAgent;
    let pass_through = [
        HOOK_NAME_BEFORE_TOOL,
        HOOK_NAME_AFTER_TOOL,
        HOOK_NAME_BEFORE_MODEL,
        HOOK_NAME_AFTER_MODEL,
        HOOK_NAME_BEFORE_TOOL_SELECTION,
        HOOK_NAME_NOTIFICATION,
    ];
    for hook_name in pass_through {
        let mut stdin = std::io::Cursor::new(r#"{"session_id":"test","transcript_path":"/t"}"#);
        let event = agent
            .parse_hook_event(hook_name, &mut stdin)
            .expect("parse should not error");
        assert!(event.is_none(), "expected nil event for {hook_name}");
    }
}

#[test]
#[allow(non_snake_case)]
fn TestParseHookEvent_UnknownHook_ReturnsNil() {
    let agent = GeminiCliAgent;
    let mut stdin =
        std::io::Cursor::new(r#"{"session_id":"unknown","transcript_path":"/tmp/unknown.json"}"#);
    let event = agent
        .parse_hook_event("unknown-hook-name", &mut stdin)
        .expect("parse should not error");
    assert!(event.is_none());
}

#[test]
#[allow(non_snake_case)]
fn TestParseHookEvent_EmptyInput() {
    let agent = GeminiCliAgent;
    let mut stdin = std::io::Cursor::new("");
    let err = agent
        .parse_hook_event(HOOK_NAME_SESSION_START, &mut stdin)
        .expect_err("expected error for empty input");
    assert!(
        err.to_string().contains("empty hook input"),
        "expected 'empty hook input' error, got: {err}"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestParseHookEvent_MalformedJSON() {
    let agent = GeminiCliAgent;
    let mut stdin = std::io::Cursor::new(r#"{"session_id":"test","transcript_path":INVALID}"#);
    let err = agent
        .parse_hook_event(HOOK_NAME_SESSION_START, &mut stdin)
        .expect_err("expected error for malformed JSON");
    assert!(
        err.to_string().contains("failed to parse hook input"),
        "expected 'failed to parse hook input' error, got: {err}"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestParseHookEvent_AllLifecycleHooks() {
    let agent = GeminiCliAgent;
    let lifecycle_produces_event = [
        (
            HOOK_NAME_SESSION_START,
            r#"{"session_id":"s1","transcript_path":"/t"}"#,
        ),
        (
            HOOK_NAME_BEFORE_AGENT,
            r#"{"session_id":"s2","transcript_path":"/t","prompt":"hi"}"#,
        ),
        (
            HOOK_NAME_AFTER_AGENT,
            r#"{"session_id":"s3","transcript_path":"/t"}"#,
        ),
        (
            HOOK_NAME_SESSION_END,
            r#"{"session_id":"s4","transcript_path":"/t"}"#,
        ),
        (
            HOOK_NAME_PRE_COMPRESS,
            r#"{"session_id":"s5","transcript_path":"/t"}"#,
        ),
    ];
    for (hook_name, input) in lifecycle_produces_event {
        let mut stdin = std::io::Cursor::new(input);
        let event = agent
            .parse_hook_event(hook_name, &mut stdin)
            .expect("parse should succeed");
        assert!(event.is_some(), "expected event for {hook_name}");
    }
    let pass_through = [
        (
            HOOK_NAME_BEFORE_TOOL,
            r#"{"session_id":"s6","transcript_path":"/t"}"#,
        ),
        (
            HOOK_NAME_AFTER_TOOL,
            r#"{"session_id":"s7","transcript_path":"/t"}"#,
        ),
        (
            HOOK_NAME_BEFORE_MODEL,
            r#"{"session_id":"s8","transcript_path":"/t"}"#,
        ),
        (
            HOOK_NAME_AFTER_MODEL,
            r#"{"session_id":"s9","transcript_path":"/t"}"#,
        ),
        (
            HOOK_NAME_BEFORE_TOOL_SELECTION,
            r#"{"session_id":"s10","transcript_path":"/t"}"#,
        ),
        (
            HOOK_NAME_NOTIFICATION,
            r#"{"session_id":"s11","transcript_path":"/t"}"#,
        ),
    ];
    for (hook_name, input) in pass_through {
        let mut stdin = std::io::Cursor::new(input);
        let event = agent
            .parse_hook_event(hook_name, &mut stdin)
            .expect("parse should succeed");
        assert!(event.is_none(), "expected nil event for {hook_name}");
    }
}

#[derive(Debug, serde::Deserialize, Default)]
struct SessionInfoRawForTest {
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    transcript_path: String,
    #[serde(default)]
    cwd: String,
}

#[derive(Debug, serde::Deserialize, Default)]
struct AgentHookInputRawForTest {
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    prompt: String,
    #[serde(default)]
    hook_event_name: String,
}

#[test]
#[allow(non_snake_case)]
fn TestReadAndParse_ValidInput() {
    let mut stdin = std::io::Cursor::new(
        r#"{"session_id":"test-123","transcript_path":"/path/to/transcript","cwd":"/home/user"}"#,
    );
    let parsed: SessionInfoRawForTest =
        crate::host::lifecycle::read_and_parse_hook_input(&mut stdin).unwrap();
    assert_eq!("test-123", parsed.session_id);
    assert_eq!("/path/to/transcript", parsed.transcript_path);
    assert_eq!("/home/user", parsed.cwd);
}

#[test]
#[allow(non_snake_case)]
fn TestReadAndParse_EmptyInput() {
    let mut stdin = std::io::Cursor::new("");
    let err =
        crate::host::lifecycle::read_and_parse_hook_input::<SessionInfoRawForTest>(&mut stdin)
            .unwrap_err();
    assert!(err.to_string().contains("empty hook input"));
}

#[test]
#[allow(non_snake_case)]
fn TestReadAndParse_InvalidJSON() {
    let mut stdin = std::io::Cursor::new("not valid json");
    let err =
        crate::host::lifecycle::read_and_parse_hook_input::<SessionInfoRawForTest>(&mut stdin)
            .unwrap_err();
    assert!(err.to_string().contains("failed to parse hook input"));
}

#[test]
#[allow(non_snake_case)]
fn TestReadAndParse_PartialJSON() {
    let mut stdin = std::io::Cursor::new(r#"{"session_id":"partial-only"}"#);
    let parsed: SessionInfoRawForTest =
        crate::host::lifecycle::read_and_parse_hook_input(&mut stdin).unwrap();
    assert_eq!("partial-only", parsed.session_id);
    assert!(parsed.transcript_path.is_empty());
}

#[test]
#[allow(non_snake_case)]
fn TestReadAndParse_ExtraFields() {
    let mut stdin = std::io::Cursor::new(
        r#"{"session_id":"test","transcript_path":"/t","extra_field":"ignored","another":123}"#,
    );
    let parsed: SessionInfoRawForTest =
        crate::host::lifecycle::read_and_parse_hook_input(&mut stdin).unwrap();
    assert_eq!("test", parsed.session_id);
}

#[test]
#[allow(non_snake_case)]
fn TestReadAndParse_AgentHookInput() {
    let input = r#"{"session_id":"agent-session","transcript_path":"/path/to/agent.json","hook_event_name":"before-agent","prompt":"User's question here"}"#;
    let parsed: AgentHookInputRawForTest =
        GeminiCliAgent::read_and_parse_hook_input(input).unwrap();
    assert_eq!("agent-session", parsed.session_id);
    assert_eq!("User's question here", parsed.prompt);
    assert_eq!("before-agent", parsed.hook_event_name);
}

fn read_gemini_settings(root: &Path) -> GeminiSettings {
    let settings_path = root.join(".gemini").join("settings.json");
    let data = std::fs::read(&settings_path).expect("failed to read settings.json");
    serde_json::from_slice(&data).expect("failed to parse settings.json")
}

fn read_raw_settings(root: &Path) -> Map<String, Value> {
    let settings_path = root.join(".gemini").join("settings.json");
    let data = std::fs::read(&settings_path).expect("failed to read settings.json");
    serde_json::from_slice(&data).expect("failed to parse raw settings.json")
}

fn read_raw_hooks(root: &Path) -> Map<String, Value> {
    let raw_settings = read_raw_settings(root);
    raw_settings
        .get("hooks")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

fn write_gemini_settings(root: &Path, content: &str) {
    let gemini_dir = root.join(".gemini");
    std::fs::create_dir_all(&gemini_dir).expect("failed to create .gemini");
    std::fs::write(gemini_dir.join("settings.json"), content.as_bytes())
        .expect("failed to write settings.json");
}

fn verify_hook_command(
    matchers: &[GeminiHookMatcher],
    expected_matcher: &str,
    expected_command: &str,
) {
    for matcher in matchers {
        if matcher.matcher == expected_matcher {
            for hook in &matcher.hooks {
                if hook.command == expected_command {
                    return;
                }
            }
        }
    }
    panic!("hook with matcher={expected_matcher:?} command={expected_command:?} not found");
}

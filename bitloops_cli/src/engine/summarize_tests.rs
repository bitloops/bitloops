use super::{
    AgentType, ClaudeGenerator, Entry, EntryType, Input, build_condensed_transcript,
    build_condensed_transcript_from_bytes, build_summarization_prompt, extract_json_from_markdown,
    format_condensed_transcript, generate_from_transcript, strip_git_env,
};
use crate::engine::transcript::types::{
    AssistantMessage, ContentBlock, Line, ToolInput, UserMessage,
};
use crate::test_support::process_state::with_env_vars;
use serde::Serialize;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

fn must_json<T: Serialize>(value: T) -> Value {
    serde_json::to_value(value).expect("serialize test json")
}

fn user_line(uuid: &str, content: Value) -> Line {
    Line {
        r#type: "user".to_string(),
        uuid: uuid.to_string(),
        message: must_json(UserMessage { content }),
    }
}

fn assistant_line(uuid: &str, content: Vec<ContentBlock>) -> Line {
    Line {
        r#type: "assistant".to_string(),
        uuid: uuid.to_string(),
        message: must_json(AssistantMessage { content }),
    }
}

// CLI-752
#[test]
fn test_claude_generator_git_isolation() {
    let captured_invocation = Arc::new(Mutex::new(None::<super::CommandInvocation>));
    let captured_invocation_clone = Arc::clone(&captured_invocation);

    let response = r#"{"result":"{\"intent\":\"test\",\"outcome\":\"test\",\"learnings\":{\"repo\":[],\"code\":[],\"workflow\":[]},\"friction\":[],\"open_items\":[]}"}"#;
    let generator = ClaudeGenerator {
        command_runner: Some(Arc::new(move |invocation| {
            *captured_invocation_clone
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(invocation);
            Ok(response.to_string())
        })),
        ..Default::default()
    };

    with_env_vars(
        &[
            ("GIT_DIR", Some("/some/repo/.git")),
            ("GIT_WORK_TREE", Some("/some/repo")),
            ("GIT_INDEX_FILE", Some("/some/repo/.git/index")),
        ],
        || {
            let input = Input {
                transcript: vec![Entry {
                    entry_type: EntryType::User,
                    content: "Hello".to_string(),
                    tool_name: String::new(),
                    tool_detail: String::new(),
                }],
                files_touched: vec![],
            };

            let result = generator.generate(input);
            assert!(result.is_ok(), "unexpected error: {result:?}");

            let maybe_invocation = captured_invocation
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            assert!(maybe_invocation.is_some(), "command was not captured");
            let invocation = maybe_invocation.expect("invocation");

            assert_eq!(
                invocation.dir,
                std::env::temp_dir().to_string_lossy().to_string(),
                "command dir should be temp dir"
            );

            for env in invocation.env {
                assert!(
                    !env.starts_with("GIT_"),
                    "found GIT_* env var in subprocess: {env}"
                );
            }
        },
    );
}

#[test]
fn test_scoped_git_env_restores_after_panic() {
    let expected_git_dir = std::env::var_os("GIT_DIR");
    let expected_git_work_tree = std::env::var_os("GIT_WORK_TREE");
    let expected_git_index_file = std::env::var_os("GIT_INDEX_FILE");

    let result = std::panic::catch_unwind(|| {
        with_env_vars(
            &[
                ("GIT_DIR", Some("/tmp/panic/.git")),
                ("GIT_WORK_TREE", Some("/tmp/panic")),
                ("GIT_INDEX_FILE", Some("/tmp/panic/.git/index")),
            ],
            || {
                panic!("intentional panic to verify env restoration");
            },
        );
    });

    assert!(result.is_err(), "catch_unwind should capture test panic");
    assert_eq!(std::env::var_os("GIT_DIR"), expected_git_dir);
    assert_eq!(std::env::var_os("GIT_WORK_TREE"), expected_git_work_tree);
    assert_eq!(std::env::var_os("GIT_INDEX_FILE"), expected_git_index_file);
}

// CLI-753
#[test]
fn test_strip_git_env() {
    let env = vec![
        "HOME=/Users/test".to_string(),
        "GIT_DIR=/repo/.git".to_string(),
        "PATH=/usr/bin".to_string(),
        "GIT_WORK_TREE=/repo".to_string(),
        "GIT_INDEX_FILE=/repo/.git/index".to_string(),
        "SHELL=/bin/zsh".to_string(),
    ];

    let filtered = strip_git_env(&env);
    let expected = [
        "HOME=/Users/test".to_string(),
        "PATH=/usr/bin".to_string(),
        "SHELL=/bin/zsh".to_string(),
    ];

    assert_eq!(
        filtered.len(),
        expected.len(),
        "filtered env length mismatch"
    );
    for (idx, actual) in filtered.iter().enumerate() {
        assert_eq!(
            actual, &expected[idx],
            "filtered env mismatch at index {idx}"
        );
    }
}

// CLI-754
#[test]
fn test_claude_generator_command_not_found() {
    let generator = ClaudeGenerator {
        command_runner: Some(Arc::new(|_| {
            Err(anyhow::anyhow!("executable file not found"))
        })),
        ..Default::default()
    };
    let input = Input {
        transcript: vec![Entry {
            entry_type: EntryType::User,
            content: "Hello".to_string(),
            tool_name: String::new(),
            tool_detail: String::new(),
        }],
        files_touched: vec![],
    };

    let err = generator.generate(input).err();
    assert!(err.is_some(), "expected error when command not found");
    let msg = err.expect("err").to_string();
    assert!(
        msg.contains("not found") || msg.contains("executable file not found"),
        "expected not-found error, got: {msg}"
    );
}

// CLI-755
#[test]
fn test_claude_generator_non_zero_exit() {
    let generator = ClaudeGenerator {
        command_runner: Some(Arc::new(|_| {
            Err(anyhow::anyhow!("claude CLI failed (exit 1): error message"))
        })),
        ..Default::default()
    };
    let input = Input {
        transcript: vec![Entry {
            entry_type: EntryType::User,
            content: "Hello".to_string(),
            tool_name: String::new(),
            tool_detail: String::new(),
        }],
        files_touched: vec![],
    };

    let err = generator.generate(input).err();
    assert!(err.is_some(), "expected error on non-zero exit");
    assert!(
        err.expect("err").to_string().contains("exit 1"),
        "expected exit code in error"
    );
}

// CLI-756
#[test]
fn test_claude_generator_error_cases() {
    let tests = vec![
        (
            "invalid JSON response",
            "not valid json",
            "parse claude CLI response",
        ),
        (
            "invalid summary JSON",
            r#"{"result":"not a valid summary object"}"#,
            "parse summary JSON",
        ),
    ];

    for (name, cmd_output, expected_error) in tests {
        let generator = ClaudeGenerator {
            command_runner: Some(Arc::new(move |_| Ok(cmd_output.to_string()))),
            ..Default::default()
        };
        let input = Input {
            transcript: vec![Entry {
                entry_type: EntryType::User,
                content: "Hello".to_string(),
                tool_name: String::new(),
                tool_detail: String::new(),
            }],
            files_touched: vec![],
        };

        let err = generator.generate(input).err();
        assert!(err.is_some(), "{name}: expected error");
        assert!(
            err.expect("err").to_string().contains(expected_error),
            "{name}: expected error containing {expected_error:?}"
        );
    }
}

#[test]
fn test_claude_generator_valid_response() {
    let response = r#"{"result":"{\"intent\":\"User wanted to fix a bug\",\"outcome\":\"Bug was fixed successfully\",\"learnings\":{\"repo\":[\"The repo uses Cargo workspaces\"],\"code\":[{\"path\":\"main.rs\",\"line\":10,\"finding\":\"Entry point\"}],\"workflow\":[\"Run tests before committing\"]},\"friction\":[\"Slow CI pipeline\"],\"open_items\":[\"Add more tests\"]}"}"#;
    let generator = ClaudeGenerator {
        command_runner: Some(Arc::new(move |_| Ok(response.to_string()))),
        ..Default::default()
    };

    let input = Input {
        transcript: vec![Entry {
            entry_type: EntryType::User,
            content: "Fix the bug".to_string(),
            tool_name: String::new(),
            tool_detail: String::new(),
        }],
        files_touched: vec![],
    };

    let summary = generator.generate(input);
    assert!(summary.is_ok(), "unexpected error: {summary:?}");
    let summary = summary.expect("summary");

    assert_eq!(summary.intent, "User wanted to fix a bug");
    assert_eq!(summary.outcome, "Bug was fixed successfully");
    assert_eq!(
        summary.learnings.repo,
        vec!["The repo uses Cargo workspaces"]
    );
    assert_eq!(summary.learnings.code.len(), 1);
    assert_eq!(summary.learnings.code[0].path, "main.rs");
    assert_eq!(summary.friction, vec!["Slow CI pipeline"]);
    assert_eq!(summary.open_items, vec!["Add more tests"]);
}

// CLI-758
#[test]
fn test_claude_generator_markdown_code_block() {
    let response = r#"{"result":"```json\n{\"intent\":\"Test markdown extraction\",\"outcome\":\"Works\",\"learnings\":{\"repo\":[],\"code\":[],\"workflow\":[]},\"friction\":[],\"open_items\":[]}\n```"}"#;
    let generator = ClaudeGenerator {
        command_runner: Some(Arc::new(move |_| Ok(response.to_string()))),
        ..Default::default()
    };

    let input = Input {
        transcript: vec![Entry {
            entry_type: EntryType::User,
            content: "Test".to_string(),
            tool_name: String::new(),
            tool_detail: String::new(),
        }],
        files_touched: vec![],
    };

    let summary = generator.generate(input);
    assert!(summary.is_ok(), "unexpected error: {summary:?}");
    let summary = summary.expect("summary");
    assert_eq!(summary.intent, "Test markdown extraction");
}

// CLI-759
#[test]
fn test_build_summarization_prompt() {
    let transcript_text = "[User] Hello\n\n[Assistant] Hi";
    let prompt = build_summarization_prompt(transcript_text);

    assert!(
        prompt.contains("<transcript>"),
        "prompt should contain <transcript>"
    );
    assert!(
        prompt.contains(transcript_text),
        "prompt should contain transcript text"
    );
    assert!(
        prompt.contains("</transcript>"),
        "prompt should contain </transcript>"
    );
    assert!(
        prompt.contains(r#""intent""#),
        "prompt should contain schema"
    );
    assert!(
        prompt.contains("Return ONLY the JSON object"),
        "prompt should contain JSON-only instruction"
    );
}

// CLI-760
#[test]
fn test_extract_json_from_markdown() {
    let tests = vec![
        ("plain JSON", r#"{"key": "value"}"#, r#"{"key": "value"}"#),
        (
            "json code block",
            "```json\n{\"key\": \"value\"}\n```",
            r#"{"key": "value"}"#,
        ),
        (
            "plain code block",
            "```\n{\"key\": \"value\"}\n```",
            r#"{"key": "value"}"#,
        ),
        (
            "with whitespace",
            "  \n```json\n{\"key\": \"value\"}\n```  \n",
            r#"{"key": "value"}"#,
        ),
        (
            "unclosed block",
            "```json\n{\"key\": \"value\"}",
            r#"{"key": "value"}"#,
        ),
    ];

    for (name, input, expected) in tests {
        let result = extract_json_from_markdown(input);
        assert_eq!(result, expected, "{name}");
    }
}

// CLI-761
#[test]
fn test_build_condensed_transcript_user_prompts() {
    let lines = vec![user_line(
        "user-1",
        Value::String("Hello, please help me with this task".to_string()),
    )];

    let entries = build_condensed_transcript(&lines);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].entry_type, EntryType::User);
    assert_eq!(entries[0].content, "Hello, please help me with this task");
}

// CLI-762
#[test]
fn test_build_condensed_transcript_assistant_responses() {
    let lines = vec![assistant_line(
        "assistant-1",
        vec![ContentBlock {
            r#type: "text".to_string(),
            text: "I'll help you with that.".to_string(),
            name: String::new(),
            input: Value::Null,
        }],
    )];

    let entries = build_condensed_transcript(&lines);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].entry_type, EntryType::Assistant);
    assert_eq!(entries[0].content, "I'll help you with that.");
}

// CLI-763
#[test]
fn test_build_condensed_transcript_tool_calls() {
    let lines = vec![assistant_line(
        "assistant-1",
        vec![ContentBlock {
            r#type: "tool_use".to_string(),
            text: String::new(),
            name: "Read".to_string(),
            input: must_json(ToolInput {
                file_path: "/path/to/file.rs".to_string(),
                notebook_path: String::new(),
                description: String::new(),
                command: String::new(),
                pattern: String::new(),
                skill: String::new(),
                url: String::new(),
                prompt: String::new(),
            }),
        }],
    )];

    let entries = build_condensed_transcript(&lines);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].entry_type, EntryType::Tool);
    assert_eq!(entries[0].tool_name, "Read");
    assert_eq!(entries[0].tool_detail, "/path/to/file.rs");
}

// CLI-764
#[test]
fn test_build_condensed_transcript_tool_call_with_command() {
    let lines = vec![assistant_line(
        "assistant-1",
        vec![ContentBlock {
            r#type: "tool_use".to_string(),
            text: String::new(),
            name: "Bash".to_string(),
            input: must_json(ToolInput {
                command: "cargo test".to_string(),
                file_path: String::new(),
                notebook_path: String::new(),
                description: String::new(),
                pattern: String::new(),
                skill: String::new(),
                url: String::new(),
                prompt: String::new(),
            }),
        }],
    )];

    let entries = build_condensed_transcript(&lines);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].tool_detail, "cargo test");
}

// CLI-765
#[test]
fn test_build_condensed_transcript_skill_tool_minimal_detail() {
    let lines = vec![assistant_line(
        "assistant-1",
        vec![ContentBlock {
            r#type: "tool_use".to_string(),
            text: String::new(),
            name: "Skill".to_string(),
            input: must_json(ToolInput {
                skill: "superpowers:brainstorming".to_string(),
                file_path: String::new(),
                notebook_path: String::new(),
                description: String::new(),
                command: String::new(),
                pattern: String::new(),
                url: String::new(),
                prompt: String::new(),
            }),
        }],
    )];

    let entries = build_condensed_transcript(&lines);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].tool_name, "Skill");
    assert_eq!(entries[0].tool_detail, "superpowers:brainstorming");
}

// CLI-766
#[test]
fn test_build_condensed_transcript_web_fetch_minimal_detail() {
    let lines = vec![assistant_line(
        "assistant-1",
        vec![ContentBlock {
            r#type: "tool_use".to_string(),
            text: String::new(),
            name: "WebFetch".to_string(),
            input: must_json(ToolInput {
                url: "https://example.com/docs".to_string(),
                prompt: "Extract the API documentation".to_string(),
                file_path: String::new(),
                notebook_path: String::new(),
                description: String::new(),
                command: String::new(),
                pattern: String::new(),
                skill: String::new(),
            }),
        }],
    )];

    let entries = build_condensed_transcript(&lines);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].tool_detail, "https://example.com/docs");
}

// CLI-767
#[test]
fn test_build_condensed_transcript_skips_skill_content_injection() {
    let skill_content = "Base directory for this skill: /Users/alex/.claude/plugins/cache/superpowers/4.1.1/skills/brainstorming\n\n# Brainstorming Ideas Into Designs";

    let lines = vec![
        user_line(
            "user-1",
            Value::String("Invoke the superpowers:brainstorming skill".to_string()),
        ),
        assistant_line(
            "assistant-1",
            vec![ContentBlock {
                r#type: "tool_use".to_string(),
                text: String::new(),
                name: "Skill".to_string(),
                input: must_json(ToolInput {
                    skill: "superpowers:brainstorming".to_string(),
                    file_path: String::new(),
                    notebook_path: String::new(),
                    description: String::new(),
                    command: String::new(),
                    pattern: String::new(),
                    url: String::new(),
                    prompt: String::new(),
                }),
            }],
        ),
        user_line("user-2", Value::String(skill_content.to_string())),
        user_line(
            "user-3",
            Value::String("Now help me brainstorm a feature".to_string()),
        ),
    ];

    let entries = build_condensed_transcript(&lines);
    assert_eq!(entries.len(), 3, "skill content should be filtered out");
    for entry in &entries {
        if entry.entry_type == EntryType::User {
            assert!(
                !entry.content.contains("Base directory for this skill"),
                "skill content injection should be filtered"
            );
        }
    }
    assert_eq!(
        entries[0].content,
        "Invoke the superpowers:brainstorming skill"
    );
    assert_eq!(entries[2].content, "Now help me brainstorm a feature");
}

// CLI-768
#[test]
fn test_build_condensed_transcript_strip_ide_context_tags() {
    let lines = vec![user_line(
        "user-1",
        Value::String(
            "<ide_opened_file>some file content</ide_opened_file>Please review this code"
                .to_string(),
        ),
    )];

    let entries = build_condensed_transcript(&lines);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].content, "Please review this code");
}

// CLI-769
#[test]
fn test_build_condensed_transcript_strip_system_tags() {
    let lines = vec![user_line(
        "user-1",
        Value::String(
            "<system-reminder>internal instructions</system-reminder>User question here"
                .to_string(),
        ),
    )];

    let entries = build_condensed_transcript(&lines);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].content, "User question here");
}

// CLI-770
#[test]
fn test_build_condensed_transcript_mixed_content() {
    let lines = vec![
        user_line("user-1", Value::String("Create a new file".to_string())),
        assistant_line(
            "assistant-1",
            vec![
                ContentBlock {
                    r#type: "text".to_string(),
                    text: "I'll create that file for you.".to_string(),
                    name: String::new(),
                    input: Value::Null,
                },
                ContentBlock {
                    r#type: "tool_use".to_string(),
                    text: String::new(),
                    name: "Write".to_string(),
                    input: must_json(ToolInput {
                        file_path: "/path/to/new.rs".to_string(),
                        notebook_path: String::new(),
                        description: String::new(),
                        command: String::new(),
                        pattern: String::new(),
                        skill: String::new(),
                        url: String::new(),
                        prompt: String::new(),
                    }),
                },
            ],
        ),
    ];

    let entries = build_condensed_transcript(&lines);
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].entry_type, EntryType::User);
    assert_eq!(entries[1].entry_type, EntryType::Assistant);
    assert_eq!(entries[2].entry_type, EntryType::Tool);
}

// CLI-771
#[test]
fn test_build_condensed_transcript_empty_transcript() {
    let entries = build_condensed_transcript(&[]);
    assert_eq!(entries.len(), 0);
}

// CLI-772
#[test]
fn test_build_condensed_transcript_user_array_content() {
    let lines = vec![user_line(
        "user-1",
        json!([
            { "type": "text", "text": "First part" },
            { "type": "text", "text": "Second part" }
        ]),
    )];

    let entries = build_condensed_transcript(&lines);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].content, "First part\n\nSecond part");
}

// CLI-773
#[test]
fn test_build_condensed_transcript_skips_empty_content() {
    let lines = vec![
        user_line(
            "user-1",
            Value::String("<ide_opened_file>only tags</ide_opened_file>".to_string()),
        ),
        assistant_line(
            "assistant-1",
            vec![ContentBlock {
                r#type: "text".to_string(),
                text: String::new(),
                name: String::new(),
                input: Value::Null,
            }],
        ),
    ];

    let entries = build_condensed_transcript(&lines);
    assert_eq!(entries.len(), 0);
}

// CLI-774
#[test]
fn test_build_condensed_transcript_from_bytes_gemini_user_and_assistant() {
    let gemini_json = r#"{"messages":[{"type":"user","content":"Help me write a Rust function"},{"type":"gemini","content":"Sure, here is a function that does what you need."}]}"#;
    let entries = build_condensed_transcript_from_bytes(gemini_json.as_bytes(), AgentType::Gemini)
        .expect("unexpected error");

    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].entry_type, EntryType::User);
    assert_eq!(entries[0].content, "Help me write a Rust function");
    assert_eq!(entries[1].entry_type, EntryType::Assistant);
    assert_eq!(
        entries[1].content,
        "Sure, here is a function that does what you need."
    );
}

// CLI-775
#[test]
fn test_build_condensed_transcript_from_bytes_gemini_tool_calls() {
    let gemini_json = r#"{"messages":[{"type":"user","content":"Read the main.rs file"},{"type":"gemini","content":"Let me read that file.","toolCalls":[{"id":"tc-1","name":"read_file","args":{"file_path":"/src/main.rs"}},{"id":"tc-2","name":"run_command","args":{"command":"cargo build"}}]}]}"#;
    let entries = build_condensed_transcript_from_bytes(gemini_json.as_bytes(), AgentType::Gemini)
        .expect("unexpected error");

    assert_eq!(entries.len(), 4);
    assert_eq!(entries[2].entry_type, EntryType::Tool);
    assert_eq!(entries[2].tool_name, "read_file");
    assert_eq!(entries[2].tool_detail, "/src/main.rs");
    assert_eq!(entries[3].tool_name, "run_command");
    assert_eq!(entries[3].tool_detail, "cargo build");
}

// CLI-776
#[test]
fn test_build_condensed_transcript_from_bytes_gemini_tool_call_arg_shapes() {
    let gemini_json = r#"{"messages":[{"type":"gemini","toolCalls":[{"id":"tc-1","name":"write_file","args":{"path":"/tmp/out.txt","content":"hello"}},{"id":"tc-2","name":"search","args":{"pattern":"TODO","description":"Search for TODOs"}},{"id":"tc-3","name":"unknown_tool","args":{"foo":"bar"}}]}]}"#;
    let entries = build_condensed_transcript_from_bytes(gemini_json.as_bytes(), AgentType::Gemini)
        .expect("unexpected error");

    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].tool_detail, "/tmp/out.txt");
    assert_eq!(entries[1].tool_detail, "Search for TODOs");
    assert_eq!(entries[2].tool_detail, "");
}

// CLI-777
#[test]
fn test_build_condensed_transcript_from_bytes_gemini_skips_empty_content() {
    let gemini_json = r#"{"messages":[{"type":"user","content":""},{"type":"gemini","content":"","toolCalls":[{"id":"tc-1","name":"read_file","args":{"file_path":"main.rs"}}]},{"type":"user","content":"Thanks"}]}"#;
    let entries = build_condensed_transcript_from_bytes(gemini_json.as_bytes(), AgentType::Gemini)
        .expect("unexpected error");

    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].entry_type, EntryType::Tool);
    assert_eq!(entries[1].entry_type, EntryType::User);
    assert_eq!(entries[1].content, "Thanks");
}

// CLI-778
#[test]
fn test_build_condensed_transcript_from_bytes_gemini_empty_transcript() {
    let entries = build_condensed_transcript_from_bytes(br#"{"messages":[]}"#, AgentType::Gemini)
        .expect("unexpected error");
    assert_eq!(entries.len(), 0);
}

// CLI-779
#[test]
fn test_build_condensed_transcript_from_bytes_gemini_invalid_json() {
    let err = build_condensed_transcript_from_bytes(b"not json", AgentType::Gemini).err();
    assert!(err.is_some(), "expected error for invalid Gemini JSON");
}

// CLI-780
#[test]
fn test_format_condensed_transcript_basic_format() {
    let input = Input {
        transcript: vec![
            Entry {
                entry_type: EntryType::User,
                content: "Hello".to_string(),
                tool_name: String::new(),
                tool_detail: String::new(),
            },
            Entry {
                entry_type: EntryType::Assistant,
                content: "Hi there".to_string(),
                tool_name: String::new(),
                tool_detail: String::new(),
            },
            Entry {
                entry_type: EntryType::Tool,
                content: String::new(),
                tool_name: "Read".to_string(),
                tool_detail: "/file.rs".to_string(),
            },
        ],
        files_touched: vec![],
    };

    let result = format_condensed_transcript(input);
    let expected = "[User] Hello\n\n[Assistant] Hi there\n\n[Tool] Read: /file.rs\n";
    assert_eq!(result, expected);
}

// CLI-781
#[test]
fn test_format_condensed_transcript_with_files() {
    let input = Input {
        transcript: vec![Entry {
            entry_type: EntryType::User,
            content: "Create files".to_string(),
            tool_name: String::new(),
            tool_detail: String::new(),
        }],
        files_touched: vec!["file1.rs".to_string(), "file2.rs".to_string()],
    };

    let result = format_condensed_transcript(input);
    let expected = "[User] Create files\n\n[Files Modified]\n- file1.rs\n- file2.rs\n";
    assert_eq!(result, expected);
}

// CLI-782
#[test]
fn test_format_condensed_transcript_tool_without_detail() {
    let input = Input {
        transcript: vec![Entry {
            entry_type: EntryType::Tool,
            content: String::new(),
            tool_name: "TaskList".to_string(),
            tool_detail: String::new(),
        }],
        files_touched: vec![],
    };

    let result = format_condensed_transcript(input);
    assert_eq!(result, "[Tool] TaskList\n");
}

// CLI-783
#[test]
fn test_format_condensed_transcript_empty_input() {
    let result = format_condensed_transcript(Input::default());
    assert_eq!(result, "");
}

// CLI-784
#[test]
fn test_generate_from_transcript() {
    let response = r#"{"result":"{\"intent\":\"Test intent\",\"outcome\":\"Test outcome\",\"learnings\":{\"repo\":[],\"code\":[],\"workflow\":[]},\"friction\":[],\"open_items\":[]}"}"#;
    let mock_generator = ClaudeGenerator {
        command_runner: Some(Arc::new(move |_| Ok(response.to_string()))),
        ..Default::default()
    };

    let transcript = br#"{"type":"user","message":{"content":"Hello"}}
{"type":"assistant","message":{"content":[{"type":"text","text":"Hi there"}]}}"#;

    let summary = generate_from_transcript(
        transcript,
        &["file.rs".to_string()],
        AgentType::ClaudeCode,
        Some(&mock_generator),
    );
    assert!(summary.is_ok(), "unexpected error: {summary:?}");
    let summary = summary.expect("summary");
    assert_eq!(summary.intent, "Test intent");
}

// CLI-785
#[test]
fn test_generate_from_transcript_empty_transcript() {
    let mock_generator = ClaudeGenerator::default();
    let summary = generate_from_transcript(&[], &[], AgentType::ClaudeCode, Some(&mock_generator));
    assert!(summary.is_err(), "expected error for empty transcript");
}

// CLI-786
#[test]
fn test_generate_from_transcript_nil_generator() {
    let transcript = br#"{"type":"user","message":{"content":"Hello"}}"#;
    let result = generate_from_transcript(transcript, &[], AgentType::ClaudeCode, None);
    if result.is_ok() {
        // This may unexpectedly succeed if claude is available.
        eprintln!("unexpected success - claude CLI may be available");
    }
}

// CLI-787
#[test]
fn test_build_condensed_transcript_from_bytes_open_code_user_and_assistant() {
    let jsonl = "{\"id\":\"msg-1\",\"role\":\"user\",\"content\":\"Fix the bug in main.rs\",\"time\":{\"created\":1708300000}}\n{\"id\":\"msg-2\",\"role\":\"assistant\",\"content\":\"I'll fix the bug.\",\"time\":{\"created\":1708300001}}\n";
    let entries = build_condensed_transcript_from_bytes(jsonl.as_bytes(), AgentType::OpenCode)
        .expect("unexpected error");

    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].entry_type, EntryType::User);
    assert_eq!(entries[0].content, "Fix the bug in main.rs");
    assert_eq!(entries[1].entry_type, EntryType::Assistant);
    assert_eq!(entries[1].content, "I'll fix the bug.");
}

// CLI-788
#[test]
fn test_build_condensed_transcript_from_bytes_open_code_tool_calls() {
    let jsonl = "{\"id\":\"msg-1\",\"role\":\"user\",\"content\":\"Edit main.rs\",\"time\":{\"created\":1708300000}}\n{\"id\":\"msg-2\",\"role\":\"assistant\",\"content\":\"Editing now.\",\"time\":{\"created\":1708300001},\"parts\":[{\"type\":\"text\",\"text\":\"Editing now.\"},{\"type\":\"tool\",\"tool\":\"edit\",\"callID\":\"call-1\",\"state\":{\"status\":\"completed\",\"input\":{\"file_path\":\"main.rs\"},\"output\":\"Applied\"}},{\"type\":\"tool\",\"tool\":\"bash\",\"callID\":\"call-2\",\"state\":{\"status\":\"completed\",\"input\":{\"command\":\"cargo test\"},\"output\":\"PASS\"}}]}\n";
    let entries = build_condensed_transcript_from_bytes(jsonl.as_bytes(), AgentType::OpenCode)
        .expect("unexpected error");

    assert_eq!(entries.len(), 4);
    assert_eq!(entries[2].entry_type, EntryType::Tool);
    assert_eq!(entries[2].tool_name, "edit");
    assert_eq!(entries[2].tool_detail, "main.rs");
    assert_eq!(entries[3].tool_name, "bash");
    assert_eq!(entries[3].tool_detail, "cargo test");
}

// CLI-789
#[test]
fn test_build_condensed_transcript_from_bytes_open_code_skips_empty_content() {
    let jsonl = "{\"id\":\"msg-1\",\"role\":\"user\",\"content\":\"\",\"time\":{\"created\":1708300000}}\n{\"id\":\"msg-2\",\"role\":\"assistant\",\"content\":\"\",\"time\":{\"created\":1708300001}}\n{\"id\":\"msg-3\",\"role\":\"user\",\"content\":\"Real prompt\",\"time\":{\"created\":1708300010}}\n";
    let entries = build_condensed_transcript_from_bytes(jsonl.as_bytes(), AgentType::OpenCode)
        .expect("unexpected error");

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].content, "Real prompt");
}

// CLI-790
#[test]
fn test_build_condensed_transcript_from_bytes_open_code_invalid_jsonl() {
    let entries = build_condensed_transcript_from_bytes(b"not json\n", AgentType::OpenCode)
        .expect("unexpected error");
    assert_eq!(entries.len(), 0);
}

#[test]
fn test_build_condensed_transcript_from_bytes_cursor_user_and_assistant() {
    let jsonl = r#"{"role":"user","content":"<user_query>Fix bug</user_query>"}{"role":"assistant","content":"Done"}"#;
    let jsonl = jsonl.replace("}{", "}\n{");
    let entries = build_condensed_transcript_from_bytes(jsonl.as_bytes(), AgentType::Cursor)
        .expect("unexpected error");
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].entry_type, EntryType::User);
    assert_eq!(entries[1].entry_type, EntryType::Assistant);
}

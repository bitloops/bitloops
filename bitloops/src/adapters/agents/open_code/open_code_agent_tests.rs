use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use crate::adapters::agents::open_code::hooks::BITLOOPS_MARKER;
use crate::adapters::agents::open_code::transcript::{extract_modified_files, parse_messages};
use crate::adapters::agents::{
    AGENT_NAME_OPEN_CODE, AGENT_TYPE_OPEN_CODE, Agent, AgentSession, HookInput, HookSupport,
    HookType,
};
use crate::test_support::process_state::{with_cwd, with_env_var};

use super::*;

const TEST_TRANSCRIPT_JSONL: &str = r#"{"id":"msg-1","role":"user","content":"Fix the bug in main.rs","time":{"created":1708300000}}
{"id":"msg-2","role":"assistant","content":"I'll fix the bug.","time":{"created":1708300001,"completed":1708300005},"tokens":{"input":150,"output":80,"reasoning":10,"cache":{"read":5,"write":15}},"cost":0.003,"parts":[{"type":"text","text":"I'll fix the bug."},{"type":"tool","tool":"edit","callID":"call-1","state":{"status":"completed","input":{"file_path":"main.rs"},"output":"Applied edit"}}]}
{"id":"msg-3","role":"user","content":"Also fix util.rs","time":{"created":1708300010}}
{"id":"msg-4","role":"assistant","content":"Done fixing util.rs.","time":{"created":1708300011,"completed":1708300015},"tokens":{"input":200,"output":100,"reasoning":5,"cache":{"read":10,"write":20}},"cost":0.005,"parts":[{"type":"tool","tool":"write","callID":"call-2","state":{"status":"completed","input":{"file_path":"util.rs"},"output":"File written"}},{"type":"text","text":"Done fixing util.rs."}]}
"#;

fn write_test_transcript(content: &str) -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("failed to create temp dir");
    let path = dir.path().join("test-session.jsonl");
    fs::write(&path, content).expect("failed to write test transcript");
    (dir, path)
}

#[test]
#[allow(non_snake_case)]
fn TestMetadataPresenceAndSessionPathHelpers() {
    let agent = new_open_code_agent();
    assert_eq!(agent.name(), AGENT_NAME_OPEN_CODE);
    assert_eq!(agent.agent_type(), AGENT_TYPE_OPEN_CODE);
    assert_eq!(
        agent.description(),
        "OpenCode - AI-powered terminal coding agent"
    );
    assert!(agent.is_preview());

    let direct = OpenCodeAgent;
    assert_eq!(direct.protected_dirs(), vec![".opencode".to_string()]);
    assert_eq!(
        direct.hook_names(),
        vec![
            HOOK_NAME_SESSION_START.to_string(),
            HOOK_NAME_SESSION_END.to_string(),
            HOOK_NAME_TURN_START.to_string(),
            HOOK_NAME_TURN_END.to_string(),
            HOOK_NAME_COMPACTION.to_string(),
        ]
    );
    assert_eq!(
        direct.get_supported_hooks(),
        vec![
            HookType::SessionStart,
            HookType::SessionEnd,
            HookType::UserPromptSubmit,
            HookType::Stop,
        ]
    );
    assert_eq!(
        direct.get_session_id(&HookInput {
            session_id: "session-123".to_string(),
            ..HookInput::default()
        }),
        "session-123"
    );
    assert_eq!(direct.format_resume_command(""), "opencode");
    assert_eq!(
        direct.format_resume_command("session-123"),
        "opencode -s session-123"
    );

    with_env_var(
        "BITLOOPS_TEST_OPENCODE_PROJECT_DIR",
        Some("/tmp/opencode-override"),
        || {
            let session_dir = direct
                .get_session_dir("/repo/path")
                .expect("override path should succeed");
            assert_eq!(session_dir, "/tmp/opencode-override");
        },
    );

    let session_dir = direct
        .get_session_dir("/tmp/My Repo/nested")
        .expect("session dir should succeed");
    assert!(session_dir.contains("bitloops-opencode"));
    assert!(session_dir.ends_with("tmp-My-Repo-nested"));
    assert_eq!(
        direct.resolve_session_file("/tmp/opencode-sessions", "abc123"),
        "/tmp/opencode-sessions/abc123.jsonl"
    );
    assert_eq!(
        sanitize_path_for_opencode("/tmp/My Repo/nested"),
        "-tmp-My-Repo-nested"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestDetectPresenceAndPluginHelpers() {
    let dir = tempfile::tempdir().expect("failed to create temp dir");
    with_cwd(dir.path(), || {
        let agent = OpenCodeAgent;
        assert!(
            !agent
                .detect_presence()
                .expect("detect presence should work"),
            "fresh repo should not look like OpenCode"
        );

        fs::write(dir.path().join("opencode.json"), "{}").expect("write opencode.json");
        assert!(
            agent
                .detect_presence()
                .expect("detect presence should work"),
            "opencode.json should be enough to detect presence"
        );

        let plugin_path = agent.plugin_path().expect("plugin path should resolve");
        assert!(
            plugin_path.ends_with(
                Path::new(".opencode")
                    .join("plugins")
                    .join("bitloops.ts")
                    .as_path()
            ),
            "unexpected plugin path: {}",
            plugin_path.display()
        );

        let rendered = agent
            .render_plugin(false)
            .expect("rendered production plugin should succeed");
        assert!(rendered.contains(BITLOOPS_MARKER));
        assert!(OpenCodeAgent::ensure_plugin_marker(&rendered).is_ok());
        let err = OpenCodeAgent::ensure_plugin_marker("export default {}")
            .expect_err("missing marker should fail");
        assert!(err.to_string().contains("does not contain Bitloops marker"));
    });
}

#[test]
#[allow(non_snake_case)]
fn TestParseHookEventAndTranscriptReads() {
    let agent = OpenCodeAgent;
    let supported = [
        (
            HOOK_NAME_SESSION_START,
            r#"{"session_id":"s1","transcript_path":"/tmp/t1.jsonl"}"#,
        ),
        (
            HOOK_NAME_TURN_START,
            r#"{"session_id":"s2","transcript_path":"/tmp/t2.jsonl","prompt":"hello"}"#,
        ),
        (
            HOOK_NAME_TURN_END,
            r#"{"session_id":"s3","transcript_path":"/tmp/t3.jsonl"}"#,
        ),
        (
            HOOK_NAME_SESSION_END,
            r#"{"session_id":"s4","transcript_path":"/tmp/t4.jsonl"}"#,
        ),
        (
            HOOK_NAME_COMPACTION,
            r#"{"session_id":"s5","transcript_path":"/tmp/t5.jsonl"}"#,
        ),
    ];

    for (hook_name, payload) in supported {
        let mut stdin = std::io::Cursor::new(payload);
        let event = agent
            .parse_hook_event(hook_name, &mut stdin)
            .expect("supported payload should parse");
        assert!(event.is_some(), "expected event for {hook_name}");
    }

    let mut stdin = std::io::Cursor::new(r#"{"session_id":"s6","transcript_path":"/tmp/t"}"#);
    let event = agent
        .parse_hook_event("unknown-hook", &mut stdin)
        .expect("unknown hook should be ignored");
    assert!(event.is_none());

    let mut bad_stdin = std::io::Cursor::new("");
    let err = agent
        .parse_hook_event(HOOK_NAME_SESSION_START, &mut bad_stdin)
        .expect_err("empty payload should fail");
    assert!(err.to_string().contains("empty hook input"));

    let (_dir, path) = write_test_transcript(TEST_TRANSCRIPT_JSONL);
    let transcript = agent
        .read_transcript(path.to_string_lossy().as_ref())
        .expect("read transcript should succeed");
    assert_eq!(transcript, TEST_TRANSCRIPT_JSONL.as_bytes());
}

#[test]
#[allow(non_snake_case)]
fn TestReadAndWriteSessionHelpers() {
    let (_dir, path) = write_test_transcript(TEST_TRANSCRIPT_JSONL);
    let agent = OpenCodeAgent;

    let session = agent
        .read_session(&HookInput {
            session_id: "session-123".to_string(),
            session_ref: path.to_string_lossy().to_string(),
            ..HookInput::default()
        })
        .expect("read session should succeed")
        .expect("read session should produce a session");
    assert_eq!(session.session_id, "session-123");
    assert_eq!(session.agent_name, AGENT_NAME_OPEN_CODE);
    assert_eq!(session.modified_files, vec!["main.rs", "util.rs"]);

    let out_dir = tempfile::tempdir().expect("failed to create output temp dir");
    let session_ref = out_dir.path().join("nested").join("session.jsonl");
    let writable = AgentSession {
        session_id: "session-456".to_string(),
        agent_name: AGENT_NAME_OPEN_CODE.to_string(),
        session_ref: session_ref.to_string_lossy().to_string(),
        native_data: TEST_TRANSCRIPT_JSONL.as_bytes().to_vec(),
        ..AgentSession::default()
    };
    agent
        .write_session(&writable)
        .expect("write session should succeed");
    assert_eq!(
        fs::read_to_string(&session_ref).expect("read written transcript"),
        TEST_TRANSCRIPT_JSONL
    );
}

#[test]
#[allow(non_snake_case)]
fn TestReadAndWriteSessionErrorsAndMissingTranscriptHelpers() {
    let agent = OpenCodeAgent;

    let read_err = agent
        .read_session(&HookInput::default())
        .expect_err("missing session ref should fail");
    assert!(read_err.to_string().contains("no session ref provided"));

    let write_err = agent
        .write_session(&AgentSession {
            agent_name: AGENT_NAME_OPEN_CODE.to_string(),
            native_data: b"{}".to_vec(),
            ..AgentSession::default()
        })
        .expect_err("missing session ref should fail");
    assert!(write_err.to_string().contains("no session ref to write to"));

    let write_err = agent
        .write_session(&AgentSession {
            session_ref: "/tmp/opencode-session.jsonl".to_string(),
            ..AgentSession::default()
        })
        .expect_err("missing data should fail");
    assert!(write_err.to_string().contains("no session data to write"));

    let (files, pos) = agent
        .extract_modified_files_from_offset("/nonexistent/path.jsonl", 0)
        .expect("missing transcript should be tolerated");
    assert!(files.is_empty());
    assert_eq!(pos, 0);

    let prompts = agent
        .extract_prompts("/nonexistent/path.jsonl", 0)
        .expect("missing transcript should return empty prompts");
    assert!(prompts.is_empty());

    let summary = agent
        .extract_summary("/nonexistent/path.jsonl")
        .expect("missing transcript should return empty summary");
    assert!(summary.is_empty());
}

#[test]
#[allow(non_snake_case)]
fn TestImportSessionIntoOpencodeRejectsEmptyInputs() {
    let agent = OpenCodeAgent;

    let err = agent
        .import_session_into_opencode("", b"{}")
        .expect_err("empty session id should fail");
    assert!(err.to_string().contains("session id is required"));

    let err = agent
        .import_session_into_opencode("session-123", b"")
        .expect_err("empty export data should fail");
    assert!(err.to_string().contains("export data is required"));
}

#[test]
#[allow(non_snake_case)]
fn TestInstallHooks_FreshInstall() {
    let dir = tempfile::tempdir().expect("failed to create temp dir");
    with_cwd(dir.path(), || {
        let agent = OpenCodeAgent;

        let count = agent.install_hooks(false, false).expect("unexpected error");
        assert_eq!(count, 1, "expected one hook install");

        let plugin_path = dir
            .path()
            .join(".opencode")
            .join("plugins")
            .join("bitloops.ts");
        let content = fs::read_to_string(&plugin_path).expect("plugin file not created");
        assert!(
            content.contains(r#"const BITLOOPS_CMD = "bitloops""#),
            "plugin file does not contain production command constant"
        );
        assert!(
            content.contains("hooks opencode"),
            "plugin file does not contain 'hooks opencode'"
        );
        assert!(
            content.contains("BitloopsPlugin"),
            "plugin file does not contain BitloopsPlugin export"
        );
        assert!(
            !content.contains("cargo run --"),
            "plugin file should not contain cargo run in production mode"
        );
    });
}

#[test]
#[allow(non_snake_case)]
fn TestInstallHooks_Idempotent() {
    let dir = tempfile::tempdir().expect("failed to create temp dir");
    with_cwd(dir.path(), || {
        let agent = OpenCodeAgent;

        let count1 = agent
            .install_hooks(false, false)
            .expect("first install failed");
        assert_eq!(count1, 1, "first install should create one hook");

        let count2 = agent
            .install_hooks(false, false)
            .expect("second install failed");
        assert_eq!(count2, 0, "second install should be idempotent");
    });
}

#[test]
#[allow(non_snake_case)]
fn TestInstallHooks_LocalDev() {
    let dir = tempfile::tempdir().expect("failed to create temp dir");
    with_cwd(dir.path(), || {
        let agent = OpenCodeAgent;

        let count = agent.install_hooks(true, false).expect("unexpected error");
        assert_eq!(count, 1, "expected one hook install");

        let plugin_path = dir
            .path()
            .join(".opencode")
            .join("plugins")
            .join("bitloops.ts");
        let content = fs::read_to_string(&plugin_path).expect("plugin file not created");
        assert!(
            content.contains("cargo run --"),
            "local dev mode plugin should contain cargo run command"
        );
    });
}

#[test]
#[allow(non_snake_case)]
fn TestInstallHooks_ForceReinstall() {
    let dir = tempfile::tempdir().expect("failed to create temp dir");
    with_cwd(dir.path(), || {
        let agent = OpenCodeAgent;

        agent
            .install_hooks(false, false)
            .expect("first install failed");

        let count = agent
            .install_hooks(false, true)
            .expect("force reinstall failed");
        assert_eq!(count, 1, "force reinstall should write plugin again");
    });
}

#[test]
#[allow(non_snake_case)]
fn TestUninstallHooks() {
    let dir = tempfile::tempdir().expect("failed to create temp dir");
    with_cwd(dir.path(), || {
        let agent = OpenCodeAgent;

        agent
            .install_hooks(false, false)
            .expect("install should succeed before uninstall");
        agent
            .uninstall_hooks()
            .expect("uninstall should remove plugin");

        let plugin_path = dir
            .path()
            .join(".opencode")
            .join("plugins")
            .join("bitloops.ts");
        assert!(
            !plugin_path.exists(),
            "plugin file should not exist after uninstall"
        );
    });
}

#[test]
#[allow(non_snake_case)]
fn TestUninstallHooks_NoFile() {
    let dir = tempfile::tempdir().expect("failed to create temp dir");
    with_cwd(dir.path(), || {
        let agent = OpenCodeAgent;

        agent
            .uninstall_hooks()
            .expect("uninstall should be non-fatal when plugin does not exist");
    });
}

#[test]
#[allow(non_snake_case)]
fn TestAreHooksInstalled() {
    let dir = tempfile::tempdir().expect("failed to create temp dir");
    with_cwd(dir.path(), || {
        let agent = OpenCodeAgent;

        assert!(
            !agent.are_hooks_installed(),
            "hooks should not be installed initially"
        );

        agent
            .install_hooks(false, false)
            .expect("install hooks should succeed");
        assert!(
            agent.are_hooks_installed(),
            "hooks should be installed after InstallHooks"
        );

        agent
            .uninstall_hooks()
            .expect("uninstall hooks should succeed");
        assert!(
            !agent.are_hooks_installed(),
            "hooks should not be installed after UninstallHooks"
        );
    });
}

#[test]
#[allow(non_snake_case)]
fn TestParseMessages() {
    let messages = parse_messages(TEST_TRANSCRIPT_JSONL.as_bytes()).expect("unexpected error");
    assert_eq!(messages.len(), 4, "expected 4 messages");
    assert_eq!(messages[0].id, "msg-1", "first message id mismatch");
    assert_eq!(messages[0].role, "user", "first message role mismatch");
}

#[test]
#[allow(non_snake_case)]
fn TestParseMessages_Empty() {
    let messages = parse_messages(b"").expect("unexpected error for empty transcript");
    assert!(
        messages.is_empty(),
        "expected no messages for empty transcript"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestParseMessages_InvalidLines() {
    let data = b"not json\n{\"id\":\"msg-1\",\"role\":\"user\",\"content\":\"hello\"}\n";
    let messages = parse_messages(data).expect("unexpected error for invalid-line transcript");
    assert_eq!(messages.len(), 1, "expected one valid message");
    assert_eq!(
        messages[0].content, "hello",
        "expected valid line content to be parsed"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestGetTranscriptPosition() {
    let (_dir, path) = write_test_transcript(TEST_TRANSCRIPT_JSONL);
    let agent = OpenCodeAgent;

    let pos = agent
        .get_transcript_position(path.to_string_lossy().as_ref())
        .expect("unexpected error");
    assert_eq!(pos, 4, "expected 4-message position");
}

#[test]
#[allow(non_snake_case)]
fn TestGetTranscriptPosition_NonexistentFile() {
    let agent = OpenCodeAgent;
    let pos = agent
        .get_transcript_position("/nonexistent/path.jsonl")
        .expect("unexpected error for nonexistent transcript");
    assert_eq!(pos, 0, "expected 0 for nonexistent transcript");
}

#[test]
#[allow(non_snake_case)]
fn TestExtractModifiedFilesFromOffset() {
    let (_dir, path) = write_test_transcript(TEST_TRANSCRIPT_JSONL);
    let agent = OpenCodeAgent;

    let (files, pos) = agent
        .extract_modified_files_from_offset(path.to_string_lossy().as_ref(), 0)
        .expect("unexpected error");
    assert_eq!(pos, 4, "expected position 4");
    assert_eq!(files.len(), 2, "expected two modified files at offset 0");

    let (files, pos) = agent
        .extract_modified_files_from_offset(path.to_string_lossy().as_ref(), 2)
        .expect("unexpected error");
    assert_eq!(pos, 4, "expected position 4");
    assert_eq!(files.len(), 1, "expected one modified file at offset 2");
    assert_eq!(files[0], "util.rs", "expected util.rs from offset 2");
}

#[test]
#[allow(non_snake_case)]
fn TestExtractPrompts() {
    let (_dir, path) = write_test_transcript(TEST_TRANSCRIPT_JSONL);
    let agent = OpenCodeAgent;

    let prompts = agent
        .extract_prompts(path.to_string_lossy().as_ref(), 0)
        .expect("unexpected error");
    assert_eq!(prompts.len(), 2, "expected two prompts from offset 0");
    assert_eq!(prompts[0], "Fix the bug in main.rs");

    let prompts = agent
        .extract_prompts(path.to_string_lossy().as_ref(), 2)
        .expect("unexpected error");
    assert_eq!(prompts.len(), 1, "expected one prompt from offset 2");
    assert_eq!(prompts[0], "Also fix util.rs");
}

#[test]
#[allow(non_snake_case)]
fn TestExtractSummary() {
    let (_dir, path) = write_test_transcript(TEST_TRANSCRIPT_JSONL);
    let agent = OpenCodeAgent;

    let summary = agent
        .extract_summary(path.to_string_lossy().as_ref())
        .expect("unexpected error");
    assert_eq!(summary, "Done fixing util.rs.");
}

#[test]
#[allow(non_snake_case)]
fn TestExtractSummary_EmptyTranscript() {
    let (_dir, path) = write_test_transcript("");
    let agent = OpenCodeAgent;

    let summary = agent
        .extract_summary(path.to_string_lossy().as_ref())
        .expect("unexpected error");
    assert_eq!(summary, "", "expected empty summary");
}

#[test]
#[allow(non_snake_case)]
fn TestCalculateTokenUsage() {
    let (_dir, path) = write_test_transcript(TEST_TRANSCRIPT_JSONL);
    let agent = OpenCodeAgent;

    let usage = agent
        .calculate_token_usage(path.to_string_lossy().as_ref(), 0)
        .expect("unexpected error")
        .expect("expected non-nil token usage");
    assert_eq!(usage.input_tokens, 350, "input token count mismatch");
    assert_eq!(usage.output_tokens, 180, "output token count mismatch");
    assert_eq!(usage.cache_read_tokens, 15, "cache read token mismatch");
    assert_eq!(
        usage.cache_creation_tokens, 35,
        "cache creation token mismatch"
    );
    assert_eq!(usage.api_call_count, 2, "api call count mismatch");
}

#[test]
#[allow(non_snake_case)]
fn TestCalculateTokenUsage_FromOffset() {
    let (_dir, path) = write_test_transcript(TEST_TRANSCRIPT_JSONL);
    let agent = OpenCodeAgent;

    let usage = agent
        .calculate_token_usage(path.to_string_lossy().as_ref(), 2)
        .expect("unexpected error")
        .expect("expected non-nil token usage");
    assert_eq!(usage.input_tokens, 200);
    assert_eq!(usage.output_tokens, 100);
    assert_eq!(usage.api_call_count, 1);
}

#[test]
#[allow(non_snake_case)]
fn TestCalculateTokenUsage_NonexistentFile() {
    let agent = OpenCodeAgent;

    let usage = agent
        .calculate_token_usage("/nonexistent/path.jsonl", 0)
        .expect("unexpected error for nonexistent transcript");
    assert!(usage.is_none(), "expected nil usage for missing transcript");
}

#[test]
#[allow(non_snake_case)]
fn TestChunkTranscript_SmallContent() {
    let agent = OpenCodeAgent;
    let content = TEST_TRANSCRIPT_JSONL.as_bytes();

    let chunks = agent
        .chunk_transcript(content, content.len() + 1000)
        .expect("unexpected chunking error");
    assert_eq!(chunks.len(), 1, "small content should remain single chunk");
    assert_eq!(chunks[0], content, "single chunk should match original");
}

#[test]
#[allow(non_snake_case)]
fn TestChunkTranscript_SplitsLargeContent() {
    let agent = OpenCodeAgent;
    let content = TEST_TRANSCRIPT_JSONL.as_bytes();

    let chunks = agent
        .chunk_transcript(content, 500)
        .expect("unexpected chunking error");
    assert!(chunks.len() >= 2, "large content should split into chunks");

    for (idx, chunk) in chunks.iter().enumerate() {
        let messages = parse_messages(chunk)
            .unwrap_or_else(|err| panic!("chunk {idx} should parse as JSONL: {err}"));
        assert!(
            !messages.is_empty(),
            "chunk {idx} should contain at least one message"
        );
    }
}

#[test]
#[allow(non_snake_case)]
fn TestChunkTranscript_RoundTrip() {
    let agent = OpenCodeAgent;
    let content = TEST_TRANSCRIPT_JSONL.as_bytes();

    let chunks = agent
        .chunk_transcript(content, 500)
        .expect("chunking should succeed");
    let reassembled = agent
        .reassemble_transcript(&chunks)
        .expect("reassembly should succeed");

    let original = parse_messages(content).expect("failed to parse original transcript");
    let result = parse_messages(&reassembled).expect("failed to parse reassembled transcript");

    assert_eq!(
        result.len(),
        original.len(),
        "message count must round-trip"
    );
    for (idx, msg) in result.iter().enumerate() {
        assert_eq!(
            msg.id, original[idx].id,
            "message ID mismatch at index {idx}"
        );
    }
}

#[test]
#[allow(non_snake_case)]
fn TestChunkTranscript_EmptyContent() {
    let agent = OpenCodeAgent;
    let chunks = agent
        .chunk_transcript(b"", 100)
        .expect("chunking empty content should succeed");
    assert_eq!(chunks.len(), 0, "expected zero chunks for empty content");
}

#[test]
#[allow(non_snake_case)]
fn TestReassembleTranscript_SingleChunk() {
    let agent = OpenCodeAgent;
    let content = TEST_TRANSCRIPT_JSONL.as_bytes().to_vec();
    let result = agent
        .reassemble_transcript(std::slice::from_ref(&content))
        .expect("single-chunk reassembly should succeed");
    assert_eq!(
        result, content,
        "single chunk reassembly should preserve original"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestReassembleTranscript_Empty() {
    let agent = OpenCodeAgent;
    let result = agent
        .reassemble_transcript(&[])
        .expect("empty reassembly should succeed");
    assert!(
        result.is_empty(),
        "expected empty output for empty reassembly input"
    );
}

#[test]
#[allow(non_snake_case)]
fn TestExtractModifiedFiles() {
    let files = extract_modified_files(TEST_TRANSCRIPT_JSONL.as_bytes()).expect("unexpected error");
    assert_eq!(files.len(), 2, "expected two modified files");
    assert_eq!(files[0], "main.rs", "first modified file mismatch");
    assert_eq!(files[1], "util.rs", "second modified file mismatch");
}

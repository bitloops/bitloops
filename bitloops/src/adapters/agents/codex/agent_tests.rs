use super::*;
use crate::adapters::agents::{Agent, TokenCalculator};
use crate::host::checkpoints::lifecycle::adapters::{
    CODEX_HOOK_POST_TOOL_USE, CODEX_HOOK_PRE_TOOL_USE, CODEX_HOOK_SESSION_START, CODEX_HOOK_STOP,
    CODEX_HOOK_USER_PROMPT_SUBMIT,
};
use crate::test_support::process_state::with_env_var;
use serde_json::json;

fn init_repo(path: &std::path::Path) {
    let output = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(path)
        .output()
        .expect("git init");
    assert!(output.status.success(), "git init should succeed");
}

#[test]
fn identity_and_preview() {
    let agent = CodexAgent;
    assert_eq!(agent.name(), AGENT_NAME_CODEX);
    assert_eq!(agent.agent_type(), AGENT_TYPE_CODEX);
    assert!(agent.is_preview());
    assert_eq!(agent.protected_dirs(), vec![".codex".to_string()]);
}

#[test]
fn hook_names_expose_full_codex_surface() {
    let agent = CodexAgent;
    assert_eq!(
        agent.hook_names(),
        vec![
            CODEX_HOOK_SESSION_START.to_string(),
            CODEX_HOOK_USER_PROMPT_SUBMIT.to_string(),
            CODEX_HOOK_PRE_TOOL_USE.to_string(),
            CODEX_HOOK_POST_TOOL_USE.to_string(),
            CODEX_HOOK_STOP.to_string(),
        ]
    );
}

#[test]
fn detect_presence_checks_dot_codex_directory() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_repo(dir.path());
    let agent = CodexAgent;
    assert!(!agent.detect_presence_at(dir.path()));
    std::fs::create_dir_all(dir.path().join(".codex")).expect("create .codex");
    assert!(agent.detect_presence_at(dir.path()));
}

#[test]
fn read_and_write_session_roundtrip() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("session.jsonl");

    let agent = CodexAgent;
    let session = AgentSession {
        session_id: "codex-session-1".to_string(),
        agent_name: AGENT_NAME_CODEX.to_string(),
        session_ref: path.to_string_lossy().to_string(),
        native_data: br#"{"role":"user","content":"hello"}"#.to_vec(),
        ..AgentSession::default()
    };
    agent.write_session(&session).expect("write");

    let input = HookInput {
        session_id: "codex-session-1".to_string(),
        session_ref: path.to_string_lossy().to_string(),
        ..HookInput::default()
    };
    let read = agent.read_session(&input).expect("read").expect("session");
    assert_eq!(read.native_data, session.native_data);
}

#[test]
fn path_based_hooks_api_manages_hooks_without_cwd() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_repo(dir.path());

    let installed = super::hooks::install_hooks_at(dir.path(), false, false).expect("install");
    assert_eq!(installed, 5);
    assert!(super::hooks::are_hooks_installed_at(dir.path()));

    super::hooks::uninstall_hooks_at(dir.path()).expect("uninstall");
    assert!(!super::hooks::are_hooks_installed_at(dir.path()));
}

#[test]
fn get_session_dir_uses_override() {
    let agent = CodexAgent;
    with_env_var(
        "BITLOOPS_TEST_CODEX_SESSION_DIR",
        Some("/tmp/codex-sessions"),
        || {
            let got = agent.get_session_dir("").expect("session dir");
            assert_eq!(got, "/tmp/codex-sessions");
        },
    );
}

#[test]
fn resolve_session_file_prefers_date_sharded_rollout() {
    let root = tempfile::tempdir().expect("tempdir");
    let sessions_dir = root.path().join("sessions");
    let day_dir = sessions_dir.join("2026").join("04").join("16");
    std::fs::create_dir_all(&day_dir).expect("create session dir");

    let session_id = "019d9664-2636-79c0-9658-f76bfb8af4b4";
    let transcript_path = day_dir.join(format!("rollout-2026-04-16T16-03-59-{session_id}.jsonl"));
    std::fs::write(&transcript_path, "{}\n").expect("write transcript");
    std::fs::write(
        root.path().join("session_index.jsonl"),
        format!(
            r#"{{"id":"{session_id}","thread_name":"Investigate checkpoint loss","updated_at":"2026-04-16T13:04:37.997446Z"}}"#
        ),
    )
    .expect("write session index");

    let agent = CodexAgent;
    let resolved = agent.resolve_session_file(sessions_dir.to_string_lossy().as_ref(), session_id);
    assert_eq!(resolved, transcript_path.to_string_lossy().to_string());
}

#[test]
fn calculate_token_usage_uses_token_count_delta_from_offset() {
    let dir = tempfile::tempdir().expect("tempdir");
    let transcript_path = dir.path().join("codex.jsonl");
    let transcript = [
        json!({
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": "first prompt"}],
            }
        })
        .to_string(),
        json!({
            "type": "event_msg",
            "payload": {
                "type": "token_count",
                "info": {
                    "total_token_usage": {
                        "input_tokens": 100,
                        "cached_input_tokens": 30,
                        "output_tokens": 20,
                        "reasoning_output_tokens": 5,
                        "total_tokens": 120
                    }
                }
            }
        })
        .to_string(),
        json!({
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "first answer"}],
            }
        })
        .to_string(),
        json!({
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": "second prompt"}],
            }
        })
        .to_string(),
        json!({
            "type": "event_msg",
            "payload": {
                "type": "token_count",
                "info": {
                    "total_token_usage": {
                        "input_tokens": 170,
                        "cached_input_tokens": 50,
                        "output_tokens": 35,
                        "reasoning_output_tokens": 9,
                        "total_tokens": 205
                    }
                }
            }
        })
        .to_string(),
        json!({
            "type": "event_msg",
            "payload": {
                "type": "token_count",
                "info": {
                    "total_token_usage": {
                        "input_tokens": 210,
                        "cached_input_tokens": 70,
                        "output_tokens": 45,
                        "reasoning_output_tokens": 11,
                        "total_tokens": 255
                    }
                }
            }
        })
        .to_string(),
    ]
    .join("\n");
    std::fs::write(&transcript_path, format!("{transcript}\n")).expect("write transcript");

    let agent = CodexAgent;
    let usage = agent
        .calculate_token_usage(transcript_path.to_string_lossy().as_ref(), 3)
        .expect("calculate token usage");

    assert_eq!(usage.input_tokens, 70);
    assert_eq!(usage.cache_read_tokens, 40);
    assert_eq!(usage.output_tokens, 25);
    assert_eq!(usage.api_call_count, 2);
}

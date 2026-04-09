use super::*;
use crate::adapters::agents::Agent;
use crate::host::checkpoints::lifecycle::adapters::{
    CODEX_HOOK_POST_TOOL_USE, CODEX_HOOK_PRE_TOOL_USE, CODEX_HOOK_SESSION_START, CODEX_HOOK_STOP,
    CODEX_HOOK_USER_PROMPT_SUBMIT,
};

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

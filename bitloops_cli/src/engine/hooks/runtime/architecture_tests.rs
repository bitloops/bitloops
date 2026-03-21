use std::fs;
use std::path::Path;

#[test]
fn cursor_hooks_cmd_does_not_import_claude_hooks_cmd() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let src = fs::read_to_string(root.join("src/adapters/agents/cursor/hooks_cmd.rs"))
        .expect("failed to read cursor hooks_cmd source");
    assert!(
        !src.contains("claude_code::hooks_cmd"),
        "cursor hooks_cmd must not import claude hooks_cmd"
    );
}

#[test]
fn claude_hooks_cmd_is_runtime_adapter_only() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let src = fs::read_to_string(root.join("src/adapters/agents/claude_code/hooks_cmd.rs"))
        .expect("failed to read claude hooks_cmd source");
    assert!(
        src.contains("hooks::runtime::agent_runtime"),
        "claude hooks_cmd should adapt from shared runtime module"
    );
}

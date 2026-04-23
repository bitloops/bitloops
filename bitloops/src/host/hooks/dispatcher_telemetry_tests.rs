use super::*;
use crate::host::checkpoints::lifecycle::adapters::{
    CLAUDE_HOOK_POST_TOOL_USE, CLAUDE_HOOK_PRE_TOOL_USE, CODEX_HOOK_POST_TOOL_USE,
    CODEX_HOOK_PRE_TOOL_USE, CODEX_HOOK_SESSION_START, CODEX_HOOK_STOP,
    CODEX_HOOK_USER_PROMPT_SUBMIT,
};
use serde_json::Value;

#[test]
fn hook_action_descriptor_uses_canonical_event_name() {
    let descriptor = hook_action_descriptor(AGENT_NAME_CODEX, "stop");
    assert_eq!(descriptor.event, "bitloops hook");
    assert_eq!(descriptor.surface, "hook");
}

#[test]
fn hook_action_descriptor_includes_only_safe_properties() {
    let descriptor = hook_action_descriptor(AGENT_NAME_CURSOR, "before-submit-prompt");
    assert_eq!(
        descriptor.properties.get("command"),
        Some(&Value::String("bitloops hook".to_string()))
    );
    assert_eq!(
        descriptor.properties.get("agent"),
        Some(&Value::String("cursor".to_string()))
    );
    assert_eq!(
        descriptor.properties.get("hook"),
        Some(&Value::String("before-submit-prompt".to_string()))
    );
    assert_eq!(
        descriptor.properties.get("hook_type"),
        Some(&Value::String("agent".to_string()))
    );
    assert_eq!(
        descriptor.properties.len(),
        4,
        "hook telemetry should not include payload/session data"
    );
}

#[test]
fn codex_hook_verb_names_cover_full_surface() {
    let expected = [
        (CodexHookVerb::SessionStart, CODEX_HOOK_SESSION_START),
        (
            CodexHookVerb::UserPromptSubmit,
            CODEX_HOOK_USER_PROMPT_SUBMIT,
        ),
        (CodexHookVerb::PreToolUse, CODEX_HOOK_PRE_TOOL_USE),
        (CodexHookVerb::PostToolUse, CODEX_HOOK_POST_TOOL_USE),
        (CodexHookVerb::Stop, CODEX_HOOK_STOP),
    ];

    for (verb, hook_name) in expected {
        assert_eq!(verb.hook_name(), hook_name);
    }
}

#[test]
fn codex_bash_hooks_are_classified_as_tool_hooks() {
    assert_eq!(get_hook_type(AGENT_NAME_CODEX, "pre-tool-use"), "tool");
    assert_eq!(get_hook_type(AGENT_NAME_CODEX, "post-tool-use"), "tool");
}

#[test]
fn claude_ordinary_tool_hooks_are_classified_as_tool_hooks() {
    assert_eq!(
        get_hook_type(AGENT_NAME_CLAUDE_CODE, CLAUDE_HOOK_PRE_TOOL_USE),
        "tool"
    );
    assert_eq!(
        get_hook_type(AGENT_NAME_CLAUDE_CODE, CLAUDE_HOOK_POST_TOOL_USE),
        "tool"
    );
}

use super::*;

#[test]
fn agent_type_from_str_maps_codex_to_codex() {
    assert_eq!(
        agent_type_from_str(crate::engine::agent::AGENT_TYPE_CODEX),
        AgentType::Codex
    );
}

#[test]
fn metadata_from_json_sets_codex_agent_type() {
    let meta = serde_json::json!({
        "session_id": "session-1",
        "created_at": "2026-03-12T00:00:00Z",
        "files_touched": ["src/main.rs"],
        "checkpoints_count": 1,
        "checkpoint_transcript_start": 0,
        "agent": crate::engine::agent::AGENT_TYPE_CODEX,
    });

    let parsed = metadata_from_json(&meta, "cp-1");

    assert_eq!(parsed.agent_type, AgentType::Codex);
}

#[test]
fn metadata_from_json_unknown_agent_defaults_to_claude() {
    let meta = serde_json::json!({
        "agent": "unknown-agent"
    });

    let parsed = metadata_from_json(&meta, "cp-2");

    assert_eq!(parsed.agent_type, AgentType::ClaudeCode);
}

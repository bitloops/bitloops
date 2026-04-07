use std::path::Path;

use super::super::*;

use crate::host::interactions::types::{InteractionSession, InteractionTurn};

pub(crate) type ClickHouseTestEnv = (String, Option<String>, Option<String>, Option<String>);

pub(crate) fn fake_interaction_session(
    repo_root: &Path,
    repo_id: &str,
    session_id: &str,
) -> InteractionSession {
    InteractionSession {
        session_id: session_id.to_string(),
        repo_id: repo_id.to_string(),
        agent_type: "codex".to_string(),
        model: "gpt-5.4".to_string(),
        first_prompt: "ship it".to_string(),
        transcript_path: repo_root
            .join("transcript.jsonl")
            .to_string_lossy()
            .to_string(),
        worktree_path: repo_root.to_string_lossy().to_string(),
        worktree_id: "main".to_string(),
        started_at: "2026-04-05T10:00:00Z".to_string(),
        last_event_at: "2026-04-05T10:00:01Z".to_string(),
        updated_at: "2026-04-05T10:00:01Z".to_string(),
        ..Default::default()
    }
}

pub(crate) fn fake_interaction_turn(
    repo_id: &str,
    session_id: &str,
    turn_id: &str,
    files: &[&str],
) -> InteractionTurn {
    InteractionTurn {
        turn_id: turn_id.to_string(),
        session_id: session_id.to_string(),
        repo_id: repo_id.to_string(),
        turn_number: 1,
        prompt: "make the change".to_string(),
        agent_type: "codex".to_string(),
        model: "gpt-5.4".to_string(),
        started_at: "2026-04-05T10:00:01Z".to_string(),
        ended_at: Some("2026-04-05T10:00:02Z".to_string()),
        token_usage: Some(TokenUsageMetadata {
            input_tokens: 10,
            output_tokens: 5,
            ..Default::default()
        }),
        summary: format!("summary for {turn_id}"),
        prompt_count: 1,
        transcript_offset_start: Some(0),
        transcript_offset_end: Some(1),
        transcript_fragment: format!(
            "{{\"type\":\"user\",\"content\":\"make the change {turn_id}\"}}\n{{\"type\":\"assistant\",\"content\":\"done {turn_id}\"}}\n"
        ),
        files_modified: files.iter().map(|file| file.to_string()).collect(),
        updated_at: "2026-04-05T10:00:02Z".to_string(),
        ..Default::default()
    }
}

pub(crate) fn clickhouse_test_env() -> Option<ClickHouseTestEnv> {
    let url = std::env::var("BITLOOPS_TEST_CLICKHOUSE_URL").ok()?;
    let user = std::env::var("BITLOOPS_TEST_CLICKHOUSE_USER").ok();
    let password = std::env::var("BITLOOPS_TEST_CLICKHOUSE_PASSWORD").ok();
    let database = std::env::var("BITLOOPS_TEST_CLICKHOUSE_DATABASE").ok();
    Some((url, user, password, database))
}

use std::collections::BTreeSet;

use super::*;
use crate::adapters::agents::TokenUsage;
use crate::host::checkpoints::session::state::SessionState;
use crate::host::interactions::types::{InteractionSession, InteractionTurn};

pub(crate) fn aggregate_turn_token_usage(turns: &[InteractionTurn]) -> Option<TokenUsageMetadata> {
    turns.iter().fold(None, |acc, turn| {
        aggregate_token_usage(acc, turn.token_usage.clone())
    })
}

pub(crate) fn aggregate_turn_files(turns: &[InteractionTurn]) -> Vec<String> {
    let mut files = BTreeSet::new();
    for turn in turns {
        for file in &turn.files_modified {
            if !file.trim().is_empty() {
                files.insert(file.clone());
            }
        }
    }
    files.into_iter().collect()
}

#[allow(dead_code)]
pub(crate) fn aggregate_turn_prompts(turns: &[InteractionTurn]) -> Vec<String> {
    turns
        .iter()
        .filter_map(|turn| {
            let trimmed = turn.prompt.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        })
        .collect()
}

pub(crate) fn turns_overlap_committed_files(
    turns: &[InteractionTurn],
    committed_files: &std::collections::HashSet<String>,
) -> bool {
    turns.iter().any(|turn| {
        turn.files_modified
            .iter()
            .any(|file| committed_files.contains(file))
    })
}

pub(crate) fn hydrate_session_state_from_turns(
    state: &mut SessionState,
    turns: &[InteractionTurn],
) {
    let files_touched = aggregate_turn_files(turns);
    state.files_touched = files_touched;
    state.step_count = turns.len().min(u32::MAX as usize) as u32;
    state.turn_id = turns
        .last()
        .map(|turn| turn.turn_id.clone())
        .unwrap_or_default();
    state.token_usage = aggregate_turn_token_usage(turns).map(runtime_token_usage_from_metadata);
}

pub(crate) fn runtime_token_usage_from_metadata(metadata: TokenUsageMetadata) -> TokenUsage {
    TokenUsage {
        input_tokens: metadata.input_tokens.min(i32::MAX as u64) as i32,
        cache_creation_tokens: metadata.cache_creation_tokens.min(i32::MAX as u64) as i32,
        cache_read_tokens: metadata.cache_read_tokens.min(i32::MAX as u64) as i32,
        output_tokens: metadata.output_tokens.min(i32::MAX as u64) as i32,
        api_call_count: metadata.api_call_count.min(i32::MAX as u64) as i32,
        subagent_tokens: metadata
            .subagent_tokens
            .map(|nested| Box::new(runtime_token_usage_from_metadata(*nested))),
    }
}

pub(crate) fn synthetic_session_state_from_interaction(
    session: &InteractionSession,
    turns: &[InteractionTurn],
) -> SessionState {
    let mut state = SessionState {
        session_id: session.session_id.clone(),
        worktree_path: session.worktree_path.clone(),
        worktree_id: session.worktree_id.clone(),
        started_at: session.started_at.clone(),
        ended_at: session.ended_at.clone(),
        transcript_path: session.transcript_path.clone(),
        first_prompt: session.first_prompt.clone(),
        agent_type: session.agent_type.clone(),
        last_interaction_time: (!session.last_event_at.trim().is_empty())
            .then(|| session.last_event_at.clone()),
        ..Default::default()
    };
    hydrate_session_state_from_turns(&mut state, turns);
    state
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_turn(
        turn_id: &str,
        prompt: &str,
        files: &[&str],
        input_tokens: u64,
        output_tokens: u64,
    ) -> InteractionTurn {
        InteractionTurn {
            turn_id: turn_id.to_string(),
            session_id: "sess-1".to_string(),
            repo_id: "repo-1".to_string(),
            turn_number: 1,
            prompt: prompt.to_string(),
            agent_type: "codex".to_string(),
            started_at: "2026-04-04T10:00:00Z".to_string(),
            token_usage: Some(TokenUsageMetadata {
                input_tokens,
                output_tokens,
                ..Default::default()
            }),
            files_modified: files.iter().map(|value| value.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn aggregate_turn_files_deduplicates_and_sorts() {
        let turns = vec![
            make_turn("t1", "one", &["b.rs", "a.rs"], 1, 1),
            make_turn("t2", "two", &["a.rs", "c.rs"], 1, 1),
        ];
        assert_eq!(aggregate_turn_files(&turns), vec!["a.rs", "b.rs", "c.rs"]);
    }

    #[test]
    fn aggregate_turn_token_usage_sums() {
        let turns = vec![
            make_turn("t1", "one", &["a.rs"], 10, 5),
            make_turn("t2", "two", &["b.rs"], 20, 7),
        ];
        let usage = aggregate_turn_token_usage(&turns).expect("usage");
        assert_eq!(usage.input_tokens, 30);
        assert_eq!(usage.output_tokens, 12);
    }

    #[test]
    fn hydrate_session_state_from_turns_updates_checkpoint_inputs() {
        let turns = vec![
            make_turn("t1", "one", &["a.rs"], 10, 5),
            make_turn("t2", "two", &["b.rs"], 20, 7),
        ];
        let mut state = SessionState::default();
        hydrate_session_state_from_turns(&mut state, &turns);

        assert_eq!(state.step_count, 2);
        assert_eq!(state.turn_id, "t2");
        assert_eq!(state.files_touched, vec!["a.rs", "b.rs"]);
        assert_eq!(state.token_usage.expect("token usage").input_tokens, 30);
    }
}

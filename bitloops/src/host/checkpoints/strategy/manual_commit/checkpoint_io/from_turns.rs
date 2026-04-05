use std::collections::BTreeSet;

use super::*;
use crate::host::interactions::types::InteractionTurn;

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
}

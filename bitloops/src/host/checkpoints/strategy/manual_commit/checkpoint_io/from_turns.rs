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

pub(crate) fn aggregate_turn_prompts(turns: &[InteractionTurn]) -> Vec<String> {
    turns
        .iter()
        .filter_map(|turn| {
            let trimmed = turn.prompt.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        })
        .collect()
}

pub(crate) fn aggregate_turn_transcript_bounds(
    turns: &[InteractionTurn],
) -> Option<(usize, usize)> {
    let mut min_start: Option<usize> = None;
    let mut max_end: Option<usize> = None;

    for turn in turns {
        let start = usize::try_from(turn.transcript_offset_start?).ok()?;
        let end = usize::try_from(turn.transcript_offset_end?).ok()?;
        if end < start {
            return None;
        }
        min_start = Some(min_start.map_or(start, |current| current.min(start)));
        max_end = Some(max_end.map_or(end, |current| current.max(end)));
    }

    match (min_start, max_end) {
        (Some(start), Some(end)) if end >= start => Some((start, end)),
        _ => None,
    }
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
            summary: format!("summary-{turn_id}"),
            prompt_count: 1,
            transcript_offset_start: Some(0),
            transcript_offset_end: Some(1),
            transcript_fragment: format!("{{\"turn_id\":\"{turn_id}\"}}\n"),
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
    fn aggregate_turn_transcript_bounds_returns_outer_span() {
        let turns = vec![
            make_turn("t1", "one", &["a.rs"], 1, 1),
            make_turn("t2", "two", &["b.rs"], 1, 1),
        ];
        let mut turns = turns;
        turns[0].transcript_offset_start = Some(5);
        turns[0].transcript_offset_end = Some(8);
        turns[1].transcript_offset_start = Some(2);
        turns[1].transcript_offset_end = Some(6);

        assert_eq!(aggregate_turn_transcript_bounds(&turns), Some((2, 8)));
    }
}

use super::*;

use crate::host::interactions::db_store::SqliteInteractionEventStore;
use crate::host::interactions::store::InteractionEventStore;
use crate::host::interactions::types::InteractionTurn;

// ── Build WriteCommittedOptions from interaction turns ────────────────────────

/// Aggregates data from interaction turns into a [`WriteCommittedOptions`] suitable
/// for persisting a committed checkpoint.
///
/// This is the event-derived alternative to reading temporary checkpoint trees.
/// Transcript/context bytes are left empty; they will come from blob storage in
/// a future iteration.
#[allow(dead_code)]
pub(crate) fn build_committed_options_from_turns(
    turns: &[InteractionTurn],
    checkpoint_id: &str,
    session_id: &str,
    agent: &str,
    strategy: &str,
) -> WriteCommittedOptions {
    // Aggregate token usage across all turns.
    let mut aggregated_usage: Option<TokenUsageMetadata> = None;
    for turn in turns {
        aggregated_usage = aggregate_token_usage(aggregated_usage, turn.token_usage.clone());
    }

    // Collect and deduplicate files_modified from all turns.
    let mut files_set = std::collections::BTreeSet::new();
    for turn in turns {
        for file in &turn.files_modified {
            files_set.insert(file.clone());
        }
    }
    let files_touched: Vec<String> = files_set.into_iter().collect();

    // Take the first turn's prompt as the prompt list.
    let prompts: Vec<String> = turns
        .first()
        .map(|t| t.prompt.clone())
        .filter(|p| !p.is_empty())
        .into_iter()
        .collect();

    WriteCommittedOptions {
        checkpoint_id: checkpoint_id.to_string(),
        session_id: session_id.to_string(),
        strategy: strategy.to_string(),
        agent: agent.to_string(),
        transcript: Vec::new(),
        prompts: if prompts.is_empty() {
            None
        } else {
            Some(prompts)
        },
        context: None,
        checkpoints_count: turns.len() as u32,
        files_touched,
        token_usage: aggregated_usage,
        turn_id: turns.first().map(|t| t.turn_id.clone()).unwrap_or_default(),
        ..Default::default()
    }
}

// ── Derive a committed checkpoint from pending interaction turns ──────────────

/// Opens the interaction event store and derives a committed checkpoint from
/// pending (unassigned) turns for the given session.
///
/// Returns `Ok(Some(checkpoint_id))` if a checkpoint was created, `Ok(None)` if
/// there are no pending turns, or an error on failure.
///
/// This function is intended to run alongside the existing `condense_session`
/// path. It will be wired into `post_commit` once we have confidence in the new
/// derivation path.
#[allow(dead_code)]
pub(crate) fn derive_checkpoint_from_interaction_turns(
    repo_root: &Path,
    session_id: &str,
) -> Result<Option<String>> {
    let store = resolve_interaction_event_store(repo_root)?;

    let pending = store
        .pending_turns_for_session(session_id)
        .context("loading pending interaction turns")?;
    if pending.is_empty() {
        return Ok(None);
    }

    // Determine agent from the first turn (or fall back to empty).
    let agent = pending
        .first()
        .map(|t| t.agent_type.as_str())
        .unwrap_or_default();

    let checkpoint_id = generate_checkpoint_id();

    let opts = build_committed_options_from_turns(
        &pending,
        &checkpoint_id,
        session_id,
        agent,
        "manual-commit",
    );

    write_committed(repo_root, opts)?;

    // Link the turns to the newly created checkpoint.
    let turn_ids: Vec<&str> = pending.iter().map(|t| t.turn_id.as_str()).collect();
    store
        .assign_checkpoint_to_turns(&turn_ids, &checkpoint_id)
        .context("assigning checkpoint to interaction turns")?;

    Ok(Some(checkpoint_id))
}

/// Resolves a [`SqliteInteractionEventStore`] for the given repo root.
///
/// Returns an error if the store cannot be created (missing database, etc.).
#[allow(dead_code)]
fn resolve_interaction_event_store(repo_root: &Path) -> Result<SqliteInteractionEventStore> {
    let sqlite_path = resolve_temporary_checkpoint_sqlite_path(repo_root)
        .context("resolving SQLite path for interaction event store")?;
    let sqlite = crate::storage::SqliteConnectionPool::connect(sqlite_path)
        .context("connecting to SQLite for interaction event store")?;
    sqlite
        .initialise_checkpoint_schema()
        .context("initialising checkpoint schema for interaction event store")?;
    let repo_id = crate::host::devql::resolve_repo_identity(repo_root)
        .context("resolving repo identity for interaction event store")?
        .repo_id;
    Ok(SqliteInteractionEventStore::new(sqlite, repo_id))
}

// ── Tests ────────────────────────────────────────────────────────────────────

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
            agent_type: "claude-code".to_string(),
            started_at: "2026-04-04T10:00:00Z".to_string(),
            token_usage: Some(TokenUsageMetadata {
                input_tokens,
                output_tokens,
                ..Default::default()
            }),
            files_modified: files.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn single_turn_aggregation() {
        let turns = vec![make_turn("t1", "fix bug", &["src/main.rs"], 100, 50)];

        let opts = build_committed_options_from_turns(
            &turns,
            "cp-1",
            "sess-1",
            "claude-code",
            "manual-commit",
        );

        assert_eq!(opts.checkpoint_id, "cp-1");
        assert_eq!(opts.session_id, "sess-1");
        assert_eq!(opts.agent, "claude-code");
        assert_eq!(opts.strategy, "manual-commit");
        assert_eq!(opts.checkpoints_count, 1);
        assert_eq!(opts.files_touched, vec!["src/main.rs"]);
        assert_eq!(opts.turn_id, "t1");

        let usage = opts.token_usage.unwrap();
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 50);

        // Prompt from first turn
        assert_eq!(opts.prompts, Some(vec!["fix bug".to_string()]));
    }

    #[test]
    fn multiple_turns_sums_tokens_and_deduplicates_files() {
        let turns = vec![
            make_turn("t1", "fix bug", &["src/main.rs", "src/lib.rs"], 100, 50),
            make_turn(
                "t2",
                "add tests",
                &["src/main.rs", "tests/test.rs"],
                200,
                80,
            ),
            make_turn("t3", "refactor", &["src/lib.rs"], 50, 30),
        ];

        let opts = build_committed_options_from_turns(
            &turns,
            "cp-2",
            "sess-1",
            "claude-code",
            "manual-commit",
        );

        assert_eq!(opts.checkpoints_count, 3);

        // Files should be deduplicated and sorted (BTreeSet).
        assert_eq!(
            opts.files_touched,
            vec!["src/lib.rs", "src/main.rs", "tests/test.rs"]
        );

        // Token usage should be summed.
        let usage = opts.token_usage.unwrap();
        assert_eq!(usage.input_tokens, 350); // 100 + 200 + 50
        assert_eq!(usage.output_tokens, 160); // 50 + 80 + 30

        // Prompt comes from first turn only.
        assert_eq!(opts.prompts, Some(vec!["fix bug".to_string()]));

        // turn_id comes from first turn.
        assert_eq!(opts.turn_id, "t1");
    }

    #[test]
    fn empty_turns_returns_sensible_defaults() {
        let turns: Vec<InteractionTurn> = Vec::new();

        let opts = build_committed_options_from_turns(
            &turns,
            "cp-3",
            "sess-1",
            "claude-code",
            "manual-commit",
        );

        assert_eq!(opts.checkpoint_id, "cp-3");
        assert_eq!(opts.session_id, "sess-1");
        assert_eq!(opts.checkpoints_count, 0);
        assert!(opts.files_touched.is_empty());
        assert!(opts.token_usage.is_none());
        assert!(opts.prompts.is_none());
        assert!(opts.turn_id.is_empty());
        assert!(opts.transcript.is_empty());
        assert!(opts.context.is_none());
    }

    #[test]
    fn turns_with_no_token_usage() {
        let turns = vec![InteractionTurn {
            turn_id: "t1".to_string(),
            session_id: "sess-1".to_string(),
            repo_id: "repo-1".to_string(),
            turn_number: 1,
            prompt: "hello".to_string(),
            agent_type: "claude-code".to_string(),
            started_at: "2026-04-04T10:00:00Z".to_string(),
            token_usage: None,
            files_modified: vec!["readme.md".to_string()],
            ..Default::default()
        }];

        let opts = build_committed_options_from_turns(
            &turns,
            "cp-4",
            "sess-1",
            "claude-code",
            "manual-commit",
        );

        assert!(opts.token_usage.is_none());
        assert_eq!(opts.files_touched, vec!["readme.md"]);
        assert_eq!(opts.checkpoints_count, 1);
    }

    #[test]
    fn turns_with_empty_prompt_produces_no_prompts() {
        let turns = vec![InteractionTurn {
            turn_id: "t1".to_string(),
            session_id: "sess-1".to_string(),
            repo_id: "repo-1".to_string(),
            prompt: String::new(),
            started_at: "2026-04-04T10:00:00Z".to_string(),
            ..Default::default()
        }];

        let opts = build_committed_options_from_turns(
            &turns,
            "cp-5",
            "sess-1",
            "claude-code",
            "manual-commit",
        );

        assert!(opts.prompts.is_none());
    }
}

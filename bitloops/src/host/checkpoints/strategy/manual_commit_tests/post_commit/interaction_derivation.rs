use super::*;
use crate::host::interactions::db_store::{SqliteInteractionSpool, interaction_spool_db_path};
use crate::host::interactions::store::InteractionSpool;
use crate::host::interactions::types::{InteractionSession, InteractionTurn};

fn open_test_spool(repo_root: &Path) -> SqliteInteractionSpool {
    let repo_id = crate::host::devql::resolve_repo_identity(repo_root)
        .expect("resolve repo identity")
        .repo_id;
    let sqlite = SqliteConnectionPool::connect(interaction_spool_db_path(repo_root))
        .expect("open interaction spool sqlite");
    SqliteInteractionSpool::new(sqlite, repo_id).expect("initialise interaction spool")
}

fn seed_interaction_turn(repo_root: &Path, session_id: &str, turn_id: &str, files: &[&str]) {
    let spool = open_test_spool(repo_root);
    let session = InteractionSession {
        session_id: session_id.to_string(),
        repo_id: spool.repo_id().to_string(),
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
    };
    let turn = InteractionTurn {
        turn_id: turn_id.to_string(),
        session_id: session_id.to_string(),
        repo_id: spool.repo_id().to_string(),
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
        files_modified: files.iter().map(|file| file.to_string()).collect(),
        updated_at: "2026-04-05T10:00:02Z".to_string(),
        ..Default::default()
    };
    spool.record_session(&session).expect("record session");
    spool.record_turn(&turn).expect("record turn");
}

#[test]
pub(crate) fn post_commit_derives_checkpoint_from_interaction_turns() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    init_devql_schema(dir.path());
    std::fs::write(dir.path().join("transcript.jsonl"), "{}\n").unwrap();
    seed_interaction_turn(dir.path(), "sess-1", "turn-1", &["change.txt"]);

    std::fs::write(dir.path().join("change.txt"), "hello from interaction\n").unwrap();
    git_ok(dir.path(), &["add", "change.txt"]);
    git_ok(
        dir.path(),
        &["commit", "-m", "derive checkpoint from interaction"],
    );
    let head = git_ok(dir.path(), &["rev-parse", "HEAD"]);

    let strategy = ManualCommitStrategy::new(dir.path());
    strategy.post_commit().expect("post_commit should succeed");

    let checkpoint_id = read_commit_checkpoint_mappings(dir.path())
        .expect("mappings")
        .get(&head)
        .cloned()
        .expect("checkpoint mapping for derived commit");
    let summary = read_committed(dir.path(), &checkpoint_id)
        .expect("read derived checkpoint")
        .expect("derived checkpoint summary");
    assert_eq!(summary.files_touched, vec!["change.txt"]);

    let turns = open_test_spool(dir.path())
        .list_turns_for_session("sess-1", 10)
        .expect("list turns after derivation");
    assert_eq!(turns.len(), 1);
    assert_eq!(
        turns[0].checkpoint_id.as_deref(),
        Some(checkpoint_id.as_str())
    );
}

#[test]
pub(crate) fn post_commit_skips_non_overlapping_interaction_turns() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    init_devql_schema(dir.path());
    std::fs::write(dir.path().join("transcript.jsonl"), "{}\n").unwrap();
    seed_interaction_turn(dir.path(), "sess-2", "turn-2", &["design-notes.md"]);

    std::fs::write(dir.path().join("change.txt"), "real commit\n").unwrap();
    git_ok(dir.path(), &["add", "change.txt"]);
    git_ok(dir.path(), &["commit", "-m", "non-overlapping interaction"]);
    let head = git_ok(dir.path(), &["rev-parse", "HEAD"]);

    let strategy = ManualCommitStrategy::new(dir.path());
    strategy.post_commit().expect("post_commit should succeed");

    let mappings = read_commit_checkpoint_mappings(dir.path()).expect("mappings");
    assert!(
        !mappings.contains_key(&head),
        "non-overlapping interaction turn should not derive a checkpoint"
    );

    let turns = open_test_spool(dir.path())
        .list_turns_for_session("sess-2", 10)
        .expect("list turns after skipped derivation");
    assert_eq!(turns.len(), 1);
    assert!(turns[0].checkpoint_id.is_none());
}

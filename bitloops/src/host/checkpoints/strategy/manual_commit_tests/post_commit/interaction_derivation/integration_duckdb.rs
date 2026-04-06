use super::super::*;

use crate::host::interactions::store::InteractionSpool;

#[test]
#[ignore = "ad hoc DuckDB integration coverage"]
pub(crate) fn post_commit_derives_checkpoint_from_interaction_turns_duckdb_integration() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    init_devql_schema(dir.path());
    seed_interaction_turn(dir.path(), "sess-1", "turn-1", &["change.txt"]);

    std::fs::write(dir.path().join("change.txt"), "hello from interaction\n").unwrap();
    git_ok(dir.path(), &["add", "change.txt"]);
    git_ok(
        dir.path(),
        &["commit", "-m", "derive checkpoint from interaction"],
    );
    let head = git_ok(dir.path(), &["rev-parse", "HEAD"]);
    std::fs::remove_file(dir.path().join("transcript.jsonl")).expect("delete transcript file");

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
#[ignore = "ad hoc DuckDB integration coverage"]
pub(crate) fn post_commit_skips_non_overlapping_interaction_turns_duckdb_integration() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    init_devql_schema(dir.path());
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

#[test]
#[ignore = "ad hoc DuckDB integration coverage"]
pub(crate) fn post_commit_errors_when_duckdb_interaction_turn_is_missing_transcript_fragment() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    init_devql_schema(dir.path());
    seed_interaction_turn_with_fragment(
        dir.path(),
        "sess-missing-fragment-db",
        "turn-missing-fragment-db",
        &["change.txt"],
        "",
    );

    std::fs::write(dir.path().join("change.txt"), "hello from interaction\n").unwrap();
    git_ok(dir.path(), &["add", "change.txt"]);
    git_ok(dir.path(), &["commit", "-m", "missing transcript fragment"]);
    let head = git_ok(dir.path(), &["rev-parse", "HEAD"]);

    let err = ManualCommitStrategy::new(dir.path())
        .post_commit()
        .expect_err("post_commit should fail when transcript fragments are missing");
    assert!(err.to_string().contains("missing transcript_fragment"));
    assert!(
        !read_commit_checkpoint_mappings(dir.path())
            .expect("mappings")
            .contains_key(&head),
        "failed derivation must not write a commit mapping"
    );
}

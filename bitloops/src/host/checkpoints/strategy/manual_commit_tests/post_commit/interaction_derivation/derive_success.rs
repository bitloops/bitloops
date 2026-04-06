use std::sync::{Arc, Mutex};

use super::super::*;
use super::fakes::{FakeInteractionRepository, FakeInteractionSpool};
use super::fixtures::{fake_interaction_session, fake_interaction_turn};

#[test]
pub(crate) fn derive_post_commit_from_event_db_turns_with_fake_sources() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    init_devql_schema(dir.path());
    std::fs::write(dir.path().join("transcript.jsonl"), "{}\n").unwrap();

    let repo_id = crate::host::devql::resolve_repo_identity(dir.path())
        .expect("resolve repo identity")
        .repo_id;
    let operations = Arc::new(Mutex::new(Vec::new()));
    let repository = FakeInteractionRepository::new(&repo_id, Arc::clone(&operations))
        .with_session(fake_interaction_session(dir.path(), &repo_id, "sess-1"))
        .with_turn(fake_interaction_turn(
            &repo_id,
            "sess-1",
            "turn-1",
            &["change.txt"],
        ));
    let mut spool = FakeInteractionSpool::new(&repo_id);
    spool.operations = Arc::clone(&operations);

    std::fs::write(dir.path().join("change.txt"), "hello from interaction\n").unwrap();
    git_ok(dir.path(), &["add", "change.txt"]);
    git_ok(
        dir.path(),
        &["commit", "-m", "derive checkpoint from fake event repo"],
    );
    let head = git_ok(dir.path(), &["rev-parse", "HEAD"]);
    let committed_files = files_changed_in_commit(dir.path(), &head).expect("committed files");

    let strategy = ManualCommitStrategy::new(dir.path());
    let checkpoint_id = strategy
        .derive_post_commit_from_interaction_sources(
            &head,
            &committed_files,
            false,
            &repository,
            Some(&spool),
        )
        .expect("derive from event db")
        .expect("checkpoint id");

    let summary = read_committed(dir.path(), &checkpoint_id)
        .expect("read derived checkpoint")
        .expect("derived checkpoint summary");
    assert_eq!(summary.files_touched, vec!["change.txt"]);
    assert_eq!(
        repository.checkpoint_id_for("turn-1").as_deref(),
        Some(checkpoint_id.as_str())
    );

    let sequence = operations.lock().expect("lock operations").clone();
    assert_eq!(
        sequence[..2],
        ["spool.flush", "repo.list_uncheckpointed_turns"],
        "post_commit should flush the spool before reading the Event DB"
    );
}

#[test]
pub(crate) fn derive_post_commit_keeps_partially_committed_turns_available_for_later_commits() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    init_devql_schema(dir.path());
    std::fs::write(dir.path().join("transcript.jsonl"), "{}\n").unwrap();

    let repo_id = crate::host::devql::resolve_repo_identity(dir.path())
        .expect("resolve repo identity")
        .repo_id;
    let repository = FakeInteractionRepository::new(&repo_id, Arc::new(Mutex::new(Vec::new())))
        .with_session(fake_interaction_session(dir.path(), &repo_id, "sess-split"))
        .with_turn(fake_interaction_turn(
            &repo_id,
            "sess-split",
            "turn-split",
            &["file_a.txt", "file_b.txt"],
        ));
    let spool = FakeInteractionSpool::new(&repo_id);

    std::fs::write(dir.path().join("file_a.txt"), "A\n").unwrap();
    std::fs::write(dir.path().join("file_b.txt"), "B\n").unwrap();

    git_ok(dir.path(), &["add", "file_a.txt"]);
    git_ok(dir.path(), &["commit", "-m", "commit file_a"]);
    let head_a = git_ok(dir.path(), &["rev-parse", "HEAD"]);
    let committed_a = files_changed_in_commit(dir.path(), &head_a).expect("committed file_a");

    let strategy = ManualCommitStrategy::new(dir.path());
    let checkpoint_a = strategy
        .derive_post_commit_from_interaction_sources(
            &head_a,
            &committed_a,
            false,
            &repository,
            Some(&spool),
        )
        .expect("derive checkpoint for file_a")
        .expect("checkpoint for file_a");

    assert!(
        repository.checkpoint_id_for("turn-split").is_none(),
        "partially committed turns must remain available for later commits"
    );
    assert_eq!(
        repository.files_modified_for("turn-split"),
        vec!["file_b.txt".to_string()],
        "remaining file hints should be narrowed to the uncommitted overlap"
    );

    git_ok(dir.path(), &["add", "file_b.txt"]);
    git_ok(dir.path(), &["commit", "-m", "commit file_b"]);
    let head_b = git_ok(dir.path(), &["rev-parse", "HEAD"]);
    let committed_b = files_changed_in_commit(dir.path(), &head_b).expect("committed file_b");

    let checkpoint_b = strategy
        .derive_post_commit_from_interaction_sources(
            &head_b,
            &committed_b,
            false,
            &repository,
            Some(&spool),
        )
        .expect("derive checkpoint for file_b")
        .expect("checkpoint for file_b");

    assert_ne!(checkpoint_a, checkpoint_b);
    assert_eq!(
        repository.checkpoint_id_for("turn-split").as_deref(),
        Some(checkpoint_b.as_str()),
        "the turn should be fully assigned after its remaining files are committed"
    );
}

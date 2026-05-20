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
    assert_eq!(
        spool.checkpoint_id_for("turn-1").as_deref(),
        Some(checkpoint_id.as_str()),
        "local spool state should refresh from canonical event-store progress without re-queueing"
    );

    let sequence = operations.lock().expect("lock operations").clone();
    let flush_index = sequence
        .iter()
        .position(|entry| *entry == "spool.flush")
        .expect("expected spool.flush operation");
    let repo_list_index = sequence
        .iter()
        .position(|entry| *entry == "repo.list_uncheckpointed_turns")
        .expect("expected repo.list_uncheckpointed_turns operation");
    assert!(
        flush_index < repo_list_index,
        "post_commit should flush the spool before reading the Event DB, got sequence {sequence:?}"
    );
    assert_eq!(
        sequence
            .iter()
            .filter(|entry| **entry == "spool.flush")
            .count(),
        1,
        "checkpoint progress refreshes should not queue a second canonical event-store flush"
    );
}

#[test]
pub(crate) fn derive_post_commit_enqueues_context_guidance_history_distillation() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    init_devql_schema(dir.path());
    seed_interaction_turn(
        dir.path(),
        "sess-guidance",
        "turn-guidance",
        &["change.txt"],
    );

    std::fs::write(dir.path().join("change.txt"), "hello from interaction\n").unwrap();
    git_ok(dir.path(), &["add", "change.txt"]);
    git_ok(
        dir.path(),
        &["commit", "-m", "derive context guidance work"],
    );
    let head = git_ok(dir.path(), &["rev-parse", "HEAD"]);

    ManualCommitStrategy::new(dir.path())
        .post_commit()
        .expect("post_commit should succeed");

    let checkpoint_id = read_commit_checkpoint_mappings(dir.path())
        .expect("mappings")
        .get(&head)
        .cloned()
        .expect("checkpoint mapping for derived commit");
    let runtime_db = crate::config::resolve_repo_runtime_db_path_for_repo(dir.path())
        .expect("resolve runtime db");
    let conn = rusqlite::Connection::open(runtime_db).expect("open runtime db");
    let (mailbox, dedupe_key, payload): (String, String, String) = conn
        .query_row(
            "SELECT mailbox_name, dedupe_key, payload
             FROM capability_workplane_jobs
             WHERE capability_id = 'context_guidance'
               AND mailbox_name = 'context_guidance.history_distillation'
               AND status = 'pending'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("pending context guidance job");
    let payload: serde_json::Value = serde_json::from_str(&payload).expect("payload json");

    assert_eq!(mailbox, "context_guidance.history_distillation");
    assert!(dedupe_key.starts_with("history_turn:sess-guidance:turn-guidance:"));
    assert_eq!(
        payload["historyTurn"]["checkpointId"].as_str(),
        Some(checkpoint_id.as_str())
    );
    assert_eq!(
        payload["historyTurn"]["sessionId"].as_str(),
        Some("sess-guidance")
    );
    assert_eq!(
        payload["historyTurn"]["turnId"].as_str(),
        Some("turn-guidance")
    );
    assert!(
        payload["historyTurn"]["inputHash"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
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

#[test]
pub(crate) fn derive_post_commit_errors_instead_of_falling_back_to_local_spool_when_flush_fails() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    init_devql_schema(dir.path());
    std::fs::write(dir.path().join("transcript.jsonl"), "{}\n").unwrap();

    let repo_id = crate::host::devql::resolve_repo_identity(dir.path())
        .expect("resolve repo identity")
        .repo_id;
    let operations = Arc::new(Mutex::new(Vec::new()));
    let session = fake_interaction_session(dir.path(), &repo_id, "sess-spool-fallback");
    let turn = fake_interaction_turn(
        &repo_id,
        "sess-spool-fallback",
        "turn-spool-fallback",
        &["change.txt"],
    );
    let repository = FakeInteractionRepository::new(&repo_id, Arc::clone(&operations));
    let mut spool = FakeInteractionSpool::new(&repo_id)
        .with_session(session)
        .with_turn(turn);
    spool.pending_mutations = true;
    spool.flush_error = Some("forced flush failure".to_string());
    spool.operations = Arc::clone(&operations);

    std::fs::write(dir.path().join("change.txt"), "hello from local spool\n").unwrap();
    git_ok(dir.path(), &["add", "change.txt"]);
    git_ok(
        dir.path(),
        &["commit", "-m", "derive checkpoint from local spool"],
    );
    let head = git_ok(dir.path(), &["rev-parse", "HEAD"]);
    let committed_files = files_changed_in_commit(dir.path(), &head).expect("committed files");

    let strategy = ManualCommitStrategy::new(dir.path());
    let err = strategy
        .derive_post_commit_from_interaction_sources(
            &head,
            &committed_files,
            false,
            &repository,
            Some(&spool),
        )
        .expect_err(
            "flush failure should stop post_commit derivation instead of reading spool-local turns",
        );

    assert!(
        format!("{err:#}").contains("flushing interaction spool before post_commit derivation"),
        "unexpected error: {err:#}"
    );
    assert_eq!(
        spool.checkpoint_id_for("turn-spool-fallback").as_deref(),
        None,
        "spool-only turn data must not be treated as canonical after a flush failure"
    );

    let sequence = operations.lock().expect("lock operations").clone();
    assert_eq!(
        sequence,
        ["spool.flush"],
        "post_commit should stop after the canonical event-store flush fails instead of querying the local spool"
    );
}

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use super::super::*;
use super::fakes::{FakeInteractionRepository, FakeInteractionSpool};
use super::fixtures::{fake_interaction_session, fake_interaction_turn};

#[test]
pub(crate) fn derive_post_commit_errors_when_overlapping_turn_is_missing_transcript_fragment() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    init_devql_schema(dir.path());

    let repo_id = crate::host::devql::resolve_repo_identity(dir.path())
        .expect("resolve repo identity")
        .repo_id;
    let session = fake_interaction_session(dir.path(), &repo_id, "sess-missing-fragment");
    let mut turn = fake_interaction_turn(
        &repo_id,
        "sess-missing-fragment",
        "turn-missing-fragment",
        &["change.txt"],
    );
    turn.transcript_fragment.clear();
    let repository = FakeInteractionRepository::new(&repo_id, Arc::new(Mutex::new(Vec::new())))
        .with_session(session)
        .with_turn(turn);
    let spool = FakeInteractionSpool::new(&repo_id);

    std::fs::write(dir.path().join("change.txt"), "hello from interaction\n").unwrap();
    git_ok(dir.path(), &["add", "change.txt"]);
    git_ok(dir.path(), &["commit", "-m", "missing transcript fragment"]);
    let head = git_ok(dir.path(), &["rev-parse", "HEAD"]);
    let committed_files = files_changed_in_commit(dir.path(), &head).expect("committed files");

    let err = ManualCommitStrategy::new(dir.path())
        .derive_post_commit_from_interaction_sources(
            &head,
            &committed_files,
            false,
            &repository,
            Some(&spool),
        )
        .unwrap_err();

    assert!(
        err.to_string().contains("missing transcript_fragment"),
        "unexpected error: {err}"
    );
    assert!(
        format!("{err:#}").contains(&format!("commit={head}")),
        "error should include the commit context: {err:#}"
    );
    assert!(
        format!("{err:#}").contains("session_id=sess-missing-fragment"),
        "error should include the session context: {err:#}"
    );
    assert!(
        format!("{err:#}").contains("turn-missing-fragment"),
        "error should include the turn context: {err:#}"
    );
    assert!(
        !read_commit_checkpoint_mappings(dir.path())
            .expect("read mappings")
            .contains_key(&head),
        "failed derivation must not write a commit mapping"
    );
}

#[test]
pub(crate) fn derive_post_commit_errors_when_spool_flush_fails_without_local_turn_data() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    init_devql_schema(dir.path());

    let repo_id = crate::host::devql::resolve_repo_identity(dir.path())
        .expect("resolve repo identity")
        .repo_id;
    let repository = FakeInteractionRepository::new(&repo_id, Arc::new(Mutex::new(Vec::new())));
    let spool = FakeInteractionSpool::new(&repo_id)
        .with_pending_mutations(true)
        .with_flush_error("forced flush failure");

    let strategy = ManualCommitStrategy::new(dir.path());
    let err = strategy
        .derive_post_commit_from_interaction_sources(
            "deadbeef",
            &HashSet::new(),
            false,
            &repository,
            Some(&spool),
        )
        .expect_err("flush failure should not degrade to local spool authority even without local turn rows");
    assert!(
        format!("{err:#}").contains("flushing interaction spool before post_commit derivation"),
        "unexpected error: {err:#}"
    );
}

#[test]
pub(crate) fn derive_post_commit_returns_error_when_overlapping_turn_session_is_missing() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    init_devql_schema(dir.path());
    std::fs::write(dir.path().join("transcript.jsonl"), "{}\n").unwrap();

    std::fs::write(dir.path().join("change.txt"), "hello\n").unwrap();
    git_ok(dir.path(), &["add", "change.txt"]);
    git_ok(dir.path(), &["commit", "-m", "missing interaction session"]);
    let head = git_ok(dir.path(), &["rev-parse", "HEAD"]);
    let committed_files = files_changed_in_commit(dir.path(), &head).expect("committed files");

    let repo_id = crate::host::devql::resolve_repo_identity(dir.path())
        .expect("resolve repo identity")
        .repo_id;
    let repository =
        FakeInteractionRepository::new(&repo_id, Arc::new(Mutex::new(Vec::new()))).with_turn(
            fake_interaction_turn(&repo_id, "missing-session", "turn-1", &["change.txt"]),
        );
    let spool = FakeInteractionSpool::new(&repo_id);

    let strategy = ManualCommitStrategy::new(dir.path());
    let err = strategy
        .derive_post_commit_from_interaction_sources(
            &head,
            &committed_files,
            false,
            &repository,
            Some(&spool),
        )
        .expect_err("missing interaction session should error");
    assert!(format!("{err:#}").contains("missing interaction session"));
    assert!(
        format!("{err:#}").contains(&format!("commit={head}")),
        "error should include the commit context: {err:#}"
    );
    assert!(
        format!("{err:#}").contains("session_id=missing-session"),
        "error should include the session context: {err:#}"
    );
    assert!(
        format!("{err:#}").contains("turn-1"),
        "error should include the overlapping turn context: {err:#}"
    );
}

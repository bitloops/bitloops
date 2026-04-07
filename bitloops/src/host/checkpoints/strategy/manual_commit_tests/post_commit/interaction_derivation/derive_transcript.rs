use std::sync::{Arc, Mutex};

use super::super::*;
use super::fakes::{FakeInteractionRepository, FakeInteractionSpool};
use super::fixtures::{fake_interaction_session, fake_interaction_turn};

#[test]
pub(crate) fn derive_post_commit_scopes_committed_transcript_from_turn_offsets() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    init_devql_schema(dir.path());
    std::fs::write(
        dir.path().join("transcript.jsonl"),
        "{\"role\":\"user\",\"content\":\"before\"}\n{\"role\":\"user\",\"content\":\"captured prompt\"}\n{\"role\":\"assistant\",\"content\":\"captured answer\"}\n{\"role\":\"assistant\",\"content\":\"after\"}\n",
    )
    .unwrap();

    let repo_id = crate::host::devql::resolve_repo_identity(dir.path())
        .expect("resolve repo identity")
        .repo_id;
    let session = fake_interaction_session(dir.path(), &repo_id, "sess-sliced");
    let mut turn = fake_interaction_turn(&repo_id, "sess-sliced", "turn-sliced", &["change.txt"]);
    turn.prompt = "captured prompt".into();
    turn.transcript_offset_start = Some(1);
    turn.transcript_offset_end = Some(3);
    turn.transcript_fragment =
        "{\"role\":\"user\",\"content\":\"captured prompt\"}\n{\"role\":\"assistant\",\"content\":\"captured answer\"}\n".into();

    let repository = FakeInteractionRepository::new(&repo_id, Arc::new(Mutex::new(Vec::new())))
        .with_session(session)
        .with_turn(turn);
    let spool = FakeInteractionSpool::new(&repo_id);

    std::fs::write(dir.path().join("change.txt"), "hello from interaction\n").unwrap();
    git_ok(dir.path(), &["add", "change.txt"]);
    git_ok(dir.path(), &["commit", "-m", "derive sliced transcript"]);
    let head = git_ok(dir.path(), &["rev-parse", "HEAD"]);
    let committed_files = files_changed_in_commit(dir.path(), &head).expect("committed files");

    let checkpoint_id = ManualCommitStrategy::new(dir.path())
        .derive_post_commit_from_interaction_sources(
            &head,
            &committed_files,
            false,
            &repository,
            Some(&spool),
        )
        .expect("derive from interaction source")
        .expect("checkpoint id");

    let session_content =
        read_session_content(dir.path(), &checkpoint_id, 0).expect("read session content");
    assert!(
        session_content.transcript.contains("captured prompt"),
        "scoped transcript should contain the recorded turn prompt"
    );
    assert!(
        session_content.transcript.contains("captured answer"),
        "scoped transcript should contain the recorded turn answer"
    );
    assert!(
        !session_content.transcript.contains("before"),
        "scoped transcript should exclude content before the recorded turn offsets"
    );
    assert!(
        !session_content.transcript.contains("after"),
        "scoped transcript should exclude content after the recorded turn offsets"
    );
}

#[test]
pub(crate) fn derive_post_commit_uses_event_native_transcript_when_turn_offsets_are_missing() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    init_devql_schema(dir.path());
    let transcript_path = dir.path().join("transcript.jsonl");
    std::fs::write(
        &transcript_path,
        "{\"role\":\"user\",\"content\":\"before\"}\n{\"role\":\"user\",\"content\":\"captured prompt\"}\n{\"role\":\"assistant\",\"content\":\"captured answer\"}\n",
    )
    .unwrap();

    let repo_id = crate::host::devql::resolve_repo_identity(dir.path())
        .expect("resolve repo identity")
        .repo_id;
    let session = fake_interaction_session(dir.path(), &repo_id, "sess-fallback");
    let mut turn =
        fake_interaction_turn(&repo_id, "sess-fallback", "turn-fallback", &["change.txt"]);
    turn.prompt = "captured prompt".into();
    turn.transcript_offset_start = None;
    turn.transcript_offset_end = None;
    turn.transcript_fragment =
        "{\"role\":\"user\",\"content\":\"captured prompt\"}\n{\"role\":\"assistant\",\"content\":\"captured answer\"}\n".into();

    let repository = FakeInteractionRepository::new(&repo_id, Arc::new(Mutex::new(Vec::new())))
        .with_session(session)
        .with_turn(turn);
    let spool = FakeInteractionSpool::new(&repo_id);

    std::fs::write(dir.path().join("change.txt"), "hello from interaction\n").unwrap();
    git_ok(dir.path(), &["add", "change.txt"]);
    git_ok(dir.path(), &["commit", "-m", "derive transcript fallback"]);
    let head = git_ok(dir.path(), &["rev-parse", "HEAD"]);
    let committed_files = files_changed_in_commit(dir.path(), &head).expect("committed files");
    std::fs::remove_file(&transcript_path).expect("delete transcript file");

    let checkpoint_id = ManualCommitStrategy::new(dir.path())
        .derive_post_commit_from_interaction_sources(
            &head,
            &committed_files,
            false,
            &repository,
            Some(&spool),
        )
        .expect("derive from interaction source")
        .expect("checkpoint id");

    let session_content =
        read_session_content(dir.path(), &checkpoint_id, 0).expect("read session content");
    assert!(
        session_content.transcript.contains("captured answer"),
        "event-native transcript should include the recorded turn content"
    );
    assert!(
        !session_content.transcript.contains("before"),
        "checkpoint derivation should no longer fall back to the transcript file"
    );
}

use std::sync::{Arc, Mutex};

use super::super::*;
use super::fakes::{FakeInteractionRepository, FakeInteractionSpool};

use crate::host::checkpoints::session::state::PendingCheckpointState;

#[test]
pub(crate) fn derive_post_commit_ignores_session_state_without_interaction_turns() {
    let dir = tempfile::tempdir().unwrap();
    let head_before = setup_git_repo(&dir);
    init_devql_schema(dir.path());
    let backend = session_backend(dir.path());
    backend
        .save_session(&SessionState {
            session_id: "sess-state-only".to_string(),
            phase: SessionPhase::Idle,
            base_commit: head_before,
            pending: PendingCheckpointState {
                step_count: 2,
                files_touched: vec!["state-only.txt".to_string()],
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap();

    std::fs::write(
        dir.path().join("state-only.txt"),
        "pending but uncaptured\n",
    )
    .unwrap();
    git_ok(dir.path(), &["add", "state-only.txt"]);
    git_ok(dir.path(), &["commit", "-m", "state only pending work"]);
    let head = git_ok(dir.path(), &["rev-parse", "HEAD"]);
    let committed_files = files_changed_in_commit(dir.path(), &head).expect("committed files");

    let repo_id = crate::host::devql::resolve_repo_identity(dir.path())
        .expect("resolve repo identity")
        .repo_id;
    let repository = FakeInteractionRepository::new(&repo_id, Arc::new(Mutex::new(Vec::new())));
    let spool = FakeInteractionSpool::new(&repo_id);

    let strategy = ManualCommitStrategy::new(dir.path());
    let checkpoint_id = strategy
        .derive_post_commit_from_interaction_sources(
            &head,
            &committed_files,
            false,
            &repository,
            Some(&spool),
        )
        .expect("derive without interaction turns");

    assert!(checkpoint_id.is_none());
    assert!(
        !read_commit_checkpoint_mappings(dir.path())
            .expect("mappings")
            .contains_key(&head),
        "session-state-only pending work must not derive a checkpoint"
    );
}

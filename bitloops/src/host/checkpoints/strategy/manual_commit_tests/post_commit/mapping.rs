use super::*;

#[test]
pub(crate) fn post_commit_creates_checkpoint_mapping_and_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);

    // Create a session with active state.
    let backend = session_backend(dir.path());
    let state = SessionState {
        session_id: "pc1".to_string(),
        phase: crate::host::checkpoints::session::phase::SessionPhase::Idle,
        base_commit: head.clone(),
        agent_type: "claude-code".to_string(),
        first_prompt: "test prompt".to_string(),
        step_count: 1,
        files_touched: vec!["change.txt".to_string()],
        ..Default::default()
    };
    backend.save_session(&state).unwrap();
    seed_interaction_turn(dir.path(), "pc1", "pc1-turn", &["change.txt"]);

    // Make a regular commit.
    fs::write(dir.path().join("change.txt"), "change").unwrap();
    git_command()
        .args(["add", "."])
        .current_dir(dir.path())
        .output()
        .unwrap();
    git_command()
        .args(["commit", "-m", "fix: something"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    let head_sha = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();

    let strategy = ManualCommitStrategy::new(dir.path());
    strategy.post_commit().unwrap();

    let checkpoint_id = query_commit_checkpoint_id(dir.path(), &head_sha)
        .expect("checkpoint mapping should exist after post_commit");
    assert!(
        is_valid_checkpoint_id(&checkpoint_id),
        "post_commit should generate a valid checkpoint id: {checkpoint_id}"
    );

    let summary = read_committed(dir.path(), &checkpoint_id)
        .expect("read committed checkpoint")
        .expect("checkpoint should exist after post_commit");
    assert_eq!(summary.checkpoint_id, checkpoint_id);
    assert_eq!(summary.strategy, "manual-commit");
    let result = run_git(dir.path(), &["rev-parse", "bitloops/checkpoints/v1"]);
    assert!(
        result.is_err(),
        "post_commit should no longer materialize metadata branch commits"
    );
}

// New test: post_commit creates full checkpoint structure.
#[test]
pub(crate) fn post_commit_creates_full_checkpoint_structure() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);

    let backend = session_backend(dir.path());
    let state = SessionState {
        session_id: "pc2".to_string(),
        phase: crate::host::checkpoints::session::phase::SessionPhase::Idle,
        base_commit: head.clone(),
        agent_type: "claude-code".to_string(),
        files_touched: vec!["change2.txt".to_string()],
        ..Default::default()
    };
    backend.save_session(&state).unwrap();
    seed_interaction_turn(dir.path(), "pc2", "pc2-turn", &["change2.txt"]);

    // post_commit should assign and persist checkpoint ID.
    fs::write(dir.path().join("change2.txt"), "change2").unwrap();
    git_command()
        .args(["add", "."])
        .current_dir(dir.path())
        .output()
        .unwrap();
    git_command()
        .args(["commit", "-m", "fix"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    let head_sha = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();

    let strategy = ManualCommitStrategy::new(dir.path());
    strategy.post_commit().unwrap();

    let checkpoint_id = query_commit_checkpoint_id(dir.path(), &head_sha)
        .expect("checkpoint mapping should exist after post_commit");
    let summary = read_committed(dir.path(), &checkpoint_id)
        .expect("read committed checkpoint")
        .expect("checkpoint should exist");
    assert_eq!(summary.checkpoint_id, checkpoint_id);
    assert_eq!(summary.strategy, "manual-commit");
    assert_eq!(summary.sessions.len(), 1);

    let session = read_session_content(dir.path(), &checkpoint_id, 0).expect("read session");
    assert_eq!(session.metadata["checkpoint_id"], checkpoint_id);
    assert_eq!(session.metadata["strategy"], "manual-commit");
}

#[test]
pub(crate) fn post_commit_without_checkpoint_condenses_pending_session_and_maps_head() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let backend = session_backend(dir.path());
    backend
        .save_session(&SessionState {
            session_id: "pc-no-checkpoint-condense".to_string(),
            phase: SessionPhase::Idle,
            base_commit: head,
            step_count: 1,
            files_touched: vec!["condense.txt".to_string()],
            ..Default::default()
        })
        .unwrap();
    seed_interaction_turn(
        dir.path(),
        "pc-no-checkpoint-condense",
        "pc-no-checkpoint-condense-turn",
        &["condense.txt"],
    );

    fs::write(dir.path().join("condense.txt"), "condense").unwrap();
    git_ok(dir.path(), &["add", "condense.txt"]);
    git_ok(dir.path(), &["commit", "-m", "regular commit"]);
    let head_sha = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();

    ManualCommitStrategy::new(dir.path()).post_commit().unwrap();

    let checkpoint_id = query_commit_checkpoint_id(dir.path(), &head_sha)
        .expect("post_commit should map HEAD to a generated checkpoint ID");
    assert!(
        read_committed(dir.path(), &checkpoint_id)
            .unwrap()
            .is_some(),
        "post_commit should persist checkpoint content for mapped id"
    );
}

#[test]
pub(crate) fn post_commit_squash_commit_condenses_pending_session_and_maps_head() {
    let dir = tempfile::tempdir().unwrap();
    let initial_head = setup_git_repo(&dir);
    let backend = session_backend(dir.path());
    backend
        .save_session(&SessionState {
            session_id: "pc-squash".to_string(),
            phase: SessionPhase::Idle,
            base_commit: initial_head,
            step_count: 2,
            files_touched: vec!["squash.txt".to_string()],
            ..Default::default()
        })
        .unwrap();
    seed_interaction_turn(dir.path(), "pc-squash", "pc-squash-turn", &["squash.txt"]);

    fs::write(dir.path().join("squash.txt"), "first\n").unwrap();
    git_ok(dir.path(), &["add", "squash.txt"]);
    git_ok(dir.path(), &["commit", "-m", "first commit"]);

    fs::write(dir.path().join("squash.txt"), "second\n").unwrap();
    git_ok(dir.path(), &["add", "squash.txt"]);
    git_ok(dir.path(), &["commit", "-m", "second commit"]);

    git_ok(dir.path(), &["reset", "--soft", "HEAD~2"]);
    git_ok(dir.path(), &["commit", "-m", "squashed commit"]);
    let squashed_head = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();

    ManualCommitStrategy::new(dir.path()).post_commit().unwrap();

    let checkpoint_id = query_commit_checkpoint_id(dir.path(), &squashed_head)
        .expect("post_commit should map squashed HEAD to a generated checkpoint ID");
    assert!(
        read_committed(dir.path(), &checkpoint_id)
            .unwrap()
            .is_some(),
        "post_commit should persist checkpoint content for squashed commit mapping"
    );

    let loaded = backend.load_session("pc-squash").unwrap().unwrap();
    assert_eq!(
        loaded.step_count, 0,
        "squash commit should condense pending session state"
    );
    assert!(
        loaded.files_touched.is_empty(),
        "files_touched should be reset after squash condensation"
    );
}

#[test]
pub(crate) fn post_commit_without_checkpoint_updates_active_base_commit() {
    let dir = tempfile::tempdir().unwrap();
    let head_before = setup_git_repo(&dir);
    let backend = session_backend(dir.path());
    backend
        .save_session(&SessionState {
            session_id: "pc-no-checkpoint".to_string(),
            phase: crate::host::checkpoints::session::phase::SessionPhase::Active,
            base_commit: head_before.clone(),
            ..Default::default()
        })
        .unwrap();

    // Create a regular commit.
    fs::write(dir.path().join("plain.txt"), "plain").unwrap();
    git_command()
        .args(["add", "."])
        .current_dir(dir.path())
        .output()
        .unwrap();
    git_command()
        .args(["commit", "-m", "plain commit"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    let new_head = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();
    assert_ne!(head_before, new_head);

    let strategy = ManualCommitStrategy::new(dir.path());
    strategy.post_commit().unwrap();

    let loaded = backend.load_session("pc-no-checkpoint").unwrap().unwrap();
    assert_eq!(
        loaded.base_commit, new_head,
        "base_commit should advance when post-commit sees no checkpoint mapping"
    );
    assert_eq!(
        loaded.phase,
        crate::host::checkpoints::session::phase::SessionPhase::Active,
        "phase should remain active on no-checkpoint commits"
    );
}

#[test]
pub(crate) fn post_commit_skips_already_mapped_head() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let backend = session_backend(dir.path());
    backend
        .save_session(&SessionState {
            session_id: "pc-skip-mapped".to_string(),
            phase: SessionPhase::Active,
            base_commit: head,
            step_count: 1,
            files_touched: vec!["mapped.txt".to_string()],
            ..Default::default()
        })
        .unwrap();
    seed_interaction_turn(
        dir.path(),
        "pc-skip-mapped",
        "pc-skip-mapped-turn",
        &["mapped.txt"],
    );

    fs::write(dir.path().join("mapped.txt"), "first").unwrap();
    git_ok(dir.path(), &["add", "mapped.txt"]);
    git_ok(dir.path(), &["commit", "-m", "first mapped commit"]);
    let head_sha = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();

    let strategy = ManualCommitStrategy::new(dir.path());
    strategy.post_commit().unwrap();
    assert_eq!(
        query_commit_checkpoint_count(dir.path(), &head_sha),
        1,
        "first post_commit should create one commit mapping"
    );

    let mut resumed = backend.load_session("pc-skip-mapped").unwrap().unwrap();
    resumed.phase = SessionPhase::Active;
    resumed.step_count = 1;
    resumed.files_touched = vec!["mapped.txt".to_string()];
    backend.save_session(&resumed).unwrap();

    strategy.post_commit().unwrap();

    let loaded = backend.load_session("pc-skip-mapped").unwrap().unwrap();
    assert_eq!(
        loaded.step_count, 1,
        "already-mapped HEAD should be ignored by post_commit"
    );
    assert_eq!(
        query_commit_checkpoint_count(dir.path(), &head_sha),
        1,
        "post_commit should not add duplicate mappings for the same HEAD commit"
    );
}

#[test]
pub(crate) fn post_commit_without_checkpoint_updates_active_base_commit_during_rebase() {
    let dir = tempfile::tempdir().unwrap();
    let head_before = setup_git_repo(&dir);
    let backend = session_backend(dir.path());
    backend
        .save_session(&SessionState {
            session_id: "pc-no-checkpoint-rebase".to_string(),
            phase: SessionPhase::Active,
            base_commit: head_before.clone(),
            ..Default::default()
        })
        .unwrap();

    fs::create_dir_all(dir.path().join(".git").join("rebase-merge")).unwrap();

    // Create a regular commit.
    fs::write(dir.path().join("plain-rebase.txt"), "plain").unwrap();
    git_command()
        .args(["add", "."])
        .current_dir(dir.path())
        .output()
        .unwrap();
    git_command()
        .args(["commit", "-m", "plain commit during rebase"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    let new_head = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();
    assert_ne!(head_before, new_head);

    ManualCommitStrategy::new(dir.path()).post_commit().unwrap();

    let loaded = backend
        .load_session("pc-no-checkpoint-rebase")
        .unwrap()
        .unwrap();
    assert_eq!(
        loaded.base_commit, new_head,
        "base_commit should advance even when rebase markers are present"
    );
    assert_eq!(loaded.phase, SessionPhase::Active);
}

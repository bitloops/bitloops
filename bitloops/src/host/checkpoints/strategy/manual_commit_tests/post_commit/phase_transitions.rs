use super::*;

use super::helpers::commit_file;

#[test]
pub(crate) fn post_commit_active_session_condenses_immediately() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let backend = session_backend(dir.path());
    backend
        .save_session(&SessionState {
            session_id: "pc-active".to_string(),
            phase: crate::host::checkpoints::session::phase::SessionPhase::Active,
            base_commit: head,
            step_count: 2,
            files_touched: vec!["active.txt".to_string()],
            ..Default::default()
        })
        .unwrap();

    commit_file(dir.path(), "active.txt", "active.txt content");
    let head_sha = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();
    ManualCommitStrategy::new(dir.path()).post_commit().unwrap();

    let loaded = backend.load_session("pc-active").unwrap().unwrap();
    assert_eq!(
        loaded.phase,
        crate::host::checkpoints::session::phase::SessionPhase::Active
    );
    assert_eq!(loaded.step_count, 0, "active session should be condensed");
    let checkpoint_id = query_commit_checkpoint_id(dir.path(), &head_sha)
        .expect("post_commit should map active commit to a checkpoint");
    assert!(
        read_committed(dir.path(), &checkpoint_id)
            .unwrap()
            .is_some(),
        "condensation should persist committed checkpoint content"
    );
    assert!(
        run_git(dir.path(), &["rev-parse", "bitloops/checkpoints/v1"]).is_err(),
        "condensation should not materialize metadata branch"
    );
}

#[test]
pub(crate) fn post_commit_active_session_records_turn_checkpoint_ids() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let backend = session_backend(dir.path());
    backend
        .save_session(&SessionState {
            session_id: "pc-active-turn".to_string(),
            phase: SessionPhase::Active,
            base_commit: head,
            step_count: 1,
            files_touched: vec!["index.html".to_string()],
            ..Default::default()
        })
        .unwrap();

    commit_file(dir.path(), "index.html", "index.html content");
    let head_sha = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();
    ManualCommitStrategy::new(dir.path()).post_commit().unwrap();

    let checkpoint_id = query_commit_checkpoint_id(dir.path(), &head_sha)
        .expect("post_commit should map active commit to a checkpoint");
    let loaded = backend.load_session("pc-active-turn").unwrap().unwrap();
    assert_eq!(
        loaded.turn_checkpoint_ids,
        vec![checkpoint_id],
        "ACTIVE post-commit should record checkpoint IDs for stop-time finalization"
    );
}

#[test]
pub(crate) fn post_commit_idle_session_condenses() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let backend = session_backend(dir.path());
    backend
        .save_session(&SessionState {
            session_id: "pc-idle".to_string(),
            phase: crate::host::checkpoints::session::phase::SessionPhase::Idle,
            base_commit: head,
            step_count: 1,
            files_touched: vec!["idle.txt".to_string()],
            ..Default::default()
        })
        .unwrap();

    commit_file(dir.path(), "idle.txt", "idle.txt content");
    ManualCommitStrategy::new(dir.path()).post_commit().unwrap();

    let loaded = backend.load_session("pc-idle").unwrap().unwrap();
    assert_eq!(loaded.step_count, 0);
    assert!(
        loaded.files_touched.is_empty(),
        "files_touched should be reset"
    );
}

#[test]
pub(crate) fn post_commit_rebase_during_active_skips_transition() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let backend = session_backend(dir.path());
    backend
        .save_session(&SessionState {
            session_id: "pc-rebase".to_string(),
            phase: crate::host::checkpoints::session::phase::SessionPhase::Active,
            base_commit: head,
            step_count: 3,
            files_touched: vec!["rebase.txt".to_string()],
            ..Default::default()
        })
        .unwrap();

    fs::create_dir_all(dir.path().join(".git").join("rebase-merge")).unwrap();
    commit_file(dir.path(), "rebase.txt", "rebase.txt content");

    ManualCommitStrategy::new(dir.path()).post_commit().unwrap();

    let loaded = backend.load_session("pc-rebase").unwrap().unwrap();
    assert_eq!(
        loaded.step_count, 3,
        "during rebase post-commit should be a no-op for session state"
    );
    assert!(
        run_git(dir.path(), &["rev-parse", "bitloops/checkpoints/v1"]).is_err(),
        "during rebase no condensation metadata branch should be written"
    );
}

#[test]
pub(crate) fn post_commit_files_touched_resets_after_condensation() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let backend = session_backend(dir.path());
    backend
        .save_session(&SessionState {
            session_id: "pc-files".to_string(),
            phase: crate::host::checkpoints::session::phase::SessionPhase::Idle,
            base_commit: head,
            step_count: 1,
            files_touched: vec!["f1.rs".to_string(), "f2.rs".to_string()],
            ..Default::default()
        })
        .unwrap();

    fs::write(dir.path().join("f1.rs"), "f1").unwrap();
    fs::write(dir.path().join("f2.rs"), "f2").unwrap();
    git_ok(dir.path(), &["add", "f1.rs", "f2.rs"]);
    git_ok(dir.path(), &["commit", "-m", "test commit"]);
    ManualCommitStrategy::new(dir.path()).post_commit().unwrap();
    let loaded = backend.load_session("pc-files").unwrap().unwrap();
    assert!(loaded.files_touched.is_empty());
}

#[test]
pub(crate) fn handle_turn_end_finalizes_and_clears_turn_checkpoint_ids() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let backend = session_backend(dir.path());
    let transcript_path = dir.path().join("live-transcript.jsonl");
    fs::write(
        &transcript_path,
        "{\"type\":\"user\",\"message\":{\"content\":\"old prompt\"}}\n",
    )
    .unwrap();

    backend
        .save_session(&SessionState {
            session_id: "turn-end-session".to_string(),
            phase: SessionPhase::Active,
            base_commit: head,
            step_count: 1,
            files_touched: vec!["turn-end.txt".to_string()],
            transcript_path: transcript_path.to_string_lossy().to_string(),
            ..Default::default()
        })
        .unwrap();

    commit_file(dir.path(), "turn-end.txt", "turn-end.txt content");
    let head_sha = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();
    let strategy = ManualCommitStrategy::new(dir.path());
    strategy.post_commit().unwrap();
    let checkpoint_id = query_commit_checkpoint_id(dir.path(), &head_sha)
        .expect("post_commit should map turn-end commit to a checkpoint");

    // Update the live transcript so turn-end finalization has richer content to persist.
    let new_transcript = "{\"type\":\"user\",\"message\":{\"content\":\"latest prompt\"}}\n\
{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"latest answer\"}]}}\n";
    fs::write(&transcript_path, new_transcript).unwrap();

    let mut state = backend.load_session("turn-end-session").unwrap().unwrap();
    assert_eq!(state.turn_checkpoint_ids.len(), 1);
    state
        .turn_checkpoint_ids
        .push("invalid-checkpoint".to_string());

    strategy.handle_turn_end(&mut state).unwrap();
    assert!(
        state.turn_checkpoint_ids.is_empty(),
        "turn checkpoint IDs should be cleared even if one update fails"
    );

    let committed = read_session_content(dir.path(), &checkpoint_id, 0)
        .expect("read checkpoint session after turn-end")
        .transcript;
    assert!(
        committed.contains("latest answer"),
        "turn-end should replace provisional transcript with full transcript"
    );
}

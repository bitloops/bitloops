#[test]
fn get_git_author_from_repo_global_fallback() {
    let home = tempfile::tempdir().unwrap();
    with_env_vars(
        &[
            ("HOME", Some(home.path().to_string_lossy().as_ref())),
            (ALLOW_HOST_GIT_CONFIG_ENV, Some("1")),
        ],
        || {
        fs::write(
            home.path().join(".gitconfig"),
            "[user]\n\tname = Global Author\n\temail = global@test.com\n",
        )
        .unwrap();

        let dir = tempfile::tempdir().unwrap();
        setup_git_repo(&dir);
        run_git(dir.path(), &["config", "--unset", "user.name"]).ok();
        run_git(dir.path(), &["config", "--unset", "user.email"]).ok();

        let author = get_git_author_from_repo(dir.path());
        assert!(
            author.is_ok(),
            "expected global git config fallback, got {author:?}"
        );
        let (name, email) = author.unwrap();
        assert_eq!(name, "Global Author");
        assert_eq!(email, "global@test.com");
    },
    );
}

#[test]
fn get_git_author_from_repo_no_config() {
    let home = tempfile::tempdir().unwrap();
    with_env_vars(
        &[
            ("HOME", Some(home.path().to_string_lossy().as_ref())),
            (ALLOW_HOST_GIT_CONFIG_ENV, Some("1")),
        ],
        || {
        let dir = tempfile::tempdir().unwrap();
        setup_git_repo(&dir);
        run_git(dir.path(), &["config", "--unset", "user.name"]).ok();
        run_git(dir.path(), &["config", "--unset", "user.email"]).ok();

        let author = get_git_author_from_repo(dir.path());
        assert!(
            author.is_ok(),
            "expected defaults when no git config exists, got {author:?}"
        );
        let (name, email) = author.unwrap();
        assert_eq!(name, "Unknown");
        assert_eq!(email, "unknown@local");
    },
    );
}

#[test]
fn prepare_commit_msg_is_noop_even_with_active_session() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    // Create an active session.
    let backend = LocalFileBackend::new(dir.path());
    let state = SessionState {
        session_id: "sa1".to_string(),
        phase: crate::host::checkpoints::session::phase::SessionPhase::Active,
        base_commit: "abc1234".to_string(),
        ..Default::default()
    };
    backend.save_session(&state).unwrap();

    let strategy = ManualCommitStrategy::new(dir.path());
    let msg_file = dir.path().join("COMMIT_EDITMSG");
    let original = "fix: my change\n";
    fs::write(&msg_file, original).unwrap();

    strategy.prepare_commit_msg(&msg_file, None).unwrap();

    let content = fs::read_to_string(&msg_file).unwrap();
    assert_eq!(content, original, "commit message should be unchanged");
    assert!(
        !content.contains(CHECKPOINT_TRAILER_KEY),
        "no checkpoint trailer should be injected: {content}"
    );
}

#[test]
fn add_checkpoint_trailer_no_comment() {
    let msg = "feat: implement parser\n";
    let out = add_checkpoint_trailer(msg, "abc123def456");
    assert!(out.contains("feat: implement parser"));
    assert!(out.contains("Bitloops-Checkpoint: abc123def456"));
}

#[test]
fn add_checkpoint_trailer_with_comment_has_comment() {
    let msg = "feat: implement parser\n\nDetailed body line\n";
    let out = add_checkpoint_trailer(msg, "abc123def456");
    assert!(out.contains("Detailed body line"));
    assert!(out.contains("Bitloops-Checkpoint: abc123def456"));
}

#[test]
fn add_checkpoint_trailer_with_comment_no_prompt() {
    let msg = "";
    let out = add_checkpoint_trailer(msg, "abc123def456");
    assert!(out.contains("Bitloops-Checkpoint: abc123def456"));
}

#[test]
fn add_checkpoint_trailer_conventional_commit_subject() {
    let msg = "fix(auth): handle nil token\n";
    let out = add_checkpoint_trailer(msg, "abc123def456");
    assert!(out.starts_with("fix(auth): handle nil token"));
    assert!(out.contains("Bitloops-Checkpoint: abc123def456"));
}

#[test]
fn add_checkpoint_trailer_existing_trailers() {
    let msg = "feat: update\n\nSigned-off-by: Dev <dev@test.com>\n";
    let out = add_checkpoint_trailer(msg, "abc123def456");
    assert!(out.contains("Signed-off-by: Dev <dev@test.com>"));
    assert!(out.contains("Bitloops-Checkpoint: abc123def456"));
}

#[test]
fn prepare_commit_msg_skips_merge() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    let strategy = ManualCommitStrategy::new(dir.path());
    let msg_file = dir.path().join("MERGE_MSG");
    let original = "Merge branch 'feature'\n";
    fs::write(&msg_file, original).unwrap();

    strategy
        .prepare_commit_msg(&msg_file, Some("merge"))
        .unwrap();

    let content = fs::read_to_string(&msg_file).unwrap();
    assert_eq!(
        content, original,
        "merge commit message should be unchanged"
    );
}

#[test]
fn prepare_commit_msg_is_noop_for_amend_source() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    let strategy = ManualCommitStrategy::new(dir.path());
    let msg_file = dir.path().join("COMMIT_EDITMSG");
    let existing_msg = "fix: my change\n\nBitloops-Checkpoint: abcdef123456\n";
    fs::write(&msg_file, existing_msg).unwrap();

    strategy
        .prepare_commit_msg(&msg_file, Some("commit"))
        .unwrap();

    let content = fs::read_to_string(&msg_file).unwrap();
    assert_eq!(content, existing_msg, "amend message should be unchanged");
}

#[test]
fn prepare_commit_msg_noop_no_session() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    // No sessions exist.
    let strategy = ManualCommitStrategy::new(dir.path());
    let msg_file = dir.path().join("COMMIT_EDITMSG");
    let original = "chore: no session active\n";
    fs::write(&msg_file, original).unwrap();

    strategy.prepare_commit_msg(&msg_file, None).unwrap();

    let content = fs::read_to_string(&msg_file).unwrap();
    assert_eq!(
        content, original,
        "message should be unchanged when no sessions exist"
    );
}

#[test]
fn prepare_commit_msg_is_noop_for_idle_sessions_without_pending_steps() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    let backend = LocalFileBackend::new(dir.path());
    backend
        .save_session(&SessionState {
            session_id: "idle-no-steps".to_string(),
            phase: crate::host::checkpoints::session::phase::SessionPhase::Idle,
            step_count: 0,
            ..Default::default()
        })
        .unwrap();

    let strategy = ManualCommitStrategy::new(dir.path());
    let msg_file = dir.path().join("COMMIT_EDITMSG");
    let original = "docs: unrelated follow-up commit\n";
    fs::write(&msg_file, original).unwrap();

    strategy.prepare_commit_msg(&msg_file, None).unwrap();

    let content = fs::read_to_string(&msg_file).unwrap();
    assert_eq!(
        content, original,
        "idle sessions with no pending steps should keep message unchanged"
    );
}

#[test]
fn commit_msg_is_noop_for_trailer_only_message() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    let strategy = ManualCommitStrategy::new(dir.path());
    let msg_file = dir.path().join("COMMIT_EDITMSG");
    let original = "Bitloops-Checkpoint: abcdef123456\n";
    fs::write(&msg_file, original).unwrap();

    strategy.commit_msg(&msg_file).unwrap();

    let content = fs::read_to_string(&msg_file).unwrap();
    assert_eq!(content, original, "commit message should be unchanged");
}

#[test]
fn commit_msg_is_noop_for_real_message() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    let strategy = ManualCommitStrategy::new(dir.path());
    let msg_file = dir.path().join("COMMIT_EDITMSG");
    let msg = "fix: real change\n\nBitloops-Checkpoint: abcdef123456\n";
    fs::write(&msg_file, msg).unwrap();

    strategy.commit_msg(&msg_file).unwrap();

    let content = fs::read_to_string(&msg_file).unwrap();
    assert_eq!(content, msg, "commit message should be unchanged");
}

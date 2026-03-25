use super::*;

#[test]
pub(crate) fn subtract_files_compat() {
    let files_touched = vec!["a.rs".to_string(), "b.rs".to_string(), "c.rs".to_string()];
    let committed_files = std::collections::HashSet::from(["a.rs".to_string(), "c.rs".to_string()]);
    let remaining = subtract_files_by_name(&files_touched, &committed_files);
    assert_eq!(remaining, vec!["b.rs".to_string()]);
}

#[test]
pub(crate) fn files_changed_in_commit_compat() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    fs::write(dir.path().join("changed.rs"), "package changed").unwrap();
    git_ok(dir.path(), &["add", "changed.rs"]);
    git_ok(dir.path(), &["commit", "-m", "change tracked file"]);
    let head = git_ok(dir.path(), &["rev-parse", "HEAD"]);
    let changed = files_changed_in_commit(dir.path(), &head).unwrap();
    assert!(changed.contains("changed.rs"));
}

#[test]
pub(crate) fn files_changed_in_commit_initial_commit_compat() {
    let dir = tempfile::tempdir().unwrap();
    setup_empty_git_repo(&dir);
    fs::write(dir.path().join("initial.rs"), "package initial").unwrap();
    git_ok(dir.path(), &["add", "initial.rs"]);
    git_ok(dir.path(), &["commit", "-m", "initial commit"]);
    let head = git_ok(dir.path(), &["rev-parse", "HEAD"]);
    let changed = files_changed_in_commit(dir.path(), &head).unwrap();
    assert!(changed.contains("initial.rs"));
}

#[test]
pub(crate) fn post_commit_no_head_is_noop() {
    let dir = tempfile::tempdir().unwrap();
    setup_empty_git_repo(&dir);

    let strategy = ManualCommitStrategy::new(dir.path());
    let result = strategy.post_commit();
    assert!(
        result.is_ok(),
        "post_commit should no-op when HEAD is missing: {result:?}"
    );
}

#[test]
pub(crate) fn update_base_commit_no_head_is_noop() {
    let dir = tempfile::tempdir().unwrap();
    setup_empty_git_repo(&dir);

    let strategy = ManualCommitStrategy::new(dir.path());
    let backend = session_backend(dir.path());
    backend
        .save_session(&SessionState {
            session_id: "s_update_base_no_head".to_string(),
            phase: crate::host::checkpoints::session::phase::SessionPhase::Active,
            base_commit: "deadbeef".to_string(),
            ..Default::default()
        })
        .unwrap();

    let result = strategy.update_base_commit_for_active_sessions();
    assert!(
        result.is_ok(),
        "update_base_commit_for_active_sessions should no-op when HEAD is missing: {result:?}"
    );

    let loaded = backend
        .load_session("s_update_base_no_head")
        .unwrap()
        .unwrap();
    assert_eq!(loaded.base_commit, "deadbeef");
}

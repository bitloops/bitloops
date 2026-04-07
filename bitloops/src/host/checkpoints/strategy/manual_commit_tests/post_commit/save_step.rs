use super::*;

#[test]
pub(crate) fn save_step_empty_base_commit_recovery() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let backend = session_backend(dir.path());
    backend
        .save_session(&SessionState {
            session_id: "save-recovery".to_string(),
            base_commit: String::new(),
            ..Default::default()
        })
        .unwrap();

    let strategy = ManualCommitStrategy::new(dir.path());
    let ctx = StepContext {
        session_id: "save-recovery".to_string(),
        commit_message: "checkpoint".to_string(),
        metadata: None,
        new_files: vec![],
        modified_files: vec![],
        deleted_files: vec![],
        author_name: "Test".to_string(),
        author_email: "test@test.com".to_string(),
        ..Default::default()
    };
    strategy.save_step(&ctx).unwrap();
    let loaded = backend.load_session("save-recovery").unwrap().unwrap();
    assert!(!loaded.base_commit.is_empty());
}

#[test]
pub(crate) fn save_step_uses_ctx_agent_type_when_no_session_state() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let strategy = ManualCommitStrategy::new(dir.path());
    let backend = session_backend(dir.path());

    strategy
        .save_step(&StepContext {
            session_id: "save-agent-none".to_string(),
            agent_type: "gemini".to_string(),
            commit_message: "checkpoint".to_string(),
            ..Default::default()
        })
        .unwrap();

    let loaded = backend.load_session("save-agent-none").unwrap().unwrap();
    assert_eq!(loaded.agent_type, "gemini");
    assert_eq!(loaded.turn_id.len(), 12);
    assert!(
        loaded
            .turn_id
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
        "turn_id should be 12-char lowercase hex: {}",
        loaded.turn_id
    );
}

#[test]
pub(crate) fn save_step_uses_ctx_agent_type_when_partial_state() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let backend = session_backend(dir.path());
    backend
        .save_session(&SessionState {
            session_id: "save-agent-partial".to_string(),
            base_commit: String::new(),
            agent_type: String::new(),
            ..Default::default()
        })
        .unwrap();

    let strategy = ManualCommitStrategy::new(dir.path());
    strategy
        .save_step(&StepContext {
            session_id: "save-agent-partial".to_string(),
            agent_type: "gemini".to_string(),
            commit_message: "checkpoint".to_string(),
            ..Default::default()
        })
        .unwrap();

    let loaded = backend.load_session("save-agent-partial").unwrap().unwrap();
    assert_eq!(loaded.agent_type, "gemini");
    assert_eq!(loaded.turn_id.len(), 12);
    assert!(
        loaded
            .turn_id
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
        "turn_id should be 12-char lowercase hex: {}",
        loaded.turn_id
    );
}

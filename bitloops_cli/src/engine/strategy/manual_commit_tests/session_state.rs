#[test]
fn save_step_persists_temporary_checkpoint_without_shadow_branch() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);

    // Modify a file so there's something to snapshot.
    fs::write(dir.path().join("file.txt"), "hello").unwrap();

    let strategy = ManualCommitStrategy::new(dir.path());

    // Pre-create session state so save_step can load it.
    let backend = LocalFileBackend::new(dir.path());
    let state = SessionState {
        session_id: "s1".to_string(),
        base_commit: head.clone(),
        phase: crate::engine::session::phase::SessionPhase::Active,
        ..Default::default()
    };
    backend.save_session(&state).unwrap();

    let ctx = StepContext {
        session_id: "s1".to_string(),
        modified_files: vec![],
        new_files: vec!["file.txt".to_string()],
        deleted_files: vec![],
        metadata_dir: String::new(),
        metadata_dir_abs: String::new(),
        commit_message: String::new(),
        transcript_path: String::new(),
        author_name: String::new(),
        author_email: String::new(),
        agent_type: String::new(),
        step_transcript_identifier: String::new(),
        step_transcript_start: 0,
        token_usage: None,
    };
    strategy.save_step(&ctx).unwrap();

    // New flow writes a DB-backed temporary checkpoint tree and does not create a shadow branch.
    let shadow = shadow_branch_ref(&head, "");
    let result = run_git(dir.path(), &["rev-parse", &shadow]);
    assert!(
        result.is_err(),
        "shadow branch should not be created after save_step"
    );

    let tree_hash = latest_temporary_tree_hash(dir.path(), "s1")
        .expect("latest temporary checkpoint tree hash should be persisted");
    let file_content = run_git(dir.path(), &["show", &format!("{tree_hash}:file.txt")]).unwrap();
    assert_eq!(file_content, "hello");
}

#[test]
fn save_step_checkpoint_tree_has_modified_file() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);

    fs::write(dir.path().join("src.rs"), "fn main() {}").unwrap();

    let strategy = ManualCommitStrategy::new(dir.path());
    let backend = LocalFileBackend::new(dir.path());
    let state = SessionState {
        session_id: "s2".to_string(),
        base_commit: head.clone(),
        phase: crate::engine::session::phase::SessionPhase::Active,
        ..Default::default()
    };
    backend.save_session(&state).unwrap();

    let ctx = StepContext {
        session_id: "s2".to_string(),
        modified_files: vec![],
        new_files: vec!["src.rs".to_string()],
        deleted_files: vec![],
        metadata_dir: String::new(),
        metadata_dir_abs: String::new(),
        commit_message: String::new(),
        transcript_path: String::new(),
        author_name: String::new(),
        author_email: String::new(),
        agent_type: String::new(),
        step_transcript_identifier: String::new(),
        step_transcript_start: 0,
        token_usage: None,
    };
    strategy.save_step(&ctx).unwrap();

    // Check file exists in the latest temporary checkpoint tree.
    let tree_hash = latest_temporary_tree_hash(dir.path(), "s2")
        .expect("latest temporary checkpoint tree hash should exist");
    let result = run_git(dir.path(), &["ls-tree", &tree_hash, "src.rs"]);
    assert!(
        result.is_ok(),
        "src.rs should be in temporary checkpoint tree"
    );
    assert!(result.unwrap().contains("src.rs"));
}

#[test]
fn save_step_skips_when_no_changes() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);

    fs::write(dir.path().join("file.txt"), "hello").unwrap();

    let strategy = ManualCommitStrategy::new(dir.path());
    let backend = LocalFileBackend::new(dir.path());
    let state = SessionState {
        session_id: "s3".to_string(),
        base_commit: head.clone(),
        phase: crate::engine::session::phase::SessionPhase::Active,
        ..Default::default()
    };
    backend.save_session(&state).unwrap();

    let ctx = StepContext {
        session_id: "s3".to_string(),
        modified_files: vec![],
        new_files: vec!["file.txt".to_string()],
        deleted_files: vec![],
        metadata_dir: String::new(),
        metadata_dir_abs: String::new(),
        commit_message: String::new(),
        transcript_path: String::new(),
        author_name: String::new(),
        author_email: String::new(),
        agent_type: String::new(),
        step_transcript_identifier: String::new(),
        step_transcript_start: 0,
        token_usage: None,
    };

    strategy.save_step(&ctx).unwrap();
    let s1 = backend.load_session("s3").unwrap().unwrap();
    let count1 = s1.step_count;

    // Second call with same context — tree is identical → skip.
    strategy.save_step(&ctx).unwrap();
    let s2 = backend.load_session("s3").unwrap().unwrap();

    assert_eq!(
        s2.step_count, count1,
        "step_count should not increase for identical tree"
    );
}

#[test]
fn save_step_increments_step_count() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);

    fs::write(dir.path().join("a.txt"), "a").unwrap();

    let strategy = ManualCommitStrategy::new(dir.path());
    let backend = LocalFileBackend::new(dir.path());
    let state = SessionState {
        session_id: "s4".to_string(),
        base_commit: head.clone(),
        phase: crate::engine::session::phase::SessionPhase::Active,
        step_count: 0,
        ..Default::default()
    };
    backend.save_session(&state).unwrap();

    let ctx = StepContext {
        session_id: "s4".to_string(),
        modified_files: vec![],
        new_files: vec!["a.txt".to_string()],
        deleted_files: vec![],
        metadata_dir: String::new(),
        metadata_dir_abs: String::new(),
        commit_message: String::new(),
        transcript_path: String::new(),
        author_name: String::new(),
        author_email: String::new(),
        agent_type: String::new(),
        step_transcript_identifier: String::new(),
        step_transcript_start: 0,
        token_usage: None,
    };
    strategy.save_step(&ctx).unwrap();

    let loaded = backend.load_session("s4").unwrap().unwrap();
    assert_eq!(
        loaded.step_count, 1,
        "step_count should be 1 after first save_step"
    );
}

#[test]
fn save_step_sets_base_commit() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);

    fs::write(dir.path().join("b.txt"), "b").unwrap();

    let strategy = ManualCommitStrategy::new(dir.path());
    let backend = LocalFileBackend::new(dir.path());
    let state = SessionState {
        session_id: "s5".to_string(),
        base_commit: head.clone(),
        phase: crate::engine::session::phase::SessionPhase::Active,
        ..Default::default()
    };
    backend.save_session(&state).unwrap();

    let ctx = StepContext {
        session_id: "s5".to_string(),
        modified_files: vec![],
        new_files: vec!["b.txt".to_string()],
        deleted_files: vec![],
        metadata_dir: String::new(),
        metadata_dir_abs: String::new(),
        commit_message: String::new(),
        transcript_path: String::new(),
        author_name: String::new(),
        author_email: String::new(),
        agent_type: String::new(),
        step_transcript_identifier: String::new(),
        step_transcript_start: 0,
        token_usage: None,
    };
    strategy.save_step(&ctx).unwrap();

    let loaded = backend.load_session("s5").unwrap().unwrap();
    assert_eq!(loaded.base_commit, head, "base_commit should equal HEAD");
}

#[test]
fn save_task_step_keeps_existing_base_commit_without_shadow_migration() {
    let dir = tempfile::tempdir().unwrap();
    let base_commit = setup_git_repo(&dir);

    fs::write(dir.path().join("head-advance-task.txt"), "head moved").unwrap();
    git_ok(dir.path(), &["add", "head-advance-task.txt"]);
    git_ok(
        dir.path(),
        &["commit", "-m", "advance head for task checkpoint"],
    );
    let current_head = git_ok(dir.path(), &["rev-parse", "HEAD"]);
    assert_ne!(base_commit, current_head, "HEAD should have advanced");

    let backend = LocalFileBackend::new(dir.path());
    backend
        .save_session(&SessionState {
            session_id: "task-no-migrate".to_string(),
            base_commit: base_commit.clone(),
            phase: SessionPhase::Active,
            ..Default::default()
        })
        .unwrap();

    let strategy = ManualCommitStrategy::new(dir.path());
    strategy
        .save_task_step(&TaskStepContext {
            session_id: "task-no-migrate".to_string(),
            tool_use_id: "toolu_nomigrate".to_string(),
            agent_id: "agent_nomigrate".to_string(),
            checkpoint_uuid: "task-checkpoint-1".to_string(),
            agent_type: AGENT_TYPE_CLAUDE_CODE.to_string(),
            ..Default::default()
        })
        .unwrap();

    let loaded = backend.load_session("task-no-migrate").unwrap().unwrap();
    assert_eq!(
        loaded.base_commit, base_commit,
        "save_task_step should not migrate base_commit via shadow branch logic"
    );
}

#[test]
fn initialize_session_sets_pending_prompt_attribution() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let strategy = ManualCommitStrategy::new(dir.path());
    let backend = LocalFileBackend::new(dir.path());

    strategy
        .initialize_session("attr-pending", AGENT_TYPE_CLAUDE_CODE, "", "initial prompt")
        .unwrap();

    let loaded = backend.load_session("attr-pending").unwrap().unwrap();
    assert!(
        loaded.pending_prompt_attribution.is_some(),
        "turn start should always persist pending prompt attribution"
    );
    assert_eq!(
        loaded
            .pending_prompt_attribution
            .as_ref()
            .map(|pa| pa.checkpoint_number),
        Some(1)
    );
}

#[test]
fn initialize_session_keeps_existing_base_commit_without_shadow_migration() {
    let dir = tempfile::tempdir().unwrap();
    let base_commit = setup_git_repo(&dir);

    fs::write(dir.path().join("head-advance.txt"), "head moved").unwrap();
    git_ok(dir.path(), &["add", "head-advance.txt"]);
    git_ok(dir.path(), &["commit", "-m", "advance head"]);
    let current_head = git_ok(dir.path(), &["rev-parse", "HEAD"]);
    assert_ne!(base_commit, current_head, "HEAD should have advanced");

    let backend = LocalFileBackend::new(dir.path());
    backend
        .save_session(&SessionState {
            session_id: "init-no-migrate".to_string(),
            base_commit: base_commit.clone(),
            phase: SessionPhase::Active,
            ..Default::default()
        })
        .unwrap();

    let strategy = ManualCommitStrategy::new(dir.path());
    strategy
        .initialize_session(
            "init-no-migrate",
            AGENT_TYPE_CLAUDE_CODE,
            "",
            "keep base commit",
        )
        .unwrap();

    let loaded = backend.load_session("init-no-migrate").unwrap().unwrap();
    assert_eq!(
        loaded.base_commit, base_commit,
        "initialize_session should not migrate base_commit via shadow branch logic"
    );
}

#[test]
fn initialize_session_prompt_attribution_uses_latest_temporary_checkpoint_tree_hash() {
    let dir = tempfile::tempdir().unwrap();
    let base_commit = setup_git_repo(&dir);
    let session_id = "attr-latest-temp-tree";

    fs::write(dir.path().join("README.md"), "agent baseline\n").unwrap();
    let metadata_dir_abs = create_checkpoint_metadata_dir(dir.path(), session_id);
    let result = write_temporary(
        dir.path(),
        first_checkpoint_opts(session_id, &base_commit, &metadata_dir_abs),
    )
    .unwrap();
    assert!(!result.skipped);

    // Ensure there is no shadow-branch fallback available.
    let shadow = shadow_branch_ref(&base_commit, "");
    let short_shadow = shadow
        .strip_prefix("refs/heads/")
        .unwrap_or(shadow.as_str())
        .to_string();
    let _ = run_git(dir.path(), &["branch", "-D", &short_shadow]);
    assert!(
        run_git(dir.path(), &["rev-parse", &shadow]).is_err(),
        "shadow branch should be absent so attribution must rely on DB tree hash"
    );

    let strategy = ManualCommitStrategy::new(dir.path());
    strategy
        .initialize_session(session_id, AGENT_TYPE_CLAUDE_CODE, "", "prompt")
        .unwrap();

    let backend = LocalFileBackend::new(dir.path());
    let loaded = backend.load_session(session_id).unwrap().unwrap();
    let pending = loaded
        .pending_prompt_attribution
        .expect("pending prompt attribution should be set");
    assert_eq!(
        pending.user_lines_added, 0,
        "worktree matches latest temporary checkpoint tree, so user_lines_added should be 0"
    );
    assert_eq!(
        pending.user_lines_removed, 0,
        "worktree matches latest temporary checkpoint tree, so user_lines_removed should be 0"
    );
}

#[test]
fn save_step_consumes_pending_prompt_attribution() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    fs::write(dir.path().join("tracked.txt"), "line1\nline2\n").unwrap();

    let strategy = ManualCommitStrategy::new(dir.path());
    let backend = LocalFileBackend::new(dir.path());
    backend
        .save_session(&SessionState {
            session_id: "attr-save".to_string(),
            base_commit: head,
            phase: SessionPhase::Active,
            pending_prompt_attribution: Some(SessionPromptAttribution {
                checkpoint_number: 1,
                user_lines_added: 2,
                user_lines_removed: 0,
                agent_lines_added: 0,
                agent_lines_removed: 0,
                user_added_per_file: BTreeMap::from([("tracked.txt".to_string(), 2)]),
            }),
            ..Default::default()
        })
        .unwrap();

    let ctx = StepContext {
        session_id: "attr-save".to_string(),
        modified_files: vec!["tracked.txt".to_string()],
        new_files: vec![],
        deleted_files: vec![],
        metadata_dir: String::new(),
        metadata_dir_abs: String::new(),
        commit_message: String::new(),
        transcript_path: String::new(),
        author_name: String::new(),
        author_email: String::new(),
        agent_type: AGENT_TYPE_CLAUDE_CODE.to_string(),
        step_transcript_identifier: String::new(),
        step_transcript_start: 0,
        token_usage: None,
    };
    strategy.save_step(&ctx).unwrap();

    let loaded = backend.load_session("attr-save").unwrap().unwrap();
    assert!(
        loaded.pending_prompt_attribution.is_none(),
        "pending attribution should be cleared after checkpoint save"
    );
    assert_eq!(
        loaded.prompt_attributions.len(),
        1,
        "saved checkpoint should append prompt attribution"
    );
    assert_eq!(loaded.prompt_attributions[0].user_lines_added, 2);
}

// New test: save_step includes transcript in the temporary checkpoint tree.
#[test]
fn save_step_includes_transcript_in_checkpoint_tree() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);

    // Write a fake transcript file.
    let transcript_path = dir.path().join("transcript.jsonl");
    fs::write(&transcript_path, r#"{"role":"user","content":"hello"}"#).unwrap();

    fs::write(dir.path().join("changed.txt"), "content").unwrap();

    let strategy = ManualCommitStrategy::new(dir.path());
    let backend = LocalFileBackend::new(dir.path());
    let state = SessionState {
        session_id: "s_transcript".to_string(),
        base_commit: head.clone(),
        phase: crate::engine::session::phase::SessionPhase::Active,
        ..Default::default()
    };
    backend.save_session(&state).unwrap();

    let ctx = StepContext {
        session_id: "s_transcript".to_string(),
        modified_files: vec![],
        new_files: vec!["changed.txt".to_string()],
        deleted_files: vec![],
        metadata_dir: String::new(),
        metadata_dir_abs: String::new(),
        commit_message: String::new(),
        transcript_path: transcript_path.to_string_lossy().to_string(),
        author_name: String::new(),
        author_email: String::new(),
        agent_type: String::new(),
        step_transcript_identifier: String::new(),
        step_transcript_start: 0,
        token_usage: None,
    };
    strategy.save_step(&ctx).unwrap();

    let shadow = shadow_branch_ref(&head, "");
    let shadow_branch = run_git(dir.path(), &["rev-parse", &shadow]);
    assert!(
        shadow_branch.is_err(),
        "save_step should not create a shadow branch"
    );

    // Latest checkpoint tree should contain the transcript metadata files.
    let tree_hash = latest_temporary_tree_hash(dir.path(), "s_transcript")
        .expect("latest temporary checkpoint tree hash should exist");
    let result = run_git(dir.path(), &["ls-tree", "-r", "--name-only", &tree_hash]);
    assert!(result.is_ok(), "temporary checkpoint tree should exist");
    let files = result.unwrap();
    assert!(
        files.contains(".bitloops/metadata/s_transcript/full.jsonl"),
        "checkpoint tree should contain full.jsonl: {files}"
    );
    assert!(
        files.contains(".bitloops/metadata/s_transcript/prompt.txt"),
        "checkpoint tree should contain prompt.txt: {files}"
    );
    assert!(
        files.contains(".bitloops/metadata/s_transcript/summary.txt"),
        "checkpoint tree should contain summary.txt: {files}"
    );
    assert!(
        files.contains(".bitloops/metadata/s_transcript/context.md"),
        "checkpoint tree should contain context.md: {files}"
    );
}

// New test: save_step with untracked directory does not crash.
#[test]
fn save_step_with_untracked_dir_does_not_crash() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);

    // Create an untracked subdirectory (appears as "dir/" in git status --porcelain).
    let sub = dir.path().join("untracked_dir");
    fs::create_dir_all(&sub).unwrap();
    fs::write(sub.join("file.txt"), "content").unwrap();

    let strategy = ManualCommitStrategy::new(dir.path());
    let backend = LocalFileBackend::new(dir.path());
    let state = SessionState {
        session_id: "s_dir".to_string(),
        base_commit: head.clone(),
        phase: crate::engine::session::phase::SessionPhase::Active,
        ..Default::default()
    };
    backend.save_session(&state).unwrap();

    // Pass empty file lists to exercise the working_tree_changes() fallback.
    let ctx = StepContext {
        session_id: "s_dir".to_string(),
        modified_files: vec![],
        new_files: vec![],
        deleted_files: vec![],
        metadata_dir: String::new(),
        metadata_dir_abs: String::new(),
        commit_message: String::new(),
        transcript_path: String::new(),
        author_name: String::new(),
        author_email: String::new(),
        agent_type: String::new(),
        step_transcript_identifier: String::new(),
        step_transcript_start: 0,
        token_usage: None,
    };
    // Should not panic or return an error.
    let result = strategy.save_step(&ctx);
    assert!(
        result.is_ok(),
        "save_step should not crash with untracked directory: {result:?}"
    );
}

#[test]
fn save_step_no_head_is_noop() {
    let dir = tempfile::tempdir().unwrap();
    setup_empty_git_repo(&dir);

    let strategy = ManualCommitStrategy::new(dir.path());
    let ctx = StepContext {
        session_id: "s_no_head".to_string(),
        modified_files: vec![],
        new_files: vec!["file.txt".to_string()],
        deleted_files: vec![],
        metadata_dir: String::new(),
        metadata_dir_abs: String::new(),
        commit_message: String::new(),
        transcript_path: String::new(),
        author_name: String::new(),
        author_email: String::new(),
        agent_type: String::new(),
        step_transcript_identifier: String::new(),
        step_transcript_start: 0,
        token_usage: None,
    };

    let result = strategy.save_step(&ctx);
    assert!(
        result.is_ok(),
        "save_step should no-op when HEAD is missing: {result:?}"
    );
}


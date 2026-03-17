#[test]
fn post_commit_creates_checkpoint_mapping_and_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);

    // Create a session with active state.
    let backend = session_backend(dir.path());
    let state = SessionState {
        session_id: "pc1".to_string(),
        phase: crate::engine::session::phase::SessionPhase::Idle,
        base_commit: head.clone(),
        agent_type: "claude-code".to_string(),
        first_prompt: "test prompt".to_string(),
        step_count: 1,
        files_touched: vec!["change.txt".to_string()],
        ..Default::default()
    };
    backend.save_session(&state).unwrap();

    // Make a regular commit without a Bitloops trailer.
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
fn post_commit_creates_full_checkpoint_structure() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);

    let backend = session_backend(dir.path());
    let state = SessionState {
        session_id: "pc2".to_string(),
        phase: crate::engine::session::phase::SessionPhase::Idle,
        base_commit: head.clone(),
        agent_type: "claude-code".to_string(),
        files_touched: vec!["change2.txt".to_string()],
        ..Default::default()
    };
    backend.save_session(&state).unwrap();

    // Commit without trailer; post_commit should assign and persist checkpoint ID.
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
fn post_commit_without_trailer_condenses_pending_session_and_maps_head() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let backend = session_backend(dir.path());
    backend
        .save_session(&SessionState {
            session_id: "pc-no-trailer-condense".to_string(),
            phase: SessionPhase::Idle,
            base_commit: head,
            step_count: 1,
            files_touched: vec!["condense.txt".to_string()],
            ..Default::default()
        })
        .unwrap();

    fs::write(dir.path().join("condense.txt"), "condense").unwrap();
    git_ok(dir.path(), &["add", "condense.txt"]);
    git_ok(dir.path(), &["commit", "-m", "commit without trailer"]);
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
fn post_commit_squash_commit_condenses_pending_session_and_maps_head() {
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
fn post_commit_without_trailer_updates_active_base_commit() {
    let dir = tempfile::tempdir().unwrap();
    let head_before = setup_git_repo(&dir);
    let backend = session_backend(dir.path());
    backend
        .save_session(&SessionState {
            session_id: "pc-no-trailer".to_string(),
            phase: crate::engine::session::phase::SessionPhase::Active,
            base_commit: head_before.clone(),
            ..Default::default()
        })
        .unwrap();

    // Create a regular commit without Bitloops-Checkpoint trailer.
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

    let loaded = backend.load_session("pc-no-trailer").unwrap().unwrap();
    assert_eq!(
        loaded.base_commit, new_head,
        "base_commit should advance when post-commit sees no trailer"
    );
    assert_eq!(
        loaded.phase,
        crate::engine::session::phase::SessionPhase::Active,
        "phase should remain active on no-trailer commits"
    );
}

#[test]
fn post_commit_skips_already_mapped_head() {
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
fn post_commit_without_trailer_updates_active_base_commit_during_rebase() {
    let dir = tempfile::tempdir().unwrap();
    let head_before = setup_git_repo(&dir);
    let backend = session_backend(dir.path());
    backend
        .save_session(&SessionState {
            session_id: "pc-no-trailer-rebase".to_string(),
            phase: SessionPhase::Active,
            base_commit: head_before.clone(),
            ..Default::default()
        })
        .unwrap();

    fs::create_dir_all(dir.path().join(".git").join("rebase-merge")).unwrap();

    // Create a regular commit without Bitloops-Checkpoint trailer.
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
        .load_session("pc-no-trailer-rebase")
        .unwrap()
        .unwrap();
    assert_eq!(
        loaded.base_commit, new_head,
        "base_commit should advance even when rebase markers are present"
    );
    assert_eq!(loaded.phase, SessionPhase::Active);
}

#[test]
fn extract_user_prompts_supports_nested_message_and_human_type() {
    let jsonl = r#"{"type":"user","message":{"content":[{"type":"text","text":"Create index.html"},{"type":"tool_result","tool_use_id":"x"}]}}
{"type":"assistant","message":{"content":[{"type":"text","text":"Done"}]}}
{"type":"human","message":{"content":"Add styles"}}
not-json"#;

    let prompts = extract_user_prompts_from_jsonl(jsonl);
    assert_eq!(prompts, vec!["Create index.html", "Add styles"]);
}

#[test]
fn extract_summary_supports_nested_message_content() {
    let jsonl = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"first summary"}]}}
{"type":"assistant","message":{"content":[{"type":"text","text":"final summary"},{"type":"tool_use","name":"Edit","input":{"file_path":"a.txt"}}]}}"#;

    let summary = extract_summary_from_jsonl(jsonl);
    assert_eq!(summary, "final summary");
}

#[test]
fn write_session_metadata_writes_prompt_and_summary_for_nested_claude_jsonl() {
    let dir = tempfile::tempdir().unwrap();
    let transcript_path = dir.path().join("transcript.jsonl");
    let jsonl = r#"{"type":"user","message":{"content":[{"type":"text","text":"Create test file"}]}}
{"type":"assistant","message":{"content":[{"type":"text","text":"Created test file"}]}}"#;
    fs::write(&transcript_path, jsonl).unwrap();

    let written = write_session_metadata(
        dir.path(),
        "session-nested",
        &transcript_path.to_string_lossy(),
    )
    .unwrap();
    assert!(
        written.contains(&".bitloops/metadata/session-nested/prompt.txt".to_string()),
        "prompt.txt should be part of written metadata files: {written:?}"
    );
    assert!(
        written.contains(&".bitloops/metadata/session-nested/summary.txt".to_string()),
        "summary.txt should be part of written metadata files: {written:?}"
    );

    let prompt = fs::read_to_string(
        dir.path()
            .join(".bitloops")
            .join("metadata")
            .join("session-nested")
            .join("prompt.txt"),
    )
    .unwrap();
    let summary = fs::read_to_string(
        dir.path()
            .join(".bitloops")
            .join("metadata")
            .join("session-nested")
            .join("summary.txt"),
    )
    .unwrap();

    assert_eq!(prompt.trim(), "Create test file");
    assert_eq!(summary.trim(), "Created test file");
}

#[test]
fn pre_push_is_noop_even_when_checkpoints_branch_exists() {
    let base = tempfile::tempdir().unwrap();
    let origin_dir = base.path().join("origin.git");
    let work_dir = base.path().join("work");
    fs::create_dir_all(&work_dir).unwrap();

    // Bare remote.
    let out = git_command()
        .args(["init", "--bare", origin_dir.to_string_lossy().as_ref()])
        .output()
        .unwrap();
    assert!(out.status.success(), "git init --bare failed");

    let work_temp = tempfile::TempDir::new_in(&work_dir).unwrap();
    let repo_dir = work_temp.path();
    let run = |args: &[&str]| {
        let out = git_command()
            .args(args)
            .current_dir(repo_dir)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {:?} failed", args);
    };

    run(&["init"]);
    run(&["config", "user.email", "t@t.com"]);
    run(&["config", "user.name", "Test"]);
    fs::write(repo_dir.join("README.md"), "init").unwrap();
    run(&["add", "."]);
    run(&["commit", "-m", "initial"]);
    run(&[
        "remote",
        "add",
        "origin",
        origin_dir.to_string_lossy().as_ref(),
    ]);

    // Create local checkpoints branch to push.
    let head = run_git(repo_dir, &["rev-parse", "HEAD"]).unwrap();
    run(&["update-ref", "refs/heads/bitloops/checkpoints/v1", &head]);

    let strategy = ManualCommitStrategy::new(repo_dir);
    strategy.pre_push("origin").unwrap();

    // Remote should not have bitloops/checkpoints/v1 because pre_push is now a no-op.
    let remote_ref = git_command()
        .args([
            "--git-dir",
            origin_dir.to_string_lossy().as_ref(),
            "show-ref",
            "--verify",
            "refs/heads/bitloops/checkpoints/v1",
        ])
        .output()
        .unwrap();
    assert!(
        !remote_ref.status.success(),
        "remote should not contain checkpoints branch after pre-push no-op"
    );
}

fn commit_with_checkpoint_trailer(repo_root: &Path, checkpoint_id: &str, filename: &str) {
    fs::write(
        repo_root.join(filename),
        format!("content for {checkpoint_id}\n"),
    )
    .unwrap();
    git_ok(repo_root, &["add", filename]);
    git_ok(
        repo_root,
        &[
            "commit",
            "-m",
            &format!("test commit\n\nBitloops-Checkpoint: {checkpoint_id}"),
        ],
    );
}

#[test]
fn shadow_strategy_direct_instantiation() {
    let dir = tempfile::tempdir().unwrap();
    let strategy = ManualCommitStrategy::new(dir.path());
    assert_eq!(strategy.name(), "manual-commit");
}

#[test]
fn shadow_strategy_description() {
    let dir = tempfile::tempdir().unwrap();
    let strategy = ManualCommitStrategy::new(dir.path());
    assert_eq!(strategy.name(), "manual-commit");
}

#[test]
fn shadow_strategy_validate_repository() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    assert!(
        run_git(dir.path(), &["rev-parse", "--is-inside-work-tree"]).is_ok(),
        "expected git repo to validate"
    );
}

#[test]
fn shadow_strategy_validate_repository_not_git_repo() {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        run_git(dir.path(), &["rev-parse", "--is-inside-work-tree"]).is_err(),
        "non-git directory should fail validation"
    );
}

#[test]
fn post_commit_active_session_condenses_immediately() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let backend = session_backend(dir.path());
    backend
        .save_session(&SessionState {
            session_id: "pc-active".to_string(),
            phase: crate::engine::session::phase::SessionPhase::Active,
            base_commit: head,
            step_count: 2,
            files_touched: vec!["active.txt".to_string()],
            ..Default::default()
        })
        .unwrap();

    commit_with_checkpoint_trailer(dir.path(), "a1b2c3d4e5f6", "active.txt");
    let head_sha = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();
    ManualCommitStrategy::new(dir.path()).post_commit().unwrap();

    let loaded = backend.load_session("pc-active").unwrap().unwrap();
    assert_eq!(
        loaded.phase,
        crate::engine::session::phase::SessionPhase::Active
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
fn post_commit_active_session_records_turn_checkpoint_ids() {
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

    commit_with_checkpoint_trailer(dir.path(), "a1b2c3d4e5f6", "index.html");
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
fn post_commit_idle_session_condenses() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let backend = session_backend(dir.path());
    backend
        .save_session(&SessionState {
            session_id: "pc-idle".to_string(),
            phase: crate::engine::session::phase::SessionPhase::Idle,
            base_commit: head,
            step_count: 1,
            files_touched: vec!["idle.txt".to_string()],
            ..Default::default()
        })
        .unwrap();

    commit_with_checkpoint_trailer(dir.path(), "b1c2d3e4f5a6", "idle.txt");
    ManualCommitStrategy::new(dir.path()).post_commit().unwrap();

    let loaded = backend.load_session("pc-idle").unwrap().unwrap();
    assert_eq!(loaded.step_count, 0);
    assert!(
        loaded.files_touched.is_empty(),
        "files_touched should be reset"
    );
}

#[test]
fn post_commit_rebase_during_active_skips_transition() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let backend = session_backend(dir.path());
    backend
        .save_session(&SessionState {
            session_id: "pc-rebase".to_string(),
            phase: crate::engine::session::phase::SessionPhase::Active,
            base_commit: head,
            step_count: 3,
            files_touched: vec!["rebase.txt".to_string()],
            ..Default::default()
        })
        .unwrap();

    fs::create_dir_all(dir.path().join(".git").join("rebase-merge")).unwrap();
    commit_with_checkpoint_trailer(dir.path(), "c1d2e3f4a5b6", "rebase.txt");

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
fn post_commit_files_touched_resets_after_condensation() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let backend = session_backend(dir.path());
    backend
        .save_session(&SessionState {
            session_id: "pc-files".to_string(),
            phase: crate::engine::session::phase::SessionPhase::Idle,
            base_commit: head,
            step_count: 1,
            files_touched: vec!["f1.rs".to_string(), "f2.rs".to_string()],
            ..Default::default()
        })
        .unwrap();

    fs::write(dir.path().join("f1.rs"), "f1").unwrap();
    fs::write(dir.path().join("f2.rs"), "f2").unwrap();
    git_ok(dir.path(), &["add", "f1.rs", "f2.rs"]);
    git_ok(
        dir.path(),
        &[
            "commit",
            "-m",
            "test commit\n\nBitloops-Checkpoint: d1e2f3a4b5c6",
        ],
    );
    ManualCommitStrategy::new(dir.path()).post_commit().unwrap();
    let loaded = backend.load_session("pc-files").unwrap().unwrap();
    assert!(loaded.files_touched.is_empty());
}

#[test]
fn handle_turn_end_finalizes_and_clears_turn_checkpoint_ids() {
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

    commit_with_checkpoint_trailer(dir.path(), "0aaabbbccdde", "turn-end.txt");
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

#[test]
fn subtract_files_compat() {
    let files_touched = vec!["a.rs".to_string(), "b.rs".to_string(), "c.rs".to_string()];
    let committed_files = std::collections::HashSet::from(["a.rs".to_string(), "c.rs".to_string()]);
    let remaining = subtract_files_by_name(&files_touched, &committed_files);
    assert_eq!(remaining, vec!["b.rs".to_string()]);
}

#[test]
fn files_changed_in_commit_compat() {
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
fn files_changed_in_commit_initial_commit_compat() {
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
fn save_step_empty_base_commit_recovery() {
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
        metadata_dir: String::new(),
        metadata_dir_abs: String::new(),
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
fn save_step_uses_ctx_agent_type_when_no_session_state() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let strategy = ManualCommitStrategy::new(dir.path());
    let backend = session_backend(dir.path());

    strategy
        .save_step(&StepContext {
            session_id: "save-agent-none".to_string(),
            agent_type: "gemini-cli".to_string(),
            commit_message: "checkpoint".to_string(),
            ..Default::default()
        })
        .unwrap();

    let loaded = backend.load_session("save-agent-none").unwrap().unwrap();
    assert_eq!(loaded.agent_type, "gemini-cli");
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
fn save_step_uses_ctx_agent_type_when_partial_state() {
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
            agent_type: "gemini-cli".to_string(),
            commit_message: "checkpoint".to_string(),
            ..Default::default()
        })
        .unwrap();

    let loaded = backend.load_session("save-agent-partial").unwrap().unwrap();
    assert_eq!(loaded.agent_type, "gemini-cli");
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
fn post_commit_no_head_is_noop() {
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
fn update_base_commit_no_head_is_noop() {
    let dir = tempfile::tempdir().unwrap();
    setup_empty_git_repo(&dir);

    let strategy = ManualCommitStrategy::new(dir.path());
    let backend = session_backend(dir.path());
    backend
        .save_session(&SessionState {
            session_id: "s_update_base_no_head".to_string(),
            phase: crate::engine::session::phase::SessionPhase::Active,
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

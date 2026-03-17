fn checkpoint_id_path(id: &str) -> String {
    let (a, b) = checkpoint_dir_parts(id);
    if b.is_empty() { a } else { format!("{a}/{b}") }
}

fn read_checkpoint_session_metadata_from_branch(
    repo_root: &Path,
    checkpoint_id: &str,
) -> serde_json::Value {
    read_session_content(repo_root, checkpoint_id, 0)
        .expect("read session content")
        .metadata
}

fn read_checkpoint_top_metadata_from_branch(
    repo_root: &Path,
    checkpoint_id: &str,
) -> serde_json::Value {
    let summary = read_committed(repo_root, checkpoint_id)
        .expect("read committed summary")
        .expect("checkpoint should exist");
    serde_json::to_value(summary).expect("serialize summary")
}

#[test]
fn checkpoint_id_methods() {
    let id = "a1b2c3d4e5f6".to_string();
    assert_eq!(id, "a1b2c3d4e5f6");
    assert!(
        String::new().is_empty(),
        "empty checkpoint id should be empty"
    );
    assert!(
        !id.is_empty(),
        "non-empty checkpoint id should not be empty"
    );
    assert_eq!(checkpoint_id_path(&id), "a1/b2c3d4e5f6");
}

#[test]
fn new_checkpoint_id_validation_via_trailer_parser() {
    let cases = [
        ("a1b2c3d4e5f6", false),
        ("a1b2c3", true),
        ("a1b2c3d4e5f6789012", true),
        ("a1b2c3d4e5gg", true),
        ("A1B2C3D4E5F6", true),
        ("", true),
    ];
    for (input, want_err) in cases {
        let msg = format!("{CHECKPOINT_TRAILER_KEY}: {input}");
        let got = parse_checkpoint_id(&msg);
        if want_err {
            assert!(
                got.is_none(),
                "expected invalid checkpoint id for {input:?}"
            );
        } else {
            assert_eq!(got.as_deref(), Some(input), "valid checkpoint id mismatch");
        }
    }
}

#[test]
fn generate_checkpoint_id_properties() {
    let id = generate_checkpoint_id();
    assert!(
        !id.is_empty(),
        "generated checkpoint id should not be empty"
    );
    assert_eq!(id.len(), 12, "generated checkpoint id should be 12 chars");
    assert!(
        id.chars().all(|c| c.is_ascii_hexdigit()),
        "generated checkpoint id should be hex"
    );
}

#[test]
fn checkpoint_id_path_cases() {
    let cases = [
        ("a1b2c3d4e5f6", "a1/b2c3d4e5f6"),
        ("abcdef123456", "ab/cdef123456"),
        ("", ""),
        ("a", "a"),
        ("ab", "ab"),
        ("abc", "ab/c"),
    ];
    for (input, expected) in cases {
        assert_eq!(
            checkpoint_id_path(input),
            expected,
            "checkpoint path mismatch for {input:?}"
        );
    }
}

#[test]
fn checkpoint_type_values() {
    assert_ne!(
        CheckpointType::Temporary,
        CheckpointType::Committed,
        "temporary and committed checkpoint types should differ"
    );
    let default_type = CheckpointType::default();
    assert_eq!(
        default_type,
        CheckpointType::Temporary,
        "default checkpoint type should be temporary"
    );
}

#[test]
fn checkpoint_info_json_round_trip() {
    let original = CheckpointTopMetadata {
        cli_version: "0.0.3".to_string(),
        checkpoint_id: "a1b2c3d4e5f6".to_string(),
        strategy: "manual-commit".to_string(),
        branch: "main".to_string(),
        checkpoints_count: 2,
        files_touched: vec!["a.rs".to_string()],
        sessions: vec![
            CheckpointSessionRef {
                metadata: "/a1/b2c3d4e5f6/0/metadata.json".to_string(),
                transcript: "/a1/b2c3d4e5f6/0/full.jsonl".to_string(),
                context: "/a1/b2c3d4e5f6/0/context.md".to_string(),
                content_hash: "/a1/b2c3d4e5f6/0/content_hash.txt".to_string(),
                prompt: "/a1/b2c3d4e5f6/0/prompt.txt".to_string(),
            },
            CheckpointSessionRef {
                metadata: "/a1/b2c3d4e5f6/1/metadata.json".to_string(),
                transcript: "/a1/b2c3d4e5f6/1/full.jsonl".to_string(),
                context: "/a1/b2c3d4e5f6/1/context.md".to_string(),
                content_hash: "/a1/b2c3d4e5f6/1/content_hash.txt".to_string(),
                prompt: "/a1/b2c3d4e5f6/1/prompt.txt".to_string(),
            },
        ],
        token_usage: Some(TokenUsageMetadata {
            input_tokens: 150,
            output_tokens: 50,
            api_call_count: 3,
            ..Default::default()
        }),
    };

    let json = serde_json::to_string(&original).unwrap();
    let restored: CheckpointTopMetadata = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.cli_version, "0.0.3");
    assert_eq!(restored.checkpoint_id, "a1b2c3d4e5f6");
    assert_eq!(restored.strategy, "manual-commit");
    assert_eq!(restored.branch, "main");
    assert_eq!(restored.checkpoints_count, 2);
    assert_eq!(restored.files_touched, vec!["a.rs".to_string()]);
    assert_eq!(restored.sessions.len(), 2);
    assert_eq!(
        restored.sessions[0].prompt,
        "/a1/b2c3d4e5f6/0/prompt.txt".to_string()
    );
    assert_eq!(
        restored.sessions[0].content_hash,
        "/a1/b2c3d4e5f6/0/content_hash.txt".to_string()
    );
}

#[test]
fn read_committed_missing_token_usage() {
    let metadata_without_token_usage = serde_json::json!({
        "checkpoint_id": "def456abc123",
        "cli_version": env!("CARGO_PKG_VERSION"),
        "strategy": "manual-commit",
        "checkpoints_count": 1,
        "files_touched": [],
        "sessions": [{
            "metadata": "/de/f456abc123/0/metadata.json",
            "transcript": "/de/f456abc123/0/full.jsonl",
            "context": "/de/f456abc123/0/context.md",
            "content_hash": "/de/f456abc123/0/content_hash.txt",
            "prompt": "/de/f456abc123/0/prompt.txt"
        }]
    })
    .to_string();

    let summary: CheckpointTopMetadata =
        serde_json::from_str(&metadata_without_token_usage).unwrap();
    assert_eq!(summary.checkpoint_id, "def456abc123");
    assert!(summary.token_usage.is_none());
}

#[cfg(unix)]
#[test]
fn write_session_metadata_skips_symlink_transcript() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let sensitive = dir.path().join("sensitive.jsonl");
    fs::write(&sensitive, "SECRET DATA").unwrap();
    let linked = dir.path().join("linked.jsonl");
    symlink(&sensitive, &linked).unwrap();

    let result = write_session_metadata(
        dir.path(),
        "symlink-session",
        linked.to_string_lossy().as_ref(),
    );
    assert!(
        result.is_err(),
        "symlink transcript should be rejected to avoid symlink traversal"
    );
}

#[test]
fn write_committed_agent_field() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let checkpoint_id = "ab1234567890";
    write_committed(
        dir.path(),
        WriteCommittedOptions {
            checkpoint_id: checkpoint_id.to_string(),
            session_id: "agent-field".to_string(),
            strategy: "manual-commit".to_string(),
            agent: AGENT_TYPE_CLAUDE_CODE.to_string(),
            transcript: b"{\"type\":\"assistant\",\"message\":{\"content\":\"agent\"}}\n".to_vec(),
            prompts: Some(vec!["agent prompt".to_string()]),
            context: None,
            checkpoints_count: 1,
            files_touched: vec![],
            token_usage_input: None,
            token_usage_output: None,
            token_usage_api_call_count: None,
            turn_id: String::new(),
            transcript_identifier_at_start: String::new(),
            checkpoint_transcript_start: 0,
            token_usage: None,
            initial_attribution: None,
            author_name: "Test Author".to_string(),
            author_email: "test@example.com".to_string(),
            summary: None,
            is_task: false,
            tool_use_id: String::new(),
            agent_id: String::new(),
            transcript_path: String::new(),
            subagent_transcript_path: String::new(),
        },
    )
    .unwrap();

    let metadata = read_checkpoint_session_metadata_from_branch(dir.path(), checkpoint_id);
    assert_eq!(metadata["agent"], AGENT_TYPE_CLAUDE_CODE);
    assert!(
        metadata.get("checkpoint_number").is_none(),
        "metadata schema does not include checkpoint_number"
    );
    assert!(
        metadata.get("turn_id").is_none(),
        "metadata omits empty turn_id"
    );
    assert!(
        metadata.get("transcript_identifier_at_start").is_none(),
        "metadata omits empty transcript_identifier_at_start"
    );
    assert!(
        run_git(dir.path(), &["rev-parse", paths::METADATA_BRANCH_NAME]).is_err(),
        "write_committed should not materialize metadata branch commits"
    );
}

#[test]
fn write_temporary_deduplication() {
    with_git_env_cleared(|| {
        let dir = tempfile::tempdir().unwrap();
        let head = setup_git_repo(&dir);
        fs::write(dir.path().join("test.rs"), "package main\n").unwrap();

        let backend = LocalFileBackend::new(dir.path());
        backend
            .save_session(&SessionState {
                session_id: "dedup-session".to_string(),
                phase: crate::engine::session::phase::SessionPhase::Active,
                base_commit: head.clone(),
                ..Default::default()
            })
            .unwrap();

        let strategy = ManualCommitStrategy::new(dir.path());
        let ctx = StepContext {
            session_id: "dedup-session".to_string(),
            modified_files: vec!["test.rs".to_string()],
            new_files: vec![],
            deleted_files: vec![],
            metadata_dir: String::new(),
            metadata_dir_abs: String::new(),
            commit_message: "Checkpoint 1".to_string(),
            transcript_path: String::new(),
            author_name: "Test".to_string(),
            author_email: "test@test.com".to_string(),
            agent_type: "claude-code".to_string(),
            step_transcript_identifier: String::new(),
            step_transcript_start: 0,
            token_usage: None,
        };

        strategy.save_step(&ctx).unwrap();
        let shadow = shadow_branch_ref(&head, "");
        assert!(
            run_git(dir.path(), &["rev-parse", &shadow]).is_err(),
            "save_step should not create a shadow branch"
        );
        let first_hash = latest_temporary_tree_hash(dir.path(), "dedup-session")
            .expect("first temporary checkpoint row should exist");
        let count_after_first = temporary_checkpoint_count(dir.path(), "dedup-session");
        assert_eq!(count_after_first, 1);

        strategy.save_step(&ctx).unwrap();
        let second_hash = latest_temporary_tree_hash(dir.path(), "dedup-session")
            .expect("latest temporary checkpoint row should exist after second save");
        let count_after_second = temporary_checkpoint_count(dir.path(), "dedup-session");
        assert_eq!(
            second_hash, first_hash,
            "identical content should keep the same temporary checkpoint tree hash"
        );
        assert_eq!(
            count_after_second, count_after_first,
            "identical content should not insert a duplicate temporary checkpoint row"
        );

        fs::write(
            dir.path().join("test.rs"),
            "package main\n\nfunc main() {}\n",
        )
        .unwrap();
        strategy.save_step(&ctx).unwrap();
        let third_hash = latest_temporary_tree_hash(dir.path(), "dedup-session")
            .expect("latest temporary checkpoint row should exist after content change");
        let count_after_third = temporary_checkpoint_count(dir.path(), "dedup-session");
        assert_ne!(
            third_hash, first_hash,
            "modified content should create a new temporary checkpoint tree hash"
        );
        assert_eq!(
            count_after_third,
            count_after_second + 1,
            "changed content should insert a new temporary checkpoint row"
        );
    });
}

#[test]
fn write_committed_branch_field() {
    // On branch: expect branch field persisted.
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    run_git(dir.path(), &["checkout", "-b", "feature/test-branch"]).unwrap();

    let cp_on = "bc1234567890";
    write_committed(
        dir.path(),
        WriteCommittedOptions {
            checkpoint_id: cp_on.to_string(),
            session_id: "branch-on".to_string(),
            strategy: "manual-commit".to_string(),
            agent: "claude-code".to_string(),
            transcript: b"{\"type\":\"assistant\",\"message\":{\"content\":\"branch\"}}\n".to_vec(),
            prompts: Some(vec!["branch prompt".to_string()]),
            context: None,
            checkpoints_count: 1,
            files_touched: vec![],
            token_usage_input: None,
            token_usage_output: None,
            token_usage_api_call_count: None,
            turn_id: String::new(),
            transcript_identifier_at_start: String::new(),
            checkpoint_transcript_start: 0,
            token_usage: None,
            initial_attribution: None,
            author_name: "Test Author".to_string(),
            author_email: "test@example.com".to_string(),
            summary: None,
            is_task: false,
            tool_use_id: String::new(),
            agent_id: String::new(),
            transcript_path: String::new(),
            subagent_transcript_path: String::new(),
        },
    )
    .unwrap();

    let on_meta = read_checkpoint_session_metadata_from_branch(dir.path(), cp_on);
    assert_eq!(
        on_meta["branch"], "feature/test-branch",
        "branch field should be captured while on branch"
    );

    // Detached HEAD: expect branch field omitted/empty.
    let detached = tempfile::tempdir().unwrap();
    let detached_head = setup_git_repo(&detached);
    run_git(detached.path(), &["checkout", &detached_head]).unwrap();

    let cp_detached = "cd1234567890";
    write_committed(
        detached.path(),
        WriteCommittedOptions {
            checkpoint_id: cp_detached.to_string(),
            session_id: "branch-detached".to_string(),
            strategy: "manual-commit".to_string(),
            agent: "claude-code".to_string(),
            transcript: b"{\"type\":\"assistant\",\"message\":{\"content\":\"detached\"}}\n"
                .to_vec(),
            prompts: Some(vec!["detached prompt".to_string()]),
            context: None,
            checkpoints_count: 1,
            files_touched: vec![],
            token_usage_input: None,
            token_usage_output: None,
            token_usage_api_call_count: None,
            turn_id: String::new(),
            transcript_identifier_at_start: String::new(),
            checkpoint_transcript_start: 0,
            token_usage: None,
            initial_attribution: None,
            author_name: "Test Author".to_string(),
            author_email: "test@example.com".to_string(),
            summary: None,
            is_task: false,
            tool_use_id: String::new(),
            agent_id: String::new(),
            transcript_path: String::new(),
            subagent_transcript_path: String::new(),
        },
    )
    .unwrap();

    let detached_meta = read_checkpoint_session_metadata_from_branch(detached.path(), cp_detached);
    assert!(
        detached_meta.get("branch").is_none() || detached_meta["branch"] == "",
        "branch should be absent/empty in detached HEAD metadata"
    );
}


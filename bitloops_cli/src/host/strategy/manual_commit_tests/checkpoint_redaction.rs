#[test]
fn write_committed_duplicate_session_id_clears_stale_files() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let checkpoint_id = "dedd0abcdef2";

    let mut session_a_v1 = default_write_committed_opts(checkpoint_id, "session-A", r#"{"v": 1}"#);
    session_a_v1.prompts = Some(vec!["original prompt".to_string()]);
    session_a_v1.context = Some(b"original context".to_vec());
    session_a_v1.checkpoints_count = 1;
    let write_a_v1 = write_committed(dir.path(), session_a_v1);
    assert!(
        write_a_v1.is_ok(),
        "session A v1 write should succeed: {write_a_v1:?}"
    );

    let mut session_b =
        default_write_committed_opts(checkpoint_id, "session-B", r#"{"session":"B"}"#);
    session_b.prompts = Some(vec!["B prompt".to_string()]);
    session_b.context = Some(b"B context".to_vec());
    session_b.checkpoints_count = 1;
    let write_b = write_committed(dir.path(), session_b);
    assert!(
        write_b.is_ok(),
        "session B write should succeed: {write_b:?}"
    );

    let mut session_a_v2 = default_write_committed_opts(checkpoint_id, "session-A", r#"{"v": 2}"#);
    session_a_v2.prompts = None;
    session_a_v2.context = None;
    session_a_v2.checkpoints_count = 2;
    let write_a_v2 = write_committed(dir.path(), session_a_v2);
    assert!(
        write_a_v2.is_ok(),
        "session A v2 write should succeed: {write_a_v2:?}"
    );

    let content_a = read_session_content(dir.path(), checkpoint_id, 0).unwrap();
    assert_eq!(
        content_a.prompts, "",
        "stale prompts should be cleared for overwritten session"
    );
    assert_eq!(
        content_a.context, "",
        "stale context should be cleared for overwritten session"
    );
    assert!(
        content_a.transcript.contains(r#""v": 2"#),
        "session A transcript should be updated, got {}",
        content_a.transcript
    );

    let content_b = read_session_content(dir.path(), checkpoint_id, 1).unwrap();
    assert_eq!(content_b.metadata["session_id"], "session-B");
    assert!(
        content_b.prompts.contains("B prompt"),
        "session B prompts should remain untouched, got {}",
        content_b.prompts
    );
    assert!(
        content_b.context.contains("B context"),
        "session B context should remain untouched, got {}",
        content_b.context
    );
}

#[test]
fn write_committed_redacts_prompt_secrets() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let checkpoint_id = "aabbccddeef2";

    let mut opts =
        default_write_committed_opts(checkpoint_id, "redact-prompt-session", r#"{"msg":"safe"}"#);
    opts.prompts = Some(vec![format!("Set API_KEY={HIGH_ENTROPY_SECRET}")]);
    opts.checkpoints_count = 1;
    let result = write_committed(dir.path(), opts);
    assert!(
        result.is_ok(),
        "write_committed should redact secrets in prompts: {result:?}"
    );

    let content = read_session_content(dir.path(), checkpoint_id, 0).unwrap();
    assert!(
        !content.prompts.contains(HIGH_ENTROPY_SECRET),
        "prompts should not contain secret after redaction"
    );
    assert!(
        content.prompts.contains("REDACTED"),
        "prompts should contain REDACTED placeholder"
    );
}

#[test]
fn write_committed_redacts_transcript_secrets() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let checkpoint_id = "aabbccddeef1";
    let transcript =
        format!(r#"{{"role":"assistant","content":"Here is your key: {HIGH_ENTROPY_SECRET}"}}"#);

    let mut opts =
        default_write_committed_opts(checkpoint_id, "redact-transcript-session", &transcript);
    opts.checkpoints_count = 1;
    let result = write_committed(dir.path(), opts);
    assert!(
        result.is_ok(),
        "write_committed should redact secrets in transcript: {result:?}"
    );

    let content = read_session_content(dir.path(), checkpoint_id, 0).unwrap();
    assert!(
        !content.transcript.contains(HIGH_ENTROPY_SECRET),
        "transcript should not contain secret after redaction"
    );
    assert!(
        content.transcript.contains("REDACTED"),
        "transcript should contain REDACTED placeholder"
    );
}

#[test]
fn write_committed_redacts_context_secrets() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let checkpoint_id = "aabbccddeef3";

    let mut opts =
        default_write_committed_opts(checkpoint_id, "redact-context-session", r#"{"msg":"safe"}"#);
    opts.context = Some(format!("DB_PASSWORD={HIGH_ENTROPY_SECRET}").into_bytes());
    opts.checkpoints_count = 1;
    let result = write_committed(dir.path(), opts);
    assert!(
        result.is_ok(),
        "write_committed should redact secrets in context: {result:?}"
    );

    let content = read_session_content(dir.path(), checkpoint_id, 0).unwrap();
    assert!(
        !content.context.contains(HIGH_ENTROPY_SECRET),
        "context should not contain secret after redaction"
    );
    assert!(
        content.context.contains("REDACTED"),
        "context should contain REDACTED placeholder"
    );
}

#[test]
fn write_committed_cli_version_field() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let checkpoint_id = "b1c2d3e4f5a6";

    let opts =
        default_write_committed_opts(checkpoint_id, "test-session-version", "test transcript");
    let result = write_committed(dir.path(), opts);
    assert!(
        result.is_ok(),
        "write_committed should persist cli_version in root and session metadata: {result:?}"
    );

    let top_meta = read_checkpoint_top_metadata_from_branch(dir.path(), checkpoint_id);
    assert_eq!(top_meta["cli_version"], env!("CARGO_PKG_VERSION"));

    let session_meta = read_checkpoint_session_metadata_from_branch(dir.path(), checkpoint_id);
    assert_eq!(session_meta["cli_version"], env!("CARGO_PKG_VERSION"));
}

#[test]
fn copy_metadata_dir_redacts_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let metadata_dir = dir.path().join("metadata");
    fs::create_dir_all(&metadata_dir).unwrap();
    fs::write(
        metadata_dir.join("agent.jsonl"),
        format!(r#"{{"content":"key={HIGH_ENTROPY_SECRET}"}}"#),
    )
    .unwrap();
    fs::write(
        metadata_dir.join("notes.txt"),
        format!("secret: {HIGH_ENTROPY_SECRET}"),
    )
    .unwrap();

    let result = copy_metadata_dir(&metadata_dir, "cp/");
    assert!(
        result.is_ok(),
        "copy_metadata_dir should redact secrets while copying: {result:?}"
    );
    let entries = result.unwrap();

    assert!(
        entries.contains_key("cp/agent.jsonl"),
        "agent.jsonl should be included in copied entries"
    );
    assert!(
        entries.contains_key("cp/notes.txt"),
        "notes.txt should be included in copied entries"
    );

    for (path, content) in entries {
        assert!(
            !content.contains(HIGH_ENTROPY_SECRET),
            "{path} should not contain the raw secret after redaction"
        );
        assert!(
            content.contains("REDACTED"),
            "{path} should contain REDACTED placeholder"
        );
    }
}

#[test]
fn redact_summary_nil() {
    let result = redact_summary(None).expect("redact_summary(nil) should not error");
    assert!(result.is_none(), "redact_summary(None) should return None");
}

#[test]
fn redact_summary_with_secrets() {
    let summary = Summary {
        intent: format!("Set API_KEY={HIGH_ENTROPY_SECRET}"),
        outcome: format!("Configured key {HIGH_ENTROPY_SECRET} successfully"),
        friction: vec![
            format!("Had to find {HIGH_ENTROPY_SECRET} in env"),
            "No issues here".to_string(),
        ],
        open_items: vec![format!("Rotate {HIGH_ENTROPY_SECRET}")],
        learnings: LearningsSummary {
            repo: vec![format!("Found secret {HIGH_ENTROPY_SECRET} in config")],
            workflow: vec![format!("Use vault for {HIGH_ENTROPY_SECRET}")],
            code: vec![CodeLearning {
                path: "config/secrets.rs".to_string(),
                line: 42,
                end_line: 50,
                finding: format!("Key {HIGH_ENTROPY_SECRET} is hardcoded"),
            }],
        },
    };

    let redacted = redact_summary(Some(&summary))
        .expect("redact_summary should not error")
        .expect("redact_summary should return Some for non-nil input");

    assert!(
        !redacted.intent.contains(HIGH_ENTROPY_SECRET),
        "intent should not contain the secret"
    );
    assert!(
        redacted.intent.contains("REDACTED"),
        "intent should contain REDACTED placeholder"
    );
    assert!(
        !redacted.outcome.contains(HIGH_ENTROPY_SECRET),
        "outcome should not contain the secret"
    );
    assert!(
        !redacted.friction[0].contains(HIGH_ENTROPY_SECRET),
        "friction[0] should not contain the secret"
    );
    assert_eq!(redacted.friction[1], "No issues here");
    assert!(
        !redacted.open_items[0].contains(HIGH_ENTROPY_SECRET),
        "open_items[0] should not contain the secret"
    );
    assert!(
        !redacted.learnings.repo[0].contains(HIGH_ENTROPY_SECRET),
        "learnings.repo[0] should not contain the secret"
    );
    assert!(
        !redacted.learnings.workflow[0].contains(HIGH_ENTROPY_SECRET),
        "learnings.workflow[0] should not contain the secret"
    );

    let code = &redacted.learnings.code[0];
    assert_eq!(code.path, "config/secrets.rs");
    assert_eq!(code.line, 42);
    assert_eq!(code.end_line, 50);
    assert!(
        !code.finding.contains(HIGH_ENTROPY_SECRET),
        "code learning finding should not contain the secret"
    );
    assert!(
        code.finding.contains("REDACTED"),
        "code learning finding should contain REDACTED placeholder"
    );

    assert!(
        summary.intent.contains(HIGH_ENTROPY_SECRET),
        "original summary should remain unmodified"
    );
}

#[test]
fn redact_summary_no_secrets() {
    let summary = Summary {
        intent: "Fix a bug".to_string(),
        outcome: "Bug fixed".to_string(),
        friction: vec!["None".to_string()],
        open_items: vec![],
        learnings: LearningsSummary {
            repo: vec!["Found the pattern".to_string()],
            workflow: vec!["Use TDD".to_string()],
            code: vec![CodeLearning {
                path: "main.rs".to_string(),
                line: 1,
                end_line: 0,
                finding: "Good code".to_string(),
            }],
        },
    };

    let redacted = redact_summary(Some(&summary))
        .expect("redact_summary should not error")
        .expect("redact_summary should return Some for non-nil input");

    assert_eq!(redacted.intent, "Fix a bug");
    assert_eq!(redacted.outcome, "Bug fixed");
    assert_eq!(redacted.learnings.code[0].finding, "Good code");
}

#[test]
fn redact_string_slice_nil_and_empty() {
    let nil_result = redact_string_slice(None).expect("redact_string_slice(nil) should not error");
    assert!(nil_result.is_none(), "nil input should return None");

    let empty: Vec<String> = vec![];
    let empty_result =
        redact_string_slice(Some(&empty)).expect("redact_string_slice(empty) should not error");
    assert!(
        empty_result.is_some(),
        "empty slice should return Some(empty), not None"
    );
    assert_eq!(
        empty_result.unwrap().len(),
        0,
        "empty slice should stay empty"
    );
}

#[test]
fn redact_code_learnings_nil_and_empty() {
    let nil_result =
        redact_code_learnings(None).expect("redact_code_learnings(nil) should not error");
    assert!(nil_result.is_none(), "nil input should return None");

    let empty: Vec<CodeLearning> = vec![];
    let empty_result =
        redact_code_learnings(Some(&empty)).expect("redact_code_learnings(empty) should not error");
    assert!(
        empty_result.is_some(),
        "empty slice should return Some(empty), not None"
    );
    assert_eq!(
        empty_result.unwrap().len(),
        0,
        "empty slice should stay empty"
    );
}

#[test]
fn write_committed_redacts_summary_secrets() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let checkpoint_id = "aabbccddeef7";

    let mut opts =
        default_write_committed_opts(checkpoint_id, "redact-summary-session", r#"{"msg":"safe"}"#);
    opts.checkpoints_count = 1;
    opts.summary = Some(serde_json::json!({
        "intent": format!("Used key {HIGH_ENTROPY_SECRET} to auth"),
        "outcome": format!("Authenticated with {HIGH_ENTROPY_SECRET}")
    }));

    let result = write_committed(dir.path(), opts);
    assert!(
        result.is_ok(),
        "write_committed should redact summary secrets: {result:?}"
    );

    let content = read_session_content(dir.path(), checkpoint_id, 0).unwrap();
    assert!(
        !content.metadata["summary"].is_null(),
        "summary should not be null"
    );
    let intent = content
        .metadata
        .pointer("/summary/intent")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let outcome = content
        .metadata
        .pointer("/summary/outcome")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    assert!(
        !intent.contains(HIGH_ENTROPY_SECRET),
        "summary intent should not contain secret after redaction"
    );
    assert!(
        intent.contains("REDACTED"),
        "summary intent should contain REDACTED placeholder"
    );
    assert!(
        !outcome.contains(HIGH_ENTROPY_SECRET),
        "summary outcome should not contain secret after redaction"
    );
}

#[test]
fn update_summary_redacts_secrets() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let checkpoint_id = "aabbccddeef8";

    let write_result = write_committed(
        dir.path(),
        default_write_committed_opts(checkpoint_id, "update-summary-session", r#"{"msg":"safe"}"#),
    );
    assert!(
        write_result.is_ok(),
        "initial write_committed should succeed before update_summary: {write_result:?}"
    );

    let update_result = update_summary(
        dir.path(),
        checkpoint_id,
        serde_json::json!({
            "intent": format!("Rotated key {HIGH_ENTROPY_SECRET}"),
            "outcome": "Done"
        }),
    );
    assert!(
        update_result.is_ok(),
        "update_summary should redact summary secrets: {update_result:?}"
    );

    let content = read_session_content(dir.path(), checkpoint_id, 0).unwrap();
    let intent = content
        .metadata
        .pointer("/summary/intent")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    assert!(
        !intent.contains(HIGH_ENTROPY_SECRET),
        "updated summary intent should not contain secret"
    );
    assert!(
        intent.contains("REDACTED"),
        "updated summary intent should contain REDACTED placeholder"
    );
}

#[test]
fn write_committed_subagent_transcript_jsonl_fallback() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let checkpoint_id = "aabbccddeef9";

    let transcript_path = dir.path().join("agent.jsonl");
    let invalid_jsonl =
        format!("this is not valid JSON but has a secret {HIGH_ENTROPY_SECRET} in it");
    fs::write(&transcript_path, invalid_jsonl).unwrap();

    let mut opts =
        default_write_committed_opts(checkpoint_id, "jsonl-fallback-session", r#"{"msg":"safe"}"#);
    opts.is_task = true;
    opts.tool_use_id = "toolu_test123".to_string();
    opts.agent_id = "agent1".to_string();
    opts.subagent_transcript_path = transcript_path.to_string_lossy().to_string();

    let result = write_committed(dir.path(), opts);
    assert!(
        result.is_ok(),
        "write_committed should keep subagent transcript and redact via fallback: {result:?}"
    );

    let content = read_session_content(dir.path(), checkpoint_id, 0).unwrap();
    assert!(
        content.metadata["is_task"].as_bool().unwrap_or(false),
        "task session metadata should be persisted"
    );
    assert_eq!(content.metadata["tool_use_id"], "toolu_test123");
    assert!(
        run_git(dir.path(), &["rev-parse", paths::METADATA_BRANCH_NAME]).is_err(),
        "task writes should not create metadata-branch artefacts"
    );
    let stored_path = query_checkpoint_subagent_transcript_path(
        dir.path(),
        checkpoint_id,
        "jsonl-fallback-session",
    )
    .expect("subagent transcript path should be stored");
    assert_eq!(stored_path, transcript_path.to_string_lossy());
}

#[test]
fn write_temporary_task_subagent_transcript_redacts_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let base_commit = setup_git_repo(&dir);

    let transcript_path = dir.path().join("agent-transcript.jsonl");
    let invalid_jsonl =
        format!("this is not valid JSON but has a secret {HIGH_ENTROPY_SECRET} in it");
    fs::write(&transcript_path, invalid_jsonl).unwrap();

    let result = write_temporary_task(
        dir.path(),
        WriteTemporaryTaskOptions {
            session_id: "test-session".to_string(),
            base_commit: base_commit.clone(),
            step_number: 1,
            tool_use_id: "toolu_test456".to_string(),
            agent_id: "agent1".to_string(),
            modified_files: vec![],
            new_files: vec![],
            deleted_files: vec![],
            transcript_path: String::new(),
            subagent_transcript_path: transcript_path.to_string_lossy().to_string(),
            checkpoint_uuid: "test-uuid".to_string(),
            is_incremental: false,
            incremental_sequence: 0,
            incremental_type: String::new(),
            incremental_data: String::new(),
            commit_message: "Task checkpoint".to_string(),
            author_name: "Test".to_string(),
            author_email: "test@test.com".to_string(),
        },
    );
    assert!(
        result.is_ok(),
        "write_temporary_task should redact subagent transcript secrets: {result:?}"
    );
    let result = result.unwrap();

    let shadow_branch = shadow_branch_ref(&base_commit, "");
    assert!(
        run_git(dir.path(), &["rev-parse", &shadow_branch]).is_err(),
        "write_temporary_task should not create a shadow branch"
    );

    let latest_tree_hash = latest_temporary_tree_hash(dir.path(), "test-session")
        .expect("task checkpoint should persist a temporary_checkpoints row");
    assert_eq!(
        latest_tree_hash, result.commit_hash,
        "write_temporary_task result should return the persisted tree hash"
    );

    let agent_path =
        ".bitloops/metadata/test-session/tasks/toolu_test456/agent-agent1.jsonl".to_string();
    let content = run_git(
        dir.path(),
        &["show", &format!("{}:{agent_path}", result.commit_hash)],
    )
    .unwrap();
    assert!(
        !content.is_empty(),
        "subagent transcript should not be empty"
    );
    assert!(
        !content.contains(HIGH_ENTROPY_SECRET),
        "subagent transcript in checkpoint tree should not contain secret"
    );
    assert!(
        content.contains("REDACTED"),
        "subagent transcript in checkpoint tree should contain REDACTED"
    );
}

#[test]
fn add_directory_to_entries_path_traversal() {
    let dir = tempfile::tempdir().unwrap();
    let metadata_dir = dir.path().join("metadata");
    let sub_dir = metadata_dir.join("sub");
    fs::create_dir_all(&sub_dir).unwrap();
    fs::write(sub_dir.join("data.txt"), "safe content").unwrap();

    let result =
        add_directory_to_entries_with_abs_path(&metadata_dir, ".bitloops/metadata/session");
    assert!(
        result.is_ok(),
        "add_directory_to_entries_with_abs_path should include regular files: {result:?}"
    );

    let entries = result.unwrap();
    let expected = ".bitloops/metadata/session/sub/data.txt";
    assert!(
        entries.contains_key(expected),
        "expected entry {expected}, got {entries:?}"
    );
}

#[cfg(unix)]
#[test]
fn add_directory_to_entries_skips_symlinks() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let metadata_dir = dir.path().join("metadata");
    fs::create_dir_all(&metadata_dir).unwrap();
    fs::write(metadata_dir.join("regular.txt"), "regular content").unwrap();
    let sensitive_file = dir.path().join("sensitive.txt");
    fs::write(&sensitive_file, "SECRET DATA").unwrap();
    symlink(&sensitive_file, metadata_dir.join("sneaky-link")).unwrap();

    let result = add_directory_to_entries_with_abs_path(&metadata_dir, "checkpoint/");
    assert!(
        result.is_ok(),
        "add_directory_to_entries_with_abs_path should not fail when symlinks exist: {result:?}"
    );
    let entries = result.unwrap();
    assert!(
        entries.contains_key("checkpoint/regular.txt"),
        "regular file should be included"
    );
    assert!(
        !entries.contains_key("checkpoint/sneaky-link"),
        "symlink should be skipped"
    );
    assert_eq!(entries.len(), 1, "only regular file should be present");
}

#[cfg(unix)]
#[test]
fn add_directory_to_entries_skips_symlinked_directories() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let metadata_dir = dir.path().join("metadata");
    fs::create_dir_all(&metadata_dir).unwrap();
    fs::write(metadata_dir.join("regular.txt"), "regular content").unwrap();

    let external_dir = dir.path().join("external-secrets");
    fs::create_dir_all(&external_dir).unwrap();
    fs::write(external_dir.join("secret.txt"), "SECRET DATA").unwrap();
    symlink(&external_dir, metadata_dir.join("evil-dir-link")).unwrap();

    let result = add_directory_to_entries_with_abs_path(&metadata_dir, "checkpoint/");
    assert!(
        result.is_ok(),
        "add_directory_to_entries_with_abs_path should skip symlinked directories: {result:?}"
    );
    let entries = result.unwrap();
    assert!(
        entries.contains_key("checkpoint/regular.txt"),
        "regular file should be included"
    );
    assert!(
        !entries.contains_key("checkpoint/evil-dir-link/secret.txt"),
        "files inside symlinked directories should be skipped"
    );
    assert_eq!(entries.len(), 1, "only regular file should be present");
}


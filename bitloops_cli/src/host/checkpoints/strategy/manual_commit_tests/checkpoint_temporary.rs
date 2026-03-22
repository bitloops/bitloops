use super::*;

pub(crate) fn create_checkpoint_metadata_dir(repo_root: &Path, session_id: &str) -> String {
    let metadata_dir = repo_root
        .join(".bitloops")
        .join("metadata")
        .join(session_id);
    fs::create_dir_all(&metadata_dir).unwrap();
    fs::write(
        metadata_dir.join(paths::TRANSCRIPT_FILE_NAME),
        r#"{"test": true}"#,
    )
    .unwrap();
    metadata_dir.to_string_lossy().to_string()
}

pub(crate) fn first_checkpoint_opts(
    session_id: &str,
    base_commit: &str,
    metadata_dir_abs: &str,
) -> WriteTemporaryOptions {
    WriteTemporaryOptions {
        session_id: session_id.to_string(),
        base_commit: base_commit.to_string(),
        step_number: 1,
        modified_files: vec![],
        new_files: vec![],
        deleted_files: vec![],
        metadata_dir: format!(".bitloops/metadata/{session_id}"),
        metadata_dir_abs: metadata_dir_abs.to_string(),
        commit_message: "First checkpoint".to_string(),
        author_name: "Test".to_string(),
        author_email: "test@test.com".to_string(),
        is_first_checkpoint: true,
    }
}

pub(crate) fn default_write_committed_opts(
    checkpoint_id: &str,
    session_id: &str,
    transcript: &str,
) -> WriteCommittedOptions {
    WriteCommittedOptions {
        checkpoint_id: checkpoint_id.to_string(),
        session_id: session_id.to_string(),
        strategy: "manual-commit".to_string(),
        agent: "claude-code".to_string(),
        transcript: transcript.as_bytes().to_vec(),
        prompts: None,
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
    }
}

#[test]
pub(crate) fn read_session_content_nonexistent_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    run_git(
        dir.path(),
        &[
            "update-ref",
            &format!("refs/heads/{}", paths::METADATA_BRANCH_NAME),
            &head,
        ],
    )
    .unwrap();

    let result = read_session_content(dir.path(), "eeeeeeeeeeee", 0);
    assert!(result.is_err(), "expected checkpoint-not-found error");
    let msg = format!("{:#}", result.unwrap_err());
    assert!(
        msg.contains("checkpoint not found"),
        "expected checkpoint-not-found error, got: {msg}"
    );
}

#[test]
pub(crate) fn write_temporary_first_checkpoint_captures_modified_tracked_files() {
    let dir = tempfile::tempdir().unwrap();
    let base_commit = setup_git_repo(&dir);
    let modified_content = "# Modified by User\n\nThis change was made before the agent started.\n";
    fs::write(dir.path().join("README.md"), modified_content).unwrap();
    let metadata_dir_abs = create_checkpoint_metadata_dir(dir.path(), "test-session");

    let result = write_temporary(
        dir.path(),
        first_checkpoint_opts("test-session", &base_commit, &metadata_dir_abs),
    )
    .unwrap();
    assert!(!result.skipped, "first checkpoint should not be skipped");

    let content = run_git(
        dir.path(),
        &["show", &format!("{}:README.md", result.commit_hash)],
    )
    .unwrap();
    assert_eq!(content, modified_content);
}

#[test]
pub(crate) fn write_temporary_first_checkpoint_captures_untracked_files() {
    let dir = tempfile::tempdir().unwrap();
    let base_commit = setup_git_repo(&dir);
    let untracked_content = r#"{"key": "secret_value"}"#;
    fs::write(dir.path().join("config.local.json"), untracked_content).unwrap();
    let metadata_dir_abs = create_checkpoint_metadata_dir(dir.path(), "test-session");

    let result = write_temporary(
        dir.path(),
        first_checkpoint_opts("test-session", &base_commit, &metadata_dir_abs),
    )
    .unwrap();
    assert!(
        !result.skipped,
        "first checkpoint with untracked files should not be skipped"
    );

    let content = run_git(
        dir.path(),
        &["show", &format!("{}:config.local.json", result.commit_hash)],
    )
    .unwrap();
    assert_eq!(content, untracked_content);
}

#[test]
pub(crate) fn write_temporary_first_checkpoint_excludes_gitignored_files() {
    let dir = tempfile::tempdir().unwrap();
    let _ = setup_git_repo(&dir);
    fs::write(dir.path().join(".gitignore"), "node_modules/\n").unwrap();
    run_git(dir.path(), &["add", ".gitignore"]).unwrap();
    run_git(dir.path(), &["commit", "-m", "add gitignore"]).unwrap();
    let base_commit = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();

    fs::create_dir_all(dir.path().join("node_modules")).unwrap();
    fs::write(
        dir.path().join("node_modules").join("some-package.js"),
        "module.exports = {}",
    )
    .unwrap();
    let metadata_dir_abs = create_checkpoint_metadata_dir(dir.path(), "test-session");

    let result = write_temporary(
        dir.path(),
        first_checkpoint_opts("test-session", &base_commit, &metadata_dir_abs),
    )
    .unwrap();
    assert!(!result.skipped);

    let ignored = run_git(
        dir.path(),
        &[
            "show",
            &format!("{}:node_modules/some-package.js", result.commit_hash),
        ],
    );
    assert!(
        ignored.is_err(),
        "gitignored file should not be present in checkpoint tree"
    );
}

#[test]
pub(crate) fn write_temporary_first_checkpoint_user_and_agent_changes() {
    with_git_env_cleared(|| {
        let dir = tempfile::tempdir().unwrap();
        let _ = setup_git_repo(&dir);
        fs::write(dir.path().join("main.rs"), "package main\n").unwrap();
        run_git(dir.path(), &["add", "main.rs"]).unwrap();
        run_git(dir.path(), &["commit", "-m", "add main.rs"]).unwrap();
        let base_commit = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();

        let user_modified = "# Modified by User\n";
        fs::write(dir.path().join("README.md"), user_modified).unwrap();
        let agent_modified = "package main\n\nfunc main() {\n\tprintln(\"Hello\")\n}\n";
        fs::write(dir.path().join("main.rs"), agent_modified).unwrap();
        let metadata_dir_abs = create_checkpoint_metadata_dir(dir.path(), "test-session");

        let mut opts = first_checkpoint_opts("test-session", &base_commit, &metadata_dir_abs);
        opts.modified_files = vec!["main.rs".to_string()];
        let result = write_temporary(dir.path(), opts).unwrap();
        assert!(!result.skipped);

        let readme = run_git(
            dir.path(),
            &["show", &format!("{}:README.md", result.commit_hash)],
        )
        .unwrap();
        assert_eq!(readme, user_modified);

        let main_go = run_git(
            dir.path(),
            &["show", &format!("{}:main.rs", result.commit_hash)],
        )
        .unwrap();
        assert_eq!(main_go, agent_modified);
    });
}

#[test]
pub(crate) fn write_temporary_first_checkpoint_captures_user_deleted_files() {
    let dir = tempfile::tempdir().unwrap();
    setup_empty_git_repo(&dir);
    fs::write(dir.path().join("keep.txt"), "keep this").unwrap();
    fs::write(dir.path().join("delete-me.txt"), "delete this").unwrap();
    run_git(dir.path(), &["add", "."]).unwrap();
    run_git(dir.path(), &["commit", "-m", "initial"]).unwrap();
    let base_commit = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();

    fs::remove_file(dir.path().join("delete-me.txt")).unwrap();
    let metadata_dir_abs = create_checkpoint_metadata_dir(dir.path(), "test-session");

    let result = write_temporary(
        dir.path(),
        first_checkpoint_opts("test-session", &base_commit, &metadata_dir_abs),
    )
    .unwrap();
    assert!(!result.skipped);

    let keep = run_git(
        dir.path(),
        &["show", &format!("{}:keep.txt", result.commit_hash)],
    )
    .unwrap();
    assert_eq!(keep, "keep this");

    let deleted = run_git(
        dir.path(),
        &["show", &format!("{}:delete-me.txt", result.commit_hash)],
    );
    assert!(
        deleted.is_err(),
        "user-deleted file should be absent from checkpoint tree"
    );
}

#[test]
pub(crate) fn write_temporary_first_checkpoint_captures_renamed_files() {
    let dir = tempfile::tempdir().unwrap();
    setup_empty_git_repo(&dir);
    fs::write(dir.path().join("old-name.txt"), "content").unwrap();
    run_git(dir.path(), &["add", "old-name.txt"]).unwrap();
    run_git(dir.path(), &["commit", "-m", "initial"]).unwrap();
    let base_commit = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();

    run_git(dir.path(), &["mv", "old-name.txt", "new-name.txt"]).unwrap();
    let metadata_dir_abs = create_checkpoint_metadata_dir(dir.path(), "test-session");

    let result = write_temporary(
        dir.path(),
        first_checkpoint_opts("test-session", &base_commit, &metadata_dir_abs),
    )
    .unwrap();
    assert!(!result.skipped);

    let renamed = run_git(
        dir.path(),
        &["show", &format!("{}:new-name.txt", result.commit_hash)],
    );
    assert!(
        renamed.is_ok(),
        "renamed file should exist in checkpoint tree"
    );

    let old = run_git(
        dir.path(),
        &["show", &format!("{}:old-name.txt", result.commit_hash)],
    );
    assert!(old.is_err(), "old file path should be absent after rename");
}

#[test]
pub(crate) fn write_temporary_first_checkpoint_filenames_with_spaces() {
    let dir = tempfile::tempdir().unwrap();
    setup_empty_git_repo(&dir);
    fs::write(dir.path().join("simple.txt"), "simple").unwrap();
    run_git(dir.path(), &["add", "simple.txt"]).unwrap();
    run_git(dir.path(), &["commit", "-m", "initial"]).unwrap();
    let base_commit = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();

    fs::write(
        dir.path().join("file with spaces.txt"),
        "content with spaces",
    )
    .unwrap();
    let metadata_dir_abs = create_checkpoint_metadata_dir(dir.path(), "test-session");

    let result = write_temporary(
        dir.path(),
        first_checkpoint_opts("test-session", &base_commit, &metadata_dir_abs),
    )
    .unwrap();
    assert!(!result.skipped);

    let with_spaces = run_git(
        dir.path(),
        &[
            "show",
            &format!("{}:file with spaces.txt", result.commit_hash),
        ],
    );
    assert!(
        with_spaces.is_ok(),
        "filename with spaces should be present in checkpoint tree"
    );
}

#[test]
pub(crate) fn write_temporary_task_incremental_persists_metadata_and_payload() {
    let dir = tempfile::tempdir().unwrap();
    let base_commit = setup_git_repo(&dir);

    let result = write_temporary_task(
        dir.path(),
        WriteTemporaryTaskOptions {
            session_id: "temp-session".to_string(),
            base_commit,
            step_number: 3,
            tool_use_id: "toolu_temp123".to_string(),
            agent_id: "agent1".to_string(),
            modified_files: vec![],
            new_files: vec![],
            deleted_files: vec![],
            transcript_path: String::new(),
            subagent_transcript_path: String::new(),
            checkpoint_uuid: "checkpoint-temp-123".to_string(),
            is_incremental: true,
            incremental_sequence: 3,
            incremental_type: "TodoWrite".to_string(),
            incremental_data: r#"{"todo":"document dependencies"}"#.to_string(),
            commit_message: "Incremental task checkpoint".to_string(),
            author_name: "Test".to_string(),
            author_email: "test@test.com".to_string(),
        },
    )
    .expect("write_temporary_task should persist incremental checkpoint");

    let payload_path =
        ".bitloops/metadata/temp-session/tasks/toolu_temp123/checkpoints/003-toolu_temp123.json";
    let payload_raw = run_git(
        dir.path(),
        &["show", &format!("{}:{payload_path}", result.commit_hash)],
    )
    .expect("incremental checkpoint payload should be present in checkpoint tree");
    let payload: serde_json::Value =
        serde_json::from_str(&payload_raw).expect("incremental payload should be valid json");
    assert_eq!(payload["type"], "TodoWrite");
    assert_eq!(payload["tool_use_id"], "toolu_temp123");
    assert_eq!(payload["data"]["todo"], "document dependencies");
    assert!(
        payload["timestamp"].as_str().is_some(),
        "incremental payload should include a timestamp"
    );

    let legacy_checkpoint_json = run_git(
        dir.path(),
        &[
            "show",
            &format!(
                "{}:.bitloops/metadata/temp-session/tasks/toolu_temp123/checkpoint.json",
                result.commit_hash
            ),
        ],
    );
    assert!(
        legacy_checkpoint_json.is_err(),
        "incremental flow should not emit task checkpoint.json payload"
    );

    let sqlite = SqliteConnectionPool::connect(temporary_checkpoints_db_path(dir.path())).unwrap();
    sqlite.initialise_checkpoint_schema().unwrap();
    let repo_id = crate::host::devql::resolve_repo_identity(dir.path())
        .unwrap()
        .repo_id;

    let row = sqlite
        .with_connection(|conn| {
            Ok(conn.query_row(
                "SELECT is_incremental, incremental_sequence, incremental_type, incremental_data, tool_use_id, agent_id
                 FROM temporary_checkpoints
                 WHERE session_id = ?1 AND repo_id = ?2
                 ORDER BY id DESC
                 LIMIT 1",
                rusqlite::params!["temp-session", repo_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Option<i64>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<String>>(5)?,
                    ))
                },
            )?)
        })
        .expect("query temporary checkpoint row");

    assert_eq!(row.0, 1);
    assert_eq!(row.1, Some(3));
    assert_eq!(row.2.as_deref(), Some("TodoWrite"));
    assert_eq!(
        row.3.as_deref(),
        Some(r#"{"todo":"document dependencies"}"#)
    );
    assert_eq!(row.4.as_deref(), Some("toolu_temp123"));
    assert_eq!(row.5.as_deref(), Some("agent1"));
}

#[test]
pub(crate) fn write_committed_duplicate_session_id_updates_in_place() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let checkpoint_id = "deda01234567";

    let mut session_x_v1 =
        default_write_committed_opts(checkpoint_id, "session-X", r#"{"message":"session X v1"}"#);
    session_x_v1.files_touched = vec!["a.rs".to_string()];
    session_x_v1.checkpoints_count = 3;
    session_x_v1.token_usage_input = Some(100);
    session_x_v1.token_usage_output = Some(50);
    session_x_v1.token_usage_api_call_count = Some(5);
    let write_x_v1 = write_committed(dir.path(), session_x_v1);
    assert!(
        write_x_v1.is_ok(),
        "session X v1 write should succeed: {write_x_v1:?}"
    );

    let mut session_y =
        default_write_committed_opts(checkpoint_id, "session-Y", r#"{"message":"session Y"}"#);
    session_y.files_touched = vec!["b.rs".to_string()];
    session_y.checkpoints_count = 2;
    session_y.token_usage_input = Some(50);
    session_y.token_usage_output = Some(25);
    session_y.token_usage_api_call_count = Some(3);
    let write_y = write_committed(dir.path(), session_y);
    assert!(
        write_y.is_ok(),
        "session Y write should succeed: {write_y:?}"
    );

    let mut session_x_v2 =
        default_write_committed_opts(checkpoint_id, "session-X", r#"{"message":"session X v2"}"#);
    session_x_v2.files_touched = vec!["a.rs".to_string(), "c.rs".to_string()];
    session_x_v2.checkpoints_count = 5;
    session_x_v2.token_usage_input = Some(200);
    session_x_v2.token_usage_output = Some(100);
    session_x_v2.token_usage_api_call_count = Some(10);
    let write_x_v2 = write_committed(dir.path(), session_x_v2);
    assert!(
        write_x_v2.is_ok(),
        "session X overwrite should succeed: {write_x_v2:?}"
    );

    let summary = read_committed(dir.path(), checkpoint_id)
        .unwrap()
        .expect("summary should exist");
    assert_eq!(
        summary.sessions.len(),
        2,
        "duplicate session should update in place"
    );

    let content0 = read_session_content(dir.path(), checkpoint_id, 0).unwrap();
    assert_eq!(content0.metadata["session_id"], "session-X");
    assert!(content0.transcript.contains("session X v2"));

    let content1 = read_session_content(dir.path(), checkpoint_id, 1).unwrap();
    assert_eq!(content1.metadata["session_id"], "session-Y");

    assert_eq!(summary.checkpoints_count, 7);
    assert_eq!(summary.files_touched, vec!["a.rs", "b.rs", "c.rs"]);
    let top_metadata = read_checkpoint_top_metadata_from_branch(dir.path(), checkpoint_id);
    assert!(
        top_metadata.get("token_usage").is_some(),
        "summary schema uses nested token_usage object"
    );
    assert!(
        top_metadata.get("token_usage_input").is_none()
            && top_metadata.get("token_usage_output").is_none()
            && top_metadata.get("token_usage_api_call_count").is_none(),
        "summary schema does not use flat token usage fields"
    );
    assert_eq!(top_metadata["token_usage"]["input_tokens"], 250);
    assert_eq!(top_metadata["token_usage"]["output_tokens"], 125);
    assert_eq!(top_metadata["token_usage"]["api_call_count"], 13);
}

#[test]
pub(crate) fn write_committed_duplicate_session_id_single_session() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let checkpoint_id = "dedb07654321";

    let mut v1 = default_write_committed_opts(checkpoint_id, "session-X", r#"{"message":"v1"}"#);
    v1.files_touched = vec!["old.rs".to_string()];
    v1.checkpoints_count = 1;
    let write_v1 = write_committed(dir.path(), v1);
    assert!(
        write_v1.is_ok(),
        "initial write should succeed: {write_v1:?}"
    );

    let mut v2 = default_write_committed_opts(checkpoint_id, "session-X", r#"{"message":"v2"}"#);
    v2.files_touched = vec!["new.rs".to_string()];
    v2.checkpoints_count = 5;
    let write_v2 = write_committed(dir.path(), v2);
    assert!(
        write_v2.is_ok(),
        "overwrite write should succeed: {write_v2:?}"
    );

    let summary = read_committed(dir.path(), checkpoint_id)
        .unwrap()
        .expect("summary should exist");
    assert_eq!(
        summary.sessions.len(),
        1,
        "duplicate single session should not append"
    );

    let content = read_session_content(dir.path(), checkpoint_id, 0).unwrap();
    assert_eq!(content.metadata["session_id"], "session-X");
    assert!(
        content.transcript.contains("v2"),
        "session transcript should be overwritten with latest content"
    );

    assert_eq!(summary.checkpoints_count, 5);
    assert_eq!(summary.files_touched, vec!["new.rs"]);
}

#[test]
pub(crate) fn write_committed_duplicate_session_id_reuses_index() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let checkpoint_id = "dedc0abcdef1";

    let mut session_a_v1 = default_write_committed_opts(checkpoint_id, "session-A", r#"{"v": 1}"#);
    session_a_v1.checkpoints_count = 1;
    let write_a_v1 = write_committed(dir.path(), session_a_v1);
    assert!(
        write_a_v1.is_ok(),
        "session A v1 write should succeed: {write_a_v1:?}"
    );

    let mut session_b = default_write_committed_opts(checkpoint_id, "session-B", r#"{"v": 2}"#);
    session_b.checkpoints_count = 1;
    let write_b = write_committed(dir.path(), session_b);
    assert!(
        write_b.is_ok(),
        "session B write should succeed: {write_b:?}"
    );

    let mut session_a_v2 = default_write_committed_opts(checkpoint_id, "session-A", r#"{"v": 3}"#);
    session_a_v2.checkpoints_count = 2;
    let write_a_v2 = write_committed(dir.path(), session_a_v2);
    assert!(
        write_a_v2.is_ok(),
        "session A v2 write should succeed: {write_a_v2:?}"
    );

    let summary = read_committed(dir.path(), checkpoint_id)
        .unwrap()
        .expect("summary should exist");
    assert_eq!(summary.sessions.len(), 2, "session count should remain 2");
    assert!(
        summary.sessions[0].transcript.contains("/0/"),
        "session A should keep index 0 transcript path, got {}",
        summary.sessions[0].transcript
    );
    assert!(
        summary.sessions[1].transcript.contains("/1/"),
        "session B should stay at index 1 transcript path, got {}",
        summary.sessions[1].transcript
    );

    let content0 = read_session_content(dir.path(), checkpoint_id, 0).unwrap();
    assert_eq!(content0.metadata["session_id"], "session-A");
    assert!(
        content0.transcript.contains(r#""v": 3"#),
        "session 0 should hold updated transcript, got {}",
        content0.transcript
    );
}

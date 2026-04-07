use super::*;

#[test]
pub(crate) fn condense_session_files_touched_fallback_empty_state() {
    let dir = tempfile::tempdir().unwrap();
    let base_head = setup_git_repo(&dir);

    fs::write(dir.path().join("agent.rs"), "package main\n").unwrap();
    git_ok(dir.path(), &["add", "agent.rs"]);
    git_ok(dir.path(), &["commit", "-m", "Add agent.rs"]);
    let new_head = git_ok(dir.path(), &["rev-parse", "HEAD"]);

    let strategy = ManualCommitStrategy::new(dir.path());
    let session_id = "test-empty-files";
    let mut state = idle_state(session_id, &base_head, vec![], 1);
    let transcript_path = write_session_transcript(
        dir.path(),
        session_id,
        r#"{"type":"human","message":{"content":"create agent.rs"}}
{"type":"assistant","message":{"content":"Done"}}"#,
    );
    state.transcript_path = transcript_path.to_string_lossy().to_string();

    let checkpoint_id = "fa11bac00001";
    strategy
        .condense_session(&mut state, checkpoint_id, &new_head)
        .unwrap();

    let metadata = read_checkpoint_session_metadata_from_branch(dir.path(), checkpoint_id);
    assert!(
        metadata.get("files_touched").is_none(),
        "committed session metadata should no longer persist files_touched"
    );
    let summary = read_committed(dir.path(), checkpoint_id)
        .unwrap()
        .expect("expected committed checkpoint summary");
    assert_eq!(
        summary.files_touched,
        vec!["agent.rs".to_string()],
        "checkpoint summary should derive files_touched from checkpoint_files"
    );
}
#[test]
pub(crate) fn condense_session_files_touched_no_fallback_no_overlap() {
    let dir = tempfile::tempdir().unwrap();
    let base_head = setup_git_repo(&dir);

    fs::write(dir.path().join("session_file.rs"), "package session\n").unwrap();
    fs::write(dir.path().join("other_file.rs"), "package other\n").unwrap();
    git_ok(dir.path(), &["add", "other_file.rs"]);
    git_ok(dir.path(), &["commit", "-m", "Add other_file.rs"]);
    let new_head = git_ok(dir.path(), &["rev-parse", "HEAD"]);

    let strategy = ManualCommitStrategy::new(dir.path());
    let session_id = "test-no-overlap";
    let mut state = idle_state(
        session_id,
        &base_head,
        vec!["session_file.rs".to_string()],
        1,
    );
    let transcript_path = write_session_transcript(
        dir.path(),
        session_id,
        r#"{"type":"human","message":{"content":"work on session_file.rs"}}
{"type":"assistant","message":{"content":"Done"}}"#,
    );
    state.transcript_path = transcript_path.to_string_lossy().to_string();

    let checkpoint_id = "00001a000001";
    strategy
        .condense_session(&mut state, checkpoint_id, &new_head)
        .unwrap();

    let metadata = read_checkpoint_session_metadata_from_branch(dir.path(), checkpoint_id);
    assert!(
        metadata.get("files_touched").is_none(),
        "committed session metadata should no longer persist files_touched"
    );
    let summary = read_committed(dir.path(), checkpoint_id)
        .unwrap()
        .expect("expected committed checkpoint summary");
    assert_eq!(
        summary.files_touched,
        vec!["other_file.rs".to_string()],
        "checkpoint summary should reflect the committed diff, not transient session files_touched"
    );
}

// Committed session metadata keeps turn/transcript start fields and token usage.
#[test]
pub(crate) fn condense_session_writes_turn_and_transcript_start_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let base_head = setup_git_repo(&dir);

    fs::write(dir.path().join("agent.rs"), "package main\n").unwrap();
    git_ok(dir.path(), &["add", "agent.rs"]);
    git_ok(dir.path(), &["commit", "-m", "Add agent.rs"]);
    let new_head = git_ok(dir.path(), &["rev-parse", "HEAD"]);

    let strategy = ManualCommitStrategy::new(dir.path());
    let session_id = "test-turn-and-transcript-start";
    let mut state = idle_state(session_id, &base_head, vec!["agent.rs".to_string()], 1);
    state.turn_id = "turn-123".to_string();
    state.pending.transcript_identifier_at_start = "user-1".to_string();
    state.pending.checkpoint_transcript_start = 1;
    let transcript_path = write_session_transcript(
        dir.path(),
        session_id,
        r#"{"uuid":"user-1","type":"user","message":{"content":"create agent.rs"}}
{"uuid":"assistant-1","type":"assistant","message":{"id":"msg_1","usage":{"input_tokens":8,"output_tokens":5}}}
"#,
    );
    state.transcript_path = transcript_path.to_string_lossy().to_string();

    let checkpoint_id = "00aa11bb22cc";
    strategy
        .condense_session(&mut state, checkpoint_id, &new_head)
        .unwrap();

    let metadata = read_checkpoint_session_metadata_from_branch(dir.path(), checkpoint_id);
    assert_eq!(metadata["turn_id"], "turn-123");
    assert_eq!(metadata["transcript_identifier_at_start"], "user-1");
    assert_eq!(metadata["checkpoint_transcript_start"], 1);
    assert_eq!(metadata["transcript_lines_at_start"], 1);
    assert_eq!(metadata["token_usage"]["input_tokens"], 8);
    assert_eq!(metadata["token_usage"]["output_tokens"], 5);
    assert_eq!(metadata["token_usage"]["api_call_count"], 1);
    assert_eq!(
        metadata["transcript_path"],
        transcript_path.to_string_lossy().to_string()
    );
    assert!(
        metadata.get("initial_attribution").is_some(),
        "manual-commit session metadata should include initial_attribution"
    );
    assert!(metadata["initial_attribution"]["calculated_at"].is_string());
    assert!(
        metadata["initial_attribution"]["agent_lines"]
            .as_i64()
            .unwrap_or_default()
            > 0
    );
    assert!(
        metadata["initial_attribution"]["total_committed"]
            .as_i64()
            .unwrap_or_default()
            > 0
    );
}

#[test]
pub(crate) fn update_summary_updates_session_metadata() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let checkpoint_id = "f1e2d3c4b5a6";
    let session_id = "test-session-summary";
    write_committed(
        dir.path(),
        WriteCommittedOptions {
            checkpoint_id: checkpoint_id.to_string(),
            session_id: session_id.to_string(),
            strategy: "manual-commit".to_string(),
            agent: "claude-code".to_string(),
            transcript:
                b"{\"type\":\"assistant\",\"message\":{\"content\":\"test transcript content\"}}\n"
                    .to_vec(),
            prompts: Some(vec!["summary prompt".to_string()]),
            context: None,
            checkpoints_count: 1,
            files_touched: vec!["file1.rs".to_string(), "file2.rs".to_string()],
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

    let before = read_checkpoint_session_metadata_from_branch(dir.path(), checkpoint_id);
    assert!(
        before.get("summary").is_none(),
        "initial checkpoint should not have summary field"
    );

    let summary = serde_json::json!({
        "intent": "Test intent",
        "outcome": "Test outcome",
        "learnings": {
            "repo": ["Repo learning 1"],
            "code": [{"path":"file1.rs","line":10,"finding":"Code finding"}],
            "workflow": ["Workflow learning"]
        },
        "friction": ["Some friction"],
        "open_items": ["Open item 1"]
    });

    let result = update_summary(dir.path(), checkpoint_id, summary.clone());
    assert!(
        result.is_ok(),
        "expected update_summary to persist summary into session metadata: {result:?}"
    );

    let after = read_checkpoint_session_metadata_from_branch(dir.path(), checkpoint_id);
    assert_eq!(after["summary"]["intent"], "Test intent");
    assert_eq!(after["summary"]["outcome"], "Test outcome");
    assert_eq!(after["session_id"], session_id);
    assert!(
        after.get("files_touched").is_none(),
        "committed session metadata should no longer persist files_touched"
    );
}

#[test]
pub(crate) fn update_summary_not_found() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let result = update_summary(
        dir.path(),
        "000000000000",
        serde_json::json!({"intent":"Test","outcome":"Test"}),
    );
    assert!(
        result.is_err(),
        "non-existent checkpoint should return error"
    );
    let msg = format!("{:#}", result.unwrap_err());
    assert!(
        msg.contains("checkpoint not found"),
        "expected checkpoint-not-found error, got: {msg}"
    );
}

#[test]
pub(crate) fn list_committed_reads_db_entries_without_metadata_branch() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let checkpoint_id = "abcdef123456";
    write_committed(
        dir.path(),
        WriteCommittedOptions {
            checkpoint_id: checkpoint_id.to_string(),
            session_id: "db-session-id".to_string(),
            strategy: "manual-commit".to_string(),
            agent: "claude-code".to_string(),
            transcript: b"{\"type\":\"assistant\",\"message\":{\"content\":\"db transcript\"}}\n"
                .to_vec(),
            prompts: Some(vec!["db prompt".to_string()]),
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

    assert!(
        run_git(dir.path(), &["rev-parse", paths::METADATA_BRANCH_NAME]).is_err(),
        "local metadata branch should not exist"
    );
    let checkpoints = list_committed(dir.path()).expect("list committed checkpoints");
    assert_eq!(checkpoints.len(), 1, "expected one committed checkpoint");
    assert_eq!(checkpoints[0].checkpoint_id, checkpoint_id);
}

#[test]
pub(crate) fn get_checkpoint_author_no_sessions_branch() {
    let dir = tempfile::tempdir().unwrap();
    let init = git_command()
        .args(["init"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(init.status.success());
    ensure_test_store_backends(dir.path());

    let result = get_checkpoint_author(dir.path(), "aabbccddeeff");
    assert!(
        result.is_ok(),
        "expected empty author (no error) when metadata branch is missing: {result:?}"
    );
    let author = result.unwrap();
    assert_eq!(author.name, "");
    assert_eq!(author.email, "");
}

#[test]
pub(crate) fn get_checkpoint_author_returns_author() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let checkpoint_id = "a1b2c3d4e5f6";
    write_committed(
        dir.path(),
        WriteCommittedOptions {
            checkpoint_id: checkpoint_id.to_string(),
            session_id: "author-session".to_string(),
            strategy: "manual-commit".to_string(),
            agent: "claude-code".to_string(),
            transcript:
                b"{\"type\":\"assistant\",\"message\":{\"content\":\"author transcript\"}}\n"
                    .to_vec(),
            prompts: Some(vec!["author prompt".to_string()]),
            context: None,
            checkpoints_count: 1,
            files_touched: vec!["main.rs".to_string()],
            token_usage_input: None,
            token_usage_output: None,
            token_usage_api_call_count: None,
            turn_id: String::new(),
            transcript_identifier_at_start: String::new(),
            checkpoint_transcript_start: 0,
            token_usage: None,
            initial_attribution: None,
            author_name: "Alice Developer".to_string(),
            author_email: "alice@example.com".to_string(),
            summary: None,
            is_task: false,
            tool_use_id: String::new(),
            agent_id: String::new(),
            transcript_path: String::new(),
            subagent_transcript_path: String::new(),
        },
    )
    .unwrap();

    let result = get_checkpoint_author(dir.path(), checkpoint_id);
    assert!(
        result.is_ok(),
        "expected checkpoint author lookup to succeed: {result:?}"
    );
    let author = result.unwrap();
    assert_eq!(author.name, "Alice Developer");
    assert_eq!(author.email, "alice@example.com");
}

#[test]
pub(crate) fn get_checkpoint_author_not_found() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let result = get_checkpoint_author(dir.path(), "ffffffffffff");
    assert!(
        result.is_ok(),
        "expected empty author (no error) for missing checkpoint: {result:?}"
    );
    let author = result.unwrap();
    assert_eq!(author.name, "");
    assert_eq!(author.email, "");
}

#[test]
pub(crate) fn write_committed_multiple_sessions_same_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let checkpoint_id = "a1a2a3a4a5a6";
    let strategy = ManualCommitStrategy::new(dir.path());

    let mut state_one = idle_state("session-one", &head, vec!["file1.rs".to_string()], 3);
    let mut state_two = idle_state("session-two", &head, vec!["file2.rs".to_string()], 2);

    condense_with_transcript(
        &strategy,
        &mut state_one,
        checkpoint_id,
        &head,
        r#"{"role":"assistant","content":"first session"}"#,
    );
    condense_with_transcript(
        &strategy,
        &mut state_two,
        checkpoint_id,
        &head,
        r#"{"role":"assistant","content":"second session"}"#,
    );

    let summary = read_committed(dir.path(), checkpoint_id)
        .unwrap()
        .expect("expected checkpoint summary");
    assert_eq!(summary.sessions.len(), 2, "expected 2 sessions in summary");
    assert!(summary.sessions[0].transcript.contains("/0/"));
    assert!(summary.sessions[1].transcript.contains("/1/"));

    let content0 = read_session_content(dir.path(), checkpoint_id, 0).unwrap();
    assert_eq!(content0.metadata["session_id"], "session-one");
    let content1 = read_session_content(dir.path(), checkpoint_id, 1).unwrap();
    assert_eq!(content1.metadata["session_id"], "session-two");
}

fn write_session_transcript(repo_root: &Path, session_id: &str, transcript_jsonl: &str) {
    let meta_dir = repo_root.join(paths::session_metadata_dir_from_session_id(session_id));
    fs::create_dir_all(&meta_dir).unwrap();
    fs::write(meta_dir.join(paths::TRANSCRIPT_FILE_NAME), transcript_jsonl).unwrap();
}

fn idle_state(
    session_id: &str,
    base_commit: &str,
    files_touched: Vec<String>,
    step_count: u32,
) -> SessionState {
    SessionState {
        session_id: session_id.to_string(),
        phase: crate::engine::session::phase::SessionPhase::Idle,
        base_commit: base_commit.to_string(),
        files_touched,
        step_count,
        agent_type: "claude-code".to_string(),
        ..Default::default()
    }
}

fn condense_with_transcript(
    strategy: &ManualCommitStrategy,
    state: &mut SessionState,
    checkpoint_id: &str,
    new_head: &str,
    transcript_jsonl: &str,
) {
    write_session_transcript(&strategy.repo_root, &state.session_id, transcript_jsonl);
    strategy
        .condense_session(state, checkpoint_id, new_head)
        .unwrap();
}

#[test]
fn condense_session_files_touched_fallback_empty_state() {
    let dir = tempfile::tempdir().unwrap();
    let base_head = setup_git_repo(&dir);

    fs::write(dir.path().join("agent.rs"), "package main\n").unwrap();
    git_ok(dir.path(), &["add", "agent.rs"]);
    git_ok(dir.path(), &["commit", "-m", "Add agent.rs"]);
    let new_head = git_ok(dir.path(), &["rev-parse", "HEAD"]);

    let strategy = ManualCommitStrategy::new(dir.path());
    let session_id = "test-empty-files";
    let mut state = idle_state(session_id, &base_head, vec![], 1);
    write_session_transcript(
        dir.path(),
        session_id,
        r#"{"type":"human","message":{"content":"create agent.rs"}}
{"type":"assistant","message":{"content":"Done"}}"#,
    );

    let checkpoint_id = "fa11bac00001";
    strategy
        .condense_session(&mut state, checkpoint_id, &new_head)
        .unwrap();

    let metadata = read_checkpoint_session_metadata_from_branch(dir.path(), checkpoint_id);
    let files = metadata["files_touched"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|v| v.as_str().map(ToString::to_string))
        .collect::<Vec<_>>();
    assert_eq!(
        files,
        vec!["agent.rs".to_string()],
        "fallback should use committed files when state.files_touched is empty"
    );
}

#[test]
fn condense_session_files_touched_no_fallback_no_overlap() {
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
    write_session_transcript(
        dir.path(),
        session_id,
        r#"{"type":"human","message":{"content":"work on session_file.rs"}}
{"type":"assistant","message":{"content":"Done"}}"#,
    );

    let checkpoint_id = "00001a000001";
    strategy
        .condense_session(&mut state, checkpoint_id, &new_head)
        .unwrap();

    let metadata = read_checkpoint_session_metadata_from_branch(dir.path(), checkpoint_id);
    let files = metadata["files_touched"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|v| v.as_str().map(ToString::to_string))
        .collect::<Vec<_>>();
    assert!(
        files.is_empty(),
        "should not fallback to committed files when session already tracked non-overlapping files: {files:?}"
    );
}

// Committed session metadata keeps turn/transcript start fields and token usage.
#[test]
fn condense_session_writes_turn_and_transcript_start_metadata() {
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
    state.transcript_identifier_at_start = "user-1".to_string();
    state.checkpoint_transcript_start = 1;
    state.transcript_path = "/tmp/transcript-session.jsonl".to_string();
    write_session_transcript(
        dir.path(),
        session_id,
        r#"{"uuid":"user-1","type":"user","message":{"content":"create agent.rs"}}
{"uuid":"assistant-1","type":"assistant","message":{"id":"msg_1","usage":{"input_tokens":8,"output_tokens":5}}}
"#,
    );

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
    assert_eq!(metadata["transcript_path"], "/tmp/transcript-session.jsonl");
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
fn update_summary_updates_session_metadata() {
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
    assert_eq!(after["files_touched"].as_array().map(Vec::len), Some(2));
}

#[test]
fn update_summary_not_found() {
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
fn list_committed_reads_db_entries_without_metadata_branch() {
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
fn get_checkpoint_author_no_sessions_branch() {
    let dir = tempfile::tempdir().unwrap();
    let init = git_command()
        .args(["init"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(init.status.success());

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
fn get_checkpoint_author_returns_author() {
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
fn get_checkpoint_author_not_found() {
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
fn write_committed_multiple_sessions_same_checkpoint() {
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

#[test]
fn read_committed_returns_checkpoint_summary() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let checkpoint_id = "c1c2c3c4c5c6";
    let strategy = ManualCommitStrategy::new(dir.path());

    let mut alpha = idle_state("session-alpha", &head, vec!["file0.rs".to_string()], 1);
    let mut beta = idle_state("session-beta", &head, vec!["file1.rs".to_string()], 2);
    condense_with_transcript(
        &strategy,
        &mut alpha,
        checkpoint_id,
        &head,
        r#"{"role":"assistant","content":"alpha"}"#,
    );
    condense_with_transcript(
        &strategy,
        &mut beta,
        checkpoint_id,
        &head,
        r#"{"role":"assistant","content":"beta"}"#,
    );

    let summary = read_committed(dir.path(), checkpoint_id)
        .unwrap()
        .expect("expected checkpoint summary");
    assert_eq!(summary.checkpoint_id, checkpoint_id);
    assert_eq!(summary.strategy, "manual-commit");
    assert_eq!(summary.sessions.len(), 2);
    assert!(summary.sessions[0].metadata.contains("/0/"));
    assert!(summary.sessions[1].metadata.contains("/1/"));
}

#[test]
fn write_committed_aggregation() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let checkpoint_id = "b1b2b3b4b5b6";
    write_committed(
        dir.path(),
        WriteCommittedOptions {
            checkpoint_id: checkpoint_id.to_string(),
            session_id: "session-one".to_string(),
            strategy: "manual-commit".to_string(),
            agent: "claude-code".to_string(),
            transcript: b"{\"message\":\"first\"}\n".to_vec(),
            prompts: Some(vec!["first prompt".to_string()]),
            context: None,
            checkpoints_count: 3,
            files_touched: vec!["a.rs".to_string(), "b.rs".to_string()],
            token_usage_input: Some(100),
            token_usage_output: Some(50),
            token_usage_api_call_count: Some(5),
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
    write_committed(
        dir.path(),
        WriteCommittedOptions {
            checkpoint_id: checkpoint_id.to_string(),
            session_id: "session-two".to_string(),
            strategy: "manual-commit".to_string(),
            agent: "claude-code".to_string(),
            transcript: b"{\"message\":\"second\"}\n".to_vec(),
            prompts: Some(vec!["second prompt".to_string()]),
            context: None,
            checkpoints_count: 2,
            files_touched: vec!["b.rs".to_string(), "c.rs".to_string()],
            token_usage_input: Some(50),
            token_usage_output: Some(25),
            token_usage_api_call_count: Some(3),
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

    let summary = read_committed(dir.path(), checkpoint_id)
        .unwrap()
        .expect("expected checkpoint summary");
    assert_eq!(summary.checkpoints_count, 5);
    assert_eq!(summary.files_touched, vec!["a.rs", "b.rs", "c.rs"]);

    let session_metadata = read_checkpoint_session_metadata_from_branch(dir.path(), checkpoint_id);
    assert!(
        session_metadata.get("token_usage").is_some(),
        "session metadata schema uses nested token_usage object"
    );
    assert!(
        session_metadata.get("token_usage_input").is_none()
            && session_metadata.get("token_usage_output").is_none()
            && session_metadata.get("token_usage_api_call_count").is_none(),
        "session metadata schema does not use flat token usage fields"
    );
    assert_eq!(session_metadata["token_usage"]["input_tokens"], 100);
    assert_eq!(session_metadata["token_usage"]["output_tokens"], 50);
    assert_eq!(session_metadata["token_usage"]["api_call_count"], 5);

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
    assert_eq!(top_metadata["token_usage"]["input_tokens"], 150);
    assert_eq!(top_metadata["token_usage"]["output_tokens"], 75);
    assert_eq!(top_metadata["token_usage"]["api_call_count"], 8);
}

#[test]
fn read_session_content_by_index() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let checkpoint_id = "d1d2d3d4d5d6";
    let strategy = ManualCommitStrategy::new(dir.path());

    let mut first = idle_state("session-first", &head, vec![], 1);
    let mut second = idle_state("session-second", &head, vec![], 1);
    condense_with_transcript(
        &strategy,
        &mut first,
        checkpoint_id,
        &head,
        r#"{"role":"user","content":"First user prompt"}
{"role":"assistant","content":"first"}"#,
    );
    condense_with_transcript(
        &strategy,
        &mut second,
        checkpoint_id,
        &head,
        r#"{"role":"user","content":"Second user prompt"}
{"role":"assistant","content":"second"}"#,
    );

    let content0 = read_session_content(dir.path(), checkpoint_id, 0).unwrap();
    assert_eq!(content0.metadata["session_id"], "session-first");
    assert!(
        content0.transcript.contains("first"),
        "session 0 transcript should contain first"
    );
    assert!(
        content0.prompts.contains("First"),
        "session 0 prompts should contain First"
    );

    let content1 = read_session_content(dir.path(), checkpoint_id, 1).unwrap();
    assert_eq!(content1.metadata["session_id"], "session-second");
    assert!(
        content1.transcript.contains("second"),
        "session 1 transcript should contain second"
    );
}

#[test]
fn read_session_content_invalid_index() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let checkpoint_id = "e1e2e3e4e5e6";
    let strategy = ManualCommitStrategy::new(dir.path());

    let mut only = idle_state("only-session", &head, vec![], 1);
    condense_with_transcript(
        &strategy,
        &mut only,
        checkpoint_id,
        &head,
        r#"{"single": true}"#,
    );

    let err = read_session_content(dir.path(), checkpoint_id, 1).unwrap_err();
    let msg = format!("{:#}", err);
    assert!(
        msg.contains("session 1 not found"),
        "error should mention session not found, got: {msg}"
    );
}

#[test]
fn read_latest_session_content_returns_latest() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let checkpoint_id = "f1f2f3f4f5f6";
    let strategy = ManualCommitStrategy::new(dir.path());

    for i in 0..3 {
        let session_id = format!("session-{i}");
        let mut state = idle_state(&session_id, &head, vec![], 1);
        condense_with_transcript(
            &strategy,
            &mut state,
            checkpoint_id,
            &head,
            &format!(r#"{{"index": {i}}}"#),
        );
    }

    let content = read_latest_session_content(dir.path(), checkpoint_id).unwrap();
    assert_eq!(content.metadata["session_id"], "session-2");
    assert!(
        content.transcript.contains(r#""index": 2"#),
        "latest session transcript should contain index 2"
    );
}

#[test]
fn read_session_content_by_id_lookup() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let checkpoint_id = "010203040506";
    let strategy = ManualCommitStrategy::new(dir.path());

    for session_id in ["unique-id-alpha", "unique-id-beta"] {
        let mut state = idle_state(session_id, &head, vec![], 1);
        condense_with_transcript(
            &strategy,
            &mut state,
            checkpoint_id,
            &head,
            &format!(r#"{{"session_name": "{session_id}"}}"#),
        );
    }

    let content = read_session_content_by_id(dir.path(), checkpoint_id, "unique-id-beta").unwrap();
    assert_eq!(content.metadata["session_id"], "unique-id-beta");
    assert!(
        content.transcript.contains("unique-id-beta"),
        "transcript should contain the target session id"
    );
}

#[test]
fn read_session_content_by_id_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let checkpoint_id = "111213141516";
    let strategy = ManualCommitStrategy::new(dir.path());

    let mut existing = idle_state("existing-session", &head, vec![], 1);
    condense_with_transcript(
        &strategy,
        &mut existing,
        checkpoint_id,
        &head,
        r#"{"exists": true}"#,
    );

    let err =
        read_session_content_by_id(dir.path(), checkpoint_id, "nonexistent-session").unwrap_err();
    let msg = format!("{:#}", err);
    assert!(
        msg.contains("not found"),
        "error should mention not found, got: {msg}"
    );
}

#[test]
fn list_committed_multi_session_info() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let checkpoint_id = "212223242526";
    let strategy = ManualCommitStrategy::new(dir.path());

    let mut one = idle_state("list-session-1", &head, vec!["file0.rs".to_string()], 1);
    let mut two = idle_state("list-session-2", &head, vec!["file1.rs".to_string()], 2);
    two.agent_type = "gemini-cli".to_string();
    condense_with_transcript(&strategy, &mut one, checkpoint_id, &head, r#"{"i": 0}"#);
    condense_with_transcript(&strategy, &mut two, checkpoint_id, &head, r#"{"i": 1}"#);

    let checkpoints = list_committed(dir.path()).unwrap();
    let found = checkpoints
        .into_iter()
        .find(|cp| cp.checkpoint_id == checkpoint_id)
        .expect("checkpoint should be present in list");

    assert_eq!(found.session_count, 2, "SessionCount should be 2");
    assert_eq!(
        found.session_id, "list-session-2",
        "latest session id should be exposed"
    );
    assert_eq!(
        found.agent, "gemini-cli",
        "agent should come from latest session metadata"
    );
    assert_eq!(
        found.agents,
        vec![AGENT_TYPE_CLAUDE_CODE.to_string(), "gemini-cli".to_string()],
        "agents should include all unique session agents in order"
    );
}

#[test]
fn write_committed_session_with_no_prompts() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let checkpoint_id = "313233343536";

    let result = write_committed(
        dir.path(),
        WriteCommittedOptions {
            checkpoint_id: checkpoint_id.to_string(),
            session_id: "no-prompts-session".to_string(),
            strategy: "manual-commit".to_string(),
            agent: "claude-code".to_string(),
            transcript: br#"{"no_prompts": true}"#.to_vec(),
            prompts: None,
            context: Some(b"Some context".to_vec()),
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
    );
    assert!(
        result.is_ok(),
        "expected write_committed to succeed for no-prompts session: {result:?}"
    );

    let content = read_session_content(dir.path(), checkpoint_id, 0).unwrap();
    assert_eq!(content.metadata["session_id"], "no-prompts-session");
    assert!(
        !content.transcript.is_empty(),
        "Transcript should not be empty"
    );
    assert_eq!(content.prompts, "", "Prompts should be empty");
    assert_eq!(content.context, "Some context");
}

#[test]
fn write_committed_session_with_summary() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let checkpoint_id = "aabbccddeeff";

    let summary = serde_json::json!({
        "intent": "User wanted to fix a bug",
        "outcome": "Bug was fixed"
    });
    let update = write_committed(
        dir.path(),
        WriteCommittedOptions {
            checkpoint_id: checkpoint_id.to_string(),
            session_id: "summary-session".to_string(),
            strategy: "manual-commit".to_string(),
            agent: "claude-code".to_string(),
            transcript: br#"{"test": true}"#.to_vec(),
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
            summary: Some(summary),
            is_task: false,
            tool_use_id: String::new(),
            agent_id: String::new(),
            transcript_path: String::new(),
            subagent_transcript_path: String::new(),
        },
    );
    assert!(
        update.is_ok(),
        "expected write_committed to persist summary metadata: {update:?}"
    );

    let content = read_session_content(dir.path(), checkpoint_id, 0).unwrap();
    assert!(
        !content.metadata["summary"].is_null(),
        "summary should be present in session metadata"
    );
    assert_eq!(
        content.metadata["summary"]["intent"],
        "User wanted to fix a bug"
    );
    assert_eq!(content.metadata["summary"]["outcome"], "Bug was fixed");
}

#[test]
fn write_committed_session_with_no_context() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let checkpoint_id = "414243444546";

    let result = write_committed(
        dir.path(),
        WriteCommittedOptions {
            checkpoint_id: checkpoint_id.to_string(),
            session_id: "no-context-session".to_string(),
            strategy: "manual-commit".to_string(),
            agent: "claude-code".to_string(),
            transcript: br#"{"no_context": true}"#.to_vec(),
            prompts: Some(vec!["A prompt".to_string()]),
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
    );
    assert!(
        result.is_ok(),
        "expected write_committed to succeed for no-context session: {result:?}"
    );

    let content = read_session_content(dir.path(), checkpoint_id, 0).unwrap();
    assert_eq!(content.metadata["session_id"], "no-context-session");
    assert!(
        !content.transcript.is_empty(),
        "Transcript should not be empty"
    );
    assert!(
        content.prompts.contains("A prompt"),
        "Prompts should include the user prompt"
    );
    assert_eq!(content.context, "", "Context should be empty");
}

#[test]
fn write_committed_persists_checkpoint_sessions_and_blobs_in_sqlite() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let checkpoint_id = "919293949596";
    let transcript =
        "{\"type\":\"assistant\",\"message\":{\"content\":\"db-backed transcript\"}}\n";
    let prompts = vec!["first prompt".to_string(), "second prompt".to_string()];
    let context = b"db context payload".to_vec();

    let result = write_committed(
        dir.path(),
        WriteCommittedOptions {
            checkpoint_id: checkpoint_id.to_string(),
            session_id: "db-session".to_string(),
            strategy: "manual-commit".to_string(),
            agent: AGENT_TYPE_CLAUDE_CODE.to_string(),
            transcript: transcript.as_bytes().to_vec(),
            prompts: Some(prompts.clone()),
            context: Some(context.clone()),
            checkpoints_count: 2,
            files_touched: vec!["src/lib.rs".to_string()],
            token_usage_input: Some(10),
            token_usage_output: Some(5),
            token_usage_api_call_count: Some(1),
            turn_id: "turn-db-1".to_string(),
            transcript_identifier_at_start: "msg-1".to_string(),
            checkpoint_transcript_start: 0,
            token_usage: None,
            initial_attribution: None,
            author_name: "DB Test".to_string(),
            author_email: "db@test.com".to_string(),
            summary: None,
            is_task: false,
            tool_use_id: String::new(),
            agent_id: String::new(),
            transcript_path: String::new(),
            subagent_transcript_path: String::new(),
        },
    );
    assert!(
        result.is_ok(),
        "write_committed should persist to DB/blob storage: {result:?}"
    );

    let sqlite = SqliteConnectionPool::connect(temporary_checkpoints_db_path(dir.path()))
        .expect("connect checkpoint sqlite");
    sqlite
        .initialise_checkpoint_schema()
        .expect("initialise checkpoint schema");
    let repo_id = crate::engine::devql::resolve_repo_identity(dir.path())
        .expect("resolve repo identity")
        .repo_id;

    let checkpoint_rows = sqlite
        .with_connection(|conn| {
            let count: i64 = conn.query_row(
                "SELECT COUNT(*)
                 FROM checkpoints
                 WHERE checkpoint_id = ?1 AND repo_id = ?2",
                rusqlite::params![checkpoint_id, repo_id.as_str()],
                |row| row.get(0),
            )?;
            Ok(count)
        })
        .expect("query checkpoint row count");
    assert_eq!(
        checkpoint_rows, 1,
        "expected checkpoints row for write_committed"
    );

    let content_hash = query_checkpoint_session_content_hash(dir.path(), checkpoint_id, "db-session")
        .expect("checkpoint_sessions row should exist");
    assert_eq!(
        content_hash,
        format!("sha256:{}", sha256_hex(transcript.as_bytes())),
        "session row should persist transcript hash"
    );

    let expected_blobs = [
        ("transcript", transcript.to_string(), "transcript.jsonl"),
        ("prompts", prompts.join("\n\n---\n\n"), "prompts.txt"),
        (
            "context",
            String::from_utf8_lossy(&context).to_string(),
            "context.md",
        ),
    ];
    for (blob_type, expected_content, expected_file_name) in expected_blobs {
        let row = query_checkpoint_blob_row(dir.path(), checkpoint_id, 0, blob_type)
            .unwrap_or_else(|| panic!("expected checkpoint_blobs row for blob_type={blob_type}"));
        let payload = read_blob_payload_from_storage(dir.path(), &row.storage_path);
        assert_eq!(String::from_utf8_lossy(&payload), expected_content);
        assert!(
            row.storage_path.ends_with(expected_file_name),
            "storage path should end with {expected_file_name}, got {}",
            row.storage_path
        );
    }
}

#[test]
fn update_committed_updates_db_blob_and_content_hash() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let checkpoint_id = "929394959697";

    let mut initial = default_write_committed_opts(
        checkpoint_id,
        "update-db-session",
        "{\"type\":\"assistant\",\"message\":{\"content\":\"before\"}}\n",
    );
    initial.prompts = Some(vec!["before prompt".to_string()]);
    initial.context = Some(b"before context".to_vec());
    write_committed(dir.path(), initial).expect("initial write_committed");

    let before_hash = query_checkpoint_session_content_hash(dir.path(), checkpoint_id, "update-db-session")
        .expect("content hash before update");

    let updated_transcript = "{\"type\":\"assistant\",\"message\":{\"content\":\"after\"}}\n";
    let update = update_committed(
        dir.path(),
        UpdateCommittedOptions {
            checkpoint_id: checkpoint_id.to_string(),
            session_id: "update-db-session".to_string(),
            transcript: Some(updated_transcript.as_bytes().to_vec()),
            prompts: Some(vec!["after prompt".to_string()]),
            context: Some(b"after context".to_vec()),
            agent: AGENT_TYPE_CLAUDE_CODE.to_string(),
        },
    );
    assert!(
        update.is_ok(),
        "update_committed should update DB/blob storage: {update:?}"
    );

    let after_hash = query_checkpoint_session_content_hash(dir.path(), checkpoint_id, "update-db-session")
        .expect("content hash after update");
    assert_ne!(before_hash, after_hash, "content hash should be refreshed");
    assert_eq!(
        after_hash,
        format!("sha256:{}", sha256_hex(updated_transcript.as_bytes()))
    );

    let transcript_blob = query_checkpoint_blob_row(dir.path(), checkpoint_id, 0, "transcript")
        .expect("transcript blob reference should exist");
    assert_eq!(
        transcript_blob.content_hash,
        format!("sha256:{}", sha256_hex(updated_transcript.as_bytes()))
    );
    let transcript_payload = read_blob_payload_from_storage(dir.path(), &transcript_blob.storage_path);
    assert_eq!(
        String::from_utf8_lossy(&transcript_payload),
        updated_transcript
    );
}

#[test]
fn write_committed_records_local_backend_in_blob_row() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let checkpoint_id = "949596979899";

    let result = write_committed(
        dir.path(),
        default_write_committed_opts(
            checkpoint_id,
            "fallback-session",
            "{\"type\":\"assistant\",\"message\":{\"content\":\"fallback\"}}\n",
        ),
    );
    assert!(
        result.is_ok(),
        "write_committed should persist transcript blobs locally: {result:?}"
    );

    let transcript_blob = query_checkpoint_blob_row(dir.path(), checkpoint_id, 0, "transcript")
        .expect("transcript blob reference should exist");
    assert_eq!(
        transcript_blob.storage_backend, "local",
        "storage_backend should record effective local fallback backend"
    );
}

#[test]
fn update_summary_persists_summary_in_checkpoint_sessions_table() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let checkpoint_id = "939495969798";

    write_committed(
        dir.path(),
        default_write_committed_opts(
            checkpoint_id,
            "summary-db-session",
            "{\"type\":\"assistant\",\"message\":{\"content\":\"summary\"}}\n",
        ),
    )
    .expect("initial write_committed");

    let summary = serde_json::json!({
        "intent": "Persist summary in DB",
        "outcome": "Summary updated"
    });
    let update = update_summary(dir.path(), checkpoint_id, summary.clone());
    assert!(
        update.is_ok(),
        "update_summary should persist to checkpoint_sessions: {update:?}"
    );

    let sqlite = SqliteConnectionPool::connect(temporary_checkpoints_db_path(dir.path()))
        .expect("connect checkpoint sqlite");
    sqlite
        .initialise_checkpoint_schema()
        .expect("initialise checkpoint schema");
    let summary_json = sqlite
        .with_connection(|conn| {
            conn.query_row(
                "SELECT summary
                 FROM checkpoint_sessions
                 WHERE checkpoint_id = ?1 AND session_id = ?2
                 LIMIT 1",
                rusqlite::params![checkpoint_id, "summary-db-session"],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(anyhow::Error::from)
        })
        .expect("query checkpoint_sessions summary")
        .flatten()
        .expect("summary column should be populated");
    let saved: serde_json::Value =
        serde_json::from_str(&summary_json).expect("parse summary JSON");
    assert_eq!(saved["intent"], "Persist summary in DB");
    assert_eq!(saved["outcome"], "Summary updated");
}

#[test]
fn write_committed_three_sessions() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let checkpoint_id = "515253545556";

    for i in 0..3 {
        let result = write_committed(
            dir.path(),
            WriteCommittedOptions {
                checkpoint_id: checkpoint_id.to_string(),
                session_id: format!("three-session-{i}"),
                strategy: "manual-commit".to_string(),
                agent: "claude-code".to_string(),
                transcript: format!(r#"{{"session_number": {i}}}"#).into_bytes(),
                prompts: None,
                context: None,
                checkpoints_count: (i + 1) as u32,
                files_touched: vec![format!("s{i}.rs")],
                token_usage_input: Some((i as u64 + 1) * 100),
                token_usage_output: Some((i as u64 + 1) * 50),
                token_usage_api_call_count: Some((i as u64 + 1) * 5),
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
        );
        assert!(
            result.is_ok(),
            "expected write_committed to succeed for session {i}: {result:?}"
        );
    }

    let summary = read_committed(dir.path(), checkpoint_id)
        .unwrap()
        .expect("expected checkpoint summary");
    assert_eq!(summary.sessions.len(), 3, "expected 3 sessions");
    assert_eq!(
        summary.checkpoints_count, 6,
        "expected aggregated checkpoint count"
    );
    assert_eq!(
        summary.files_touched.len(),
        3,
        "expected aggregated files touched"
    );
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
    assert_eq!(
        top_metadata["token_usage"]["input_tokens"], 600,
        "expected aggregated input tokens across sessions"
    );

    for i in 0..3 {
        let content = read_session_content(dir.path(), checkpoint_id, i).unwrap();
        assert_eq!(content.metadata["session_id"], format!("three-session-{i}"));
    }
}

#[test]
fn read_committed_nonexistent_checkpoint() {
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

    let summary = read_committed(dir.path(), "ffffffffffff").unwrap();
    assert!(
        summary.is_none(),
        "nonexistent checkpoint should return None, not an error"
    );
}

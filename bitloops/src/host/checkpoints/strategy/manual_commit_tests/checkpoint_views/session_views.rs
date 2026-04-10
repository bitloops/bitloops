use super::*;

#[test]
pub(crate) fn read_committed_returns_checkpoint_summary() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo_with_checkpoint_backends(&dir);
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
pub(crate) fn write_committed_aggregation() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo_with_checkpoint_backends(&dir);
    commit_files(
        dir.path(),
        &[
            ("a.rs", "pub fn a() {}\n"),
            ("b.rs", "pub fn b() {}\n"),
            ("c.rs", "pub fn c() {}\n"),
        ],
        "prepare committed checkpoint provenance",
    );
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
pub(crate) fn read_session_content_by_index() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo_with_checkpoint_backends(&dir);
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
pub(crate) fn read_session_content_invalid_index() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo_with_checkpoint_backends(&dir);
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
pub(crate) fn read_latest_session_content_returns_latest() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo_with_checkpoint_backends(&dir);
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
pub(crate) fn read_session_content_by_id_lookup() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo_with_checkpoint_backends(&dir);
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
pub(crate) fn read_session_content_by_id_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo_with_checkpoint_backends(&dir);
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
pub(crate) fn list_committed_multi_session_info() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo_with_checkpoint_backends(&dir);
    let checkpoint_id = "212223242526";
    let strategy = ManualCommitStrategy::new(dir.path());

    let mut one = idle_state("list-session-1", &head, vec!["file0.rs".to_string()], 1);
    let mut two = idle_state("list-session-2", &head, vec!["file1.rs".to_string()], 2);
    two.agent_type = "gemini".to_string();
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
        found.agent, "gemini",
        "agent should come from latest session metadata"
    );
    assert_eq!(
        found.agents,
        vec![AGENT_TYPE_CLAUDE_CODE.to_string(), "gemini".to_string()],
        "agents should include all unique session agents in order"
    );
}

#[test]
pub(crate) fn write_committed_session_with_no_prompts() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo_with_checkpoint_backends(&dir);
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
pub(crate) fn write_committed_session_with_summary() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo_with_checkpoint_backends(&dir);
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
pub(crate) fn write_committed_session_with_no_context() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo_with_checkpoint_backends(&dir);
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

use super::*;

pub(crate) fn setup_update_committed_fixture_with_sessions(
    dir: &TempDir,
    checkpoint_id: &str,
    session_ids: &[&str],
) {
    if !dir.path().join(".git").exists() {
        setup_git_repo(dir);
    }

    for session_id in session_ids {
        write_committed(
            dir.path(),
            WriteCommittedOptions {
                checkpoint_id: checkpoint_id.to_string(),
                session_id: (*session_id).to_string(),
                strategy: "manual-commit".to_string(),
                agent: AGENT_TYPE_CLAUDE_CODE.to_string(),
                transcript: format!("provisional transcript for {session_id}\n").into_bytes(),
                prompts: Some(vec![format!("initial prompt for {session_id}")]),
                context: Some(format!("initial context for {session_id}").into_bytes()),
                checkpoints_count: 1,
                files_touched: vec!["README.md".to_string()],
                token_usage_input: None,
                token_usage_output: None,
                token_usage_api_call_count: None,
                turn_id: "turn-001".to_string(),
                transcript_identifier_at_start: "transcript-start".to_string(),
                checkpoint_transcript_start: 0,
                token_usage: None,
                initial_attribution: None,
                author_name: "Test".to_string(),
                author_email: "test@test.com".to_string(),
                summary: None,
                is_task: false,
                tool_use_id: String::new(),
                agent_id: String::new(),
                transcript_path: String::new(),
                subagent_transcript_path: String::new(),
            },
        )
        .unwrap();
    }
}

pub(crate) fn setup_update_committed_fixture(dir: &TempDir) -> String {
    let cp = "a1b2c3d4e5f6".to_string();
    setup_update_committed_fixture_with_sessions(dir, &cp, &["session-001"]);
    cp
}

pub(crate) fn read_update_fixture_file(
    dir: &TempDir,
    checkpoint_id: &str,
    session_index: usize,
    file_name: &str,
) -> String {
    match file_name {
        paths::TRANSCRIPT_FILE_NAME => {
            read_session_content(dir.path(), checkpoint_id, session_index)
                .expect("read session content")
                .transcript
        }
        paths::PROMPT_FILE_NAME => {
            read_session_content(dir.path(), checkpoint_id, session_index)
                .expect("read session content")
                .prompts
        }
        paths::CONTEXT_FILE_NAME => {
            read_session_content(dir.path(), checkpoint_id, session_index)
                .expect("read session content")
                .context
        }
        paths::CONTENT_HASH_FILE_NAME => query_checkpoint_session_content_hash_by_index(
            dir.path(),
            checkpoint_id,
            session_index as i64,
        )
        .expect("session content hash should exist"),
        _ => panic!("unsupported fixture file read: {file_name}"),
    }
}

#[test]
pub(crate) fn update_committed_replaces_transcript() {
    let dir = tempfile::tempdir().unwrap();
    let cp = setup_update_committed_fixture(&dir);
    let full_transcript =
        "full transcript line 1\nfull transcript line 2\nfull transcript line 3\n";
    let opts = UpdateCommittedOptions {
        checkpoint_id: cp.clone(),
        session_id: "session-001".to_string(),
        transcript: Some(full_transcript.as_bytes().to_vec()),
        prompts: None,
        context: None,
        agent: AGENT_TYPE_CLAUDE_CODE.to_string(),
    };
    update_committed(dir.path(), opts).unwrap();

    let content = read_session_content(dir.path(), &cp, 0).unwrap();
    assert_eq!(content.transcript, full_transcript);
}

#[test]
pub(crate) fn update_committed_replaces_prompts() {
    let dir = tempfile::tempdir().unwrap();
    let cp = setup_update_committed_fixture(&dir);
    let expected_prompts = "prompt 1\n\n---\n\nprompt 2\n\n---\n\nprompt 3";
    update_committed(
        dir.path(),
        UpdateCommittedOptions {
            checkpoint_id: cp.clone(),
            session_id: "session-001".to_string(),
            transcript: None,
            prompts: Some(vec![
                "prompt 1".to_string(),
                "prompt 2".to_string(),
                "prompt 3".to_string(),
            ]),
            context: None,
            agent: AGENT_TYPE_CLAUDE_CODE.to_string(),
        },
    )
    .unwrap();

    let content = read_session_content(dir.path(), &cp, 0).unwrap();
    assert_eq!(content.prompts, expected_prompts);
}

#[test]
pub(crate) fn update_committed_replaces_context() {
    let dir = tempfile::tempdir().unwrap();
    let cp = setup_update_committed_fixture(&dir);
    let expected_context = "updated context with full session info";
    update_committed(
        dir.path(),
        UpdateCommittedOptions {
            checkpoint_id: cp.clone(),
            session_id: "session-001".to_string(),
            transcript: None,
            prompts: None,
            context: Some(expected_context.as_bytes().to_vec()),
            agent: AGENT_TYPE_CLAUDE_CODE.to_string(),
        },
    )
    .unwrap();

    let content = read_session_content(dir.path(), &cp, 0).unwrap();
    assert_eq!(content.context, expected_context);
}

#[test]
pub(crate) fn update_committed_replaces_all_fields_together() {
    let dir = tempfile::tempdir().unwrap();
    let cp = setup_update_committed_fixture(&dir);
    let expected_transcript = "complete transcript\n";
    let expected_prompts = "final prompt";
    let expected_context = "final context";
    update_committed(
        dir.path(),
        UpdateCommittedOptions {
            checkpoint_id: cp.clone(),
            session_id: "session-001".to_string(),
            transcript: Some(expected_transcript.as_bytes().to_vec()),
            prompts: Some(vec!["final prompt".to_string()]),
            context: Some(expected_context.as_bytes().to_vec()),
            agent: AGENT_TYPE_CLAUDE_CODE.to_string(),
        },
    )
    .unwrap();

    let content = read_session_content(dir.path(), &cp, 0).unwrap();
    assert_eq!(content.transcript, expected_transcript);
    assert_eq!(content.prompts, expected_prompts);
    assert_eq!(content.context, expected_context);
}

#[test]
pub(crate) fn update_committed_nonexistent_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let result = update_committed(
        dir.path(),
        UpdateCommittedOptions {
            checkpoint_id: "deadbeef1234".to_string(),
            session_id: "session-001".to_string(),
            transcript: Some(b"should fail".to_vec()),
            prompts: None,
            context: None,
            agent: AGENT_TYPE_CLAUDE_CODE.to_string(),
        },
    );
    assert!(result.is_err(), "expected nonexistent checkpoint error");
}

#[test]
pub(crate) fn update_committed_preserves_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let cp = setup_update_committed_fixture(&dir);
    let before = read_session_content(dir.path(), &cp, 0).unwrap();

    update_committed(
        dir.path(),
        UpdateCommittedOptions {
            checkpoint_id: cp.clone(),
            session_id: "session-001".to_string(),
            transcript: Some(b"updated transcript\n".to_vec()),
            prompts: None,
            context: None,
            agent: AGENT_TYPE_CLAUDE_CODE.to_string(),
        },
    )
    .unwrap();

    let after = read_session_content(dir.path(), &cp, 0).unwrap();
    assert_eq!(after.metadata["session_id"], before.metadata["session_id"]);
    assert_eq!(after.metadata["strategy"], before.metadata["strategy"]);
}

#[test]
pub(crate) fn update_committed_multiple_checkpoints() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let cp1 = "a1b2c3d4e5f6".to_string();
    let cp2 = "b2c3d4e5f6a1".to_string();
    setup_update_committed_fixture_with_sessions(&dir, &cp1, &["session-001"]);
    setup_update_committed_fixture_with_sessions(&dir, &cp2, &["session-001"]);

    let full_transcript = "complete full transcript\n";
    for checkpoint_id in [&cp1, &cp2] {
        update_committed(
            dir.path(),
            UpdateCommittedOptions {
                checkpoint_id: checkpoint_id.to_string(),
                session_id: "session-001".to_string(),
                transcript: Some(full_transcript.as_bytes().to_vec()),
                prompts: Some(vec![
                    "final prompt 1".to_string(),
                    "final prompt 2".to_string(),
                ]),
                context: Some(b"final context".to_vec()),
                agent: AGENT_TYPE_CLAUDE_CODE.to_string(),
            },
        )
        .unwrap();
    }

    for checkpoint_id in [&cp1, &cp2] {
        let content = read_session_content(dir.path(), checkpoint_id, 0).unwrap();
        assert_eq!(content.transcript, full_transcript);
    }
}

#[test]
pub(crate) fn update_committed_updates_content_hash() {
    let dir = tempfile::tempdir().unwrap();
    let cp = setup_update_committed_fixture(&dir);
    let old_hash = read_update_fixture_file(&dir, &cp, 0, paths::CONTENT_HASH_FILE_NAME);
    let new_transcript = "new full transcript content\n";

    update_committed(
        dir.path(),
        UpdateCommittedOptions {
            checkpoint_id: cp.clone(),
            session_id: "session-001".to_string(),
            transcript: Some(new_transcript.as_bytes().to_vec()),
            prompts: None,
            context: None,
            agent: AGENT_TYPE_CLAUDE_CODE.to_string(),
        },
    )
    .unwrap();

    let new_hash = read_update_fixture_file(&dir, &cp, 0, paths::CONTENT_HASH_FILE_NAME);
    assert!(new_hash.starts_with("sha256:"));
    assert_ne!(new_hash, old_hash);
    assert_eq!(
        new_hash,
        format!("sha256:{}", sha256_hex(new_transcript.as_bytes()))
    );
}

#[test]
pub(crate) fn update_committed_empty_checkpoint_id() {
    let dir = tempfile::tempdir().unwrap();
    let result = update_committed(
        dir.path(),
        UpdateCommittedOptions {
            checkpoint_id: String::new(),
            session_id: "session-001".to_string(),
            transcript: Some(b"should fail".to_vec()),
            prompts: None,
            context: None,
            agent: AGENT_TYPE_CLAUDE_CODE.to_string(),
        },
    );
    assert!(result.is_err(), "expected error for empty checkpoint id");
}

#[test]
pub(crate) fn update_committed_falls_back_to_latest_session() {
    let dir = tempfile::tempdir().unwrap();
    let cp = "f1e2d3c4b5a6".to_string();
    setup_update_committed_fixture_with_sessions(&dir, &cp, &["session-001", "session-002"]);
    let session0_before = read_update_fixture_file(&dir, &cp, 0, paths::TRANSCRIPT_FILE_NAME);
    let updated = "updated via fallback\n";

    update_committed(
        dir.path(),
        UpdateCommittedOptions {
            checkpoint_id: cp.clone(),
            session_id: "nonexistent-session".to_string(),
            transcript: Some(updated.as_bytes().to_vec()),
            prompts: None,
            context: None,
            agent: AGENT_TYPE_CLAUDE_CODE.to_string(),
        },
    )
    .unwrap();

    assert_eq!(
        read_update_fixture_file(&dir, &cp, 1, paths::TRANSCRIPT_FILE_NAME),
        updated
    );
    assert_eq!(
        read_update_fixture_file(&dir, &cp, 0, paths::TRANSCRIPT_FILE_NAME),
        session0_before
    );
}

#[test]
pub(crate) fn update_committed_summary_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let cp = setup_update_committed_fixture(&dir);
    let before = read_committed(dir.path(), &cp).unwrap().unwrap();

    update_committed(
        dir.path(),
        UpdateCommittedOptions {
            checkpoint_id: cp.clone(),
            session_id: "session-001".to_string(),
            transcript: Some(b"updated\n".to_vec()),
            prompts: None,
            context: None,
            agent: AGENT_TYPE_CLAUDE_CODE.to_string(),
        },
    )
    .unwrap();

    let after = read_committed(dir.path(), &cp).unwrap().unwrap();
    assert_eq!(after.checkpoint_id, before.checkpoint_id);
    assert_eq!(after.sessions.len(), before.sessions.len());
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub(crate) struct StateSnippet {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    turn_checkpoint_ids: Vec<String>,
}

#[test]
pub(crate) fn state_turn_checkpoint_ids_json() {
    let original = StateSnippet {
        turn_checkpoint_ids: vec!["a1b2c3d4e5f6".to_string(), "b2c3d4e5f6a1".to_string()],
    };
    let data = serde_json::to_string(&original).unwrap();
    let decoded: StateSnippet = serde_json::from_str(&data).unwrap();
    assert_eq!(decoded.turn_checkpoint_ids.len(), 2);

    let empty = StateSnippet::default();
    let empty_data = serde_json::to_string(&empty).unwrap();
    assert_eq!(empty_data, "{}");
}

#[test]
pub(crate) fn update_committed_preserves_existing_author_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let cp = setup_update_committed_fixture(&dir);

    let update = update_committed(
        dir.path(),
        UpdateCommittedOptions {
            checkpoint_id: cp.clone(),
            session_id: "session-001".to_string(),
            transcript: Some(b"full transcript\n".to_vec()),
            prompts: None,
            context: None,
            agent: AGENT_TYPE_CLAUDE_CODE.to_string(),
        },
    );
    assert!(
        update.is_ok(),
        "expected update_committed to succeed: {update:?}"
    );

    let author = get_checkpoint_author(dir.path(), &cp).expect("read checkpoint author");
    assert_eq!(author.name, "Test");
    assert_eq!(author.email, "test@test.com");
}

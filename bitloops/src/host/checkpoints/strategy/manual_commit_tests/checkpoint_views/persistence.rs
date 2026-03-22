use super::*;
use rusqlite::OptionalExtension;

#[test]
pub(crate) fn write_committed_persists_checkpoint_sessions_and_blobs_in_sqlite() {
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
    let repo_id = crate::host::devql::resolve_repo_identity(dir.path())
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

    let content_hash =
        query_checkpoint_session_content_hash(dir.path(), checkpoint_id, "db-session")
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
pub(crate) fn update_committed_updates_db_blob_and_content_hash() {
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

    let before_hash =
        query_checkpoint_session_content_hash(dir.path(), checkpoint_id, "update-db-session")
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

    let after_hash =
        query_checkpoint_session_content_hash(dir.path(), checkpoint_id, "update-db-session")
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
    let transcript_payload =
        read_blob_payload_from_storage(dir.path(), &transcript_blob.storage_path);
    assert_eq!(
        String::from_utf8_lossy(&transcript_payload),
        updated_transcript
    );
}

#[test]
pub(crate) fn write_committed_records_local_backend_in_blob_row() {
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
pub(crate) fn update_summary_persists_summary_in_checkpoint_sessions_table() {
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
        .with_connection(|conn| -> anyhow::Result<Option<Option<String>>> {
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
    let saved: serde_json::Value = serde_json::from_str(&summary_json).expect("parse summary JSON");
    assert_eq!(saved["intent"], "Persist summary in DB");
    assert_eq!(saved["outcome"], "Summary updated");
}

#[test]
pub(crate) fn write_committed_three_sessions() {
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
pub(crate) fn read_committed_nonexistent_checkpoint() {
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

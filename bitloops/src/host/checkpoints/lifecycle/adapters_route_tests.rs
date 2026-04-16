use super::*;
use crate::adapters::agents::{
    AGENT_NAME_CODEX, AGENT_NAME_COPILOT, AGENT_NAME_CURSOR, AGENT_NAME_GEMINI,
    AGENT_NAME_OPEN_CODE,
};
use crate::host::interactions::db_store::interaction_spool_db_path;
use crate::test_support::process_state::{git_command, with_process_state};
use anyhow::Result;
use tempfile::TempDir;

fn git_ok(repo_root: &std::path::Path, args: &[&str]) {
    let output = git_command()
        .args(args)
        .current_dir(repo_root)
        .output()
        .unwrap_or_else(|err| panic!("failed to start git {:?}: {err}", args));
    assert!(
        output.status.success(),
        "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn seed_repo() -> TempDir {
    let dir = TempDir::new().expect("temp dir");
    let root = dir.path();
    git_ok(root, &["init"]);
    git_ok(root, &["checkout", "-B", "main"]);
    git_ok(root, &["config", "user.name", "Bitloops Test"]);
    git_ok(root, &["config", "user.email", "bitloops-test@example.com"]);
    std::fs::write(root.join(".gitignore"), "stores/\n").expect("write .gitignore");
    crate::test_support::git_fixtures::ensure_test_store_backends(root);
    std::fs::write(root.join("tracked.txt"), "one\n").expect("write tracked file");
    git_ok(root, &["add", "."]);
    git_ok(root, &["commit", "-m", "initial"]);
    dir
}

fn with_route_test_state<T>(
    repo_root: &std::path::Path,
    extra_env: &[(&str, Option<&str>)],
    f: impl FnOnce() -> T,
) -> T {
    let state_dir = repo_root.join(".route-test-state");
    let state_dir_str = state_dir.to_string_lossy().to_string();
    let mut env_vars = Vec::with_capacity(extra_env.len() + 1);
    env_vars.push((
        "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
        Some(state_dir_str.as_str()),
    ));
    env_vars.extend_from_slice(extra_env);
    with_process_state(Some(repo_root), &env_vars, f)
}

fn assert_session_start_context_matches_builder(context: &str, agent_name: &str) {
    let augmentation =
        crate::host::hooks::augmentation::builder::build_devql_session_start_augmentation(
            agent_name,
        );
    assert!(!augmentation.targeted);
    assert_eq!(context, augmentation.additional_context);
}

#[test]
fn route_codex_hooks_persist_interactions_to_event_db_when_relational_store_is_absent() -> Result<()>
{
    let repo = seed_repo();
    let session_id = "codex-session-1";
    let repo_id = crate::host::devql::resolve_repo_identity(repo.path())?.repo_id;
    let transcript_path = repo.path().join("codex-transcript.json");
    let transcript_path_str = transcript_path.to_string_lossy().to_string();
    std::fs::write(
        &transcript_path,
        r#"{"messages":[{"type":"user","content":"Refactor tracked file"},{"type":"gemini","content":"Updated tracked file"}]}"#,
    )
    .expect("write transcript");
    std::fs::write(repo.path().join("tracked.txt"), "two\n").expect("modify tracked file");

    with_route_test_state(repo.path(), &[], || -> Result<()> {
        let session_payload = serde_json::json!({
            "session_id": session_id,
            "transcript_path": transcript_path_str.clone(),
        })
        .to_string();
        route_hook_command_to_lifecycle(
            repo.path(),
            AGENT_NAME_CODEX,
            CODEX_HOOK_SESSION_START,
            &session_payload,
        )?;

        let relational_path = crate::config::resolve_store_backend_config_for_repo(repo.path())?
            .relational
            .resolve_sqlite_db_path_for_repo(repo.path())?;
        std::fs::remove_file(&relational_path).expect("remove relational sqlite");
        std::fs::create_dir_all(&relational_path)
            .expect("replace relational sqlite with directory");

        let stop_payload = serde_json::json!({
            "sessionId": session_id,
            "transcriptPath": transcript_path_str,
        })
        .to_string();
        route_hook_command_to_lifecycle(
            repo.path(),
            AGENT_NAME_CODEX,
            CODEX_HOOK_STOP,
            &stop_payload,
        )
        .expect("stop should still succeed when the runtime checkpoint store is available");
        Ok(())
    })?;

    with_route_test_state(repo.path(), &[], || -> Result<()> {
        let event_db_path = crate::config::resolve_store_backend_config_for_repo(repo.path())?
            .events
            .resolve_duckdb_db_path_for_repo(repo.path());
        assert!(
            event_db_path.is_file(),
            "expected events DuckDB at {}",
            event_db_path.display()
        );

        let duckdb = duckdb::Connection::open(&event_db_path).expect("open events duckdb");
        let session_count: i64 = duckdb
            .query_row(
                "SELECT COUNT(*) FROM interaction_sessions WHERE repo_id = ?1 AND session_id = ?2",
                duckdb::params![&repo_id, session_id],
                |row| row.get(0),
            )
            .expect("count interaction sessions");
        let turn_count: i64 = duckdb
            .query_row(
                "SELECT COUNT(*) FROM interaction_turns WHERE repo_id = ?1 AND session_id = ?2",
                duckdb::params![&repo_id, session_id],
                |row| row.get(0),
            )
            .expect("count interaction turns");
        let event_count: i64 = duckdb
            .query_row(
                "SELECT COUNT(*) FROM interaction_events WHERE repo_id = ?1 AND session_id = ?2",
                duckdb::params![&repo_id, session_id],
                |row| row.get(0),
            )
            .expect("count interaction events");
        let mut stmt = duckdb
            .prepare(
                "SELECT event_type, session_id, turn_id FROM interaction_events WHERE repo_id = ?1 AND session_id = ?2 ORDER BY event_time ASC, event_id ASC",
            )
            .expect("prepare interaction events query");
        let events = stmt
            .query_map(duckdb::params![&repo_id, session_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .expect("query interaction events")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect interaction events");
        assert_eq!(session_count, 1);
        assert_eq!(turn_count, 1);
        assert_eq!(event_count, 2);
        assert_eq!(events.len(), 2);
        let mut event_types = events
            .iter()
            .map(|event| event.0.as_str())
            .collect::<Vec<_>>();
        event_types.sort_unstable();
        assert_eq!(event_types, vec!["session_start", "turn_end"]);
        let session_start = events
            .iter()
            .find(|event| event.0 == "session_start")
            .expect("session_start event");
        assert_eq!(session_start.1, session_id);
        assert!(
            session_start.2.is_empty(),
            "session_start should not have turn_id"
        );
        let turn_end = events
            .iter()
            .find(|event| event.0 == "turn_end")
            .expect("turn_end event");
        assert_eq!(turn_end.1, session_id);
        assert!(!turn_end.2.is_empty(), "expected turn_end turn_id");

        let spool = rusqlite::Connection::open(
            interaction_spool_db_path(repo.path()).expect("resolve interaction spool path"),
        )
        .expect("open interaction spool");
        let local_session_count: i64 = spool
            .query_row(
                "SELECT COUNT(*) FROM interaction_sessions WHERE repo_id = ?1 AND session_id = ?2",
                rusqlite::params![&repo_id, session_id],
                |row| row.get(0),
            )
            .expect("count local interaction sessions");
        let local_turn_count: i64 = spool
            .query_row(
                "SELECT COUNT(*) FROM interaction_turns WHERE repo_id = ?1 AND session_id = ?2",
                rusqlite::params![&repo_id, session_id],
                |row| row.get(0),
            )
            .expect("count local interaction turns");
        let local_event_count: i64 = spool
            .query_row(
                "SELECT COUNT(*) FROM interaction_events WHERE repo_id = ?1 AND session_id = ?2",
                rusqlite::params![&repo_id, session_id],
                |row| row.get(0),
            )
            .expect("count local interaction events");
        let queued_mutations: i64 = spool
            .query_row("SELECT COUNT(*) FROM interaction_spool_queue", [], |row| {
                row.get(0)
            })
            .expect("count queued interaction mutations");
        assert_eq!(local_session_count, 1);
        assert_eq!(local_turn_count, 1);
        assert_eq!(local_event_count, 2);
        assert_eq!(queued_mutations, 0);
        Ok(())
    })?;

    Ok(())
}

#[test]
fn route_codex_user_prompt_submit_returns_targeted_additional_context_stdout() -> Result<()> {
    let repo = seed_repo();
    let session_id = "codex-session-prompt";
    let transcript_path = repo.path().join("codex-transcript.json");
    let transcript_path_str = transcript_path.to_string_lossy().to_string();
    std::fs::write(
        &transcript_path,
        r#"{"messages":[{"type":"user","content":"Inspect tracked file"},{"type":"assistant","content":"Looking"}]}"#,
    )
    .expect("write transcript");

    with_route_test_state(repo.path(), &[], || -> Result<()> {
        let session_payload = serde_json::json!({
            "session_id": session_id,
            "transcript_path": transcript_path_str.clone(),
        })
        .to_string();
        route_hook_command_to_lifecycle(
            repo.path(),
            AGENT_NAME_CODEX,
            CODEX_HOOK_SESSION_START,
            &session_payload,
        )?;

        let prompt_payload = serde_json::json!({
            "sessionId": session_id,
            "transcriptPath": transcript_path_str,
            "prompt": "Explain tracked.txt:1",
        })
        .to_string();
        let outcome = route_hook_command_to_lifecycle(
            repo.path(),
            AGENT_NAME_CODEX,
            CODEX_HOOK_USER_PROMPT_SUBMIT,
            &prompt_payload,
        )?;

        let stdout = outcome.stdout.expect("stdout");
        let json: serde_json::Value = serde_json::from_str(&stdout).expect("json stdout");
        let context = json["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .expect("additionalContext");
        assert_eq!(
            json["hookSpecificOutput"]["hookEventName"],
            serde_json::Value::String("UserPromptSubmit".to_string())
        );
        assert!(context.contains("<EXTREMELY_IMPORTANT>"));
        assert!(context.contains("Use DevQL first for this request."));
        assert!(context.contains("Suggested command:"));
        assert!(context.contains("bitloops devql query"));
        assert!(context.contains("tracked.txt"));
        assert!(context.contains("start: 1"));
        assert!(context.contains("end: 1"));
        assert!(context.contains("Run this before broad repo search."));
        assert!(!context.contains("<repo-relative-path>"));
        assert!(!context.contains("<symbol-fqn>"));
        Ok(())
    })
}

#[test]
fn route_claude_user_prompt_submit_returns_targeted_context_stdout() -> Result<()> {
    let repo = seed_repo();
    let session_id = "claude-session-prompt";
    let transcript_path = repo.path().join("claude-transcript.json");
    let transcript_path_str = transcript_path.to_string_lossy().to_string();
    std::fs::write(
        &transcript_path,
        r#"{"messages":[{"type":"user","content":"Inspect tracked file"},{"type":"assistant","content":"Looking"}]}"#,
    )
    .expect("write transcript");

    with_route_test_state(repo.path(), &[], || -> Result<()> {
        let session_payload = serde_json::json!({
            "session_id": session_id,
            "transcript_path": transcript_path_str.clone(),
        })
        .to_string();
        route_hook_command_to_lifecycle(
            repo.path(),
            crate::adapters::agents::AGENT_NAME_CLAUDE_CODE,
            CLAUDE_HOOK_SESSION_START,
            &session_payload,
        )?;

        let prompt_payload = serde_json::json!({
            "session_id": session_id,
            "transcript_path": transcript_path_str,
            "prompt": "Explain tracked.txt:1",
        })
        .to_string();
        let outcome = route_hook_command_to_lifecycle(
            repo.path(),
            crate::adapters::agents::AGENT_NAME_CLAUDE_CODE,
            CLAUDE_HOOK_USER_PROMPT_SUBMIT,
            &prompt_payload,
        )?;

        let stdout = outcome.stdout.expect("stdout");
        let json: serde_json::Value = serde_json::from_str(&stdout).expect("json stdout");
        let context = json["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .expect("additionalContext");
        assert_eq!(
            json["hookSpecificOutput"]["hookEventName"],
            serde_json::Value::String("UserPromptSubmit".to_string())
        );
        assert!(context.contains("<EXTREMELY_IMPORTANT>"));
        assert!(context.contains("Use DevQL first for this request."));
        assert!(context.contains("Suggested command:"));
        assert!(context.contains("bitloops devql query"));
        assert!(context.contains("tracked.txt"));
        assert!(context.contains("start: 1"));
        assert!(context.contains("end: 1"));
        assert!(context.contains("Run this before broad repo search."));
        assert!(!context.contains("<repo-relative-path>"));
        assert!(!context.contains("<symbol-fqn>"));
        Ok(())
    })
}

#[test]
fn route_claude_session_start_returns_additional_context_stdout() -> Result<()> {
    let repo = seed_repo();
    let session_id = "claude-session-start";
    let transcript_path = repo.path().join("claude-transcript.json");
    let transcript_path_str = transcript_path.to_string_lossy().to_string();
    std::fs::write(&transcript_path, "").expect("write transcript");

    with_route_test_state(repo.path(), &[], || -> Result<()> {
        let session_payload = serde_json::json!({
            "session_id": session_id,
            "transcript_path": transcript_path_str,
        })
        .to_string();
        let outcome = route_hook_command_to_lifecycle(
            repo.path(),
            crate::adapters::agents::AGENT_NAME_CLAUDE_CODE,
            CLAUDE_HOOK_SESSION_START,
            &session_payload,
        )?;

        let stdout = outcome.stdout.expect("stdout");
        let json: serde_json::Value = serde_json::from_str(&stdout).expect("json stdout");
        let context = json["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .expect("additionalContext");
        assert_eq!(
            json["hookSpecificOutput"]["hookEventName"],
            serde_json::Value::String("SessionStart".to_string())
        );
        assert_session_start_context_matches_builder(
            context,
            crate::adapters::agents::AGENT_NAME_CLAUDE_CODE,
        );
        Ok(())
    })
}

#[test]
fn route_codex_session_start_returns_additional_context_stdout() -> Result<()> {
    let repo = seed_repo();
    let session_id = "codex-session-start";
    let transcript_path = repo.path().join("codex-transcript.json");
    let transcript_path_str = transcript_path.to_string_lossy().to_string();
    std::fs::write(&transcript_path, "").expect("write transcript");

    with_route_test_state(repo.path(), &[], || -> Result<()> {
        let session_payload = serde_json::json!({
            "session_id": session_id,
            "transcript_path": transcript_path_str,
        })
        .to_string();
        let outcome = route_hook_command_to_lifecycle(
            repo.path(),
            AGENT_NAME_CODEX,
            CODEX_HOOK_SESSION_START,
            &session_payload,
        )?;

        let stdout = outcome.stdout.expect("stdout");
        let json: serde_json::Value = serde_json::from_str(&stdout).expect("json stdout");
        let context = json["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .expect("additionalContext");
        assert_eq!(
            json["hookSpecificOutput"]["hookEventName"],
            serde_json::Value::String("SessionStart".to_string())
        );
        assert_session_start_context_matches_builder(context, AGENT_NAME_CODEX);
        Ok(())
    })
}

#[test]
fn route_gemini_before_agent_returns_generic_additional_context_stdout() -> Result<()> {
    let repo = seed_repo();
    let session_id = "gemini-session-prompt";
    let transcript_path = repo.path().join("gemini-transcript.json");
    let transcript_path_str = transcript_path.to_string_lossy().to_string();
    std::fs::write(
        &transcript_path,
        r#"{"messages":[{"type":"user","content":"Inspect tracked file"},{"type":"assistant","content":"Looking"}]}"#,
    )
    .expect("write transcript");

    with_route_test_state(repo.path(), &[], || -> Result<()> {
        let session_payload = serde_json::json!({
            "session_id": session_id,
            "transcript_path": transcript_path_str.clone(),
        })
        .to_string();
        route_hook_command_to_lifecycle(
            repo.path(),
            AGENT_NAME_GEMINI,
            GEMINI_HOOK_SESSION_START,
            &session_payload,
        )?;

        let prompt_payload = serde_json::json!({
            "session_id": session_id,
            "transcript_path": transcript_path_str,
            "prompt": "Explain tracked.txt#L1",
        })
        .to_string();
        let outcome = route_hook_command_to_lifecycle(
            repo.path(),
            AGENT_NAME_GEMINI,
            GEMINI_HOOK_BEFORE_AGENT,
            &prompt_payload,
        )?;

        let stdout = outcome.stdout.expect("stdout");
        let json: serde_json::Value = serde_json::from_str(&stdout).expect("json stdout");
        let context = json["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .expect("additionalContext");
        assert_eq!(
            json["hookSpecificOutput"]["hookEventName"],
            serde_json::Value::String("BeforeAgent".to_string())
        );
        assert!(context.contains("<EXTREMELY_IMPORTANT>"));
        assert!(context.contains("Use DevQL first for this request."));
        assert!(context.contains("Suggested command:"));
        assert!(context.contains("bitloops devql query"));
        assert!(context.contains("tracked.txt"));
        assert!(context.contains("start: 1"));
        assert!(context.contains("end: 1"));
        assert!(context.contains("Run this before broad repo search."));
        assert!(!context.contains("<repo-relative-path>"));
        assert!(!context.contains("<symbol-fqn>"));
        Ok(())
    })
}

#[test]
fn route_gemini_session_start_returns_additional_context_stdout() -> Result<()> {
    let repo = seed_repo();
    let session_id = "gemini-session-start";
    let transcript_path = repo.path().join("gemini-transcript.json");
    let transcript_path_str = transcript_path.to_string_lossy().to_string();
    std::fs::write(&transcript_path, "").expect("write transcript");

    with_route_test_state(repo.path(), &[], || -> Result<()> {
        let session_payload = serde_json::json!({
            "session_id": session_id,
            "transcript_path": transcript_path_str,
        })
        .to_string();
        let outcome = route_hook_command_to_lifecycle(
            repo.path(),
            AGENT_NAME_GEMINI,
            GEMINI_HOOK_SESSION_START,
            &session_payload,
        )?;

        let stdout = outcome.stdout.expect("stdout");
        let json: serde_json::Value = serde_json::from_str(&stdout).expect("json stdout");
        let context = json["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .expect("additionalContext");
        assert_eq!(
            json["hookSpecificOutput"]["hookEventName"],
            serde_json::Value::String("SessionStart".to_string())
        );
        assert_session_start_context_matches_builder(context, AGENT_NAME_GEMINI);
        Ok(())
    })
}

#[test]
fn route_cursor_session_start_returns_additional_context_stdout() -> Result<()> {
    let repo = seed_repo();
    let transcript_path = repo.path().join("cursor-transcript.json");
    let transcript_path_str = transcript_path.to_string_lossy().to_string();
    std::fs::write(&transcript_path, "").expect("write transcript");

    with_route_test_state(repo.path(), &[], || -> Result<()> {
        let session_payload = serde_json::json!({
            "conversation_id": "cursor-session-start",
            "transcript_path": transcript_path_str,
            "modelSlug": "gpt-5.4-mini",
        })
        .to_string();
        let outcome = route_hook_command_to_lifecycle(
            repo.path(),
            AGENT_NAME_CURSOR,
            CURSOR_HOOK_SESSION_START,
            &session_payload,
        )?;

        let stdout = outcome.stdout.expect("stdout");
        let json: serde_json::Value = serde_json::from_str(&stdout).expect("json stdout");
        let context = json["additional_context"]
            .as_str()
            .expect("additional_context");
        assert_session_start_context_matches_builder(context, AGENT_NAME_CURSOR);
        Ok(())
    })
}

#[test]
fn route_copilot_session_start_returns_additional_context_stdout() -> Result<()> {
    let repo = seed_repo();
    let session_dir = repo.path().join("copilot-session-state");
    std::fs::create_dir_all(&session_dir).expect("create copilot session dir");
    let session_dir_str = session_dir.to_string_lossy().to_string();

    with_route_test_state(
        repo.path(),
        &[(
            "BITLOOPS_TEST_COPILOT_SESSION_DIR",
            Some(session_dir_str.as_str()),
        )],
        || -> Result<()> {
            let session_payload = serde_json::json!({
                "sessionId": "copilot-session-start",
                "source": "new",
                "initialPrompt": "bootstrap devql",
                "modelSlug": "gpt-5.4",
            })
            .to_string();
            let outcome = route_hook_command_to_lifecycle(
                repo.path(),
                AGENT_NAME_COPILOT,
                COPILOT_HOOK_SESSION_START,
                &session_payload,
            )?;

            let stdout = outcome.stdout.expect("stdout");
            let json: serde_json::Value = serde_json::from_str(&stdout).expect("json stdout");
            let context = json["additionalContext"]
                .as_str()
                .expect("additionalContext");
            assert_session_start_context_matches_builder(context, AGENT_NAME_COPILOT);
            Ok(())
        },
    )
}

#[test]
fn route_opencode_session_start_returns_no_additional_context_stdout() -> Result<()> {
    let repo = seed_repo();
    let transcript_path = repo.path().join("opencode-transcript.json");
    let transcript_path_str = transcript_path.to_string_lossy().to_string();
    std::fs::write(&transcript_path, "").expect("write transcript");

    with_route_test_state(repo.path(), &[], || -> Result<()> {
        let session_payload = serde_json::json!({
            "session_id": "opencode-session-start",
            "transcript_path": transcript_path_str,
        })
        .to_string();
        let outcome = route_hook_command_to_lifecycle(
            repo.path(),
            AGENT_NAME_OPEN_CODE,
            OPENCODE_HOOK_SESSION_START,
            &session_payload,
        )?;

        assert!(outcome.stdout.is_none());
        Ok(())
    })
}

use std::fs;

use tempfile::TempDir;

use crate::engine::session::create_session_backend_or_local;
use crate::engine::strategy::manual_commit::run_git;
use crate::engine::trailers::{
    METADATA_TASK_TRAILER_KEY, SESSION_TRAILER_KEY, SOURCE_REF_TRAILER_KEY, STRATEGY_TRAILER_KEY,
};
use crate::test_support::git_fixtures::ensure_test_store_backends;
use crate::test_support::process_state::git_command;
use crate::utils::paths;

use super::*;

fn setup_git_repo(dir: &TempDir) {
    let run = |args: &[&str]| {
        let out = git_command()
            .args(args)
            .current_dir(dir.path())
            .output()
            .expect("git command failed to start");
        assert!(
            out.status.success(),
            "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
            args,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    };
    run(&["init"]);
    run(&["config", "user.email", "test@test.com"]);
    run(&["config", "user.name", "Test"]);
    fs::write(dir.path().join("README.md"), "# Test").expect("write README");
    run(&["add", "README.md"]);
    run(&["commit", "-m", "Initial commit"]);
    ensure_test_store_backends(dir.path());
}

fn commit_tree_hash(dir: &TempDir, rev: &str) -> String {
    run_git(dir.path(), &["show", "-s", "--format=%T", rev]).expect("read tree hash")
}

#[test]
fn session_struct_defaults() {
    let session = Session::default();
    assert!(session.id.is_empty());
    assert!(session.description.is_empty());
    assert!(session.checkpoints.is_empty());
}

#[test]
fn checkpoint_struct_defaults() {
    let checkpoint = Checkpoint::default();
    assert!(checkpoint.checkpoint_id.is_empty());
}

#[test]
fn session_checkpoint_count_tracks_entries() {
    let session = Session {
        id: "s1".to_string(),
        description: "desc".to_string(),
        checkpoints: vec![
            Checkpoint {
                checkpoint_id: "a1b2c3d4e5f6".to_string(),
            },
            Checkpoint {
                checkpoint_id: "b1c2d3e4f5a6".to_string(),
            },
        ],
    };
    assert_eq!(session.checkpoints.len(), 2);
}

#[test]
fn empty_session_has_no_checkpoints() {
    let session = Session {
        id: "empty".to_string(),
        description: NO_DESCRIPTION.to_string(),
        checkpoints: vec![],
    };
    assert_eq!(session.id, "empty");
    assert_eq!(session.description, NO_DESCRIPTION);
    assert!(session.checkpoints.is_empty());
}

#[test]
fn list_sessions_without_git_repo_returns_empty() {
    let dir = tempfile::tempdir().expect("tempdir");
    // Intentionally no git init.
    let strategy = AutoCommitStrategy::new(dir.path());
    let sessions = strategy
        .list_sessions()
        .expect("list_sessions should be resilient outside git repo");
    assert!(sessions.is_empty());
}

#[test]
fn list_sessions_empty_repo_returns_empty() {
    let dir = tempfile::tempdir().expect("tempdir");
    setup_git_repo(&dir);

    let strategy = AutoCommitStrategy::new(dir.path());
    let sessions = strategy.list_sessions().expect("list_sessions");
    assert!(sessions.is_empty(), "no metadata branch checkpoints yet");
}

#[test]
fn get_session_context_not_found_returns_empty() {
    let dir = tempfile::tempdir().expect("tempdir");
    setup_git_repo(&dir);

    let strategy = AutoCommitStrategy::new(dir.path());
    let ctx = strategy.get_session_context("missing-session-id");
    assert!(ctx.is_empty(), "missing session context should be empty");
}

#[test]
fn auto_commit_save_step_commit_has_metadata_ref() {
    let dir = tempfile::tempdir().expect("tempdir");
    setup_git_repo(&dir);

    let strategy = AutoCommitStrategy::new(dir.path());

    let test_file = dir.path().join("test.rs");
    fs::write(&test_file, "package main").expect("write test.rs");

    let session_id = "2025-12-04-test-session-123";
    let metadata_dir_rel = format!("{}/{}", paths::BITLOOPS_METADATA_DIR, session_id);
    let metadata_dir_abs = dir.path().join(&metadata_dir_rel);
    fs::create_dir_all(&metadata_dir_abs).expect("create metadata dir");
    fs::write(
        metadata_dir_abs.join(paths::TRANSCRIPT_FILE_NAME),
        "test session log",
    )
    .expect("write transcript");

    let ctx = StepContext {
        session_id: session_id.to_string(),
        commit_message: "Test session commit".to_string(),
        metadata_dir: metadata_dir_rel,
        metadata_dir_abs: metadata_dir_abs.to_string_lossy().to_string(),
        new_files: vec!["test.rs".to_string()],
        modified_files: vec![],
        deleted_files: vec![],
        author_name: "Test".to_string(),
        author_email: "test@test.com".to_string(),
        ..Default::default()
    };

    strategy.save_step(&ctx).expect("save_step should run");

    let head_message = run_git(dir.path(), &["log", "-1", "--pretty=%B"]).expect("read HEAD");
    assert!(
        !head_message.contains(STRATEGY_TRAILER_KEY),
        "code commit should not have strategy trailer, got:\n{head_message}"
    );
    assert!(
        !head_message.contains(SOURCE_REF_TRAILER_KEY),
        "code commit should not have source-ref trailer, got:\n{head_message}"
    );
    assert!(
        !head_message.contains(SESSION_TRAILER_KEY),
        "code commit should not have session trailer, got:\n{head_message}"
    );

    let _ = run_git(
        dir.path(),
        &["rev-parse", "--verify", paths::METADATA_BRANCH_NAME],
    )
    .expect("metadata branch should exist");

    let metadata_message = run_git(
        dir.path(),
        &["log", "-1", "--pretty=%B", paths::METADATA_BRANCH_NAME],
    )
    .expect("read metadata branch commit message");
    assert!(
        metadata_message.contains(SESSION_TRAILER_KEY),
        "metadata commit should have session trailer, got:\n{metadata_message}"
    );
    assert!(
        metadata_message.contains(STRATEGY_TRAILER_KEY),
        "metadata commit should have strategy trailer, got:\n{metadata_message}"
    );
}

#[test]
fn auto_commit_save_step_metadata_ref_points_to_valid_commit() {
    let dir = tempfile::tempdir().expect("tempdir");
    setup_git_repo(&dir);

    let strategy = AutoCommitStrategy::new(dir.path());

    let test_file = dir.path().join("test.rs");
    fs::write(&test_file, "package main").expect("write test.rs");

    let session_id = "2025-12-04-test-session-456";
    let metadata_dir_rel = format!("{}/{}", paths::BITLOOPS_METADATA_DIR, session_id);
    let metadata_dir_abs = dir.path().join(&metadata_dir_rel);
    fs::create_dir_all(&metadata_dir_abs).expect("create metadata dir");
    fs::write(
        metadata_dir_abs.join(paths::TRANSCRIPT_FILE_NAME),
        "test session log",
    )
    .expect("write transcript");

    let ctx = StepContext {
        session_id: session_id.to_string(),
        commit_message: "Test session commit".to_string(),
        metadata_dir: metadata_dir_rel,
        metadata_dir_abs: metadata_dir_abs.to_string_lossy().to_string(),
        new_files: vec!["test.rs".to_string()],
        modified_files: vec![],
        deleted_files: vec![],
        author_name: "Test".to_string(),
        author_email: "test@test.com".to_string(),
        ..Default::default()
    };

    strategy.save_step(&ctx).expect("save_step should run");

    let head_message = run_git(dir.path(), &["log", "-1", "--pretty=%B"]).expect("read HEAD");
    assert!(
        !head_message.contains(SOURCE_REF_TRAILER_KEY),
        "code commit should not have source-ref trailer, got:\n{head_message}"
    );

    let _ = run_git(
        dir.path(),
        &["rev-parse", "--verify", paths::METADATA_BRANCH_NAME],
    )
    .expect("failed to get metadata branch");

    let metadata_message = run_git(
        dir.path(),
        &["log", "-1", "--pretty=%B", paths::METADATA_BRANCH_NAME],
    )
    .expect("read metadata branch commit");

    assert!(
        metadata_message.starts_with("Checkpoint: "),
        "metadata commit missing checkpoint prefix, got:\n{metadata_message}"
    );

    assert!(
        metadata_message.contains(&format!("{SESSION_TRAILER_KEY}: {session_id}")),
        "metadata commit missing session trailer for {session_id}, got:\n{metadata_message}"
    );

    assert!(
        metadata_message.contains(&format!("{STRATEGY_TRAILER_KEY}: auto-commit")),
        "metadata commit missing auto-commit strategy trailer, got:\n{metadata_message}"
    );
}

#[test]
fn auto_commit_save_task_step_commit_has_metadata_ref() {
    let dir = tempfile::tempdir().expect("tempdir");
    setup_git_repo(&dir);

    let strategy = AutoCommitStrategy::new(dir.path());

    let task_output = dir.path().join("task_output.txt");
    fs::write(&task_output, "task result").expect("write task output");

    let transcript_path = dir.path().join("session.jsonl");
    fs::write(&transcript_path, r#"{"type":"test"}"#).expect("write transcript");

    let ctx = TaskStepContext {
        session_id: "test-session-789".to_string(),
        tool_use_id: "toolu_abc123".to_string(),
        checkpoint_uuid: "checkpoint-uuid-456".to_string(),
        agent_id: "agent-xyz".to_string(),
        transcript_path: transcript_path.to_string_lossy().to_string(),
        new_files: vec!["task_output.txt".to_string()],
        modified_files: vec![],
        deleted_files: vec![],
        author_name: "Test".to_string(),
        author_email: "test@test.com".to_string(),
        ..Default::default()
    };

    strategy
        .save_task_step(&ctx)
        .expect("save_task_step should run");

    let head_message = run_git(dir.path(), &["log", "-1", "--pretty=%B"]).expect("read HEAD");
    assert!(
        !head_message.contains(SOURCE_REF_TRAILER_KEY),
        "task checkpoint code commit should not have source-ref trailer, got:\n{head_message}"
    );
    assert!(
        !head_message.contains(STRATEGY_TRAILER_KEY),
        "task checkpoint code commit should not have strategy trailer, got:\n{head_message}"
    );

    let _ = run_git(
        dir.path(),
        &["rev-parse", "--verify", paths::METADATA_BRANCH_NAME],
    )
    .expect("metadata branch should exist");

    let metadata_message = run_git(
        dir.path(),
        &["log", "-1", "--pretty=%B", paths::METADATA_BRANCH_NAME],
    )
    .expect("read metadata branch commit");
    assert!(
        metadata_message.contains("Checkpoint: "),
        "metadata commit missing checkpoint format, got:\n{metadata_message}"
    );
}

#[test]
fn auto_commit_save_task_step_no_changes_skips_commit() {
    let dir = tempfile::tempdir().expect("tempdir");
    setup_git_repo(&dir);

    let strategy = AutoCommitStrategy::new(dir.path());
    strategy.ensure_setup().expect("ensure setup");

    let initial_tree = commit_tree_hash(&dir, "HEAD");

    let ctx = TaskStepContext {
        session_id: "test-session-nochanges".to_string(),
        tool_use_id: "toolu_nochanges456".to_string(),
        is_incremental: true,
        incremental_type: "TodoWrite".to_string(),
        incremental_sequence: 2,
        todo_content: "Write some code".to_string(),
        modified_files: vec![],
        new_files: vec![],
        deleted_files: vec![],
        author_name: "Test".to_string(),
        author_email: "test@test.com".to_string(),
        ..Default::default()
    };

    strategy
        .save_task_step(&ctx)
        .expect("save_task_step should run");

    let new_head = run_git(dir.path(), &["rev-parse", "HEAD"]).expect("read HEAD");
    let new_tree = commit_tree_hash(&dir, "HEAD");
    assert_eq!(
        new_tree, initial_tree,
        "checkpoint without file changes should keep the same tree hash"
    );
    assert!(!new_head.is_empty(), "HEAD hash should be available");

    let metadata_message = run_git(
        dir.path(),
        &["log", "-1", "--pretty=%B", paths::METADATA_BRANCH_NAME],
    )
    .expect("metadata branch should exist");
    assert!(
        metadata_message.contains(METADATA_TASK_TRAILER_KEY),
        "metadata should be committed with task metadata trailer, got:\n{metadata_message}"
    );
}

#[test]
fn auto_commit_get_session_context() {
    let dir = tempfile::tempdir().expect("tempdir");
    setup_git_repo(&dir);

    let strategy = AutoCommitStrategy::new(dir.path());
    strategy.ensure_setup().expect("ensure setup");

    let test_file = dir.path().join("test.rs");
    fs::write(&test_file, "package main").expect("write test.rs");

    let session_id = "2025-12-10-test-session-context";
    let metadata_dir_rel = format!("{}/{}", paths::BITLOOPS_METADATA_DIR, session_id);
    let metadata_dir_abs = dir.path().join(&metadata_dir_rel);
    fs::create_dir_all(&metadata_dir_abs).expect("create metadata dir");
    fs::write(
        metadata_dir_abs.join(paths::TRANSCRIPT_FILE_NAME),
        "test session log",
    )
    .expect("write transcript");
    let context_content =
        "# Session Context\n\nThis is a test context.\n\n## Details\n\n- Item 1\n- Item 2";
    fs::write(
        metadata_dir_abs.join(paths::CONTEXT_FILE_NAME),
        context_content,
    )
    .expect("write context");

    let ctx = StepContext {
        session_id: session_id.to_string(),
        commit_message: "Test checkpoint".to_string(),
        metadata_dir: metadata_dir_rel,
        metadata_dir_abs: metadata_dir_abs.to_string_lossy().to_string(),
        new_files: vec!["test.rs".to_string()],
        modified_files: vec![],
        deleted_files: vec![],
        author_name: "Test".to_string(),
        author_email: "test@test.com".to_string(),
        ..Default::default()
    };

    strategy.save_step(&ctx).expect("save_step should run");

    let result = strategy.get_session_context(session_id);
    assert!(
        !result.is_empty(),
        "get_session_context should not return empty string"
    );
    assert_eq!(result, context_content);
}

#[test]
fn auto_commit_list_sessions_has_description() {
    let dir = tempfile::tempdir().expect("tempdir");
    setup_git_repo(&dir);

    let strategy = AutoCommitStrategy::new(dir.path());
    strategy.ensure_setup().expect("ensure setup");

    let test_file = dir.path().join("test.rs");
    fs::write(&test_file, "package main").expect("write test.rs");

    let session_id = "2025-12-10-test-session-description";
    let metadata_dir_rel = format!("{}/{}", paths::BITLOOPS_METADATA_DIR, session_id);
    let metadata_dir_abs = dir.path().join(&metadata_dir_rel);
    fs::create_dir_all(&metadata_dir_abs).expect("create metadata dir");
    fs::write(
        metadata_dir_abs.join(paths::TRANSCRIPT_FILE_NAME),
        "test session log",
    )
    .expect("write transcript");

    let expected_description = "Fix the authentication bug in login.rs";
    fs::write(
        metadata_dir_abs.join(paths::PROMPT_FILE_NAME),
        format!("{expected_description}\n\nMore details here..."),
    )
    .expect("write prompt");

    let ctx = StepContext {
        session_id: session_id.to_string(),
        commit_message: "Test checkpoint".to_string(),
        metadata_dir: metadata_dir_rel,
        metadata_dir_abs: metadata_dir_abs.to_string_lossy().to_string(),
        new_files: vec!["test.rs".to_string()],
        modified_files: vec![],
        deleted_files: vec![],
        author_name: "Test".to_string(),
        author_email: "test@test.com".to_string(),
        ..Default::default()
    };
    strategy.save_step(&ctx).expect("save_step should run");

    let sessions = strategy.list_sessions().expect("list_sessions");
    assert!(
        !sessions.is_empty(),
        "list_sessions should return at least one session"
    );

    let found = sessions.iter().find(|s| s.id == session_id);
    let found = found.expect("expected session to be present");
    assert_ne!(
        found.description, NO_DESCRIPTION,
        "description should not be default placeholder"
    );
    assert_eq!(found.description, expected_description);
}

#[test]
fn auto_commit_implements_session_initializer() {
    fn assert_session_initializer<T: SessionInitializer>() {}
    assert_session_initializer::<AutoCommitStrategy>();
}

#[test]
fn auto_commit_initialize_session_creates_session_state() {
    let dir = tempfile::tempdir().expect("tempdir");
    setup_git_repo(&dir);

    let strategy = AutoCommitStrategy::new(dir.path());
    strategy.ensure_setup().expect("ensure setup");

    let session_id = "2025-12-22-test-session-init";
    SessionInitializer::initialize_session(&strategy, session_id, "claude-code", "", "")
        .expect("initialize_session should run");

    let backend = create_session_backend_or_local(dir.path());
    let state = backend
        .load_session(session_id)
        .expect("load_session")
        .expect("session should be created");

    assert_eq!(state.session_id, session_id);
    assert_eq!(state.cli_version, env!("CARGO_PKG_VERSION"));
    assert_eq!(state.step_count, 0);
    assert_eq!(state.checkpoint_transcript_start, 0);
}

#[test]
fn auto_commit_get_checkpoint_log_reads_full_jsonl() {
    let dir = tempfile::tempdir().expect("tempdir");
    setup_git_repo(&dir);

    let strategy = AutoCommitStrategy::new(dir.path());
    strategy.ensure_setup().expect("ensure setup");

    fs::write(dir.path().join("task_output.txt"), "task result").expect("write task output");

    let transcript_dir = tempfile::tempdir().expect("transcript tempdir");
    let transcript_path = transcript_dir.path().join("session.jsonl");
    let expected_content = r#"{"type":"assistant","content":"test response"}"#;
    fs::write(&transcript_path, expected_content).expect("write transcript");

    let session_id = "2025-12-12-test-checkpoint-jsonl";
    let ctx = TaskStepContext {
        session_id: session_id.to_string(),
        tool_use_id: "toolu_jsonl_test".to_string(),
        checkpoint_uuid: "checkpoint-uuid-jsonl".to_string(),
        agent_id: "agent-jsonl".to_string(),
        transcript_path: transcript_path.to_string_lossy().to_string(),
        new_files: vec!["task_output.txt".to_string()],
        modified_files: vec![],
        deleted_files: vec![],
        author_name: "Test".to_string(),
        author_email: "test@test.com".to_string(),
        ..Default::default()
    };

    strategy
        .save_task_step(&ctx)
        .expect("save_task_step should run");

    let sessions = strategy.list_sessions().expect("list_sessions");
    let session = sessions
        .iter()
        .find(|s| s.id == session_id)
        .expect("session should exist");
    let checkpoint = session
        .checkpoints
        .first()
        .cloned()
        .expect("session should have checkpoint");

    let content = strategy
        .get_checkpoint_log(&checkpoint)
        .expect("get_checkpoint_log");
    assert_eq!(
        String::from_utf8(content).expect("utf8 transcript"),
        expected_content
    );
}

#[test]
fn auto_commit_save_step_files_already_committed() {
    let dir = tempfile::tempdir().expect("tempdir");
    setup_git_repo(&dir);

    let strategy = AutoCommitStrategy::new(dir.path());
    strategy.ensure_setup().expect("ensure setup");

    fs::write(dir.path().join("test.rs"), "package main").expect("write test.rs");
    run_git(dir.path(), &["add", "test.rs"]).expect("add test.rs");
    run_git(
        dir.path(),
        &[
            "-c",
            "user.name=User",
            "-c",
            "user.email=user@test.com",
            "commit",
            "-m",
            "User committed the file first",
        ],
    )
    .expect("commit as user");
    let user_commit = run_git(dir.path(), &["rev-parse", "HEAD"]).expect("read user commit");

    let sessions_commit_before = run_git(
        dir.path(),
        &["rev-parse", "--verify", paths::METADATA_BRANCH_NAME],
    )
    .expect("metadata branch should exist");

    let session_id = "2025-12-22-already-committed-test";
    let metadata_dir_rel = format!("{}/{}", paths::BITLOOPS_METADATA_DIR, session_id);
    let metadata_dir_abs = dir.path().join(&metadata_dir_rel);
    fs::create_dir_all(&metadata_dir_abs).expect("create metadata dir");
    fs::write(
        metadata_dir_abs.join(paths::TRANSCRIPT_FILE_NAME),
        "test session log",
    )
    .expect("write transcript");

    let ctx = StepContext {
        session_id: session_id.to_string(),
        commit_message: "Should be skipped - file already committed".to_string(),
        metadata_dir: metadata_dir_rel,
        metadata_dir_abs: metadata_dir_abs.to_string_lossy().to_string(),
        new_files: vec!["test.rs".to_string()],
        modified_files: vec![],
        deleted_files: vec![],
        author_name: "Test".to_string(),
        author_email: "test@test.com".to_string(),
        ..Default::default()
    };

    strategy.save_step(&ctx).expect("save_step should run");

    let head = run_git(dir.path(), &["rev-parse", "HEAD"]).expect("read HEAD");
    assert_eq!(
        head, user_commit,
        "HEAD should remain user's commit when file already committed"
    );

    let sessions_commit_after = run_git(
        dir.path(),
        &["rev-parse", "--verify", paths::METADATA_BRANCH_NAME],
    )
    .expect("metadata branch should still exist");
    assert_eq!(
        sessions_commit_after, sessions_commit_before,
        "metadata branch should not get new commits when file is already committed"
    );
}

#[test]
fn auto_commit_save_step_no_changes_skipped() {
    let dir = tempfile::tempdir().expect("tempdir");
    setup_git_repo(&dir);

    let strategy = AutoCommitStrategy::new(dir.path());
    strategy.ensure_setup().expect("ensure setup");

    let initial_commit = run_git(dir.path(), &["rev-parse", "HEAD"]).expect("initial HEAD");
    let sessions_commit_before = run_git(
        dir.path(),
        &["rev-parse", "--verify", paths::METADATA_BRANCH_NAME],
    )
    .expect("metadata branch should exist");

    let session_id = "2025-12-22-no-changes-test";
    let metadata_dir_rel = format!("{}/{}", paths::BITLOOPS_METADATA_DIR, session_id);
    let metadata_dir_abs = dir.path().join(&metadata_dir_rel);
    fs::create_dir_all(&metadata_dir_abs).expect("create metadata dir");
    fs::write(
        metadata_dir_abs.join(paths::TRANSCRIPT_FILE_NAME),
        "test session log",
    )
    .expect("write transcript");

    let ctx = StepContext {
        session_id: session_id.to_string(),
        commit_message: "Should be skipped".to_string(),
        metadata_dir: metadata_dir_rel,
        metadata_dir_abs: metadata_dir_abs.to_string_lossy().to_string(),
        new_files: vec![],
        modified_files: vec![],
        deleted_files: vec![],
        author_name: "Test".to_string(),
        author_email: "test@test.com".to_string(),
        ..Default::default()
    };

    strategy.save_step(&ctx).expect("save_step should run");

    let head = run_git(dir.path(), &["rev-parse", "HEAD"]).expect("read HEAD");
    assert_eq!(
        head, initial_commit,
        "HEAD should remain initial commit when there are no code changes"
    );

    let sessions_commit_after = run_git(
        dir.path(),
        &["rev-parse", "--verify", paths::METADATA_BRANCH_NAME],
    )
    .expect("metadata branch should still exist");
    assert_eq!(
        sessions_commit_after, sessions_commit_before,
        "metadata branch should not get new commits when there are no code changes"
    );
}

#[test]
fn list_sessions_with_checkpoints_returns_entries() {
    let dir = tempfile::tempdir().expect("tempdir");
    setup_git_repo(&dir);

    let strategy = AutoCommitStrategy::new(dir.path());
    strategy.ensure_setup().expect("ensure setup");

    fs::write(dir.path().join("x.rs"), "package x").expect("write x.rs");
    let session_id = "list-with-checkpoints";
    let metadata_dir_rel = format!("{}/{}", paths::BITLOOPS_METADATA_DIR, session_id);
    let metadata_dir_abs = dir.path().join(&metadata_dir_rel);
    fs::create_dir_all(&metadata_dir_abs).expect("create metadata dir");
    fs::write(metadata_dir_abs.join(paths::TRANSCRIPT_FILE_NAME), "log").expect("write log");

    let ctx = StepContext {
        session_id: session_id.to_string(),
        commit_message: "checkpoint".to_string(),
        metadata_dir: metadata_dir_rel,
        metadata_dir_abs: metadata_dir_abs.to_string_lossy().to_string(),
        new_files: vec!["x.rs".to_string()],
        modified_files: vec![],
        deleted_files: vec![],
        author_name: "Test".to_string(),
        author_email: "test@test.com".to_string(),
        ..Default::default()
    };
    strategy.save_step(&ctx).expect("save_step");

    let sessions = strategy.list_sessions().expect("list_sessions");
    let found = sessions
        .iter()
        .find(|s| s.id == session_id)
        .expect("session");
    assert!(!found.checkpoints.is_empty());
}

#[test]
fn list_sessions_supports_multiple_sessions() {
    let dir = tempfile::tempdir().expect("tempdir");
    setup_git_repo(&dir);
    let strategy = AutoCommitStrategy::new(dir.path());
    strategy.ensure_setup().expect("ensure setup");

    for (name, file) in [("sess-a", "a.rs"), ("sess-b", "b.rs")] {
        fs::write(dir.path().join(file), format!("package {}", &file[..1])).expect("write file");
        let metadata_dir_rel = format!("{}/{}", paths::BITLOOPS_METADATA_DIR, name);
        let metadata_dir_abs = dir.path().join(&metadata_dir_rel);
        fs::create_dir_all(&metadata_dir_abs).expect("create metadata dir");
        fs::write(metadata_dir_abs.join(paths::TRANSCRIPT_FILE_NAME), "log").expect("write log");
        let ctx = StepContext {
            session_id: name.to_string(),
            commit_message: format!("checkpoint {name}"),
            metadata_dir: metadata_dir_rel,
            metadata_dir_abs: metadata_dir_abs.to_string_lossy().to_string(),
            new_files: vec![file.to_string()],
            modified_files: vec![],
            deleted_files: vec![],
            author_name: "Test".to_string(),
            author_email: "test@test.com".to_string(),
            ..Default::default()
        };
        strategy.save_step(&ctx).expect("save_step");
    }

    let sessions = strategy.list_sessions().expect("list_sessions");
    assert!(sessions.iter().any(|s| s.id == "sess-a"));
    assert!(sessions.iter().any(|s| s.id == "sess-b"));
}

#[test]
fn shadow_strategy_list_sessions_empty_equivalent() {
    let dir = tempfile::tempdir().expect("tempdir");
    setup_git_repo(&dir);
    let strategy = AutoCommitStrategy::new(dir.path());
    let sessions = strategy.list_sessions().expect("list_sessions");
    assert!(sessions.is_empty());
}

#[test]
fn shadow_strategy_session_info_not_found_equivalents() {
    let dir = tempfile::tempdir().expect("tempdir");
    setup_git_repo(&dir);
    let strategy = AutoCommitStrategy::new(dir.path());

    assert!(
        strategy.get_session_context("missing").is_empty(),
        "missing session should return empty context"
    );

    let err = strategy.get_checkpoint_log(&Checkpoint {
        checkpoint_id: "deadbeef0000".to_string(),
    });
    assert!(err.is_err(), "missing checkpoint should return error");
}

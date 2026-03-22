mod test_command_support;

use bitloops_cli::host::session::phase::SessionPhase;
use bitloops_cli::host::session::state::{PrePromptState, SessionState};
use bitloops_cli::host::session::{SessionBackend, create_session_backend_or_local};
use bitloops_cli::host::strategy::manual_commit::{
    read_commit_checkpoint_mappings, read_committed, read_session_content,
};
use serde_json::Value;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn bitloops_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_bitloops"))
}

fn run_cmd(repo: &Path, args: &[&str], stdin: Option<&str>) -> Output {
    let bin = bitloops_bin();
    let (mut cmd, _isolated_home) =
        test_command_support::new_isolated_bitloops_command(&bin, repo, args);
    if let Some(input) = stdin {
        cmd.stdin(Stdio::piped());
        let mut child = cmd.spawn().expect("failed to spawn bitloops command");
        use std::io::Write;
        child
            .stdin
            .as_mut()
            .expect("stdin should be piped")
            .write_all(input.as_bytes())
            .expect("failed to write stdin");
        child.wait_with_output().expect("failed to wait for output")
    } else {
        cmd.output().expect("failed to execute bitloops command")
    }
}

fn run_cmd_with_env(
    repo: &Path,
    args: &[&str],
    stdin: Option<&str>,
    extra_env: &[(&str, &str)],
) -> Output {
    let bin = bitloops_bin();
    let (mut cmd, _isolated_home) =
        test_command_support::new_isolated_bitloops_command(&bin, repo, args);
    for (key, value) in extra_env {
        cmd.env(key, value);
    }
    if let Some(input) = stdin {
        cmd.stdin(Stdio::piped());
        let mut child = cmd.spawn().expect("failed to spawn bitloops command");
        use std::io::Write;
        child
            .stdin
            .as_mut()
            .expect("stdin should be piped")
            .write_all(input.as_bytes())
            .expect("failed to write stdin");
        child.wait_with_output().expect("failed to wait for output")
    } else {
        cmd.output().expect("failed to execute bitloops command")
    }
}

fn assert_success(output: &Output, context: &str) {
    assert!(
        output.status.success(),
        "{context} failed\nstatus: {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_git(repo: &Path, args: &[&str]) -> String {
    let out = run_git_output(repo, args);
    assert!(
        out.status.success(),
        "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn run_git_output(repo: &Path, args: &[&str]) -> Output {
    let mut search_paths = vec![
        bitloops_bin()
            .parent()
            .expect("bitloops test binary should have a parent directory")
            .to_path_buf(),
    ];
    if let Some(existing_path) = env::var_os("PATH") {
        search_paths.extend(env::split_paths(&existing_path));
    }
    let path = env::join_paths(search_paths).expect("failed to construct PATH");

    Command::new("git")
        .args(args)
        .current_dir(repo)
        .env("PATH", path)
        .output()
        .expect("failed to run git")
}

fn git_ref_exists(repo: &Path, reference: &str) -> bool {
    run_git_output(repo, &["show-ref", "--verify", "--quiet", reference])
        .status
        .success()
}

fn session_backend(repo: &Path) -> Box<dyn SessionBackend> {
    create_session_backend_or_local(repo)
}

fn checkpoint_sqlite_path(repo_root: &Path) -> PathBuf {
    let cfg = bitloops_cli::config::resolve_store_backend_config_for_repo(repo_root)
        .expect("resolve backend config");
    if let Some(path) = cfg.relational.sqlite_path.as_deref() {
        bitloops_cli::config::resolve_sqlite_db_path_for_repo(repo_root, Some(path))
            .expect("resolve configured sqlite path")
    } else {
        bitloops_cli::utils::paths::default_relational_db_path(repo_root)
    }
}

fn ensure_relational_store_file(repo_root: &Path) {
    let sqlite =
        bitloops_cli::storage::SqliteConnectionPool::connect(checkpoint_sqlite_path(repo_root))
            .expect("create relational sqlite file");
    sqlite
        .initialise_checkpoint_schema()
        .expect("initialise checkpoint schema");
}

fn init_repo(repo: &Path) {
    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "t@t.com"]);
    run_git(repo, &["config", "user.name", "Test"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);
    fs::write(repo.join("README.md"), "init\n").unwrap();
    run_git(repo, &["add", "."]);
    run_git(repo, &["commit", "-m", "initial"]);
}

fn enable(repo: &Path) {
    let out = run_cmd(repo, &["enable"], None);
    assert_success(&out, "bitloops enable");
}

fn init(repo: &Path) {
    let out = run_cmd(repo, &["init", "--agent", "claude-code"], None);
    assert_success(&out, "bitloops init --agent claude-code");
}

fn setup_claude_and_enable(repo: &Path) {
    init(repo);
    enable(repo);
}

fn hook_command_exists(settings: &Value, hook_type: &str, matcher: &str, command: &str) -> bool {
    let Some(matchers) = settings
        .get("hooks")
        .and_then(|h| h.get(hook_type))
        .and_then(|v| v.as_array())
    else {
        return false;
    };

    matchers.iter().any(|m| {
        let m_matcher = m.get("matcher").and_then(|v| v.as_str()).unwrap_or("");
        if m_matcher != matcher {
            return false;
        }
        m.get("hooks")
            .and_then(|h| h.as_array())
            .map(|hooks| {
                hooks.iter().any(|h| {
                    h.get("command")
                        .and_then(|c| c.as_str())
                        .map(|c| c == command)
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false)
    })
}

fn read_json(path: &Path) -> Value {
    let data = fs::read_to_string(path).expect("failed to read json file");
    serde_json::from_str(&data).expect("failed to parse json file")
}

fn session_state(repo: &Path, session_id: &str) -> SessionState {
    session_backend(repo)
        .load_session(session_id)
        .expect("failed to load session state from backend")
        .expect("expected session state to exist")
}

fn pre_prompt_state(repo: &Path, session_id: &str) -> Option<PrePromptState> {
    session_backend(repo)
        .load_pre_prompt(session_id)
        .expect("failed to load pre-prompt state from backend")
}

fn checkpoint_id_for_head(repo: &Path) -> Option<String> {
    let head = run_git(repo, &["rev-parse", "HEAD"]);
    read_commit_checkpoint_mappings(repo)
        .expect("failed to read commit-checkpoint mappings")
        .get(&head)
        .cloned()
}

fn user_prompt_submit(repo: &Path, session_id: &str, transcript_path: &str, prompt: &str) {
    let input = format!(
        r#"{{"session_id":"{session_id}","transcript_path":"{transcript_path}","prompt":{prompt_json}}}"#,
        prompt_json = serde_json::to_string(prompt).unwrap()
    );
    let out = run_cmd(
        repo,
        &["hooks", "claude-code", "user-prompt-submit"],
        Some(&input),
    );
    assert_success(&out, "hooks claude-code user-prompt-submit");
}

fn stop(repo: &Path, session_id: &str, transcript_path: &str) {
    let input = format!(r#"{{"session_id":"{session_id}","transcript_path":"{transcript_path}"}}"#);
    let out = run_cmd(repo, &["hooks", "claude-code", "stop"], Some(&input));
    assert_success(&out, "hooks claude-code stop");
}

fn write_test_session_state_for_logging(repo: &Path, session_id: &str) {
    ensure_relational_store_file(repo);
    let backend = session_backend(repo);
    backend
        .save_session(&SessionState {
            session_id: session_id.to_string(),
            worktree_path: repo.to_string_lossy().to_string(),
            started_at: "2026-01-01T00:00:00Z".to_string(),
            last_interaction_time: Some("2026-01-01T00:00:01Z".to_string()),
            phase: SessionPhase::Active,
            ..Default::default()
        })
        .expect("failed to persist session state");
}

#[test]
fn init_adds_all_required_claude_hooks() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());

    init(dir.path());

    let settings = read_json(&dir.path().join(".claude/settings.json"));
    assert!(hook_command_exists(
        &settings,
        "SessionStart",
        "",
        "bitloops hooks claude-code session-start"
    ));
    assert!(hook_command_exists(
        &settings,
        "SessionEnd",
        "",
        "bitloops hooks claude-code session-end"
    ));
    assert!(hook_command_exists(
        &settings,
        "Stop",
        "",
        "bitloops hooks claude-code stop"
    ));
    assert!(hook_command_exists(
        &settings,
        "UserPromptSubmit",
        "",
        "bitloops hooks claude-code user-prompt-submit"
    ));
    assert!(hook_command_exists(
        &settings,
        "PreToolUse",
        "Task",
        "bitloops hooks claude-code pre-task"
    ));
    assert!(hook_command_exists(
        &settings,
        "PostToolUse",
        "Task",
        "bitloops hooks claude-code post-task"
    ));
    assert!(hook_command_exists(
        &settings,
        "PostToolUse",
        "TodoWrite",
        "bitloops hooks claude-code post-todo"
    ));
}

#[test]
fn init_preserves_existing_claude_settings_and_user_hooks() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());

    fs::create_dir_all(dir.path().join(".claude")).unwrap();
    fs::write(
        dir.path().join(".claude/settings.json"),
        r#"{
  "customSetting": "should-be-preserved",
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Task",
        "hooks": [{"type": "command", "command": "echo user-task-hook"}]
      },
      {
        "matcher": "CustomTool",
        "hooks": [{"type": "command", "command": "echo custom"}]
      }
    ]
  }
}"#,
    )
    .unwrap();

    init(dir.path());

    let settings = read_json(&dir.path().join(".claude/settings.json"));
    assert_eq!(
        settings
            .get("customSetting")
            .and_then(|v| v.as_str())
            .unwrap_or_default(),
        "should-be-preserved"
    );

    assert!(hook_command_exists(
        &settings,
        "PreToolUse",
        "Task",
        "echo user-task-hook"
    ));
    assert!(hook_command_exists(
        &settings,
        "PreToolUse",
        "Task",
        "bitloops hooks claude-code pre-task"
    ));
    assert!(hook_command_exists(
        &settings,
        "PreToolUse",
        "CustomTool",
        "echo custom"
    ));
    assert!(hook_command_exists(
        &settings,
        "PostToolUse",
        "Task",
        "bitloops hooks claude-code post-task"
    ));
}

#[test]
fn enable_does_not_add_claude_hooks() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());

    enable(dir.path());

    let claude_settings = dir.path().join(".claude/settings.json");
    if claude_settings.exists() {
        let settings = read_json(&claude_settings);
        assert!(
            !hook_command_exists(
                &settings,
                "SessionStart",
                "",
                "bitloops hooks claude-code session-start"
            ),
            "enable should not install Claude hooks"
        );
    }
}

#[test]
fn hook_logging_writes_to_session_log_file() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    enable(dir.path());

    let session_id = "test-logging-session-123";
    write_test_session_state_for_logging(dir.path(), session_id);
    fs::create_dir_all(dir.path().join(".bitloops/logs")).unwrap();

    let out = run_cmd_with_env(
        dir.path(),
        &["hooks", "git", "post-commit"],
        None,
        &[("BITLOOPS_LOG_LEVEL", "debug")],
    );
    let _ = out;

    let log_file = dir.path().join(".bitloops/logs/bitloops.log");
    assert!(
        log_file.exists(),
        "expected log file at {}",
        log_file.display()
    );

    let content = fs::read_to_string(&log_file).expect("failed to read hook log file");
    assert!(
        content.contains("\"hook\""),
        "log file should contain hook field; content:\n{}",
        content
    );
    assert!(
        content.contains("\"post-commit\""),
        "log file should contain post-commit hook name; content:\n{}",
        content
    );
    assert!(
        content.contains("\"component\""),
        "log file should contain component field; content:\n{}",
        content
    );
}

#[test]
fn hook_logging_writes_without_session() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    enable(dir.path());

    let out = run_cmd_with_env(
        dir.path(),
        &["hooks", "git", "post-commit"],
        None,
        &[("BITLOOPS_LOG_LEVEL", "debug")],
    );
    let _ = out;

    let log_file = dir.path().join(".bitloops/logs/bitloops.log");
    let content = fs::read_to_string(&log_file)
        .expect("expected bitloops.log to be created even without session");
    assert!(
        !content.contains("\"session_id\""),
        "logs without active session should not contain session_id; content:\n{}",
        content
    );
}

#[test]
fn user_prompt_submit_creates_pre_prompt_state() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    setup_claude_and_enable(dir.path());

    let sid = "test-session-1";
    user_prompt_submit(dir.path(), sid, "", "Create a file");

    let pre_prompt =
        pre_prompt_state(dir.path(), sid).expect("pre-prompt state should exist in backend");
    assert_eq!(pre_prompt.session_id, sid);
    assert_eq!(pre_prompt.prompt, "Create a file");

    let state = session_state(dir.path(), sid);
    assert_eq!(state.session_id, sid);
    assert_eq!(state.phase, SessionPhase::Active);
}

#[test]
fn stop_creates_shadow_checkpoint_and_metadata_files() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    setup_claude_and_enable(dir.path());

    let sid = "test-session-2";
    let transcript_path = dir.path().join("transcript.jsonl");
    user_prompt_submit(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Create a sample file",
    );

    fs::write(dir.path().join("created.txt"), "created by claude\n").unwrap();
    fs::write(
        &transcript_path,
        r#"{"type":"user","message":{"content":[{"type":"text","text":"Create a sample file"}]}}
{"type":"assistant","message":{"content":[{"type":"text","text":"Created created.txt"}]}}
"#,
    )
    .unwrap();

    stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    let state = session_state(dir.path(), sid);
    assert_eq!(state.phase, SessionPhase::Idle);
    assert!(
        state.step_count > 0,
        "checkpoint_count should increase after stop with changes"
    );

    let refs = run_git(
        dir.path(),
        &["for-each-ref", "--format=%(refname)", "refs/heads/bitloops"],
    );
    assert!(
        refs.trim().is_empty(),
        "stop should not create bitloops/* shadow branches in DB/blob mode"
    );

    assert!(
        pre_prompt_state(dir.path(), sid).is_none(),
        "pre-prompt state should be cleaned up by stop"
    );
}

#[test]
fn stop_handles_already_committed_files_gracefully() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    setup_claude_and_enable(dir.path());

    let sid = "test-session-3";
    let transcript_dir = tempfile::tempdir().unwrap();
    let transcript_path = transcript_dir.path().join("transcript.jsonl");
    user_prompt_submit(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Create committed file",
    );

    fs::write(dir.path().join("created.txt"), "created by claude\n").unwrap();
    run_git(dir.path(), &["add", "created.txt"]);
    run_git(dir.path(), &["commit", "-m", "user committed before stop"]);

    fs::write(
        &transcript_path,
        r#"{"type":"user","message":{"content":[{"type":"text","text":"Create committed file"}]}}
{"type":"assistant","message":{"content":[{"type":"text","text":"Done"}]}}
"#,
    )
    .unwrap();

    stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    let state = session_state(dir.path(), sid);
    assert_eq!(state.phase, SessionPhase::Idle);
    assert_eq!(
        state.step_count, 0,
        "checkpoint should be skipped when no working tree changes remain"
    );

    assert!(
        pre_prompt_state(dir.path(), sid).is_none(),
        "pre-prompt state should be cleaned up by stop"
    );
}

#[test]
fn stop_subagent_only_changes_still_create_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    setup_claude_and_enable(dir.path());

    let sid = "test-session-4";
    let transcript_path = dir.path().join("transcript.jsonl");
    user_prompt_submit(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Delegate to subagent",
    );

    fs::write(
        dir.path().join("subagent_output.rs"),
        "package main\n\nfunc SubagentWork() {}\n",
    )
    .unwrap();

    // Main transcript references only Task tool activity.
    fs::write(
        &transcript_path,
        r#"{"type":"user","message":{"content":[{"type":"text","text":"Delegate to subagent"}]}}
{"type":"assistant","message":{"content":[{"type":"text","text":"I will use a subagent"},{"type":"tool_use","name":"Task","id":"toolu_1","input":{"description":"Do work"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"agentId: sub123"}]}}
{"type":"assistant","message":{"content":[{"type":"text","text":"Subagent finished"}]}}
"#,
    )
    .unwrap();

    stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    let state = session_state(dir.path(), sid);
    assert!(
        state.step_count > 0,
        "checkpoint_count should increase for subagent-only file changes"
    );
    assert!(
        state
            .files_touched
            .iter()
            .any(|f| f == "subagent_output.rs"),
        "subagent output file should be tracked in session state"
    );

    let refs = run_git(
        dir.path(),
        &["for-each-ref", "--format=%(refname)", "refs/heads/bitloops"],
    );
    assert!(
        refs.trim().is_empty(),
        "subagent-only stop should not create bitloops/* shadow branches"
    );
}

#[test]
fn user_prompt_submit_reinstalls_overwritten_git_hooks() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    setup_claude_and_enable(dir.path());

    let hooks_dir = dir.path().join(".git/hooks");
    let hook_names = [
        "prepare-commit-msg",
        "commit-msg",
        "post-commit",
        "pre-push",
    ];
    for hook in hook_names {
        fs::write(
            hooks_dir.join(hook),
            "#!/bin/sh\n# Third-party hook manager\necho third-party\n",
        )
        .unwrap();
    }

    let sid = "test-session-5";
    user_prompt_submit(dir.path(), sid, "", "Reinstall hooks");

    for hook in [
        "prepare-commit-msg",
        "commit-msg",
        "post-commit",
        "pre-push",
    ] {
        let content = fs::read_to_string(hooks_dir.join(hook)).unwrap();
        assert!(
            content.contains("# Bitloops git hooks"),
            "{hook} should be reinstalled after user-prompt-submit"
        );
        assert!(
            hooks_dir.join(format!("{hook}.pre-bitloops")).exists(),
            "{hook}.pre-bitloops backup should exist"
        );
    }
}

#[test]
fn user_prompt_submit_reinstalls_deleted_git_hooks() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    setup_claude_and_enable(dir.path());

    let hooks_dir = dir.path().join(".git/hooks");
    for hook in [
        "prepare-commit-msg",
        "commit-msg",
        "post-commit",
        "pre-push",
    ] {
        fs::remove_file(hooks_dir.join(hook)).unwrap();
    }

    let sid = "test-session-6";
    user_prompt_submit(dir.path(), sid, "", "Restore deleted hooks");

    for hook in [
        "prepare-commit-msg",
        "commit-msg",
        "post-commit",
        "pre-push",
    ] {
        let content = fs::read_to_string(hooks_dir.join(hook)).unwrap();
        assert!(
            content.contains("# Bitloops git hooks"),
            "{hook} should be recreated after user-prompt-submit"
        );
    }
}

#[test]
fn stop_creates_shadow_branch_named_after_head_prefix() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    setup_claude_and_enable(dir.path());

    let sid = "test-session-7";
    let transcript_path = dir.path().join("transcript.jsonl");
    user_prompt_submit(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Create auth module",
    );
    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::write(
        dir.path().join("src/auth.rs"),
        "package auth\n\nfunc Auth() bool { return true }\n",
    )
    .unwrap();
    fs::write(
        &transcript_path,
        r#"{"type":"user","message":{"content":[{"type":"text","text":"Create auth module"}]}}
{"type":"assistant","message":{"content":[{"type":"text","text":"Done"}]}}
"#,
    )
    .unwrap();

    stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    let state = session_state(dir.path(), sid);
    assert_eq!(state.phase, SessionPhase::Idle);
    assert!(
        state.step_count > 0,
        "stop should create a pending checkpoint"
    );

    let refs = run_git(
        dir.path(),
        &[
            "for-each-ref",
            "--format=%(refname)",
            "refs/heads/bitloops/",
        ],
    );
    assert!(
        refs.trim().is_empty(),
        "DB/blob mode should not create shadow branches; got refs:\n{refs}"
    );
}

#[test]
fn commit_condenses_metadata_to_checkpoints_branch() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    setup_claude_and_enable(dir.path());

    let sid = "test-session-8";
    let transcript_path = dir.path().join("transcript.jsonl");
    user_prompt_submit(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Create auth module",
    );

    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::write(
        dir.path().join("src/auth.rs"),
        "package auth\n\nfunc Auth() bool { return true }\n",
    )
    .unwrap();
    fs::write(
        &transcript_path,
        r#"{"type":"user","message":{"content":[{"type":"text","text":"Create auth module"}]}}
{"type":"assistant","message":{"content":[{"type":"text","text":"Implemented src/auth.rs"}]}}
"#,
    )
    .unwrap();

    stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    run_git(dir.path(), &["add", "src/auth.rs"]);
    run_git(dir.path(), &["commit", "-m", "feat: add auth module"]);

    let checkpoint_id =
        checkpoint_id_for_head(dir.path()).expect("HEAD commit should map to a checkpoint");
    let summary = read_committed(dir.path(), &checkpoint_id)
        .expect("reading committed checkpoint should succeed")
        .expect("committed checkpoint should exist");

    assert!(
        summary.files_touched.iter().any(|f| f == "src/auth.rs"),
        "checkpoint should include touched file list for src/auth.rs"
    );
    assert!(
        summary.sessions.len() == 1,
        "single-session stop+commit should produce one checkpoint session"
    );

    let content =
        read_session_content(dir.path(), &checkpoint_id, 0).expect("session content should exist");
    assert!(
        content.prompts.contains("Create auth module"),
        "checkpoint prompts should include the user prompt"
    );

    let state = session_state(dir.path(), sid);
    assert_eq!(state.last_checkpoint_id, checkpoint_id);
    assert_eq!(
        state.step_count, 0,
        "checkpoint_count should be reset after condensation"
    );
}

#[test]
fn intermediate_commit_without_new_prompt_has_no_checkpoint_trailer() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    setup_claude_and_enable(dir.path());

    let sid = "test-session-9";
    let transcript_path = dir.path().join("transcript.jsonl");
    user_prompt_submit(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Create file A",
    );
    fs::write(dir.path().join("a.rs"), "package main\n\nfunc A() {}\n").unwrap();
    fs::write(
        &transcript_path,
        r#"{"type":"user","message":{"content":[{"type":"text","text":"Create file A"}]}}
{"type":"assistant","message":{"content":[{"type":"text","text":"Done"}]}}
"#,
    )
    .unwrap();
    stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    run_git(dir.path(), &["add", "a.rs"]);
    run_git(dir.path(), &["commit", "-m", "first"]);
    let checkpoint1 = checkpoint_id_for_head(dir.path())
        .expect("first commit should be mapped to a checkpoint id");
    let mappings_after_first = read_commit_checkpoint_mappings(dir.path())
        .expect("reading commit-checkpoint mappings should succeed");
    let checkpoints_count_after_first = mappings_after_first.len();

    fs::write(dir.path().join("intermediate.txt"), "human change\n").unwrap();
    run_git(dir.path(), &["add", "intermediate.txt"]);
    run_git(dir.path(), &["commit", "-m", "intermediate"]);
    let second_head = run_git(dir.path(), &["rev-parse", "HEAD"]);
    let mappings_after_second = read_commit_checkpoint_mappings(dir.path())
        .expect("reading commit-checkpoint mappings should succeed");
    assert!(
        !mappings_after_second.contains_key(&second_head),
        "intermediate commit without a new prompt should not map to a checkpoint"
    );
    assert_eq!(
        mappings_after_second.len(),
        checkpoints_count_after_first,
        "checkpoint mapping set should not advance for non-session intermediate commit"
    );

    user_prompt_submit(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Create file B",
    );
    fs::write(dir.path().join("b.rs"), "package main\n\nfunc B() {}\n").unwrap();
    fs::write(
        &transcript_path,
        r#"{"type":"user","message":{"content":[{"type":"text","text":"Create file B"}]}}
{"type":"assistant","message":{"content":[{"type":"text","text":"Done"}]}}
"#,
    )
    .unwrap();
    stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    run_git(dir.path(), &["add", "b.rs"]);
    run_git(dir.path(), &["commit", "-m", "third"]);
    let checkpoint3 =
        checkpoint_id_for_head(dir.path()).expect("third commit should map to a checkpoint id");
    assert_ne!(
        checkpoint1, checkpoint3,
        "new session work should produce a new checkpoint id"
    );

    assert!(
        read_committed(dir.path(), &checkpoint1)
            .expect("reading first checkpoint should succeed")
            .is_some()
    );
    assert!(
        read_committed(dir.path(), &checkpoint3)
            .expect("reading third checkpoint should succeed")
            .is_some()
    );
}

#[test]
fn pre_push_pushes_checkpoints_branch_to_remote() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    setup_claude_and_enable(dir.path());

    let sid = "test-session-10";
    let transcript_path = dir.path().join("transcript.jsonl");
    user_prompt_submit(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Create push target file",
    );
    fs::write(dir.path().join("push.txt"), "checkpoint data\n").unwrap();
    fs::write(
        &transcript_path,
        r#"{"type":"user","message":{"content":[{"type":"text","text":"Create push target file"}]}}
{"type":"assistant","message":{"content":[{"type":"text","text":"Done"}]}}
"#,
    )
    .unwrap();
    stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    run_git(dir.path(), &["add", "push.txt"]);
    run_git(dir.path(), &["commit", "-m", "push test"]);
    let checkpoint_id =
        checkpoint_id_for_head(dir.path()).expect("push test commit should map to a checkpoint");
    assert!(
        !checkpoint_id.is_empty(),
        "mapped checkpoint id should not be empty"
    );
    assert!(
        !git_ref_exists(dir.path(), "refs/heads/bitloops/checkpoints/v1"),
        "DB/blob mode should not materialise a local checkpoints branch"
    );

    let remote_dir = tempfile::tempdir().unwrap();
    run_git(remote_dir.path(), &["init", "--bare"]);
    let remote_path = remote_dir.path().to_string_lossy().to_string();

    run_git(dir.path(), &["remote", "add", "origin", &remote_path]);
    let branch = run_git(dir.path(), &["rev-parse", "--abbrev-ref", "HEAD"]);
    run_git(dir.path(), &["push", "-u", "origin", &branch]);

    let out = Command::new("git")
        .args([
            "--git-dir",
            &remote_path,
            "show-ref",
            "--verify",
            "refs/heads/bitloops/checkpoints/v1",
        ])
        .output()
        .expect("failed to inspect remote refs");
    assert!(
        !out.status.success(),
        "remote should not contain bitloops/checkpoints/v1 in DB/blob mode\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

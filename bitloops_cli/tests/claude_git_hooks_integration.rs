use serde_json::Value;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn bitloops_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_bitloops"))
}

fn run_cmd(repo: &Path, args: &[&str], stdin: Option<&str>) -> Output {
    let mut cmd = Command::new(bitloops_bin());
    cmd.args(args).current_dir(repo);
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
    let mut cmd = Command::new(bitloops_bin());
    cmd.args(args).current_dir(repo);
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

fn git_file_exists_in_ref(repo: &Path, reference: &str, path: &str) -> bool {
    let out = Command::new("git")
        .args(["cat-file", "-e", &format!("{reference}:{path}")])
        .current_dir(repo)
        .output()
        .expect("failed to run git");
    out.status.success()
}

fn git_show_file(repo: &Path, reference: &str, path: &str) -> String {
    run_git(repo, &["show", &format!("{reference}:{path}")])
}

fn git_commit_message(repo: &Path, rev: &str) -> String {
    run_git(repo, &["show", "-s", "--format=%B", rev])
}

fn checkpoint_id_from_message(message: &str) -> Option<String> {
    for line in message.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("Bitloops-Checkpoint: ") {
            let id = rest.trim();
            if id.len() >= 12 && id[..12].chars().all(|c| c.is_ascii_hexdigit()) {
                return Some(id[..12].to_lowercase());
            }
        }
    }
    None
}

fn checkpoint_shard(id: &str) -> (String, String) {
    if id.len() >= 2 {
        (id[..2].to_string(), id[2..].to_string())
    } else {
        (id.to_string(), String::new())
    }
}

fn init_repo(repo: &Path) {
    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "t@t.com"]);
    run_git(repo, &["config", "user.name", "Test"]);
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

fn session_state(repo: &Path, session_id: &str) -> Value {
    read_json(
        &repo
            .join(".git/bitloops-sessions")
            .join(format!("{session_id}.json")),
    )
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
    let state_dir = repo.join(".git").join("bitloops-sessions");
    fs::create_dir_all(&state_dir).expect("failed to create session state directory");
    let state_path = state_dir.join(format!("{session_id}.json"));
    let state = serde_json::json!({
        "session_id": session_id,
        "worktree_path": repo.to_string_lossy(),
        "started_at": "2026-01-01T00:00:00Z",
        "last_interaction_time": "2026-01-01T00:00:01Z",
        "phase": "active"
    });
    fs::write(
        state_path,
        serde_json::to_vec(&state).expect("failed to serialize session state"),
    )
    .expect("failed to write session state");
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
        &[("ENTIRE_LOG_LEVEL", "debug")],
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
        &[("ENTIRE_LOG_LEVEL", "debug")],
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

    let pre_prompt_path = dir
        .path()
        .join(".bitloops/tmp")
        .join(format!("pre-prompt-{sid}.json"));
    assert!(pre_prompt_path.exists(), "pre-prompt state should exist");

    let state = session_state(dir.path(), sid);
    assert_eq!(state["session_id"], sid);
    assert_eq!(state["phase"], "active");
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

    let meta_dir = dir.path().join(".bitloops/metadata").join(sid);
    assert!(meta_dir.join("full.jsonl").exists());
    assert!(meta_dir.join("prompt.txt").exists());
    assert!(meta_dir.join("summary.txt").exists());
    assert!(meta_dir.join("context.md").exists());

    let prompt = fs::read_to_string(meta_dir.join("prompt.txt")).unwrap();
    let summary = fs::read_to_string(meta_dir.join("summary.txt")).unwrap();
    assert!(!prompt.trim().is_empty(), "prompt.txt should not be empty");
    assert!(
        !summary.trim().is_empty(),
        "summary.txt should not be empty"
    );

    let state = session_state(dir.path(), sid);
    assert_eq!(state["phase"], "idle");
    assert!(
        state["checkpoint_count"].as_u64().unwrap_or(0) > 0,
        "checkpoint_count should increase after stop with changes"
    );

    let refs = run_git(
        dir.path(),
        &["for-each-ref", "--format=%(refname)", "refs/heads/bitloops"],
    );
    assert!(
        refs.lines()
            .any(|line| line.starts_with("refs/heads/bitloops/")),
        "shadow branch should exist after stop"
    );

    let pre_prompt_path = dir
        .path()
        .join(".bitloops/tmp")
        .join(format!("pre-prompt-{sid}.json"));
    assert!(
        !pre_prompt_path.exists(),
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
    assert_eq!(state["phase"], "idle");
    assert_eq!(
        state["checkpoint_count"].as_u64().unwrap_or(0),
        0,
        "checkpoint should be skipped when no working tree changes remain"
    );

    let pre_prompt_path = dir
        .path()
        .join(".bitloops/tmp")
        .join(format!("pre-prompt-{sid}.json"));
    assert!(
        !pre_prompt_path.exists(),
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
        state["checkpoint_count"].as_u64().unwrap_or(0) > 0,
        "checkpoint_count should increase for subagent-only file changes"
    );

    let refs = run_git(
        dir.path(),
        &["for-each-ref", "--format=%(refname)", "refs/heads/bitloops"],
    );
    assert!(
        refs.lines()
            .any(|line| line.starts_with("refs/heads/bitloops/")),
        "shadow branch should exist after subagent-only changes"
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

    let base_head = run_git(dir.path(), &["rev-parse", "HEAD"]);
    let expected_prefix = format!("refs/heads/bitloops/{}-", &base_head[..7]);

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

    let refs = run_git(
        dir.path(),
        &[
            "for-each-ref",
            "--format=%(refname)",
            "refs/heads/bitloops/",
        ],
    );
    let found = refs.lines().any(|line| {
        line.starts_with(&expected_prefix) || line == expected_prefix.trim_end_matches('-')
    });
    assert!(
        found,
        "expected a shadow branch starting with: {expected_prefix}; got refs:\n{refs}"
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

    let head_msg = git_commit_message(dir.path(), "HEAD");
    let checkpoint_id =
        checkpoint_id_from_message(&head_msg).expect("HEAD commit should include checkpoint id");
    let (d1, d2) = checkpoint_shard(&checkpoint_id);
    let cp_ref = "bitloops/checkpoints/v1";

    assert!(
        git_ref_exists(dir.path(), "refs/heads/bitloops/checkpoints/v1"),
        "checkpoints branch should exist after post-commit"
    );
    assert!(
        git_file_exists_in_ref(dir.path(), cp_ref, &format!("{d1}/{d2}/metadata.json")),
        "top-level checkpoint metadata should exist"
    );
    assert!(
        git_file_exists_in_ref(dir.path(), cp_ref, &format!("{d1}/{d2}/0/metadata.json")),
        "per-session checkpoint metadata should exist"
    );
    assert!(
        git_file_exists_in_ref(dir.path(), cp_ref, &format!("{d1}/{d2}/0/full.jsonl")),
        "full transcript should exist in checkpoint tree"
    );
    assert!(
        git_file_exists_in_ref(dir.path(), cp_ref, &format!("{d1}/{d2}/0/prompt.txt")),
        "prompt.txt should exist in checkpoint tree"
    );
    assert!(
        git_file_exists_in_ref(dir.path(), cp_ref, &format!("{d1}/{d2}/0/context.md")),
        "context.md should exist in checkpoint tree"
    );

    let prompt_txt = git_show_file(dir.path(), cp_ref, &format!("{d1}/{d2}/0/prompt.txt"));
    assert!(
        prompt_txt.contains("Create auth module"),
        "prompt.txt should include user prompt"
    );

    let state = session_state(dir.path(), sid);
    assert_eq!(state["last_checkpoint_id"], checkpoint_id);
    assert_eq!(
        state["checkpoint_count"].as_u64().unwrap_or_default(),
        0,
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
    let first_msg = git_commit_message(dir.path(), "HEAD");
    let checkpoint1 =
        checkpoint_id_from_message(&first_msg).expect("first commit should include checkpoint id");

    let checkpoints_count_after_first = run_git(
        dir.path(),
        &["rev-list", "--count", "bitloops/checkpoints/v1"],
    )
    .parse::<u64>()
    .expect("checkpoint branch commit count should be numeric");

    fs::write(dir.path().join("intermediate.txt"), "human change\n").unwrap();
    run_git(dir.path(), &["add", "intermediate.txt"]);
    run_git(dir.path(), &["commit", "-m", "intermediate"]);
    let second_msg = git_commit_message(dir.path(), "HEAD");
    assert!(
        checkpoint_id_from_message(&second_msg).is_none(),
        "intermediate commit without new prompt should not get checkpoint trailer"
    );

    let checkpoints_count_after_second = run_git(
        dir.path(),
        &["rev-list", "--count", "bitloops/checkpoints/v1"],
    )
    .parse::<u64>()
    .expect("checkpoint branch commit count should be numeric");
    assert_eq!(
        checkpoints_count_after_second, checkpoints_count_after_first,
        "checkpoint branch should not advance for non-session intermediate commit"
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
    let third_msg = git_commit_message(dir.path(), "HEAD");
    let checkpoint3 =
        checkpoint_id_from_message(&third_msg).expect("third commit should include checkpoint id");
    assert_ne!(
        checkpoint1, checkpoint3,
        "new session work should produce a new checkpoint id"
    );

    let (a1, a2) = checkpoint_shard(&checkpoint1);
    let (b1, b2) = checkpoint_shard(&checkpoint3);
    assert!(git_file_exists_in_ref(
        dir.path(),
        "bitloops/checkpoints/v1",
        &format!("{a1}/{a2}/metadata.json")
    ));
    assert!(git_file_exists_in_ref(
        dir.path(),
        "bitloops/checkpoints/v1",
        &format!("{b1}/{b2}/metadata.json")
    ));
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
    assert!(
        git_ref_exists(dir.path(), "refs/heads/bitloops/checkpoints/v1"),
        "local checkpoints branch should exist before push"
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
        out.status.success(),
        "remote should contain bitloops/checkpoints/v1 after pushing main branch\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

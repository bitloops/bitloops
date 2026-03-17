use bitloops_cli::engine::session::SessionBackend;
use bitloops_cli::engine::session::create_session_backend_or_local;
use bitloops_cli::engine::session::phase::SessionPhase;
use bitloops_cli::engine::strategy::manual_commit::{
    read_commit_checkpoint_mappings, read_committed, read_session_content,
};
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn bitloops_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_bitloops"))
}

fn run_cmd_with_home(repo: &Path, home: &Path, args: &[&str], stdin: Option<&str>) -> Output {
    let xdg_config_home = home.join("xdg");
    fs::create_dir_all(&xdg_config_home).expect("create xdg config home");

    let mut cmd = Command::new(bitloops_bin());
    cmd.args(args)
        .current_dir(repo)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("XDG_CONFIG_HOME", &xdg_config_home)
        .env_remove("BITLOOPS_DEVQL_PG_DSN")
        .env_remove("BITLOOPS_DEVQL_CH_URL")
        .env_remove("BITLOOPS_DEVQL_CH_DATABASE")
        .env_remove("BITLOOPS_DEVQL_CH_USER")
        .env_remove("BITLOOPS_DEVQL_CH_PASSWORD");

    if let Some(input) = stdin {
        cmd.stdin(Stdio::piped());
        let mut child = cmd.spawn().expect("failed to spawn bitloops command");
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

fn run_git_expect_success(repo: &Path, args: &[&str], context: &str) -> Output {
    let out = run_git_output(repo, args);
    assert!(
        out.status.success(),
        "{context}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

fn git_commit_message(repo: &Path, rev: &str) -> String {
    run_git(repo, &["show", "-s", "--format=%B", rev])
}

fn init_repo(repo: &Path) {
    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "t@t.com"]);
    run_git(repo, &["config", "user.name", "Test"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);
    run_git(repo, &["config", "tag.gpgsign", "false"]);
    fs::write(repo.join("README.md"), "init\n").unwrap();
    run_git(repo, &["add", "."]);
    run_git(repo, &["commit", "-m", "initial"]);
}

fn write_transcript(path: &Path, prompt: &str, response: &str, file_path: &str) {
    let payload = format!(
        r#"{{"type":"user.message","data":{{"content":{prompt_json}}}}}
{{"type":"tool.execution_complete","data":{{"model":"gpt-5","toolTelemetry":{{"properties":{{"filePaths":{file_paths_json}}}}}}}}}
{{"type":"assistant.message","data":{{"content":{response_json},"outputTokens":42}}}}
{{"type":"session.shutdown","data":{{"modelMetrics":[{{"requests":{{"count":1}},"usage":{{"inputTokens":100,"outputTokens":42,"cacheReadTokens":0,"cacheWriteTokens":0}}}}]}}}}
"#,
        prompt_json = serde_json::to_string(prompt).unwrap(),
        response_json = serde_json::to_string(response).unwrap(),
        file_paths_json = serde_json::to_string(&format!("[\"{file_path}\"]")).unwrap(),
    );

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, payload).unwrap();
}

#[test]
fn copilot_basic_workflow() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    init_repo(dir.path());

    let init_out = run_cmd_with_home(
        dir.path(),
        home.path(),
        &["init", "--agent", "copilot"],
        None,
    );
    assert_success(&init_out, "bitloops init --agent copilot");

    let enable_out = run_cmd_with_home(dir.path(), home.path(), &["enable"], None);
    assert_success(&enable_out, "bitloops enable");

    run_git_expect_success(
        dir.path(),
        &["add", ".github/hooks/bitloops.json", ".bitloops"],
        "stage copilot infra files",
    );
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "seed copilot bitloops infra files"],
        "commit copilot infra files",
    );

    let sid = "copilot-basic-1";
    let transcript_path = home
        .path()
        .join(".copilot")
        .join("session-state")
        .join(sid)
        .join("events.jsonl");

    let out = run_cmd_with_home(
        dir.path(),
        home.path(),
        &["hooks", "copilot", "session-start"],
        Some(r#"{"sessionId":"copilot-basic-1"}"#),
    );
    assert_success(&out, "hooks copilot session-start");

    let out = run_cmd_with_home(
        dir.path(),
        home.path(),
        &["hooks", "copilot", "user-prompt-submitted"],
        Some(r#"{"sessionId":"copilot-basic-1","prompt":"Create copilot_hello.txt"}"#),
    );
    assert_success(&out, "hooks copilot user-prompt-submitted");

    fs::write(dir.path().join("copilot_hello.txt"), "hello from copilot\n").unwrap();
    write_transcript(
        &transcript_path,
        "Create copilot_hello.txt",
        "Created copilot_hello.txt",
        "copilot_hello.txt",
    );

    let agent_stop_input = format!(
        r#"{{"sessionId":"{sid}","transcriptPath":"{}"}}"#,
        transcript_path.to_string_lossy()
    );
    let out = run_cmd_with_home(
        dir.path(),
        home.path(),
        &["hooks", "copilot", "agent-stop"],
        Some(&agent_stop_input),
    );
    assert_success(&out, "hooks copilot agent-stop");

    let out = run_cmd_with_home(
        dir.path(),
        home.path(),
        &["hooks", "copilot", "session-end"],
        Some(r#"{"sessionId":"copilot-basic-1"}"#),
    );
    assert_success(&out, "hooks copilot session-end");

    run_git_expect_success(
        dir.path(),
        &["add", "copilot_hello.txt"],
        "stage copilot output file",
    );
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "add copilot hello file"],
        "commit copilot output file",
    );

    let head_sha = run_git(dir.path(), &["rev-parse", "HEAD"]);
    let mappings =
        read_commit_checkpoint_mappings(dir.path()).expect("read commit-checkpoint mappings");
    let checkpoint_id = mappings
        .get(&head_sha)
        .cloned()
        .expect("copilot workflow should map HEAD to a committed checkpoint");

    let summary = read_committed(dir.path(), &checkpoint_id)
        .expect("read committed checkpoint")
        .expect("committed checkpoint should exist");
    assert_eq!(summary.checkpoint_id, checkpoint_id);
    assert_eq!(summary.strategy, "manual-commit");

    let session = read_session_content(dir.path(), &checkpoint_id, 0).expect("read session");
    assert_eq!(session.metadata["agent"], "copilot");

    let backend = create_session_backend_or_local(dir.path());
    let state = backend
        .load_session("copilot-basic-1")
        .expect("load copilot session")
        .expect("copilot session should exist");
    assert_eq!(state.phase, SessionPhase::Ended);
    assert_eq!(state.step_count, 0);
    assert!(state.turn_checkpoint_ids.is_empty());
    assert!(
        !state.last_checkpoint_id.is_empty(),
        "condensed session should record last checkpoint id"
    );

    let head_message = git_commit_message(dir.path(), "HEAD");
    assert!(
        !head_message.contains("Bitloops-Checkpoint: "),
        "manual-commit persistence currently relies on commit-checkpoint mappings, not commit-message trailers\n{head_message}"
    );
}

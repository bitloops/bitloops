use bitloops_cli::config::resolve_sqlite_db_path_for_repo;
use bitloops_cli::config::resolve_store_backend_config_for_repo;
use bitloops_cli::host::session::create_session_backend_or_local;
use bitloops_cli::host::session::phase::SessionPhase;
use bitloops_cli::host::strategy::manual_commit::{
    read_commit_checkpoint_mappings, read_committed, read_session_content,
};
use rusqlite::Connection;
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
    write_transcript_turn(path, prompt, response, file_path, false);
}

fn write_transcript_turn(path: &Path, prompt: &str, response: &str, file_path: &str, append: bool) {
    let payload = format!(
        r#"{{"type":"user.message","data":{{"content":{prompt_json}}}}}
{{"type":"tool.execution_complete","data":{{"model":"gpt-5","toolTelemetry":{{"properties":{{"filePaths":{file_paths_json}}}}}}}}}
{{"type":"assistant.message","data":{{"content":{response_json},"outputTokens":42}}}}
"#,
        prompt_json = serde_json::to_string(prompt).unwrap(),
        response_json = serde_json::to_string(response).unwrap(),
        file_paths_json = serde_json::to_string(&format!("[\"{file_path}\"]")).unwrap(),
    );

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    if append {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        file.write_all(payload.as_bytes()).unwrap();
    } else {
        fs::write(path, payload).unwrap();
    }
}

fn append_transcript_shutdown(path: &Path) {
    let payload = r#"{"type":"session.shutdown","data":{"modelMetrics":[{"requests":{"count":1},"usage":{"inputTokens":100,"outputTokens":42,"cacheReadTokens":0,"cacheWriteTokens":0}}]}}
"#;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    file.write_all(payload.as_bytes()).unwrap();
}

fn checkpoint_sqlite_path(repo_root: &Path) -> PathBuf {
    let cfg = resolve_store_backend_config_for_repo(repo_root).expect("resolve backend config");
    if let Some(path) = cfg.relational.sqlite_path.as_deref() {
        resolve_sqlite_db_path_for_repo(repo_root, Some(path)).expect("resolve sqlite path")
    } else {
        bitloops_cli::utils::paths::default_relational_db_path(repo_root)
    }
}

fn temporary_checkpoint_count(repo_root: &Path, session_id: &str) -> i64 {
    let conn = Connection::open(checkpoint_sqlite_path(repo_root)).expect("open sqlite");
    conn.query_row(
        "SELECT COUNT(*) FROM temporary_checkpoints WHERE session_id = ?1",
        [session_id],
        |row| row.get(0),
    )
    .expect("count temporary checkpoints")
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

    let backend = create_session_backend_or_local(dir.path());
    let pre_commit_state = backend
        .load_session("copilot-basic-1")
        .expect("load pre-commit copilot session")
        .expect("pre-commit copilot session should exist");
    assert_eq!(pre_commit_state.first_prompt, "Create copilot_hello.txt");
    assert!(
        !pre_commit_state.turn_id.is_empty(),
        "turn id should be initialized before commit"
    );
    assert_eq!(
        temporary_checkpoint_count(dir.path(), "copilot-basic-1"),
        1,
        "Copilot turn end should write one temporary checkpoint before commit"
    );

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
    assert!(
        summary
            .files_touched
            .contains(&"copilot_hello.txt".to_string()),
        "committed checkpoint should include Copilot-created file"
    );

    let session = read_session_content(dir.path(), &checkpoint_id, 0).expect("read session");
    assert_eq!(session.metadata["agent"], "copilot");
    assert_eq!(session.prompts, "Create copilot_hello.txt");
    assert!(
        session.context.contains("Create copilot_hello.txt"),
        "checkpoint context should include the Copilot prompt"
    );

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

#[test]
fn copilot_agent_stop_without_transcript_path_uses_session_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    init_repo(dir.path());

    assert_success(
        &run_cmd_with_home(
            dir.path(),
            home.path(),
            &["init", "--agent", "copilot"],
            None,
        ),
        "bitloops init --agent copilot",
    );
    assert_success(
        &run_cmd_with_home(dir.path(), home.path(), &["enable"], None),
        "bitloops enable",
    );

    run_git_expect_success(
        dir.path(),
        &["add", ".github/hooks/bitloops.json", ".bitloops"],
        "stage copilot infra files",
    );
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "seed copilot infra"],
        "commit copilot infra files",
    );

    let sid = "copilot-fallback-1";
    let transcript_path = home
        .path()
        .join(".copilot")
        .join("session-state")
        .join(sid)
        .join("events.jsonl");

    assert_success(
        &run_cmd_with_home(
            dir.path(),
            home.path(),
            &["hooks", "copilot", "session-start"],
            Some(&format!(r#"{{"sessionId":"{sid}"}}"#)),
        ),
        "hooks copilot session-start",
    );
    assert_success(
        &run_cmd_with_home(
            dir.path(),
            home.path(),
            &["hooks", "copilot", "user-prompt-submitted"],
            Some(&format!(
                r#"{{"sessionId":"{sid}","prompt":"Create fallback.txt"}}"#
            )),
        ),
        "hooks copilot user-prompt-submitted",
    );

    fs::write(dir.path().join("fallback.txt"), "fallback\n").unwrap();
    write_transcript(
        &transcript_path,
        "Create fallback.txt",
        "Created fallback.txt",
        "fallback.txt",
    );

    assert_success(
        &run_cmd_with_home(
            dir.path(),
            home.path(),
            &["hooks", "copilot", "agent-stop"],
            Some(&format!(r#"{{"sessionId":"{sid}","transcriptPath":""}}"#)),
        ),
        "hooks copilot agent-stop",
    );
    assert_eq!(temporary_checkpoint_count(dir.path(), sid), 1);

    assert_success(
        &run_cmd_with_home(
            dir.path(),
            home.path(),
            &["hooks", "copilot", "session-end"],
            Some(&format!(r#"{{"sessionId":"{sid}"}}"#)),
        ),
        "hooks copilot session-end",
    );

    run_git_expect_success(dir.path(), &["add", "fallback.txt"], "stage fallback");
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "add fallback file"],
        "commit fallback",
    );

    let head_sha = run_git(dir.path(), &["rev-parse", "HEAD"]);
    let mappings = read_commit_checkpoint_mappings(dir.path()).expect("read mappings");
    assert!(
        mappings.contains_key(&head_sha),
        "Copilot fallback transcript path should still produce a commit mapping"
    );
}

#[test]
fn copilot_session_start_initial_prompt_bootstraps_first_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    init_repo(dir.path());

    assert_success(
        &run_cmd_with_home(
            dir.path(),
            home.path(),
            &["init", "--agent", "copilot"],
            None,
        ),
        "bitloops init --agent copilot",
    );
    assert_success(
        &run_cmd_with_home(dir.path(), home.path(), &["enable"], None),
        "bitloops enable",
    );

    run_git_expect_success(
        dir.path(),
        &["add", ".github/hooks/bitloops.json", ".bitloops"],
        "stage copilot infra files",
    );
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "seed copilot infra"],
        "commit copilot infra files",
    );

    let sid = "copilot-bootstrap-1";
    assert_success(
        &run_cmd_with_home(
            dir.path(),
            home.path(),
            &["hooks", "copilot", "session-start"],
            Some(&format!(
                r#"{{"sessionId":"{sid}","initialPrompt":"Bootstrap prompt"}}"#
            )),
        ),
        "hooks copilot session-start",
    );

    let backend = create_session_backend_or_local(dir.path());
    let state = backend
        .load_session(sid)
        .expect("load session")
        .expect("session should exist");
    assert_eq!(state.first_prompt, "Bootstrap prompt");
}

#[test]
fn copilot_multi_turn_session_condenses_both_prompts() {
    let dir = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    init_repo(dir.path());

    assert_success(
        &run_cmd_with_home(
            dir.path(),
            home.path(),
            &["init", "--agent", "copilot"],
            None,
        ),
        "bitloops init --agent copilot",
    );
    assert_success(
        &run_cmd_with_home(dir.path(), home.path(), &["enable"], None),
        "bitloops enable",
    );

    run_git_expect_success(
        dir.path(),
        &["add", ".github/hooks/bitloops.json", ".bitloops"],
        "stage copilot infra files",
    );
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "seed copilot infra"],
        "commit copilot infra files",
    );

    let sid = "copilot-multi-turn-1";
    let transcript_path = home
        .path()
        .join(".copilot")
        .join("session-state")
        .join(sid)
        .join("events.jsonl");

    assert_success(
        &run_cmd_with_home(
            dir.path(),
            home.path(),
            &["hooks", "copilot", "session-start"],
            Some(&format!(r#"{{"sessionId":"{sid}"}}"#)),
        ),
        "hooks copilot session-start",
    );

    assert_success(
        &run_cmd_with_home(
            dir.path(),
            home.path(),
            &["hooks", "copilot", "user-prompt-submitted"],
            Some(&format!(
                r#"{{"sessionId":"{sid}","prompt":"Create first.txt"}}"#
            )),
        ),
        "hooks copilot user-prompt-submitted first",
    );
    fs::write(dir.path().join("first.txt"), "first\n").unwrap();
    write_transcript_turn(
        &transcript_path,
        "Create first.txt",
        "Created first.txt",
        "first.txt",
        false,
    );
    assert_success(
        &run_cmd_with_home(
            dir.path(),
            home.path(),
            &["hooks", "copilot", "agent-stop"],
            Some(&format!(
                r#"{{"sessionId":"{sid}","transcriptPath":"{}"}}"#,
                transcript_path.to_string_lossy()
            )),
        ),
        "hooks copilot agent-stop first",
    );

    assert_success(
        &run_cmd_with_home(
            dir.path(),
            home.path(),
            &["hooks", "copilot", "user-prompt-submitted"],
            Some(&format!(
                r#"{{"sessionId":"{sid}","prompt":"Create second.txt"}}"#
            )),
        ),
        "hooks copilot user-prompt-submitted second",
    );
    fs::write(dir.path().join("second.txt"), "second\n").unwrap();
    write_transcript_turn(
        &transcript_path,
        "Create second.txt",
        "Created second.txt",
        "second.txt",
        true,
    );
    append_transcript_shutdown(&transcript_path);
    assert_success(
        &run_cmd_with_home(
            dir.path(),
            home.path(),
            &["hooks", "copilot", "agent-stop"],
            Some(&format!(
                r#"{{"sessionId":"{sid}","transcriptPath":"{}"}}"#,
                transcript_path.to_string_lossy()
            )),
        ),
        "hooks copilot agent-stop second",
    );

    assert_success(
        &run_cmd_with_home(
            dir.path(),
            home.path(),
            &["hooks", "copilot", "session-end"],
            Some(&format!(r#"{{"sessionId":"{sid}"}}"#)),
        ),
        "hooks copilot session-end",
    );

    run_git_expect_success(
        dir.path(),
        &["add", "first.txt", "second.txt"],
        "stage multi-turn files",
    );
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "add multi-turn files"],
        "commit multi-turn files",
    );

    let head_sha = run_git(dir.path(), &["rev-parse", "HEAD"]);
    let mappings = read_commit_checkpoint_mappings(dir.path()).expect("read mappings");
    let checkpoint_id = mappings
        .get(&head_sha)
        .cloned()
        .expect("checkpoint mapping should exist");
    let session = read_session_content(dir.path(), &checkpoint_id, 0).expect("read session");
    assert!(session.prompts.contains("Create first.txt"));
    assert!(session.prompts.contains("Create second.txt"));

    let summary = read_committed(dir.path(), &checkpoint_id)
        .expect("read committed checkpoint")
        .expect("committed checkpoint should exist");
    assert!(summary.files_touched.contains(&"first.txt".to_string()));
    assert!(summary.files_touched.contains(&"second.txt".to_string()));
}

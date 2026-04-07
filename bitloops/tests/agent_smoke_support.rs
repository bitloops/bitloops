use bitloops::host::checkpoints::strategy::manual_commit::{
    read_commit_checkpoint_mappings, read_committed, read_session_content,
};
use serde_json::Value;
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

#[derive(Debug, serde::Deserialize)]
pub struct RewindPoint {
    pub id: String,
    #[serde(default)]
    pub is_logs_only: bool,
}

pub fn bitloops_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_bitloops"))
}

pub fn run_cmd(repo: &Path, args: &[&str], stdin: Option<&str>) -> Output {
    let bin = bitloops_bin();
    let mut cmd = crate::test_command_support::new_isolated_bitloops_command(&bin, repo, args);
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

pub fn assert_success(output: &Output, context: &str) {
    assert!(
        output.status.success(),
        "{context} failed\nstatus: {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

pub fn run_git_output(repo: &Path, args: &[&str]) -> Output {
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

    let mut cmd = Command::new("git");
    cmd.args(args).current_dir(repo).env("PATH", path);
    crate::test_command_support::apply_repo_app_env(&mut cmd, repo);
    cmd.output().expect("failed to run git")
}

pub fn run_git(repo: &Path, args: &[&str]) -> String {
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

pub fn run_git_expect_success(repo: &Path, args: &[&str], context: &str) -> Output {
    let out = run_git_output(repo, args);
    assert!(
        out.status.success(),
        "{context}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

pub fn run_git_without_hooks_expect_success(repo: &Path, args: &[&str], context: &str) -> Output {
    let no_hooks_dir = crate::test_command_support::isolated_repo_aux_dir(repo, "no-hooks");

    let mut prefixed_args = Vec::with_capacity(args.len() + 2);
    prefixed_args.push("-c");
    let hooks_path_override = format!("core.hooksPath={}", no_hooks_dir.display());
    prefixed_args.push(hooks_path_override.as_str());
    prefixed_args.extend_from_slice(args);

    run_git_expect_success(repo, &prefixed_args, context)
}

pub fn git_ref_exists(repo: &Path, reference: &str) -> bool {
    run_git_output(repo, &["show-ref", "--verify", "--quiet", reference])
        .status
        .success()
}

pub fn init_repo(repo: &Path) {
    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "t@t.com"]);
    run_git(repo, &["config", "user.name", "Test"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);
    fs::write(repo.join("README.md"), "init\n").expect("write initial README");
    run_git(repo, &["add", "."]);
    run_git(repo, &["commit", "-m", "initial"]);
}

pub fn checkpoint_id_for_head(repo: &Path) -> Option<String> {
    let head = run_git(repo, &["rev-parse", "HEAD"]);
    crate::test_command_support::with_repo_app_env(repo, || {
        read_commit_checkpoint_mappings(repo)
            .expect("failed to read commit-checkpoint mappings")
            .get(&head)
            .cloned()
    })
}

pub fn committed_summary(repo: &Path, checkpoint_id: &str) -> serde_json::Value {
    crate::test_command_support::with_repo_app_env(repo, || {
        serde_json::to_value(
            read_committed(repo, checkpoint_id)
                .expect("read committed checkpoint")
                .expect("committed checkpoint should exist"),
        )
        .expect("serialize committed checkpoint summary")
    })
}

pub fn committed_content(repo: &Path, checkpoint_id: &str, session_index: usize) -> Value {
    crate::test_command_support::with_repo_app_env(repo, || {
        serde_json::to_value(
            read_session_content(repo, checkpoint_id, session_index)
                .expect("read committed session content"),
        )
        .expect("serialize committed checkpoint content")
    })
}

pub fn get_rewind_points(repo: &Path) -> Vec<RewindPoint> {
    let out = run_cmd(repo, &["rewind", "--list"], None);
    assert_success(&out, "bitloops rewind --list");
    serde_json::from_slice(&out.stdout).unwrap_or_else(|err| {
        panic!(
            "failed to parse rewind --list output: {err}\nstdout:\n{}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

pub fn rewind_to(repo: &Path, point_id: &str) -> Output {
    run_cmd(repo, &["rewind", "--to", point_id], None)
}

pub fn init_claude(repo: &Path) {
    crate::test_command_support::with_repo_app_env(repo, || {
        crate::test_command_support::ensure_repo_daemon_stores(repo);
        let policy_path = repo.join(bitloops::config::REPO_POLICY_LOCAL_FILE_NAME);
        bitloops::config::settings::write_project_bootstrap_settings(
            &policy_path,
            bitloops::config::settings::DEFAULT_STRATEGY,
            &[String::from("claude-code")],
        )
        .expect("write Claude project bootstrap settings");
        bitloops::adapters::agents::claude_code::git_hooks::install_git_hooks(repo, false)
            .expect("install git hooks");
        bitloops::adapters::agents::AgentAdapterRegistry::builtin()
            .install_agent_hooks(repo, "claude-code", false, false)
            .expect("install Claude hooks");
    });
}

pub fn claude_user_prompt_submit(
    repo: &Path,
    session_id: &str,
    transcript_path: &str,
    prompt: &str,
) {
    let input = format!(
        r#"{{"session_id":"{session_id}","transcript_path":"{transcript_path}","prompt":{prompt_json}}}"#,
        prompt_json = serde_json::to_string(prompt).expect("serialize Claude prompt")
    );
    let out = run_cmd(
        repo,
        &["hooks", "claude-code", "user-prompt-submit"],
        Some(&input),
    );
    assert_success(&out, "hooks claude-code user-prompt-submit");
}

pub fn claude_stop(repo: &Path, session_id: &str, transcript_path: &str) {
    let input = format!(r#"{{"session_id":"{session_id}","transcript_path":"{transcript_path}"}}"#);
    let out = run_cmd(repo, &["hooks", "claude-code", "stop"], Some(&input));
    assert_success(&out, "hooks claude-code stop");
}

pub fn write_claude_transcript(path: &Path, prompt: &str, response: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create Claude transcript parent");
    }
    fs::write(
        path,
        format!(
            r#"{{"type":"user","message":{{"content":[{{"type":"text","text":{prompt_json}}}]}}}}
{{"type":"assistant","message":{{"content":[{{"type":"text","text":{response_json}}}]}}}}
"#,
            prompt_json = serde_json::to_string(prompt).expect("serialize Claude prompt"),
            response_json = serde_json::to_string(response).expect("serialize Claude response"),
        ),
    )
    .expect("write Claude transcript");
}

pub fn init_cursor(repo: &Path) {
    crate::test_command_support::with_repo_app_env(repo, || {
        crate::test_command_support::ensure_repo_daemon_stores(repo);
        let policy_path = repo.join(bitloops::config::REPO_POLICY_LOCAL_FILE_NAME);
        bitloops::config::settings::write_project_bootstrap_settings(
            &policy_path,
            bitloops::config::settings::DEFAULT_STRATEGY,
            &[String::from("cursor")],
        )
        .expect("write Cursor project bootstrap settings");
        bitloops::adapters::agents::claude_code::git_hooks::install_git_hooks(repo, false)
            .expect("install git hooks");
        bitloops::adapters::agents::AgentAdapterRegistry::builtin()
            .install_agent_hooks(repo, "cursor", false, false)
            .expect("install Cursor hooks");
    });
}

pub fn init_and_enable_cursor(repo: &Path) {
    init_cursor(repo);
    run_git_expect_success(
        repo,
        &["add", ".cursor/hooks.json"],
        "stage Cursor hook file",
    );
    run_git_without_hooks_expect_success(
        repo,
        &["commit", "-m", "seed cursor bitloops infra files"],
        "commit Cursor infra files",
    );
}

pub fn cursor_before_submit_prompt(
    repo: &Path,
    conversation_id: &str,
    transcript_path: &str,
    prompt: &str,
) {
    let input = format!(
        r#"{{"conversation_id":"{conversation_id}","transcript_path":"{transcript_path}","prompt":{prompt_json}}}"#,
        prompt_json = serde_json::to_string(prompt).expect("serialize Cursor prompt"),
    );
    let out = run_cmd(
        repo,
        &["hooks", "cursor", "before-submit-prompt"],
        Some(&input),
    );
    assert_success(&out, "hooks cursor before-submit-prompt");
}

pub fn cursor_stop(repo: &Path, conversation_id: &str, transcript_path: &str) {
    let input = format!(
        r#"{{"conversation_id":"{conversation_id}","transcript_path":"{transcript_path}"}}"#
    );
    let out = run_cmd(repo, &["hooks", "cursor", "stop"], Some(&input));
    assert_success(&out, "hooks cursor stop");
}

pub fn write_cursor_transcript(path: &Path, prompt: &str, response: &str) {
    let payload = format!(
        r#"{{"type":"user","message":{{"content":[{{"type":"text","text":{prompt_json}}}]}}}}
{{"type":"assistant","message":{{"content":[{{"type":"text","text":{response_json}}}]}}}}
"#,
        prompt_json = serde_json::to_string(prompt).expect("serialize Cursor prompt"),
        response_json = serde_json::to_string(response).expect("serialize Cursor response"),
    );
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create Cursor transcript parent");
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .expect("open Cursor transcript for append");
    file.write_all(payload.as_bytes())
        .expect("append Cursor transcript payload");
}

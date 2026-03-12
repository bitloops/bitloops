mod test_command_support;

use serde_json::Value;
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

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

fn assert_success(output: &Output, context: &str) {
    assert!(
        output.status.success(),
        "{context} failed\nstatus: {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_failure(output: &Output, context: &str) {
    assert!(
        !output.status.success(),
        "{context} unexpectedly succeeded\nstatus: {:?}\nstdout:\n{}\nstderr:\n{}",
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

#[derive(Debug, serde::Deserialize)]
struct RewindPoint {
    id: String,
    #[serde(default)]
    is_logs_only: bool,
}

fn get_rewind_points(repo: &Path) -> Vec<RewindPoint> {
    let out = run_cmd(repo, &["rewind", "--list"], None);
    assert_success(&out, "bitloops rewind --list");
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "failed to parse rewind --list output as JSON: {e}\nstdout:\n{}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

fn rewind_to(repo: &Path, point_id: &str) -> Output {
    run_cmd(repo, &["rewind", "--to", point_id], None)
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

fn git_ref_exists(repo: &Path, reference: &str) -> bool {
    run_git_output(repo, &["show-ref", "--verify", "--quiet", reference])
        .status
        .success()
}

fn git_file_exists_in_ref(repo: &Path, reference: &str, path: &str) -> bool {
    run_git_output(repo, &["cat-file", "-e", &format!("{reference}:{path}")])
        .status
        .success()
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

fn all_checkpoint_ids_from_history(repo: &Path) -> Vec<String> {
    let hashes = run_git(repo, &["log", "--format=%H"]);
    hashes
        .lines()
        .filter_map(|hash| {
            let msg = git_commit_message(repo, hash);
            checkpoint_id_from_message(&msg)
        })
        .collect()
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

fn init_repo(repo: &Path) {
    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "t@t.com"]);
    run_git(repo, &["config", "user.name", "Test"]);
    fs::write(repo.join("README.md"), "init\n").unwrap();
    run_git(repo, &["add", "."]);
    run_git(repo, &["commit", "-m", "initial"]);
}

fn init_and_enable(repo: &Path) {
    let init_out = run_cmd(repo, &["init", "--agent", "claude-code"], None);
    assert_success(&init_out, "bitloops init --agent claude-code");

    let out = run_cmd(repo, &["enable"], None);
    assert_success(&out, "bitloops enable");

    // Keep infrastructure files tracked so stash/pop scenarios do not conflict
    // on .claude/.bitloops untracked paths.
    run_git_expect_success(
        repo,
        &["add", ".claude/settings.json", ".bitloops"],
        "stage enable-generated infra files",
    );
    let commit_out = run_git_output(repo, &["commit", "-m", "seed bitloops infra files"]);
    if !commit_out.status.success() {
        let stderr = String::from_utf8_lossy(&commit_out.stderr);
        assert!(
            stderr.contains("nothing to commit") || stderr.contains("no changes added to commit"),
            "unexpected failure committing infra files\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&commit_out.stdout),
            stderr
        );
    }
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

fn session_end(repo: &Path, session_id: &str, transcript_path: &str) {
    let input = format!(r#"{{"session_id":"{session_id}","transcript_path":"{transcript_path}"}}"#);
    let out = run_cmd(repo, &["hooks", "claude-code", "session-end"], Some(&input));
    assert_success(&out, "hooks claude-code session-end");
}

fn pre_task(repo: &Path, session_id: &str, transcript_path: &str, tool_use_id: &str) {
    let input = format!(
        r#"{{"session_id":"{session_id}","transcript_path":"{transcript_path}","tool_use_id":"{tool_use_id}","tool_input":{{"description":"subagent task","subagent_type":"general-purpose"}}}}"#
    );
    let out = run_cmd(repo, &["hooks", "claude-code", "pre-task"], Some(&input));
    assert_success(&out, "hooks claude-code pre-task");
}

fn post_task(
    repo: &Path,
    session_id: &str,
    transcript_path: &str,
    tool_use_id: &str,
    agent_id: &str,
) {
    let input = format!(
        r#"{{"session_id":"{session_id}","transcript_path":"{transcript_path}","tool_use_id":"{tool_use_id}","tool_input":{{"description":"subagent task","subagent_type":"general-purpose"}},"tool_response":{{"agentId":"{agent_id}"}}}}"#
    );
    let out = run_cmd(repo, &["hooks", "claude-code", "post-task"], Some(&input));
    assert_success(&out, "hooks claude-code post-task");
}

fn write_transcript(path: &Path, prompt: &str, response: &str) {
    let user_uuid = next_transcript_uuid("user");
    let assistant_uuid = next_transcript_uuid("assistant");
    let payload = format!(
        r#"{{"type":"user","uuid":{user_uuid_json},"message":{{"content":[{{"type":"text","text":{prompt_json}}}]}}}}
{{"type":"assistant","uuid":{assistant_uuid_json},"message":{{"content":[{{"type":"text","text":{response_json}}}]}}}}
"#,
        prompt_json = serde_json::to_string(prompt).unwrap(),
        response_json = serde_json::to_string(response).unwrap(),
        user_uuid_json = serde_json::to_string(&user_uuid).unwrap(),
        assistant_uuid_json = serde_json::to_string(&assistant_uuid).unwrap(),
    );
    append_transcript(path, &payload);
}

fn write_transcript_with_tool_use(
    path: &Path,
    prompt: &str,
    response: &str,
    tool_name: &str,
    file_path: &str,
) {
    let user_uuid = next_transcript_uuid("user");
    let assistant_uuid = next_transcript_uuid("assistant");
    let payload = format!(
        r#"{{"type":"user","uuid":{user_uuid_json},"message":{{"content":[{{"type":"text","text":{prompt_json}}}]}}}}
{{"type":"assistant","uuid":{assistant_uuid_json},"message":{{"content":[{{"type":"tool_use","name":{tool_name_json},"input":{{"file_path":{file_path_json}}}}},{{"type":"text","text":{response_json}}}]}}}}
"#,
        prompt_json = serde_json::to_string(prompt).unwrap(),
        response_json = serde_json::to_string(response).unwrap(),
        tool_name_json = serde_json::to_string(tool_name).unwrap(),
        file_path_json = serde_json::to_string(file_path).unwrap(),
        user_uuid_json = serde_json::to_string(&user_uuid).unwrap(),
        assistant_uuid_json = serde_json::to_string(&assistant_uuid).unwrap(),
    );
    append_transcript(path, &payload);
}

fn append_transcript(path: &Path, payload: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .expect("open transcript for append");
    file.write_all(payload.as_bytes())
        .expect("append transcript payload");
}

fn next_transcript_uuid(prefix: &str) -> String {
    static COUNTER: AtomicUsize = AtomicUsize::new(1);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{id}")
}

fn set_strategy(repo: &Path, strategy: &str) {
    let settings_path = repo.join(".bitloops/settings.json");
    let mut settings = read_json(&settings_path);
    settings["strategy"] = Value::String(strategy.to_string());
    fs::write(
        settings_path,
        serde_json::to_string_pretty(&settings).expect("serialize settings"),
    )
    .expect("write settings");
}

fn commit_with_editor_overwrite_message(repo: &Path, message: &str) {
    use std::os::unix::fs::PermissionsExt;
    let script = repo.join("strip-checkpoint-editor.sh");
    let escaped = message.replace('\\', "\\\\").replace('"', "\\\"");
    let body = format!("#!/bin/sh\nprintf \"%s\\n\" \"{escaped}\" > \"$1\"\n");
    fs::write(&script, body).expect("write editor script");
    let mut perms = fs::metadata(&script)
        .expect("editor metadata")
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&script, perms).expect("set script permissions");

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

    let out = Command::new("git")
        .args(["commit"])
        .current_dir(repo)
        .env("PATH", path)
        .env("GIT_EDITOR", &script)
        .output()
        .expect("run git commit with editor");
    assert!(
        out.status.success(),
        "git commit with editor failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn cli_1138_agent_commits_during_turn() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    init_and_enable(dir.path());

    let sid = "cli-1138";
    let transcript_path = dir.path().join("transcript-1138.jsonl");

    user_prompt_submit(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Create first file",
    );
    fs::write(
        dir.path().join("agent_mid_turn.rs"),
        "package main\n\nfunc MidTurn() {}\n",
    )
    .unwrap();
    write_transcript(
        &transcript_path,
        "Create first file",
        "Created agent_mid_turn.rs",
    );

    run_git_expect_success(
        dir.path(),
        &["add", "agent_mid_turn.rs"],
        "git add mid-turn file",
    );
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "agent commit during active turn"],
        "agent commit during turn",
    );

    stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    user_prompt_submit(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Create second file",
    );
    fs::write(
        dir.path().join("user_after_agent.rs"),
        "package main\n\nfunc UserAfterAgent() {}\n",
    )
    .unwrap();
    write_transcript(
        &transcript_path,
        "Create second file",
        "Created user_after_agent.rs",
    );
    stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    run_git_expect_success(
        dir.path(),
        &["add", "user_after_agent.rs"],
        "git add second file",
    );
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "user commit remainder"],
        "user commit remainder",
    );

    let head_msg = git_commit_message(dir.path(), "HEAD");
    assert!(
        checkpoint_id_from_message(&head_msg).is_some(),
        "final user commit should include checkpoint trailer"
    );
}

#[test]
fn cli_1139_agent_commits_mid_turn_user_commits_remainder() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    init_and_enable(dir.path());

    let sid = "cli-1139";
    let transcript_path = dir.path().join("transcript-1139.jsonl");

    user_prompt_submit(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Create three files",
    );
    fs::write(
        dir.path().join("agent_mid1.rs"),
        "package main\n\nfunc Mid1() {}\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("agent_mid2.rs"),
        "package main\n\nfunc Mid2() {}\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("user_remainder.rs"),
        "package main\n\nfunc Remainder() {}\n",
    )
    .unwrap();
    write_transcript(
        &transcript_path,
        "Create three files",
        "Created three files",
    );

    run_git_expect_success(
        dir.path(),
        &["add", "agent_mid1.rs", "agent_mid2.rs"],
        "git add agent files",
    );
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "agent mid-turn commit"],
        "agent mid-turn commit",
    );
    let before = all_checkpoint_ids_from_history(dir.path());

    stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    run_git_expect_success(
        dir.path(),
        &["add", "user_remainder.rs"],
        "git add remainder",
    );
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "user remainder commit"],
        "user remainder commit",
    );

    let after = all_checkpoint_ids_from_history(dir.path());
    assert!(
        after.len() > before.len(),
        "user remainder commit should add a new checkpoint id"
    );
    if let Some(prev) = before.first() {
        assert_ne!(
            after.first().expect("new checkpoint should exist"),
            prev,
            "user remainder checkpoint should differ from agent checkpoint"
        );
    }
}

#[test]
fn cli_1140_auto_commit_strategy() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    init_and_enable(dir.path());
    set_strategy(dir.path(), "auto-commit");

    let commits_before: u64 = run_git(dir.path(), &["rev-list", "--count", "HEAD"])
        .parse()
        .unwrap();

    let sid = "cli-1140";
    let transcript_path = dir.path().join("transcript-1140.jsonl");
    user_prompt_submit(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Create auto-commit file",
    );
    fs::write(
        dir.path().join("auto_commit.rs"),
        "package main\n\nfunc Auto() {}\n",
    )
    .unwrap();
    write_transcript(
        &transcript_path,
        "Create auto-commit file",
        "Created auto_commit.rs",
    );
    stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    let commits_after: u64 = run_git(dir.path(), &["rev-list", "--count", "HEAD"])
        .parse()
        .unwrap();
    assert!(
        commits_after > commits_before,
        "auto-commit strategy should create commits without a manual git commit"
    );
}

#[test]
fn cli_1141_basic_workflow() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    init_and_enable(dir.path());

    let sid = "cli-1141";
    let transcript_path = dir.path().join("transcript-1141.jsonl");
    user_prompt_submit(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Create hello.rs",
    );
    fs::write(
        dir.path().join("hello.rs"),
        "package main\n\nfunc main() {}\n",
    )
    .unwrap();
    write_transcript(&transcript_path, "Create hello.rs", "Created hello.rs");
    stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    run_git_expect_success(dir.path(), &["add", "hello.rs"], "git add hello.rs");
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "add hello.rs"],
        "commit hello.rs",
    );

    let head_msg = git_commit_message(dir.path(), "HEAD");
    assert!(
        checkpoint_id_from_message(&head_msg).is_some(),
        "commit should include checkpoint trailer"
    );
    assert!(
        git_ref_exists(dir.path(), "refs/heads/bitloops/checkpoints/v1"),
        "checkpoints branch should exist"
    );
}

#[test]
fn cli_1142_checkpoint_id_format() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    init_and_enable(dir.path());

    let sid = "cli-1142";
    let transcript_path = dir.path().join("transcript-1142.jsonl");
    user_prompt_submit(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Create id format file",
    );
    fs::write(
        dir.path().join("id_format.rs"),
        "package main\n\nfunc IDFormat() {}\n",
    )
    .unwrap();
    write_transcript(
        &transcript_path,
        "Create id format file",
        "Created id_format.rs",
    );
    stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    run_git_expect_success(dir.path(), &["add", "id_format.rs"], "git add id_format.rs");
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "add id format file"],
        "commit id format file",
    );
    let id = checkpoint_id_from_message(&git_commit_message(dir.path(), "HEAD"))
        .expect("checkpoint trailer should exist");
    assert_eq!(id.len(), 12, "checkpoint id should be 12 chars");
    assert!(
        id.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "checkpoint id should be lowercase hex"
    );
}

#[test]
fn cli_1143_checkpoint_metadata() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    init_and_enable(dir.path());

    let sid = "cli-1143";
    let transcript_path = dir.path().join("transcript-1143.jsonl");
    user_prompt_submit(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Create metadata file",
    );
    fs::write(
        dir.path().join("metadata_target.rs"),
        "package main\n\nfunc Metadata() {}\n",
    )
    .unwrap();
    write_transcript(
        &transcript_path,
        "Create metadata file",
        "Created metadata_target.rs",
    );
    stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    run_git_expect_success(
        dir.path(),
        &["add", "metadata_target.rs"],
        "git add metadata target",
    );
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "add metadata target"],
        "commit metadata target",
    );

    let checkpoint_id = checkpoint_id_from_message(&git_commit_message(dir.path(), "HEAD"))
        .expect("checkpoint trailer should exist");
    let (a, b) = checkpoint_shard(&checkpoint_id);
    let cp_ref = "bitloops/checkpoints/v1";

    assert!(
        git_ref_exists(dir.path(), "refs/heads/bitloops/checkpoints/v1"),
        "checkpoints branch should exist"
    );
    assert!(
        git_file_exists_in_ref(dir.path(), cp_ref, &format!("{a}/{b}/metadata.json")),
        "top-level metadata should exist"
    );
    assert!(
        git_file_exists_in_ref(dir.path(), cp_ref, &format!("{a}/{b}/0/metadata.json")),
        "session metadata should exist"
    );
}

#[test]
fn cli_1144_content_aware_overlap_revert_and_replace() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    init_and_enable(dir.path());

    let sid = "cli-1144";
    let transcript_path = dir.path().join("transcript-1144.jsonl");
    user_prompt_submit(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Create overlap file",
    );
    fs::write(
        dir.path().join("overlap.rs"),
        "package main\n\nfunc FromAgent() string { return \"agent\" }\n",
    )
    .unwrap();
    write_transcript(
        &transcript_path,
        "Create overlap file",
        "Created overlap.rs",
    );
    stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    fs::write(
        dir.path().join("overlap.rs"),
        "package main\n\nfunc FromUser() string { return \"user\" }\n",
    )
    .unwrap();
    run_git_expect_success(dir.path(), &["add", "overlap.rs"], "git add overlap.rs");
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "replace overlap content"],
        "commit replaced overlap file",
    );

    let head_msg = git_commit_message(dir.path(), "HEAD");
    assert!(
        checkpoint_id_from_message(&head_msg).is_none(),
        "content-aware overlap should skip trailer when user replaces new file content"
    );
}

#[test]
fn cli_1145_deleted_files_commit_deletion() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    init_and_enable(dir.path());

    fs::write(
        dir.path().join("to_delete.rs"),
        "package main\n\nfunc ToDelete() {}\n",
    )
    .unwrap();
    run_git_expect_success(dir.path(), &["add", "to_delete.rs"], "stage to_delete.rs");
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "seed to_delete.rs"],
        "seed commit",
    );

    let sid = "cli-1145";
    let transcript_path = dir.path().join("transcript-1145.jsonl");
    user_prompt_submit(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Delete old file and add replacement",
    );
    fs::remove_file(dir.path().join("to_delete.rs")).unwrap();
    fs::write(
        dir.path().join("replacement.rs"),
        "package main\n\nfunc Replacement() {}\n",
    )
    .unwrap();
    write_transcript(
        &transcript_path,
        "Delete old file and add replacement",
        "Deleted to_delete.rs and created replacement.rs",
    );
    stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    run_git_expect_success(
        dir.path(),
        &["add", "replacement.rs"],
        "stage replacement.rs",
    );
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "add replacement"],
        "commit replacement",
    );
    let first = checkpoint_id_from_message(&git_commit_message(dir.path(), "HEAD"));
    assert!(
        first.is_some(),
        "replacement commit should carry checkpoint"
    );

    run_git_expect_success(
        dir.path(),
        &["add", "-u", "to_delete.rs"],
        "stage deletion of to_delete.rs",
    );
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "remove to_delete.rs"],
        "commit deletion",
    );

    let all = all_checkpoint_ids_from_history(dir.path());
    assert!(
        !all.is_empty(),
        "at least one checkpoint should exist after replacement/deletion flow"
    );
}

#[test]
fn cli_1146_ended_session_user_commits_after_exit() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    init_and_enable(dir.path());

    let sid = "cli-1146";
    let transcript_path = dir.path().join("transcript-1146.jsonl");
    user_prompt_submit(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Create ended session files",
    );
    fs::write(
        dir.path().join("ended_a.rs"),
        "package main\n\nfunc EndedA() {}\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("ended_b.rs"),
        "package main\n\nfunc EndedB() {}\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("ended_c.rs"),
        "package main\n\nfunc EndedC() {}\n",
    )
    .unwrap();
    write_transcript(
        &transcript_path,
        "Create ended session files",
        "Created ended_a.rs ended_b.rs ended_c.rs",
    );
    stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());
    session_end(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    run_git_expect_success(
        dir.path(),
        &["add", "ended_a.rs", "ended_b.rs"],
        "stage ended A/B",
    );
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "commit ended A/B"],
        "commit ended A/B",
    );
    let cp_ab =
        checkpoint_id_from_message(&git_commit_message(dir.path(), "HEAD")).expect("checkpoint AB");

    run_git_expect_success(dir.path(), &["add", "ended_c.rs"], "stage ended C");
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "commit ended C"],
        "commit ended C",
    );
    let cp_c =
        checkpoint_id_from_message(&git_commit_message(dir.path(), "HEAD")).expect("checkpoint C");

    assert_ne!(
        cp_ab, cp_c,
        "separate commits should have unique checkpoints"
    );
}

#[test]
fn cli_1147_existing_files_modify_and_commit() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    init_and_enable(dir.path());

    fs::write(
        dir.path().join("config.rs"),
        "package main\n\nvar Config = map[string]string{\"version\":\"1.0\"}\n",
    )
    .unwrap();
    run_git_expect_success(dir.path(), &["add", "config.rs"], "stage config.rs seed");
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "seed config.rs"],
        "seed config.rs",
    );

    let sid = "cli-1147-modify";
    let transcript_path = dir.path().join("transcript-1147-modify.jsonl");
    user_prompt_submit(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Modify existing config",
    );
    fs::write(
        dir.path().join("config.rs"),
        "package main\n\nvar Config = map[string]string{\"version\":\"1.0\",\"debug\":\"true\"}\n",
    )
    .unwrap();
    write_transcript(
        &transcript_path,
        "Modify existing config",
        "Updated config.rs",
    );
    stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    run_git_expect_success(
        dir.path(),
        &["add", "config.rs"],
        "stage modified config.rs",
    );
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "modify config.rs"],
        "commit modified config.rs",
    );
    assert!(
        checkpoint_id_from_message(&git_commit_message(dir.path(), "HEAD")).is_some(),
        "existing-file modification should produce checkpoint trailer"
    );
}

#[test]
fn cli_1147_existing_files_stash_modifications() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    init_and_enable(dir.path());

    fs::write(
        dir.path().join("file_a.rs"),
        "package main\n\nfunc A() { /* original */ }\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("file_b.rs"),
        "package main\n\nfunc B() { /* original */ }\n",
    )
    .unwrap();
    run_git_expect_success(
        dir.path(),
        &["add", "file_a.rs", "file_b.rs"],
        "stage seed files",
    );
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "seed file_a and file_b"],
        "seed commit files",
    );

    let sid = "cli-1147-stash";
    let transcript_path = dir.path().join("transcript-1147-stash.jsonl");
    user_prompt_submit(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Modify existing files A/B",
    );
    fs::write(
        dir.path().join("file_a.rs"),
        "package main\n\nfunc A() { /* modified by agent */ }\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("file_b.rs"),
        "package main\n\nfunc B() { /* modified by agent */ }\n",
    )
    .unwrap();
    write_transcript(
        &transcript_path,
        "Modify existing files A/B",
        "Modified file_a.rs and file_b.rs",
    );
    stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    run_git_expect_success(dir.path(), &["add", "file_a.rs"], "stage file_a.rs");
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "commit file_a.rs"],
        "commit file_a.rs",
    );
    let cp_a =
        checkpoint_id_from_message(&git_commit_message(dir.path(), "HEAD")).expect("checkpoint A");

    run_git_expect_success(
        dir.path(),
        &["stash", "push", "-u", "-m", "stash file_b"],
        "stash file_b.rs",
    );
    assert!(
        fs::read_to_string(dir.path().join("file_b.rs"))
            .unwrap()
            .contains("original"),
        "file_b.rs should be reverted after stash"
    );

    run_git_expect_success(dir.path(), &["stash", "pop"], "stash pop file_b.rs");
    run_git_expect_success(dir.path(), &["add", "file_b.rs"], "stage file_b.rs");
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "commit file_b.rs"],
        "commit file_b.rs",
    );
    let cp_b =
        checkpoint_id_from_message(&git_commit_message(dir.path(), "HEAD")).expect("checkpoint B");
    assert_ne!(cp_a, cp_b, "split commits should carry unique checkpoints");
}

#[test]
fn cli_1147_existing_files_split_commits() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    init_and_enable(dir.path());

    fs::write(
        dir.path().join("model.rs"),
        "package main\n\ntype Model struct{}\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("view.rs"),
        "package main\n\ntype View struct{}\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("controller.rs"),
        "package main\n\ntype Controller struct{}\n",
    )
    .unwrap();
    run_git_expect_success(
        dir.path(),
        &["add", "model.rs", "view.rs", "controller.rs"],
        "stage seed mvc files",
    );
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "seed mvc files"],
        "seed mvc files",
    );

    let sid = "cli-1147-split";
    let transcript_path = dir.path().join("transcript-1147-split.jsonl");
    user_prompt_submit(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Modify model/view/controller",
    );
    fs::write(
        dir.path().join("model.rs"),
        "package main\n\ntype Model struct{ Name string }\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("view.rs"),
        "package main\n\ntype View struct{ Name string }\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("controller.rs"),
        "package main\n\ntype Controller struct{ Name string }\n",
    )
    .unwrap();
    write_transcript(
        &transcript_path,
        "Modify model/view/controller",
        "Updated all MVC files",
    );
    stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    run_git_expect_success(dir.path(), &["add", "model.rs"], "stage model.rs");
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "commit model"],
        "commit model.rs",
    );
    let cp_model = checkpoint_id_from_message(&git_commit_message(dir.path(), "HEAD"))
        .expect("model checkpoint");

    run_git_expect_success(dir.path(), &["add", "view.rs"], "stage view.rs");
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "commit view"],
        "commit view.rs",
    );
    let cp_view = checkpoint_id_from_message(&git_commit_message(dir.path(), "HEAD"))
        .expect("view checkpoint");

    run_git_expect_success(dir.path(), &["add", "controller.rs"], "stage controller.rs");
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "commit controller"],
        "commit controller.rs",
    );
    let cp_controller = checkpoint_id_from_message(&git_commit_message(dir.path(), "HEAD"))
        .expect("controller checkpoint");

    assert_ne!(cp_model, cp_view);
    assert_ne!(cp_view, cp_controller);
    assert_ne!(cp_model, cp_controller);
}

#[test]
fn cli_1147_existing_files_revert_modification() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    init_and_enable(dir.path());

    fs::write(
        dir.path().join("calc.rs"),
        "package main\n\n// placeholder\n",
    )
    .unwrap();
    run_git_expect_success(dir.path(), &["add", "calc.rs"], "stage calc.rs seed");
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "seed calc.rs"],
        "seed calc.rs",
    );

    let sid = "cli-1147-revert";
    let transcript_path = dir.path().join("transcript-1147-revert.jsonl");
    user_prompt_submit(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Modify calc.rs",
    );
    fs::write(
        dir.path().join("calc.rs"),
        "package main\n\nfunc AgentMultiply(a, b int) int { return a * b }\n",
    )
    .unwrap();
    write_transcript(&transcript_path, "Modify calc.rs", "Added AgentMultiply");
    stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    fs::write(
        dir.path().join("calc.rs"),
        "package main\n\nfunc UserAdd(a, b int) int { return a + b }\n",
    )
    .unwrap();
    run_git_expect_success(dir.path(), &["add", "calc.rs"], "stage user calc.rs");
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "user replace calc.rs"],
        "commit user replace calc.rs",
    );

    assert!(
        checkpoint_id_from_message(&git_commit_message(dir.path(), "HEAD")).is_some(),
        "modified tracked file should still count as overlap and keep checkpoint"
    );
}

#[test]
fn cli_1147_existing_files_mixed_new_and_modified() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    init_and_enable(dir.path());

    fs::write(
        dir.path().join("main.rs"),
        "package main\n\nfunc main() {\n}\n",
    )
    .unwrap();
    run_git_expect_success(dir.path(), &["add", "main.rs"], "stage main.rs seed");
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "seed main.rs"],
        "seed main.rs",
    );

    let sid = "cli-1147-mixed";
    let transcript_path = dir.path().join("transcript-1147-mixed.jsonl");
    user_prompt_submit(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Modify main.rs and add utils/types",
    );
    fs::write(
        dir.path().join("main.rs"),
        "package main\n\n// imports utils and types\nfunc main() {\n}\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("utils.rs"),
        "package main\n\nfunc Helper() string { return \"helper\" }\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("types.rs"),
        "package main\n\ntype Config struct { Name string }\n",
    )
    .unwrap();
    write_transcript(
        &transcript_path,
        "Modify main.rs and add utils/types",
        "Modified main.rs and created utils.rs/types.rs",
    );
    stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    run_git_expect_success(dir.path(), &["add", "main.rs"], "stage main.rs");
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "commit main.rs change"],
        "commit main.rs change",
    );
    let cp_main = checkpoint_id_from_message(&git_commit_message(dir.path(), "HEAD"))
        .expect("checkpoint main change");

    run_git_expect_success(
        dir.path(),
        &["add", "utils.rs", "types.rs"],
        "stage new utils/types",
    );
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "commit utils/types"],
        "commit utils/types",
    );
    let cp_new = checkpoint_id_from_message(&git_commit_message(dir.path(), "HEAD"))
        .expect("checkpoint new files");

    assert_ne!(
        cp_main, cp_new,
        "different commits should use different checkpoints"
    );
}

#[test]
fn cli_1148_multiple_agent_sessions() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    init_and_enable(dir.path());

    let transcript_path = dir.path().join("transcript-1148.jsonl");

    user_prompt_submit(
        dir.path(),
        "cli-1148-s1",
        transcript_path.to_string_lossy().as_ref(),
        "Session 1 file",
    );
    fs::write(
        dir.path().join("session1.rs"),
        "package main\n\nfunc Session1() {}\n",
    )
    .unwrap();
    write_transcript(&transcript_path, "Session 1 file", "Created session1.rs");
    stop(
        dir.path(),
        "cli-1148-s1",
        transcript_path.to_string_lossy().as_ref(),
    );
    run_git_expect_success(dir.path(), &["add", "session1.rs"], "stage session1.rs");
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "commit session 1"],
        "commit session 1",
    );

    user_prompt_submit(
        dir.path(),
        "cli-1148-s2",
        transcript_path.to_string_lossy().as_ref(),
        "Session 2 file",
    );
    fs::write(
        dir.path().join("session2.rs"),
        "package main\n\nfunc Session2() {}\n",
    )
    .unwrap();
    write_transcript(&transcript_path, "Session 2 file", "Created session2.rs");
    stop(
        dir.path(),
        "cli-1148-s2",
        transcript_path.to_string_lossy().as_ref(),
    );
    run_git_expect_success(dir.path(), &["add", "session2.rs"], "stage session2.rs");
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "commit session 2"],
        "commit session 2",
    );

    let ids = all_checkpoint_ids_from_history(dir.path());
    assert!(ids.len() >= 2, "should have at least two checkpoints");
    assert_ne!(
        ids[0], ids[1],
        "sessions should produce distinct checkpoint ids"
    );
}

#[test]
fn cli_1149_multiple_changes() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    init_and_enable(dir.path());

    let sid = "cli-1149";
    let transcript_path = dir.path().join("transcript-1149.jsonl");
    user_prompt_submit(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Create multiple files",
    );
    fs::write(
        dir.path().join("hello.rs"),
        "package main\n\nfunc Hello() {}\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("calc.rs"),
        "package main\n\nfunc Add(a, b int) int { return a+b }\n",
    )
    .unwrap();
    write_transcript(
        &transcript_path,
        "Create multiple files",
        "Created hello.rs and calc.rs",
    );
    stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    run_git_expect_success(
        dir.path(),
        &["add", "hello.rs", "calc.rs"],
        "stage multi-change files",
    );
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "commit multi-change files"],
        "commit multi-change files",
    );
    assert!(
        checkpoint_id_from_message(&git_commit_message(dir.path(), "HEAD")).is_some(),
        "multi-change commit should carry checkpoint trailer"
    );
}

#[test]
fn cli_1150_rewind_after_commit() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    init_and_enable(dir.path());

    let sid = "cli-1150";
    let transcript_dir = tempfile::tempdir().unwrap();
    let transcript_path = transcript_dir.path().join("transcript-1150.jsonl");
    user_prompt_submit(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Create rewind_after_commit.rs",
    );
    fs::write(
        dir.path().join("rewind_after_commit.rs"),
        "package main\n\nfunc RewindAfterCommit() {}\n",
    )
    .unwrap();
    write_transcript(
        &transcript_path,
        "Create rewind_after_commit.rs",
        "Created rewind_after_commit.rs",
    );
    stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    let points_before = get_rewind_points(dir.path());
    assert!(
        !points_before.is_empty(),
        "should have at least one rewind point before commit"
    );
    let pre_commit_point_id = points_before[0].id.clone();
    assert!(
        !points_before[0].is_logs_only,
        "pre-commit rewind point should not be logs-only"
    );

    // Commit after first checkpoint; this should create logs-only rewind points.
    run_git_expect_success(
        dir.path(),
        &["add", "rewind_after_commit.rs"],
        "stage rewind_after_commit.rs",
    );
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "commit rewind after commit test"],
        "commit rewind after commit test",
    );

    let latest_checkpoint_id = checkpoint_id_from_message(&git_commit_message(dir.path(), "HEAD"))
        .expect("latest commit should include a checkpoint trailer");
    assert!(
        !latest_checkpoint_id.is_empty(),
        "committed checkpoint id should be present"
    );

    let points_after = get_rewind_points(dir.path());
    let logs_only_point = points_after
        .iter()
        .find(|p| p.is_logs_only)
        .unwrap_or_else(|| {
            panic!("expected at least one logs-only point after commit; points={points_after:#?}")
        });
    assert_ne!(
        pre_commit_point_id, logs_only_point.id,
        "logs-only point id should differ from pre-commit shadow-branch point id"
    );

    let rewind_old = rewind_to(dir.path(), &pre_commit_point_id);
    assert_failure(
        &rewind_old,
        "rewind to deleted pre-commit shadow branch point should fail",
    );
    let stderr = String::from_utf8_lossy(&rewind_old.stderr);
    assert!(
        stderr.contains("not found"),
        "rewind to deleted pre-commit point should report not found\nstderr:\n{stderr}"
    );
}

#[test]
fn cli_1151_resume_in_relocated_repo() {
    let base = tempfile::tempdir().unwrap();
    let original_repo = base.path().join("original");
    fs::create_dir_all(&original_repo).unwrap();
    init_repo(&original_repo);
    init_and_enable(&original_repo);
    run_git_expect_success(
        &original_repo,
        &["checkout", "-b", "feature/e2e-test"],
        "create feature/e2e-test",
    );

    let sid = "cli-1151";
    let transcript_path = original_repo.join("transcript-1151.jsonl");
    user_prompt_submit(
        &original_repo,
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Create resume_relocated.rs",
    );
    fs::write(
        original_repo.join("resume_relocated.rs"),
        "package main\n\nfunc ResumeRelocated() {}\n",
    )
    .unwrap();
    write_transcript(
        &transcript_path,
        "Create resume_relocated.rs",
        "Created resume_relocated.rs",
    );
    stop(
        &original_repo,
        sid,
        transcript_path.to_string_lossy().as_ref(),
    );
    run_git_expect_success(
        &original_repo,
        &["add", "resume_relocated.rs"],
        "stage relocated resume file",
    );
    run_git_expect_success(
        &original_repo,
        &["commit", "-m", "commit relocated resume file"],
        "commit relocated resume file",
    );

    let relocated_root = base.path().join("relocated").join("repo");
    fs::create_dir_all(relocated_root.parent().unwrap()).unwrap();
    fs::rename(&original_repo, &relocated_root).unwrap();

    let out = run_cmd(
        &relocated_root,
        &["resume", "feature/e2e-test", "--force"],
        None,
    );
    assert_success(&out, "bitloops resume in relocated repository");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Restored session log from checkpoint metadata")
            || stdout.contains("Switched to branch"),
        "resume should restore from checkpoint metadata or report branch switch\nstdout:\n{}\nstderr:\n{}",
        stdout,
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn cli_1152_rewind_multiple_files() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    init_and_enable(dir.path());

    let sid = "cli-1152";
    let transcript_dir = tempfile::tempdir().unwrap();
    let transcript_path = transcript_dir.path().join("transcript-1152.jsonl");
    user_prompt_submit(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Create first file for rewind",
    );
    fs::write(
        dir.path().join("hello.rs"),
        "package main\n\nfunc Hello() {}\n",
    )
    .unwrap();
    write_transcript(
        &transcript_path,
        "Create first file for rewind",
        "Created hello.rs",
    );
    stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    let points_after_first = get_rewind_points(dir.path());
    assert!(
        !points_after_first.is_empty(),
        "should have rewind point after first file"
    );
    let after_first_file = points_after_first[0].id.clone();

    user_prompt_submit(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Create second file for rewind",
    );
    fs::write(
        dir.path().join("calc.rs"),
        "package main\n\nfunc Calc() {}\n",
    )
    .unwrap();
    write_transcript(
        &transcript_path,
        "Create second file for rewind",
        "Created calc.rs",
    );
    stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    assert!(
        dir.path().join("hello.rs").exists(),
        "hello.rs should exist"
    );
    assert!(dir.path().join("calc.rs").exists(), "calc.rs should exist");

    let rewind_out = rewind_to(dir.path(), &after_first_file);
    assert_success(
        &rewind_out,
        "rewind to first-file checkpoint should succeed",
    );

    assert!(
        dir.path().join("hello.rs").exists(),
        "hello.rs should remain after rewind"
    );
    assert!(
        !dir.path().join("calc.rs").exists(),
        "calc.rs should be removed by rewind"
    );
}

#[test]
fn cli_1153_rewind_to_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    init_and_enable(dir.path());

    let sid = "cli-1153";
    let transcript_dir = tempfile::tempdir().unwrap();
    let transcript_path = transcript_dir.path().join("transcript-1153.jsonl");
    user_prompt_submit(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Create hello.rs",
    );
    fs::write(
        dir.path().join("hello.rs"),
        "package main\n\nfunc main() {\n\tprintln(\"Hello, world!\")\n}\n",
    )
    .unwrap();
    write_transcript_with_tool_use(
        &transcript_path,
        "Create hello.rs",
        "Created hello.rs",
        "Write",
        "hello.rs",
    );
    stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    let points1 = get_rewind_points(dir.path());
    assert!(
        !points1.is_empty(),
        "should have at least one rewind point after first checkpoint"
    );
    let first_point_id = points1[0].id.clone();
    let original_content =
        fs::read_to_string(dir.path().join("hello.rs")).expect("read original hello.rs");

    user_prompt_submit(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Modify hello.rs",
    );
    fs::write(
        dir.path().join("hello.rs"),
        "package main\n\nfunc main() {\n\tprintln(\"E2E Test\")\n}\n",
    )
    .unwrap();
    write_transcript_with_tool_use(
        &transcript_path,
        "Modify hello.rs",
        "Modified hello.rs",
        "Edit",
        "hello.rs",
    );
    stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    let modified_content =
        fs::read_to_string(dir.path().join("hello.rs")).expect("read modified hello.rs");
    assert_ne!(
        original_content, modified_content,
        "hello.rs content should change after modification"
    );
    assert!(
        modified_content.contains("E2E Test"),
        "modified content should contain E2E marker"
    );

    let points2 = get_rewind_points(dir.path());
    assert!(
        points2.len() >= 2,
        "should have at least 2 rewind points after second checkpoint"
    );

    let rewind_out = rewind_to(dir.path(), &first_point_id);
    assert_success(&rewind_out, "rewind to first checkpoint should succeed");

    let restored_content =
        fs::read_to_string(dir.path().join("hello.rs")).expect("read restored hello.rs");
    assert_eq!(
        original_content, restored_content,
        "hello.rs should be restored to first-checkpoint content"
    );
    assert!(
        !restored_content.contains("E2E Test"),
        "restored content should not include modified marker"
    );
}

#[test]
fn cli_1154_scenario1_basic_flow() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    init_and_enable(dir.path());

    let sid = "cli-1154";
    let transcript_path = dir.path().join("transcript-1154.jsonl");
    user_prompt_submit(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Create scenario1.rs",
    );
    fs::write(
        dir.path().join("scenario1.rs"),
        "package main\n\nfunc Scenario1() {}\n",
    )
    .unwrap();
    write_transcript(
        &transcript_path,
        "Create scenario1.rs",
        "Created scenario1.rs",
    );
    stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    run_git_expect_success(dir.path(), &["add", "scenario1.rs"], "stage scenario1.rs");
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "commit scenario1.rs"],
        "commit scenario1.rs",
    );
    let checkpoint = checkpoint_id_from_message(&git_commit_message(dir.path(), "HEAD"));
    assert!(
        checkpoint.is_some(),
        "scenario1 basic flow should produce a checkpoint trailer"
    );
}

#[test]
fn cli_1155_scenario2_agent_commits_during_turn() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    init_and_enable(dir.path());

    let sid = "cli-1155";
    let transcript_path = dir.path().join("transcript-1155.jsonl");
    user_prompt_submit(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Create and commit agent_commit.rs",
    );
    fs::write(
        dir.path().join("agent_commit.rs"),
        "package main\n\nfunc AgentCommit() {}\n",
    )
    .unwrap();
    write_transcript(
        &transcript_path,
        "Create and commit agent_commit.rs",
        "Created agent_commit.rs",
    );
    run_git_expect_success(
        dir.path(),
        &["add", "agent_commit.rs"],
        "stage agent_commit.rs",
    );
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "agent commits during turn"],
        "commit agent_commit.rs mid-turn",
    );
    stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    let ids = all_checkpoint_ids_from_history(dir.path());
    assert!(
        !ids.is_empty(),
        "scenario2 should produce at least one checkpoint when agent commits during turn"
    );
}

#[test]
fn cli_1156_scenario3_multiple_granular_commits() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    init_and_enable(dir.path());

    let sid = "cli-1156";
    let transcript_path = dir.path().join("transcript-1156.jsonl");
    user_prompt_submit(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Create three files and commit each",
    );
    write_transcript(
        &transcript_path,
        "Create three files and commit each",
        "Created file1.rs file2.rs file3.rs",
    );

    fs::write(
        dir.path().join("file1.rs"),
        "package main\n\nfunc One() int { return 1 }\n",
    )
    .unwrap();
    run_git_expect_success(dir.path(), &["add", "file1.rs"], "stage file1.rs");
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "add file1.rs"],
        "commit file1.rs",
    );

    fs::write(
        dir.path().join("file2.rs"),
        "package main\n\nfunc Two() int { return 2 }\n",
    )
    .unwrap();
    run_git_expect_success(dir.path(), &["add", "file2.rs"], "stage file2.rs");
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "add file2.rs"],
        "commit file2.rs",
    );

    fs::write(
        dir.path().join("file3.rs"),
        "package main\n\nfunc Three() int { return 3 }\n",
    )
    .unwrap();
    run_git_expect_success(dir.path(), &["add", "file3.rs"], "stage file3.rs");
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "add file3.rs"],
        "commit file3.rs",
    );
    stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    let ids = all_checkpoint_ids_from_history(dir.path());
    assert!(
        ids.len() >= 3,
        "granular commits should produce at least three checkpoint trailers"
    );
    assert_ne!(ids[0], ids[1]);
    assert_ne!(ids[1], ids[2]);
}

#[test]
fn cli_1157_scenario4_user_splits_commits() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    init_and_enable(dir.path());

    let sid = "cli-1157";
    let transcript_path = dir.path().join("transcript-1157.jsonl");
    user_prompt_submit(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Create files A/B/C/D",
    );
    fs::write(
        dir.path().join("file_a.rs"),
        "package main\n\nfunc A() string { return \"A\" }\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("file_b.rs"),
        "package main\n\nfunc B() string { return \"B\" }\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("file_c.rs"),
        "package main\n\nfunc C() string { return \"C\" }\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("file_d.rs"),
        "package main\n\nfunc D() string { return \"D\" }\n",
    )
    .unwrap();
    write_transcript(
        &transcript_path,
        "Create files A/B/C/D",
        "Created file_a.rs file_b.rs file_c.rs file_d.rs",
    );
    stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    run_git_expect_success(
        dir.path(),
        &["add", "file_a.rs", "file_b.rs"],
        "stage file_a.rs/file_b.rs",
    );
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "commit files A/B"],
        "commit file_a.rs/file_b.rs",
    );
    let cp_ab =
        checkpoint_id_from_message(&git_commit_message(dir.path(), "HEAD")).expect("checkpoint AB");

    run_git_expect_success(
        dir.path(),
        &["add", "file_c.rs", "file_d.rs"],
        "stage file_c.rs/file_d.rs",
    );
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "commit files C/D"],
        "commit file_c.rs/file_d.rs",
    );
    let cp_cd =
        checkpoint_id_from_message(&git_commit_message(dir.path(), "HEAD")).expect("checkpoint CD");

    assert_ne!(
        cp_ab, cp_cd,
        "split user commits should have distinct checkpoints"
    );
}

#[test]
fn cli_1158_scenario5_partial_commit_stash_next_prompt() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    init_and_enable(dir.path());

    let transcript_path = dir.path().join("transcript-1158.jsonl");

    user_prompt_submit(
        dir.path(),
        "cli-1158-p1",
        transcript_path.to_string_lossy().as_ref(),
        "Create stash_a/b/c",
    );
    fs::write(
        dir.path().join("stash_a.rs"),
        "package main\n\nfunc StashA() {}\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("stash_b.rs"),
        "package main\n\nfunc StashB() {}\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("stash_c.rs"),
        "package main\n\nfunc StashC() {}\n",
    )
    .unwrap();
    write_transcript(
        &transcript_path,
        "Create stash_a/b/c",
        "Created stash_a.rs stash_b.rs stash_c.rs",
    );
    stop(
        dir.path(),
        "cli-1158-p1",
        transcript_path.to_string_lossy().as_ref(),
    );

    run_git_expect_success(dir.path(), &["add", "stash_a.rs"], "stage stash_a.rs");
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "commit stash_a.rs"],
        "commit stash_a.rs",
    );

    run_git_expect_success(
        dir.path(),
        &["stash", "push", "-u", "-m", "stash b and c"],
        "stash stash_b/stash_c",
    );
    assert!(!dir.path().join("stash_b.rs").exists());
    assert!(!dir.path().join("stash_c.rs").exists());

    user_prompt_submit(
        dir.path(),
        "cli-1158-p2",
        transcript_path.to_string_lossy().as_ref(),
        "Create stash_d/e",
    );
    fs::write(
        dir.path().join("stash_d.rs"),
        "package main\n\nfunc StashD() {}\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("stash_e.rs"),
        "package main\n\nfunc StashE() {}\n",
    )
    .unwrap();
    write_transcript(
        &transcript_path,
        "Create stash_d/e",
        "Created stash_d.rs stash_e.rs",
    );
    stop(
        dir.path(),
        "cli-1158-p2",
        transcript_path.to_string_lossy().as_ref(),
    );

    run_git_expect_success(
        dir.path(),
        &["add", "stash_d.rs", "stash_e.rs"],
        "stage stash_d/stash_e",
    );
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "commit stash_d.rs and stash_e.rs"],
        "commit stash_d/stash_e",
    );

    let ids = all_checkpoint_ids_from_history(dir.path());
    assert!(
        ids.len() >= 2,
        "scenario5 should produce at least two checkpoints"
    );
}

#[test]
fn cli_1159_scenario6_stash_second_prompt_unstash_commit_all() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    init_and_enable(dir.path());

    let transcript_path_p1 = dir.path().join("transcript-1159-p1.jsonl");
    let transcript_path_p2 = dir.path().join("transcript-1159-p2.jsonl");

    user_prompt_submit(
        dir.path(),
        "cli-1159-p1",
        transcript_path_p1.to_string_lossy().as_ref(),
        "Create combo_a/b/c",
    );
    fs::write(
        dir.path().join("combo_a.rs"),
        "package main\n\nfunc ComboA() {}\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("combo_b.rs"),
        "package main\n\nfunc ComboB() {}\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("combo_c.rs"),
        "package main\n\nfunc ComboC() {}\n",
    )
    .unwrap();
    write_transcript(
        &transcript_path_p1,
        "Create combo_a/b/c",
        "Created combo_a.rs combo_b.rs combo_c.rs",
    );
    stop(
        dir.path(),
        "cli-1159-p1",
        transcript_path_p1.to_string_lossy().as_ref(),
    );

    run_git_expect_success(dir.path(), &["add", "combo_a.rs"], "stage combo_a.rs");
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "commit combo_a.rs"],
        "commit combo_a.rs",
    );

    run_git_expect_success(
        dir.path(),
        &["stash", "push", "-u", "-m", "stash combo_b and combo_c"],
        "stash combo_b/combo_c",
    );

    user_prompt_submit(
        dir.path(),
        "cli-1159-p2",
        transcript_path_p2.to_string_lossy().as_ref(),
        "Create combo_d/e",
    );
    fs::write(
        dir.path().join("combo_d.rs"),
        "package main\n\nfunc ComboD() {}\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("combo_e.rs"),
        "package main\n\nfunc ComboE() {}\n",
    )
    .unwrap();
    write_transcript(
        &transcript_path_p2,
        "Create combo_d/e",
        "Created combo_d.rs combo_e.rs",
    );
    stop(
        dir.path(),
        "cli-1159-p2",
        transcript_path_p2.to_string_lossy().as_ref(),
    );

    run_git_expect_success(dir.path(), &["stash", "pop"], "stash pop combo_b/combo_c");
    run_git_expect_success(
        dir.path(),
        &[
            "add",
            "combo_b.rs",
            "combo_c.rs",
            "combo_d.rs",
            "combo_e.rs",
        ],
        "stage combo_b/c/d/e",
    );
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "commit combo_b/c/d/e"],
        "commit combo_b/c/d/e",
    );

    let ids = all_checkpoint_ids_from_history(dir.path());
    assert!(
        ids.len() >= 2,
        "scenario6 should produce at least two checkpoints"
    );
}

#[test]
fn cli_1160_scenario7_partial_staging_simulated() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    init_and_enable(dir.path());

    fs::write(
        dir.path().join("partial.rs"),
        "package main\n\n// placeholder\n",
    )
    .unwrap();
    run_git_expect_success(dir.path(), &["add", "partial.rs"], "stage seed partial.rs");
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "seed partial.rs"],
        "seed partial.rs",
    );

    let sid = "cli-1160";
    let transcript_path = dir.path().join("transcript-1160.jsonl");
    user_prompt_submit(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Expand partial.rs",
    );
    let full_content = "package main\n\nfunc First() int { return 1 }\n\nfunc Second() int { return 2 }\n\nfunc Third() int { return 3 }\n\nfunc Fourth() int { return 4 }\n";
    fs::write(dir.path().join("partial.rs"), full_content).unwrap();
    write_transcript(
        &transcript_path,
        "Expand partial.rs",
        "Added First/Second/Third/Fourth to partial.rs",
    );
    stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    let partial_content =
        "package main\n\nfunc First() int { return 1 }\n\nfunc Second() int { return 2 }\n";
    fs::write(dir.path().join("partial.rs"), partial_content).unwrap();
    run_git_expect_success(dir.path(), &["add", "partial.rs"], "stage partial subset");
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "commit partial content"],
        "commit partial content",
    );
    let cp_first = checkpoint_id_from_message(&git_commit_message(dir.path(), "HEAD"))
        .expect("checkpoint for partial content");

    fs::write(dir.path().join("partial.rs"), full_content).unwrap();
    run_git_expect_success(dir.path(), &["add", "partial.rs"], "stage full partial.rs");
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "commit full content"],
        "commit full content",
    );
    let cp_second = checkpoint_id_from_message(&git_commit_message(dir.path(), "HEAD"))
        .expect("checkpoint for full content");

    assert_ne!(
        cp_first, cp_second,
        "partial and full commits should have distinct checkpoints"
    );
}

#[test]
fn cli_1161_session_depleted_manual_edit_no_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    init_and_enable(dir.path());

    let sid = "cli-1161";
    let transcript_path = dir.path().join("transcript-1161.jsonl");
    user_prompt_submit(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Create depleted.rs",
    );
    fs::write(
        dir.path().join("depleted.rs"),
        "package main\n\nfunc Depleted() {}\n",
    )
    .unwrap();
    write_transcript(
        &transcript_path,
        "Create depleted.rs",
        "Created depleted.rs",
    );
    stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    run_git_expect_success(dir.path(), &["add", "depleted.rs"], "stage depleted.rs");
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "commit depleted.rs"],
        "commit depleted.rs",
    );
    let before = all_checkpoint_ids_from_history(dir.path());
    assert!(
        !before.is_empty(),
        "first commit should create a checkpoint"
    );

    fs::write(
        dir.path().join("depleted.rs"),
        "package main\n\n// manual edit\nfunc Depleted() {}\n",
    )
    .unwrap();
    run_git_expect_success(
        dir.path(),
        &["add", "depleted.rs"],
        "stage manual depleted.rs",
    );
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "manual edit depleted.rs"],
        "commit manual depleted.rs",
    );

    let after = all_checkpoint_ids_from_history(dir.path());
    assert_eq!(
        after.len(),
        before.len(),
        "manual edits after session depletion should not create new checkpoint trailers"
    );
}

#[test]
fn cli_1162_subagent_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    init_and_enable(dir.path());

    let sid = "cli-1162";
    let transcript_path = dir.path().join("transcript-1162.jsonl");
    user_prompt_submit(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Delegate to subagent",
    );
    pre_task(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "toolu_subagent_1",
    );
    fs::write(
        dir.path().join("subagent_output.txt"),
        "created by subagent\n",
    )
    .unwrap();
    write_transcript(
        &transcript_path,
        "Delegate to subagent",
        "Subagent created subagent_output.txt",
    );
    post_task(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "toolu_subagent_1",
        "agent-sub1",
    );
    stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    let state = session_state(dir.path(), sid);
    assert!(
        state["checkpoint_count"].as_u64().unwrap_or(0) > 0,
        "subagent activity should increase checkpoint_count"
    );

    run_git_expect_success(
        dir.path(),
        &["add", "subagent_output.txt"],
        "stage subagent_output.txt",
    );
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "commit subagent output"],
        "commit subagent output",
    );
    assert!(
        checkpoint_id_from_message(&git_commit_message(dir.path(), "HEAD")).is_some(),
        "subagent commit flow should produce checkpoint trailer"
    );
}

#[test]
fn cli_1163_trailer_removal_skips_condensation() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    init_and_enable(dir.path());

    let sid = "cli-1163";
    let transcript_path = dir.path().join("transcript-1163.jsonl");
    user_prompt_submit(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Create trailer_test.rs",
    );
    fs::write(
        dir.path().join("trailer_test.rs"),
        "package main\n\nfunc TrailerTest() {}\n",
    )
    .unwrap();
    write_transcript(
        &transcript_path,
        "Create trailer_test.rs",
        "Created trailer_test.rs",
    );
    stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());
    let checkpoints_before = all_checkpoint_ids_from_history(dir.path());

    run_git_expect_success(
        dir.path(),
        &["add", "trailer_test.rs"],
        "stage trailer_test.rs",
    );
    commit_with_editor_overwrite_message(dir.path(), "Add trailer_test without checkpoint trailer");

    let head_msg = git_commit_message(dir.path(), "HEAD");
    assert!(
        checkpoint_id_from_message(&head_msg).is_none(),
        "commit message should not include Bitloops-Checkpoint trailer"
    );
    let checkpoints_after = all_checkpoint_ids_from_history(dir.path());
    assert_eq!(
        checkpoints_after.len(),
        checkpoints_before.len(),
        "removing trailer should skip checkpoint condensation"
    );
}

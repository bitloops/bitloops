use std::env;
use std::fs;
use std::io::Write;
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

fn init_repo(repo: &Path) {
    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "t@t.com"]);
    run_git(repo, &["config", "user.name", "Test"]);
    fs::write(repo.join("README.md"), "init\n").unwrap();
    run_git(repo, &["add", "."]);
    run_git(repo, &["commit", "-m", "initial"]);
}

fn init_and_enable_cursor(repo: &Path) {
    let init_out = run_cmd(repo, &["init", "--agent", "cursor"], None);
    assert_success(&init_out, "bitloops init --agent cursor");

    let out = run_cmd(repo, &["enable"], None);
    assert_success(&out, "bitloops enable");

    run_git_expect_success(
        repo,
        &["add", ".cursor/hooks.json", ".bitloops"],
        "stage cursor infra files",
    );
    let commit_out = run_git_output(repo, &["commit", "-m", "seed cursor bitloops infra files"]);
    if !commit_out.status.success() {
        let stderr = String::from_utf8_lossy(&commit_out.stderr);
        assert!(
            stderr.contains("nothing to commit") || stderr.contains("no changes added to commit"),
            "unexpected failure committing cursor infra files\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&commit_out.stdout),
            stderr
        );
    }
}

fn cursor_before_submit_prompt(
    repo: &Path,
    conversation_id: &str,
    transcript_path: &str,
    prompt: &str,
) {
    let input = format!(
        r#"{{"conversation_id":"{conversation_id}","transcript_path":"{transcript_path}","prompt":{prompt_json}}}"#,
        prompt_json = serde_json::to_string(prompt).unwrap()
    );
    let out = run_cmd(
        repo,
        &["hooks", "cursor", "before-submit-prompt"],
        Some(&input),
    );
    assert_success(&out, "hooks cursor before-submit-prompt");
}

fn cursor_stop(repo: &Path, conversation_id: &str, transcript_path: &str) {
    let input = format!(
        r#"{{"conversation_id":"{conversation_id}","transcript_path":"{transcript_path}"}}"#
    );
    let out = run_cmd(repo, &["hooks", "cursor", "stop"], Some(&input));
    assert_success(&out, "hooks cursor stop");
}

fn write_transcript(path: &Path, prompt: &str, response: &str) {
    let payload = format!(
        r#"{{"type":"user","message":{{"content":[{{"type":"text","text":{prompt_json}}}]}}}}
{{"type":"assistant","message":{{"content":[{{"type":"text","text":{response_json}}}]}}}}
"#,
        prompt_json = serde_json::to_string(prompt).unwrap(),
        response_json = serde_json::to_string(response).unwrap(),
    );
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

#[test]
fn cursor_basic_workflow() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    init_and_enable_cursor(dir.path());

    let sid = "cursor-basic-1";
    let transcript_path = dir.path().join("cursor-transcript-basic.jsonl");
    cursor_before_submit_prompt(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Create cursor_hello.rs",
    );
    fs::write(
        dir.path().join("cursor_hello.rs"),
        "package main\n\nfunc CursorHello() {}\n",
    )
    .unwrap();
    write_transcript(
        &transcript_path,
        "Create cursor_hello.rs",
        "Created cursor_hello.rs",
    );
    cursor_stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    run_git_expect_success(
        dir.path(),
        &["add", "cursor_hello.rs"],
        "git add cursor_hello.rs",
    );
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "add cursor hello file"],
        "commit cursor_hello.rs",
    );

    let head_msg = git_commit_message(dir.path(), "HEAD");
    assert!(
        checkpoint_id_from_message(&head_msg).is_some(),
        "cursor workflow commit should include checkpoint trailer"
    );
}

#[test]
fn cursor_checkpoint_metadata() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    init_and_enable_cursor(dir.path());

    let sid = "cursor-meta-1";
    let transcript_path = dir.path().join("cursor-transcript-meta.jsonl");
    cursor_before_submit_prompt(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Create cursor_meta.rs",
    );
    fs::write(
        dir.path().join("cursor_meta.rs"),
        "package main\n\nfunc CursorMeta() {}\n",
    )
    .unwrap();
    write_transcript(
        &transcript_path,
        "Create cursor_meta.rs",
        "Created cursor_meta.rs",
    );
    cursor_stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    run_git_expect_success(
        dir.path(),
        &["add", "cursor_meta.rs"],
        "git add cursor_meta.rs",
    );
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "commit cursor metadata file"],
        "commit cursor_meta.rs",
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
fn cursor_mid_turn_agent_commit_user_commit_remainder() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    init_and_enable_cursor(dir.path());

    let sid = "cursor-midturn-1";
    let transcript_path = dir.path().join("cursor-transcript-midturn.jsonl");
    cursor_before_submit_prompt(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Create three files",
    );
    fs::write(
        dir.path().join("cursor_agent_mid1.rs"),
        "package main\n\nfunc CursorMid1() {}\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("cursor_agent_mid2.rs"),
        "package main\n\nfunc CursorMid2() {}\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("cursor_user_remainder.rs"),
        "package main\n\nfunc CursorRemainder() {}\n",
    )
    .unwrap();
    write_transcript(
        &transcript_path,
        "Create three files",
        "Created three files",
    );

    run_git_expect_success(
        dir.path(),
        &["add", "cursor_agent_mid1.rs", "cursor_agent_mid2.rs"],
        "git add cursor agent mid-turn files",
    );
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "cursor agent mid-turn commit"],
        "cursor agent mid-turn commit",
    );
    let before = all_checkpoint_ids_from_history(dir.path());

    cursor_stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    run_git_expect_success(
        dir.path(),
        &["add", "cursor_user_remainder.rs"],
        "git add cursor remainder file",
    );
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "cursor user remainder commit"],
        "cursor user remainder commit",
    );
    let after = all_checkpoint_ids_from_history(dir.path());
    assert!(
        after.len() > before.len(),
        "cursor user remainder commit should add a new checkpoint id"
    );
}

#[test]
fn cursor_intermediate_commit_without_new_prompt_has_no_checkpoint_trailer() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    init_and_enable_cursor(dir.path());

    let sid = "cursor-intermediate-1";
    let transcript_path = dir.path().join("cursor-transcript-intermediate.jsonl");
    cursor_before_submit_prompt(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Create cursor_a.rs",
    );
    fs::write(
        dir.path().join("cursor_a.rs"),
        "package main\n\nfunc CursorA() {}\n",
    )
    .unwrap();
    write_transcript(
        &transcript_path,
        "Create cursor_a.rs",
        "Created cursor_a.rs",
    );
    cursor_stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    run_git_expect_success(dir.path(), &["add", "cursor_a.rs"], "git add cursor_a.rs");
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "cursor first commit"],
        "cursor first commit",
    );

    fs::write(
        dir.path().join("cursor_intermediate.txt"),
        "manual change\n",
    )
    .unwrap();
    run_git_expect_success(
        dir.path(),
        &["add", "cursor_intermediate.txt"],
        "git add cursor intermediate file",
    );
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "cursor intermediate commit"],
        "cursor intermediate commit",
    );
    let second_msg = git_commit_message(dir.path(), "HEAD");
    assert!(
        checkpoint_id_from_message(&second_msg).is_none(),
        "cursor intermediate commit without new prompt should not have checkpoint trailer"
    );
}

#[test]
fn cursor_pre_push_pushes_checkpoints_branch_to_remote() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    init_and_enable_cursor(dir.path());

    let sid = "cursor-push-1";
    let transcript_path = dir.path().join("cursor-transcript-push.jsonl");
    cursor_before_submit_prompt(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Create cursor_push.txt",
    );
    fs::write(dir.path().join("cursor_push.txt"), "checkpoint data\n").unwrap();
    write_transcript(
        &transcript_path,
        "Create cursor_push.txt",
        "Created cursor_push.txt",
    );
    cursor_stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    run_git_expect_success(
        dir.path(),
        &["add", "cursor_push.txt"],
        "git add cursor_push.txt",
    );
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "cursor push test"],
        "cursor push test commit",
    );
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
        "remote should contain bitloops/checkpoints/v1 after cursor push\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

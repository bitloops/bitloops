mod test_command_support;

use bitloops::host::checkpoints::strategy::manual_commit::{
    read_commit_checkpoint_mappings, read_committed,
};
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn bitloops_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_bitloops"))
}

fn run_cmd(repo: &Path, args: &[&str], stdin: Option<&str>) -> Output {
    let bin = bitloops_bin();
    let mut cmd = test_command_support::new_isolated_bitloops_command(&bin, repo, args);
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

    let mut cmd = Command::new("git");
    cmd.args(args).current_dir(repo).env("PATH", path);
    test_command_support::apply_repo_app_env(&mut cmd, repo);
    cmd.output().expect("failed to run git")
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

fn run_git_without_hooks_expect_success(repo: &Path, args: &[&str], context: &str) -> Output {
    let no_hooks_dir = test_command_support::isolated_repo_aux_dir(repo, "no-hooks");

    let mut prefixed_args = Vec::with_capacity(args.len() + 2);
    prefixed_args.push("-c");
    let hooks_path_override = format!("core.hooksPath={}", no_hooks_dir.display());
    prefixed_args.push(hooks_path_override.as_str());
    prefixed_args.extend_from_slice(args);

    run_git_expect_success(repo, &prefixed_args, context)
}

fn git_ref_exists(repo: &Path, reference: &str) -> bool {
    run_git_output(repo, &["show-ref", "--verify", "--quiet", reference])
        .status
        .success()
}

fn checkpoint_id_for_head(repo: &Path) -> Option<String> {
    let head = run_git(repo, &["rev-parse", "HEAD"]);
    test_command_support::with_repo_app_env(repo, || {
        read_commit_checkpoint_mappings(repo)
            .expect("failed to read commit-checkpoint mappings")
            .get(&head)
            .cloned()
    })
}

fn all_checkpoint_ids_from_history(repo: &Path) -> Vec<String> {
    test_command_support::with_repo_app_env(repo, || {
        let mappings = read_commit_checkpoint_mappings(repo)
            .expect("failed to read commit-checkpoint mappings");
        let mut ids: Vec<String> = mappings.into_values().collect();
        ids.sort();
        ids.dedup();
        ids
    })
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

fn ensure_relational_store_file(repo_root: &Path) {
    test_command_support::ensure_repo_daemon_stores(repo_root);
}

fn init_cursor(repo: &Path) {
    test_command_support::with_repo_app_env(repo, || {
        ensure_relational_store_file(repo);
        let policy_path = repo.join(bitloops::config::REPO_POLICY_LOCAL_FILE_NAME);
        bitloops::config::settings::write_project_bootstrap_settings(
            &policy_path,
            bitloops::config::settings::DEFAULT_STRATEGY,
            &[String::from("cursor")],
        )
        .expect("write project bootstrap settings");
        bitloops::adapters::agents::claude_code::git_hooks::install_git_hooks(repo, false)
            .expect("install git hooks");
        bitloops::adapters::agents::AgentAdapterRegistry::builtin()
            .install_agent_hooks(repo, "cursor", false, false)
            .expect("install Cursor hooks");
    });
}

fn init_and_enable_cursor(repo: &Path) {
    init_cursor(repo);

    run_git_expect_success(
        repo,
        &["add", ".cursor/hooks.json"],
        "stage cursor hook file",
    );
    let commit_out = run_git_without_hooks_expect_success(
        repo,
        &["commit", "-m", "seed cursor bitloops infra files"],
        "commit cursor infra files",
    );
    let _ = commit_out;
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

    let checkpoint_id =
        checkpoint_id_for_head(dir.path()).expect("HEAD commit should map to a checkpoint");
    let summary = test_command_support::with_repo_app_env(dir.path(), || {
        read_committed(dir.path(), &checkpoint_id)
            .expect("reading committed checkpoint should succeed")
            .expect("committed checkpoint should exist")
    });

    assert!(
        summary.files_touched.iter().any(|f| f == "cursor_meta.rs"),
        "checkpoint should include cursor_meta.rs in files_touched"
    );
    assert!(
        summary.sessions.len() == 1,
        "single cursor stop+commit should produce one checkpoint session"
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
fn cursor_intermediate_commit_without_new_prompt_has_no_checkpoint_mapping() {
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
    let second_head = run_git(dir.path(), &["rev-parse", "HEAD"]);
    let mappings = test_command_support::with_repo_app_env(dir.path(), || {
        read_commit_checkpoint_mappings(dir.path())
            .expect("reading commit-checkpoint mappings should succeed")
    });
    assert!(
        !mappings.contains_key(&second_head),
        "cursor intermediate commit without new prompt should not map to a checkpoint"
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
    let checkpoint_id =
        checkpoint_id_for_head(dir.path()).expect("cursor push commit should map to a checkpoint");
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

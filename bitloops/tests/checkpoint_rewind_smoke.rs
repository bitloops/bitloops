mod agent_smoke_support;
mod test_command_support;

use agent_smoke_support::{
    assert_success, checkpoint_id_for_head, claude_stop, claude_user_prompt_submit,
    get_rewind_points, init_claude, init_repo, rewind_to, run_git, run_git_expect_success,
    write_claude_transcript,
};
use std::fs;
use std::path::Path;
use std::process::Command;

fn init_and_enable(repo: &Path) {
    init_claude(repo);
    run_git_expect_success(
        repo,
        &["add", ".claude/settings.json", "config.toml"],
        "stage enable-generated Claude infra files",
    );
    agent_smoke_support::run_git_without_hooks_expect_success(
        repo,
        &["commit", "-m", "seed bitloops infra files"],
        "commit Claude infra files",
    );
}

#[test]
fn checkpoint_smoke_basic_workflow_maps_commit() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_repo(dir.path());
    init_and_enable(dir.path());

    let sid = "checkpoint-smoke-1";
    let transcript_path = dir.path().join("checkpoint-smoke-1.jsonl");
    claude_user_prompt_submit(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Create hello.rs",
    );
    fs::write(
        dir.path().join("hello.rs"),
        "package main\n\nfunc main() {}\n",
    )
    .expect("write hello.rs");
    write_claude_transcript(&transcript_path, "Create hello.rs", "Created hello.rs");
    claude_stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    run_git_expect_success(dir.path(), &["add", "hello.rs"], "stage hello.rs");
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "add hello.rs"],
        "commit hello.rs",
    );

    assert!(
        checkpoint_id_for_head(dir.path()).is_some(),
        "basic checkpoint workflow should map HEAD to a checkpoint"
    );
}

#[test]
fn checkpoint_smoke_agent_commit_during_turn_maps_remainder_commit() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_repo(dir.path());
    init_and_enable(dir.path());

    let sid = "checkpoint-smoke-2";
    let transcript_path = dir.path().join("checkpoint-smoke-2.jsonl");
    claude_user_prompt_submit(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Create first file",
    );
    fs::write(
        dir.path().join("agent_mid_turn.rs"),
        "package main\n\nfunc MidTurn() {}\n",
    )
    .expect("write agent_mid_turn.rs");
    write_claude_transcript(
        &transcript_path,
        "Create first file",
        "Created agent_mid_turn.rs",
    );

    run_git_expect_success(
        dir.path(),
        &["add", "agent_mid_turn.rs"],
        "stage mid-turn file",
    );
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "agent commit during active turn"],
        "agent mid-turn commit",
    );

    claude_stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    claude_user_prompt_submit(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Create second file",
    );
    fs::write(
        dir.path().join("user_after_agent.rs"),
        "package main\n\nfunc UserAfterAgent() {}\n",
    )
    .expect("write user_after_agent.rs");
    write_claude_transcript(
        &transcript_path,
        "Create second file",
        "Created user_after_agent.rs",
    );
    claude_stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    run_git_expect_success(
        dir.path(),
        &["add", "user_after_agent.rs"],
        "stage second file",
    );
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "user commit remainder"],
        "commit remainder file",
    );

    assert!(
        checkpoint_id_for_head(dir.path()).is_some(),
        "final user commit should still map to a checkpoint"
    );
}

#[test]
fn checkpoint_smoke_rewind_restores_previous_checkpoint_tree() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_repo(dir.path());
    init_and_enable(dir.path());

    let sid = "checkpoint-smoke-3";
    let transcript_path = dir.path().join("checkpoint-smoke-3.jsonl");
    claude_user_prompt_submit(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Create hello.rs",
    );
    fs::write(
        dir.path().join("hello.rs"),
        "package main\n\nfunc main() {\n\tprintln(\"Hello, world!\")\n}\n",
    )
    .expect("write original hello.rs");
    write_claude_transcript(&transcript_path, "Create hello.rs", "Created hello.rs");
    claude_stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());
    run_git_expect_success(dir.path(), &["add", "hello.rs"], "stage original hello.rs");
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "commit hello.rs checkpoint 1"],
        "commit first checkpoint",
    );

    let points = get_rewind_points(dir.path());
    assert!(
        !points.is_empty(),
        "should have at least one rewind point after first checkpoint"
    );
    let first_point_id = points[0].id.clone();
    let first_point_is_logs_only = points[0].is_logs_only;
    let original_content =
        fs::read_to_string(dir.path().join("hello.rs")).expect("read original hello.rs");

    claude_user_prompt_submit(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Modify hello.rs",
    );
    fs::write(
        dir.path().join("hello.rs"),
        "package main\n\nfunc main() {\n\tprintln(\"Checkpoint Smoke\")\n}\n",
    )
    .expect("write modified hello.rs");
    write_claude_transcript(&transcript_path, "Modify hello.rs", "Modified hello.rs");
    claude_stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());
    run_git_expect_success(dir.path(), &["add", "hello.rs"], "stage modified hello.rs");
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "commit hello.rs checkpoint 2"],
        "commit second checkpoint",
    );

    let modified_content =
        fs::read_to_string(dir.path().join("hello.rs")).expect("read modified hello.rs");
    assert_ne!(
        original_content, modified_content,
        "hello.rs content should change after modification"
    );

    let rewind_out = rewind_to(dir.path(), &first_point_id);
    assert_success(&rewind_out, "rewind to first checkpoint");

    let restored_content =
        fs::read_to_string(dir.path().join("hello.rs")).expect("read restored hello.rs");
    if first_point_is_logs_only {
        assert_eq!(
            modified_content, restored_content,
            "logs-only rewind should leave working-tree content unchanged"
        );
    } else {
        assert_eq!(
            original_content, restored_content,
            "rewind should restore hello.rs to the first checkpoint content"
        );
    }
}

#[test]
fn checkpoint_smoke_pre_push_keeps_checkpoints_branch_out_of_remote_db_blob_mode() {
    let dir = tempfile::tempdir().expect("tempdir");
    init_repo(dir.path());
    init_claude(dir.path());

    let sid = "checkpoint-smoke-4";
    let transcript_path = dir.path().join("checkpoint-smoke-4.jsonl");
    claude_user_prompt_submit(
        dir.path(),
        sid,
        transcript_path.to_string_lossy().as_ref(),
        "Create push target file",
    );
    fs::write(dir.path().join("push.txt"), "checkpoint data\n").expect("write push target");
    write_claude_transcript(
        &transcript_path,
        "Create push target file",
        "Created push.txt",
    );
    claude_stop(dir.path(), sid, transcript_path.to_string_lossy().as_ref());

    run_git_expect_success(dir.path(), &["add", "push.txt"], "stage push target");
    run_git_expect_success(
        dir.path(),
        &["commit", "-m", "push test"],
        "commit push target",
    );
    let checkpoint_id =
        checkpoint_id_for_head(dir.path()).expect("push test commit should map to a checkpoint");
    assert!(
        !checkpoint_id.is_empty(),
        "mapped checkpoint id should not be empty"
    );

    let remote_dir = tempfile::tempdir().expect("remote tempdir");
    run_git(remote_dir.path(), &["init", "--bare"]);
    let remote_path = remote_dir.path().to_string_lossy().to_string();

    run_git(dir.path(), &["remote", "add", "origin", &remote_path]);
    let branch = run_git(dir.path(), &["rev-parse", "--abbrev-ref", "HEAD"]);
    run_git(dir.path(), &["push", "-u", "origin", &branch]);

    let remote_ref_after_push = Command::new("git")
        .args([
            "--git-dir",
            &remote_path,
            "show-ref",
            "--verify",
            "refs/heads/bitloops/checkpoints/v1",
        ])
        .output()
        .expect("inspect remote refs");
    assert!(
        !remote_ref_after_push.status.success(),
        "remote should not contain bitloops/checkpoints/v1 in DB/blob mode\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&remote_ref_after_push.stdout),
        String::from_utf8_lossy(&remote_ref_after_push.stderr)
    );
}

use super::*;
use crate::engine::agent::AGENT_TYPE_CLAUDE_CODE;
use crate::test_support::process_state::{
    git_command, isolated_git_command, with_env_var, with_git_env_cleared,
};
use serde::{Deserialize, Serialize};
use tempfile::TempDir;
const HIGH_ENTROPY_SECRET: &str = "sk-ant-api03-xK9mZ2vL8nQ5rT1wY4bC7dF0gH3jE6pA";

/// Creates a real git repository with an initial commit for testing.
fn setup_git_repo(dir: &TempDir) -> String {
    let run = |args: &[&str]| {
        let out = isolated_git_command(dir.path())
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
            args,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    };
    run(&["init"]);
    run(&["config", "user.email", "t@t.com"]);
    run(&["config", "user.name", "Test"]);
    fs::write(dir.path().join("README.md"), "initial content").unwrap();
    run(&["add", "."]);
    run(&["commit", "--allow-empty", "-m", "initial"]);
    // Return HEAD hash.
    let out = isolated_git_command(dir.path())
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git [\"rev-parse\", \"HEAD\"] failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Creates a git repo with no commits.
fn setup_empty_git_repo(dir: &TempDir) {
    let run = |args: &[&str]| {
        let out = isolated_git_command(dir.path())
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
            args,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    };
    run(&["init"]);
    run(&["config", "user.email", "t@t.com"]);
    run(&["config", "user.name", "Test"]);
}

fn git_ok(repo_root: &Path, args: &[&str]) -> String {
    let out = isolated_git_command(repo_root)
        .args(args)
        .output()
        .unwrap_or_else(|err| panic!("failed to start git {:?}: {err}", args));
    assert!(
        out.status.success(),
        "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn init_sequence_worktree_repo() -> (TempDir, PathBuf, PathBuf) {
    let parent = tempfile::tempdir().unwrap();
    let main_repo = parent.path().join("main");
    let worktree_dir = parent.path().join("worktree");
    fs::create_dir_all(&main_repo).unwrap();

    git_ok(&main_repo, &["init"]);
    git_ok(&main_repo, &["config", "user.email", "t@t.com"]);
    git_ok(&main_repo, &["config", "user.name", "Test"]);
    fs::write(main_repo.join("README.md"), "initial content").unwrap();
    git_ok(&main_repo, &["add", "."]);
    git_ok(&main_repo, &["commit", "--allow-empty", "-m", "initial"]);
    git_ok(
        &main_repo,
        &[
            "worktree",
            "add",
            worktree_dir.to_string_lossy().as_ref(),
            "-b",
            "feature",
        ],
    );

    (parent, main_repo, worktree_dir)
}

fn create_shadow_branch_with_content(repo_root: &Path, branch: &str, files: &[(&str, &str)]) {
    let current = git_ok(repo_root, &["rev-parse", "--abbrev-ref", "HEAD"]);
    git_ok(repo_root, &["checkout", "-b", branch]);

    for (path, content) in files {
        let full_path = repo_root.join(path);
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(full_path, content).unwrap();
    }
    git_ok(repo_root, &["add", "-A"]);
    git_ok(
        repo_root,
        &["commit", "--allow-empty", "-m", "shadow checkpoint"],
    );
    git_ok(repo_root, &["checkout", &current]);
}

#[test]
fn files_overlap_with_content_modified_file() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    fs::write(dir.path().join("test.txt"), "original content").unwrap();
    git_ok(dir.path(), &["add", "test.txt"]);
    git_ok(dir.path(), &["commit", "-m", "initial test.txt"]);

    let shadow_branch = "bitloops-shadow-419";
    create_shadow_branch_with_content(
        dir.path(),
        shadow_branch,
        &[("test.txt", "session modified content")],
    );

    fs::write(dir.path().join("test.txt"), "user modified further").unwrap();
    git_ok(dir.path(), &["add", "test.txt"]);
    git_ok(dir.path(), &["commit", "-m", "modify file"]);
    let head = git_ok(dir.path(), &["rev-parse", "HEAD"]);

    let files = vec!["test.txt".to_string()];
    assert!(files_overlap_with_content(
        dir.path(),
        shadow_branch,
        &head,
        &files
    ));
}

#[test]
fn files_overlap_with_content_new_file_content_match() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    let shadow_branch = "bitloops-shadow-420";
    create_shadow_branch_with_content(
        dir.path(),
        shadow_branch,
        &[("newfile.txt", "session created this content")],
    );

    fs::write(
        dir.path().join("newfile.txt"),
        "session created this content",
    )
    .unwrap();
    git_ok(dir.path(), &["add", "newfile.txt"]);
    git_ok(dir.path(), &["commit", "-m", "add new file"]);
    let head = git_ok(dir.path(), &["rev-parse", "HEAD"]);

    let files = vec!["newfile.txt".to_string()];
    assert!(files_overlap_with_content(
        dir.path(),
        shadow_branch,
        &head,
        &files
    ));
}

#[test]
fn files_overlap_with_content_new_file_content_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    let shadow_branch = "bitloops-shadow-421";
    create_shadow_branch_with_content(
        dir.path(),
        shadow_branch,
        &[("replaced.txt", "session created this")],
    );

    fs::write(
        dir.path().join("replaced.txt"),
        "user wrote something totally unrelated",
    )
    .unwrap();
    git_ok(dir.path(), &["add", "replaced.txt"]);
    git_ok(dir.path(), &["commit", "-m", "add replaced file"]);
    let head = git_ok(dir.path(), &["rev-parse", "HEAD"]);

    let files = vec!["replaced.txt".to_string()];
    assert!(!files_overlap_with_content(
        dir.path(),
        shadow_branch,
        &head,
        &files
    ));
}

#[test]
fn files_overlap_with_content_file_not_in_commit() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    let shadow_branch = "bitloops-shadow-422";
    create_shadow_branch_with_content(
        dir.path(),
        shadow_branch,
        &[
            ("fileA.txt", "file A content"),
            ("fileB.txt", "file B content"),
        ],
    );

    fs::write(dir.path().join("fileA.txt"), "file A content").unwrap();
    git_ok(dir.path(), &["add", "fileA.txt"]);
    git_ok(dir.path(), &["commit", "-m", "add only file A"]);
    let head = git_ok(dir.path(), &["rev-parse", "HEAD"]);

    let files_b = vec!["fileB.txt".to_string()];
    assert!(!files_overlap_with_content(
        dir.path(),
        shadow_branch,
        &head,
        &files_b
    ));

    let files_a = vec!["fileA.txt".to_string()];
    assert!(files_overlap_with_content(
        dir.path(),
        shadow_branch,
        &head,
        &files_a
    ));
}

#[test]
fn files_overlap_with_content_deleted_file() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    fs::write(dir.path().join("to_delete.txt"), "content to delete").unwrap();
    git_ok(dir.path(), &["add", "to_delete.txt"]);
    git_ok(dir.path(), &["commit", "-m", "add file to delete"]);

    let shadow_branch = "bitloops-shadow-423";
    create_shadow_branch_with_content(dir.path(), shadow_branch, &[("other.txt", "other content")]);

    git_ok(dir.path(), &["rm", "to_delete.txt"]);
    git_ok(dir.path(), &["commit", "-m", "delete file"]);
    let head = git_ok(dir.path(), &["rev-parse", "HEAD"]);

    let files = vec!["to_delete.txt".to_string()];
    assert!(files_overlap_with_content(
        dir.path(),
        shadow_branch,
        &head,
        &files
    ));
}

#[test]
fn files_overlap_with_content_no_shadow_branch() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    fs::write(dir.path().join("test.txt"), "content").unwrap();
    git_ok(dir.path(), &["add", "test.txt"]);
    git_ok(dir.path(), &["commit", "-m", "test commit"]);
    let head = git_ok(dir.path(), &["rev-parse", "HEAD"]);

    let files = vec!["test.txt".to_string()];
    assert!(files_overlap_with_content(
        dir.path(),
        "bitloops/nonexistent-shadow",
        &head,
        &files
    ));
}

#[test]
fn files_with_remaining_agent_changes_file_not_committed() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    let shadow_branch = "bitloops-shadow-425";
    create_shadow_branch_with_content(
        dir.path(),
        shadow_branch,
        &[("fileA.txt", "content A"), ("fileB.txt", "content B")],
    );

    fs::write(dir.path().join("fileA.txt"), "content A").unwrap();
    git_ok(dir.path(), &["add", "fileA.txt"]);
    git_ok(dir.path(), &["commit", "-m", "add file A only"]);
    let head = git_ok(dir.path(), &["rev-parse", "HEAD"]);

    let files_touched = vec!["fileA.txt".to_string(), "fileB.txt".to_string()];
    let committed_files = std::collections::HashSet::from(["fileA.txt".to_string()]);
    let remaining = files_with_remaining_agent_changes(
        dir.path(),
        shadow_branch,
        &head,
        &files_touched,
        &committed_files,
    );
    assert_eq!(remaining, vec!["fileB.txt".to_string()]);
}

#[test]
fn files_with_remaining_agent_changes_fully_committed() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    let shadow_branch = "bitloops-shadow-426";
    create_shadow_branch_with_content(
        dir.path(),
        shadow_branch,
        &[("test.txt", "exact same content")],
    );

    fs::write(dir.path().join("test.txt"), "exact same content").unwrap();
    git_ok(dir.path(), &["add", "test.txt"]);
    git_ok(dir.path(), &["commit", "-m", "add same"]);
    let head = git_ok(dir.path(), &["rev-parse", "HEAD"]);

    let files_touched = vec!["test.txt".to_string()];
    let committed_files = std::collections::HashSet::from(["test.txt".to_string()]);
    let remaining = files_with_remaining_agent_changes(
        dir.path(),
        shadow_branch,
        &head,
        &files_touched,
        &committed_files,
    );
    assert!(remaining.is_empty());
}

#[test]
fn files_with_remaining_agent_changes_partial_commit() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    let shadow_branch = "bitloops-shadow-427";
    create_shadow_branch_with_content(
        dir.path(),
        shadow_branch,
        &[("test.txt", "line 1\nline 2\nline 3\nline 4\n")],
    );

    fs::write(dir.path().join("test.txt"), "line 1\nline 2\n").unwrap();
    git_ok(dir.path(), &["add", "test.txt"]);
    git_ok(dir.path(), &["commit", "-m", "partial"]);
    let head = git_ok(dir.path(), &["rev-parse", "HEAD"]);

    let files_touched = vec!["test.txt".to_string()];
    let committed_files = std::collections::HashSet::from(["test.txt".to_string()]);
    let remaining = files_with_remaining_agent_changes(
        dir.path(),
        shadow_branch,
        &head,
        &files_touched,
        &committed_files,
    );
    assert_eq!(remaining, vec!["test.txt".to_string()]);
}

#[test]
fn files_with_remaining_agent_changes_no_shadow_branch() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    fs::write(dir.path().join("test.txt"), "content").unwrap();
    git_ok(dir.path(), &["add", "test.txt"]);
    git_ok(dir.path(), &["commit", "-m", "test"]);
    let head = git_ok(dir.path(), &["rev-parse", "HEAD"]);

    let files_touched = vec!["test.txt".to_string(), "other.txt".to_string()];
    let committed_files = std::collections::HashSet::from(["test.txt".to_string()]);
    let remaining = files_with_remaining_agent_changes(
        dir.path(),
        "bitloops/nonexistent-shadow",
        &head,
        &files_touched,
        &committed_files,
    );
    assert_eq!(remaining, vec!["other.txt".to_string()]);
}

#[test]
fn staged_files_overlap_with_content_modified_file() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    fs::write(dir.path().join("test.txt"), "base").unwrap();
    git_ok(dir.path(), &["add", "test.txt"]);
    git_ok(dir.path(), &["commit", "-m", "add test"]);

    let shadow_branch = "bitloops-shadow-429";
    create_shadow_branch_with_content(dir.path(), shadow_branch, &[("test.txt", "shadow content")]);

    fs::write(dir.path().join("test.txt"), "modified content").unwrap();
    git_ok(dir.path(), &["add", "test.txt"]);

    let staged = vec!["test.txt".to_string()];
    let touched = vec!["test.txt".to_string()];
    assert!(staged_files_overlap_with_content(
        dir.path(),
        shadow_branch,
        &staged,
        &touched
    ));
}

#[test]
fn staged_files_overlap_with_content_new_file_content_match() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    let shadow_branch = "bitloops-shadow-430";
    create_shadow_branch_with_content(
        dir.path(),
        shadow_branch,
        &[("newfile.txt", "new file content")],
    );

    fs::write(dir.path().join("newfile.txt"), "new file content").unwrap();
    git_ok(dir.path(), &["add", "newfile.txt"]);

    let staged = vec!["newfile.txt".to_string()];
    let touched = vec!["newfile.txt".to_string()];
    assert!(staged_files_overlap_with_content(
        dir.path(),
        shadow_branch,
        &staged,
        &touched
    ));
}

#[test]
fn staged_files_overlap_with_content_new_file_content_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    let shadow_branch = "bitloops-shadow-431";
    create_shadow_branch_with_content(
        dir.path(),
        shadow_branch,
        &[("newfile.txt", "agent original content")],
    );

    fs::write(dir.path().join("newfile.txt"), "user replaced content").unwrap();
    git_ok(dir.path(), &["add", "newfile.txt"]);

    let staged = vec!["newfile.txt".to_string()];
    let touched = vec!["newfile.txt".to_string()];
    assert!(!staged_files_overlap_with_content(
        dir.path(),
        shadow_branch,
        &staged,
        &touched
    ));
}

#[test]
fn staged_files_overlap_with_content_no_overlap() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    let shadow_branch = "bitloops-shadow-432";
    create_shadow_branch_with_content(
        dir.path(),
        shadow_branch,
        &[("session.txt", "session content")],
    );

    fs::write(dir.path().join("other.txt"), "other content").unwrap();
    git_ok(dir.path(), &["add", "other.txt"]);

    let staged = vec!["other.txt".to_string()];
    let touched = vec!["session.txt".to_string()];
    assert!(!staged_files_overlap_with_content(
        dir.path(),
        shadow_branch,
        &staged,
        &touched
    ));
}

#[test]
fn staged_files_overlap_with_content_deleted_file() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    fs::write(dir.path().join("to_delete.txt"), "original content").unwrap();
    git_ok(dir.path(), &["add", "to_delete.txt"]);
    git_ok(dir.path(), &["commit", "-m", "add to delete"]);

    let shadow_branch = "bitloops-shadow-433";
    create_shadow_branch_with_content(
        dir.path(),
        shadow_branch,
        &[("to_delete.txt", "agent modified content")],
    );

    git_ok(dir.path(), &["rm", "to_delete.txt"]);

    let staged = vec!["to_delete.txt".to_string()];
    let touched = vec!["to_delete.txt".to_string()];
    assert!(staged_files_overlap_with_content(
        dir.path(),
        shadow_branch,
        &staged,
        &touched
    ));
}

#[test]
fn test_extract_significant_lines() {
    let cases = vec![
        (
            "package main\n\nfunc hello() {\n\tfmt.Println(\"hello world\")\n\treturn\n}",
            vec![
                "package main",
                "func hello() {",
                "fmt.Println(\"hello world\")",
            ],
            vec!["}", "return"],
        ),
        (
            "def calculate(x, y):\n    result = x + y\n    print(f\"Result: {result}\")\n    return result",
            vec![
                "def calculate(x, y):",
                "result = x + y",
                "print(f\"Result: {result}\")",
                "return result",
            ],
            vec![],
        ),
        (
            "a = 1\nb = 2\nlongVariableName = 42",
            vec!["longVariableName = 42"],
            vec!["a = 1", "b = 2"],
        ),
        (
            "{\n  });\n  ]);\n  },\n}",
            vec![],
            vec!["{", "});", "]);", "},", "}"],
        ),
    ];

    for (content, want_keys, want_not) in cases {
        let result = extract_significant_lines(content);
        for expected in want_keys {
            assert!(
                result.contains(expected),
                "missing expected line: {expected:?}, got: {result:?}"
            );
        }
        for denied in want_not {
            assert!(
                !result.contains(denied),
                "unexpected line present: {denied:?}, got: {result:?}"
            );
        }
    }
}

#[test]
fn test_has_significant_content_overlap() {
    let cases = vec![
        (
            "this is a significant line\nanother matching line here\nshort",
            "this is a significant line\nanother matching line here\nother",
            true,
        ),
        (
            "this is a significant line\ncompletely different staged",
            "this is a significant line\ncompletely different shadow",
            false,
        ),
        ("a = 1\nb = 2\nc = 3", "x = 1\ny = 2\nz = 3", false),
        (
            "package main\nfunc NewImplementation() {}",
            "package main\nfunc OriginalCode() {}",
            false,
        ),
        (
            "package main\nfunc SharedFunction() {\nreturn nil",
            "package main\nfunc SharedFunction() {\nreturn nil",
            true,
        ),
        (
            "this is a unique line here\nshort",
            "this is a unique line here\nshort",
            true,
        ),
        ("completely different staged content", "short", false),
    ];

    for (staged, shadow, expected) in cases {
        assert_eq!(
            has_significant_content_overlap(staged, shadow),
            expected,
            "staged={staged:?} shadow={shadow:?}"
        );
    }
}

#[test]
fn test_trim_line() {
    let cases = vec![
        ("hello", "hello"),
        ("   hello", "hello"),
        ("hello   ", "hello"),
        ("   hello   ", "hello"),
        ("\t\thello", "hello"),
        ("hello\t\t", "hello"),
        (" \t hello \t ", "hello"),
        ("     ", ""),
        ("\t\t\t", ""),
        ("", ""),
        ("hello world", "hello world"),
        ("hello\tworld", "hello\tworld"),
    ];

    for (line, expected) in cases {
        assert_eq!(trim_line(line), expected);
    }
}

#[test]
fn is_git_sequence_operation_no_operation() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    assert!(
        !is_git_sequence_operation(dir.path()),
        "clean repository should not be in sequence operation"
    );
}

#[test]
fn is_git_sequence_operation_rebase_merge() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    fs::create_dir_all(dir.path().join(".git").join("rebase-merge")).unwrap();
    assert!(
        is_git_sequence_operation(dir.path()),
        "rebase-merge should be detected as sequence operation"
    );
}

#[test]
fn is_git_sequence_operation_rebase_apply() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    fs::create_dir_all(dir.path().join(".git").join("rebase-apply")).unwrap();
    assert!(
        is_git_sequence_operation(dir.path()),
        "rebase-apply should be detected as sequence operation"
    );
}

#[test]
fn is_git_sequence_operation_cherry_pick() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    fs::write(dir.path().join(".git").join("CHERRY_PICK_HEAD"), "abc123").unwrap();
    assert!(
        is_git_sequence_operation(dir.path()),
        "CHERRY_PICK_HEAD should be detected as sequence operation"
    );
}

#[test]
fn is_git_sequence_operation_revert() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    fs::write(dir.path().join(".git").join("REVERT_HEAD"), "abc123").unwrap();
    assert!(
        is_git_sequence_operation(dir.path()),
        "REVERT_HEAD should be detected as sequence operation"
    );
}

#[test]
fn is_git_sequence_operation_worktree() {
    let (_parent, _main_repo, worktree_dir) = init_sequence_worktree_repo();
    assert!(
        !is_git_sequence_operation(&worktree_dir),
        "clean worktree should not be in sequence operation"
    );

    let worktree_git_dir_raw = git_ok(&worktree_dir, &["rev-parse", "--git-dir"]);
    let worktree_git_dir = if Path::new(&worktree_git_dir_raw).is_absolute() {
        PathBuf::from(worktree_git_dir_raw)
    } else {
        worktree_dir.join(worktree_git_dir_raw)
    };
    fs::create_dir_all(worktree_git_dir.join("rebase-merge")).unwrap();

    assert!(
        is_git_sequence_operation(&worktree_dir),
        "worktree rebase state should be detected as sequence operation"
    );
}

#[test]
fn save_step_creates_shadow_branch() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);

    // Modify a file so there's something to snapshot.
    fs::write(dir.path().join("file.txt"), "hello").unwrap();

    let strategy = ManualCommitStrategy::new(dir.path());

    // Pre-create session state so save_step can load it.
    let backend = LocalFileBackend::new(dir.path());
    let state = SessionState {
        session_id: "s1".to_string(),
        base_commit: head.clone(),
        phase: crate::engine::session::phase::SessionPhase::Active,
        ..Default::default()
    };
    backend.save_session(&state).unwrap();

    let ctx = StepContext {
        session_id: "s1".to_string(),
        modified_files: vec![],
        new_files: vec!["file.txt".to_string()],
        deleted_files: vec![],
        metadata_dir: String::new(),
        metadata_dir_abs: String::new(),
        commit_message: String::new(),
        transcript_path: String::new(),
        author_name: String::new(),
        author_email: String::new(),
        agent_type: String::new(),
        step_transcript_identifier: String::new(),
        step_transcript_start: 0,
        token_usage: None,
    };
    strategy.save_step(&ctx).unwrap();

    // Shadow branch should exist.
    let shadow = shadow_branch_ref(&head, "");
    let result = run_git(dir.path(), &["rev-parse", &shadow]);
    assert!(result.is_ok(), "shadow branch should exist after save_step");
}

#[test]
fn save_step_shadow_has_modified_file() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);

    fs::write(dir.path().join("src.rs"), "fn main() {}").unwrap();

    let strategy = ManualCommitStrategy::new(dir.path());
    let backend = LocalFileBackend::new(dir.path());
    let state = SessionState {
        session_id: "s2".to_string(),
        base_commit: head.clone(),
        phase: crate::engine::session::phase::SessionPhase::Active,
        ..Default::default()
    };
    backend.save_session(&state).unwrap();

    let ctx = StepContext {
        session_id: "s2".to_string(),
        modified_files: vec![],
        new_files: vec!["src.rs".to_string()],
        deleted_files: vec![],
        metadata_dir: String::new(),
        metadata_dir_abs: String::new(),
        commit_message: String::new(),
        transcript_path: String::new(),
        author_name: String::new(),
        author_email: String::new(),
        agent_type: String::new(),
        step_transcript_identifier: String::new(),
        step_transcript_start: 0,
        token_usage: None,
    };
    strategy.save_step(&ctx).unwrap();

    // Check file exists in shadow branch tree.
    let shadow = shadow_branch_ref(&head, "");
    let result = run_git(dir.path(), &["ls-tree", &shadow, "src.rs"]);
    assert!(result.is_ok(), "src.rs should be in shadow branch tree");
    assert!(result.unwrap().contains("src.rs"));
}

#[test]
fn save_step_skips_when_no_changes() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);

    fs::write(dir.path().join("file.txt"), "hello").unwrap();

    let strategy = ManualCommitStrategy::new(dir.path());
    let backend = LocalFileBackend::new(dir.path());
    let state = SessionState {
        session_id: "s3".to_string(),
        base_commit: head.clone(),
        phase: crate::engine::session::phase::SessionPhase::Active,
        ..Default::default()
    };
    backend.save_session(&state).unwrap();

    let ctx = StepContext {
        session_id: "s3".to_string(),
        modified_files: vec![],
        new_files: vec!["file.txt".to_string()],
        deleted_files: vec![],
        metadata_dir: String::new(),
        metadata_dir_abs: String::new(),
        commit_message: String::new(),
        transcript_path: String::new(),
        author_name: String::new(),
        author_email: String::new(),
        agent_type: String::new(),
        step_transcript_identifier: String::new(),
        step_transcript_start: 0,
        token_usage: None,
    };

    strategy.save_step(&ctx).unwrap();
    let s1 = backend.load_session("s3").unwrap().unwrap();
    let count1 = s1.step_count;

    // Second call with same context — tree is identical → skip.
    strategy.save_step(&ctx).unwrap();
    let s2 = backend.load_session("s3").unwrap().unwrap();

    assert_eq!(
        s2.step_count, count1,
        "step_count should not increase for identical tree"
    );
}

#[test]
fn save_step_increments_step_count() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);

    fs::write(dir.path().join("a.txt"), "a").unwrap();

    let strategy = ManualCommitStrategy::new(dir.path());
    let backend = LocalFileBackend::new(dir.path());
    let state = SessionState {
        session_id: "s4".to_string(),
        base_commit: head.clone(),
        phase: crate::engine::session::phase::SessionPhase::Active,
        step_count: 0,
        ..Default::default()
    };
    backend.save_session(&state).unwrap();

    let ctx = StepContext {
        session_id: "s4".to_string(),
        modified_files: vec![],
        new_files: vec!["a.txt".to_string()],
        deleted_files: vec![],
        metadata_dir: String::new(),
        metadata_dir_abs: String::new(),
        commit_message: String::new(),
        transcript_path: String::new(),
        author_name: String::new(),
        author_email: String::new(),
        agent_type: String::new(),
        step_transcript_identifier: String::new(),
        step_transcript_start: 0,
        token_usage: None,
    };
    strategy.save_step(&ctx).unwrap();

    let loaded = backend.load_session("s4").unwrap().unwrap();
    assert_eq!(
        loaded.step_count, 1,
        "step_count should be 1 after first save_step"
    );
}

#[test]
fn save_step_sets_base_commit() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);

    fs::write(dir.path().join("b.txt"), "b").unwrap();

    let strategy = ManualCommitStrategy::new(dir.path());
    let backend = LocalFileBackend::new(dir.path());
    let state = SessionState {
        session_id: "s5".to_string(),
        base_commit: head.clone(),
        phase: crate::engine::session::phase::SessionPhase::Active,
        ..Default::default()
    };
    backend.save_session(&state).unwrap();

    let ctx = StepContext {
        session_id: "s5".to_string(),
        modified_files: vec![],
        new_files: vec!["b.txt".to_string()],
        deleted_files: vec![],
        metadata_dir: String::new(),
        metadata_dir_abs: String::new(),
        commit_message: String::new(),
        transcript_path: String::new(),
        author_name: String::new(),
        author_email: String::new(),
        agent_type: String::new(),
        step_transcript_identifier: String::new(),
        step_transcript_start: 0,
        token_usage: None,
    };
    strategy.save_step(&ctx).unwrap();

    let loaded = backend.load_session("s5").unwrap().unwrap();
    assert_eq!(loaded.base_commit, head, "base_commit should equal HEAD");
}

#[test]
fn initialize_session_sets_pending_prompt_attribution() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let strategy = ManualCommitStrategy::new(dir.path());
    let backend = LocalFileBackend::new(dir.path());

    strategy
        .initialize_session("attr-pending", AGENT_TYPE_CLAUDE_CODE, "", "initial prompt")
        .unwrap();

    let loaded = backend.load_session("attr-pending").unwrap().unwrap();
    assert!(
        loaded.pending_prompt_attribution.is_some(),
        "turn start should always persist pending prompt attribution"
    );
    assert_eq!(
        loaded
            .pending_prompt_attribution
            .as_ref()
            .map(|pa| pa.checkpoint_number),
        Some(1)
    );
}

#[test]
fn save_step_consumes_pending_prompt_attribution() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    fs::write(dir.path().join("tracked.txt"), "line1\nline2\n").unwrap();

    let strategy = ManualCommitStrategy::new(dir.path());
    let backend = LocalFileBackend::new(dir.path());
    backend
        .save_session(&SessionState {
            session_id: "attr-save".to_string(),
            base_commit: head,
            phase: SessionPhase::Active,
            pending_prompt_attribution: Some(SessionPromptAttribution {
                checkpoint_number: 1,
                user_lines_added: 2,
                user_lines_removed: 0,
                agent_lines_added: 0,
                agent_lines_removed: 0,
                user_added_per_file: BTreeMap::from([("tracked.txt".to_string(), 2)]),
            }),
            ..Default::default()
        })
        .unwrap();

    let ctx = StepContext {
        session_id: "attr-save".to_string(),
        modified_files: vec!["tracked.txt".to_string()],
        new_files: vec![],
        deleted_files: vec![],
        metadata_dir: String::new(),
        metadata_dir_abs: String::new(),
        commit_message: String::new(),
        transcript_path: String::new(),
        author_name: String::new(),
        author_email: String::new(),
        agent_type: AGENT_TYPE_CLAUDE_CODE.to_string(),
        step_transcript_identifier: String::new(),
        step_transcript_start: 0,
        token_usage: None,
    };
    strategy.save_step(&ctx).unwrap();

    let loaded = backend.load_session("attr-save").unwrap().unwrap();
    assert!(
        loaded.pending_prompt_attribution.is_none(),
        "pending attribution should be cleared after checkpoint save"
    );
    assert_eq!(
        loaded.prompt_attributions.len(),
        1,
        "saved checkpoint should append prompt attribution"
    );
    assert_eq!(loaded.prompt_attributions[0].user_lines_added, 2);
}

// New test: save_step includes transcript in shadow branch tree.
#[test]
fn save_step_includes_transcript_in_shadow_branch() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);

    // Write a fake transcript file.
    let transcript_path = dir.path().join("transcript.jsonl");
    fs::write(&transcript_path, r#"{"role":"user","content":"hello"}"#).unwrap();

    fs::write(dir.path().join("changed.txt"), "content").unwrap();

    let strategy = ManualCommitStrategy::new(dir.path());
    let backend = LocalFileBackend::new(dir.path());
    let state = SessionState {
        session_id: "s_transcript".to_string(),
        base_commit: head.clone(),
        phase: crate::engine::session::phase::SessionPhase::Active,
        ..Default::default()
    };
    backend.save_session(&state).unwrap();

    let ctx = StepContext {
        session_id: "s_transcript".to_string(),
        modified_files: vec![],
        new_files: vec!["changed.txt".to_string()],
        deleted_files: vec![],
        metadata_dir: String::new(),
        metadata_dir_abs: String::new(),
        commit_message: String::new(),
        transcript_path: transcript_path.to_string_lossy().to_string(),
        author_name: String::new(),
        author_email: String::new(),
        agent_type: String::new(),
        step_transcript_identifier: String::new(),
        step_transcript_start: 0,
        token_usage: None,
    };
    strategy.save_step(&ctx).unwrap();

    // Shadow branch should contain the transcript metadata file.
    let shadow = shadow_branch_ref(&head, "");
    let result = run_git(dir.path(), &["ls-tree", "-r", "--name-only", &shadow]);
    assert!(result.is_ok(), "shadow branch should exist");
    let files = result.unwrap();
    assert!(
        files.contains(".bitloops/metadata/s_transcript/full.jsonl"),
        "shadow branch should contain full.jsonl: {files}"
    );
    assert!(
        files.contains(".bitloops/metadata/s_transcript/prompt.txt"),
        "shadow branch should contain prompt.txt: {files}"
    );
    assert!(
        files.contains(".bitloops/metadata/s_transcript/summary.txt"),
        "shadow branch should contain summary.txt: {files}"
    );
    assert!(
        files.contains(".bitloops/metadata/s_transcript/context.md"),
        "shadow branch should contain context.md: {files}"
    );
}

// New test: save_step with untracked directory does not crash.
#[test]
fn save_step_with_untracked_dir_does_not_crash() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);

    // Create an untracked subdirectory (appears as "dir/" in git status --porcelain).
    let sub = dir.path().join("untracked_dir");
    fs::create_dir_all(&sub).unwrap();
    fs::write(sub.join("file.txt"), "content").unwrap();

    let strategy = ManualCommitStrategy::new(dir.path());
    let backend = LocalFileBackend::new(dir.path());
    let state = SessionState {
        session_id: "s_dir".to_string(),
        base_commit: head.clone(),
        phase: crate::engine::session::phase::SessionPhase::Active,
        ..Default::default()
    };
    backend.save_session(&state).unwrap();

    // Pass empty file lists to exercise the working_tree_changes() fallback.
    let ctx = StepContext {
        session_id: "s_dir".to_string(),
        modified_files: vec![],
        new_files: vec![],
        deleted_files: vec![],
        metadata_dir: String::new(),
        metadata_dir_abs: String::new(),
        commit_message: String::new(),
        transcript_path: String::new(),
        author_name: String::new(),
        author_email: String::new(),
        agent_type: String::new(),
        step_transcript_identifier: String::new(),
        step_transcript_start: 0,
        token_usage: None,
    };
    // Should not panic or return an error.
    let result = strategy.save_step(&ctx);
    assert!(
        result.is_ok(),
        "save_step should not crash with untracked directory: {result:?}"
    );
}

#[test]
fn save_step_no_head_is_noop() {
    let dir = tempfile::tempdir().unwrap();
    setup_empty_git_repo(&dir);

    let strategy = ManualCommitStrategy::new(dir.path());
    let ctx = StepContext {
        session_id: "s_no_head".to_string(),
        modified_files: vec![],
        new_files: vec!["file.txt".to_string()],
        deleted_files: vec![],
        metadata_dir: String::new(),
        metadata_dir_abs: String::new(),
        commit_message: String::new(),
        transcript_path: String::new(),
        author_name: String::new(),
        author_email: String::new(),
        agent_type: String::new(),
        step_transcript_identifier: String::new(),
        step_transcript_start: 0,
        token_usage: None,
    };

    let result = strategy.save_step(&ctx);
    assert!(
        result.is_ok(),
        "save_step should no-op when HEAD is missing: {result:?}"
    );
}

#[test]
fn hash_worktree_id_is_six_chars() {
    for worktree_id in ["", "test-123", "feature/auth-system"] {
        let got = sha256_hex(worktree_id.as_bytes());
        assert_eq!(
            got[..6].len(),
            6,
            "hash prefix should be 6 chars for {worktree_id:?}"
        );
    }
}

#[test]
fn hash_worktree_id_is_deterministic() {
    let id = "test-worktree";
    let h1 = sha256_hex(id.as_bytes());
    let h2 = sha256_hex(id.as_bytes());
    assert_eq!(h1[..6], h2[..6], "hash prefix should be deterministic");
}

#[test]
fn hash_worktree_id_differs_for_different_inputs() {
    let h1 = sha256_hex("worktree-a".as_bytes());
    let h2 = sha256_hex("worktree-b".as_bytes());
    assert_ne!(
        h1[..6],
        h2[..6],
        "different worktrees should hash differently"
    );
}

#[test]
fn shadow_branch_name_for_commit() {
    let cases = [
        (
            "abc1234567890",
            "",
            format!(
                "refs/heads/bitloops/abc1234-{}",
                &sha256_hex("".as_bytes())[..6]
            ),
        ),
        (
            "abc1234567890",
            "test-123",
            format!(
                "refs/heads/bitloops/abc1234-{}",
                &sha256_hex("test-123".as_bytes())[..6]
            ),
        ),
        (
            "abc",
            "wt",
            format!(
                "refs/heads/bitloops/abc-{}",
                &sha256_hex("wt".as_bytes())[..6]
            ),
        ),
    ];

    for (base_commit, worktree_id, expected) in cases {
        let got = shadow_branch_ref(base_commit, worktree_id);
        assert_eq!(
            got, expected,
            "unexpected shadow branch for {base_commit}/{worktree_id}"
        );
    }
}

#[test]
fn parse_shadow_branch_name_cases() {
    let cases = [
        ("bitloops/abc1234-e3b0c4", "abc1234", "e3b0c4", true),
        ("bitloops/abc1234", "abc1234", "", true),
        (
            "bitloops/abcdef1234567890-fedcba",
            "abcdef1234567890",
            "fedcba",
            true,
        ),
        ("main", "", "", false),
        (paths::METADATA_BRANCH_NAME, "checkpoints/v1", "", true),
        ("bitloops/", "", "", true),
    ];

    for (branch, want_commit, want_worktree, want_ok) in cases {
        let (commit, worktree, ok) = parse_shadow_branch_name(branch);
        assert_eq!(ok, want_ok, "ok mismatch for {branch}");
        assert_eq!(commit, want_commit, "commit mismatch for {branch}");
        assert_eq!(worktree, want_worktree, "worktree mismatch for {branch}");
    }
}

#[test]
fn parse_shadow_branch_name_round_trip() {
    for (base_commit, worktree_id) in [
        ("abc1234567890", ""),
        ("abc1234567890", "test-worktree"),
        ("deadbeef", "feature/auth"),
    ] {
        let branch_name = shadow_branch_ref(base_commit, worktree_id);
        let (commit_prefix, worktree_hash, ok) = parse_shadow_branch_name(&branch_name);
        assert!(ok, "parse should succeed for {branch_name}");
        let expected_commit = if base_commit.len() > 7 {
            &base_commit[..7]
        } else {
            base_commit
        };
        assert_eq!(commit_prefix, expected_commit, "commit prefix mismatch");
        assert_eq!(worktree_hash, &sha256_hex(worktree_id.as_bytes())[..6]);
    }
}

#[test]
fn is_shadow_branch_cases() {
    let cases = [
        ("bitloops/abc1234", true),
        ("bitloops/1234567", true),
        ("bitloops/abcdef0123456789abcdef0123456789abcdef01", true),
        ("bitloops/AbCdEf1", true),
        ("bitloops/abc1234-e3b0c4", true),
        ("bitloops/1234567-123456", true),
        ("bitloops/abcdef0123456789-fedcba", true),
        ("bitloops/AbCdEf1-AbCdEf", true),
        ("bitloops/", false),
        ("bitloops/abc123", false),
        ("bitloops/a", false),
        ("bitloops/ghijklm", false),
        (paths::METADATA_BRANCH_NAME, false),
        ("abc1234", false),
        ("feature/abc1234", false),
        ("main", false),
        ("master", false),
        ("", false),
        ("bitloops", false),
        ("bitloops/abc1234-e3b0c", false),
        ("bitloops/abc1234-e3b0c44", false),
        ("bitloops/abc1234-ghijkl", false),
        ("bitloops/-e3b0c4", false),
    ];

    for (branch_name, want) in cases {
        let got = is_shadow_branch(branch_name);
        assert_eq!(got, want, "is_shadow_branch({branch_name:?})");
    }
}

#[test]
fn list_shadow_branches_filters_expected_refs() {
    let dir = tempfile::tempdir().unwrap();
    let _head = setup_git_repo(&dir);

    run_git(dir.path(), &["branch", "bitloops/abc1234-e3b0c4"]).unwrap();
    run_git(dir.path(), &["branch", "bitloops/def5678-f1e2d3"]).unwrap();
    run_git(dir.path(), &["branch", paths::METADATA_BRANCH_NAME]).unwrap();
    run_git(dir.path(), &["branch", "feature/foo"]).unwrap();

    let branches = list_shadow_branches(dir.path()).unwrap();
    assert_eq!(
        branches.len(),
        2,
        "unexpected shadow branches: {branches:?}"
    );
    assert!(branches.contains(&"bitloops/abc1234-e3b0c4".to_string()));
    assert!(branches.contains(&"bitloops/def5678-f1e2d3".to_string()));
    assert!(
        !branches.contains(&paths::METADATA_BRANCH_NAME.to_string()),
        "metadata branch must be excluded"
    );
}

#[test]
fn list_shadow_branches_empty() {
    let dir = tempfile::tempdir().unwrap();
    let _head = setup_git_repo(&dir);

    let branches = list_shadow_branches(dir.path()).unwrap();
    assert!(branches.is_empty(), "expected empty list, got {branches:?}");
}

#[test]
fn delete_shadow_branches_existing() {
    let dir = tempfile::tempdir().unwrap();
    let _head = setup_git_repo(&dir);
    run_git(dir.path(), &["branch", "bitloops/abc1234-e3b0c4"]).unwrap();
    run_git(dir.path(), &["branch", "bitloops/def5678-f1e2d3"]).unwrap();

    let input = vec![
        "bitloops/abc1234-e3b0c4".to_string(),
        "bitloops/def5678-f1e2d3".to_string(),
    ];
    let (deleted, failed) = delete_shadow_branches(dir.path(), &input);
    assert_eq!(deleted.len(), 2);
    assert!(failed.is_empty(), "failed branches: {failed:?}");

    let listed_a = run_git(dir.path(), &["branch", "--list", "bitloops/abc1234-e3b0c4"]).unwrap();
    let listed_b = run_git(dir.path(), &["branch", "--list", "bitloops/def5678-f1e2d3"]).unwrap();
    assert!(listed_a.is_empty(), "branch still exists: {listed_a:?}");
    assert!(listed_b.is_empty(), "branch still exists: {listed_b:?}");
}

#[test]
fn delete_shadow_branches_non_existent() {
    let dir = tempfile::tempdir().unwrap();
    let _head = setup_git_repo(&dir);

    let input = vec!["bitloops/doesnotexist-abc123".to_string()];
    let (deleted, failed) = delete_shadow_branches(dir.path(), &input);
    assert!(
        deleted.is_empty(),
        "deleted unexpected branches: {deleted:?}"
    );
    assert_eq!(failed.len(), 1, "failed branches: {failed:?}");
}

#[test]
fn delete_shadow_branches_empty() {
    let dir = tempfile::tempdir().unwrap();
    let _head = setup_git_repo(&dir);

    let (deleted, failed) = delete_shadow_branches(dir.path(), &[]);
    assert!(deleted.is_empty());
    assert!(failed.is_empty());
}

#[test]
fn list_orphaned_session_states_recent_session_not_orphaned() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let backend = LocalFileBackend::new(dir.path());
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    backend
        .save_session(&SessionState {
            session_id: "recent-session-123".to_string(),
            base_commit: head,
            started_at: now_secs.to_string(),
            step_count: 0,
            ..Default::default()
        })
        .unwrap();

    let orphaned = list_orphaned_session_states(dir.path()).unwrap();
    assert!(
        !orphaned.iter().any(|item| item.id == "recent-session-123"),
        "recent session should not be marked orphaned: {orphaned:?}"
    );
}

#[test]
fn list_orphaned_session_states_shadow_branch_matching() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let backend = LocalFileBackend::new(dir.path());
    let old_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .saturating_sub(3600);

    backend
        .save_session(&SessionState {
            session_id: "session-with-shadow-branch".to_string(),
            base_commit: head.clone(),
            worktree_id: "".to_string(),
            started_at: old_secs.to_string(),
            step_count: 1,
            ..Default::default()
        })
        .unwrap();

    let shadow_ref = shadow_branch_ref(&head, "");
    run_git(dir.path(), &["update-ref", &shadow_ref, &head]).unwrap();

    let shadow_branches = list_shadow_branches(dir.path()).unwrap();
    let expected_short = shadow_ref.strip_prefix("refs/heads/").unwrap().to_string();
    assert!(
        shadow_branches.contains(&expected_short),
        "expected shadow branch not listed: {shadow_branches:?}"
    );

    let orphaned = list_orphaned_session_states(dir.path()).unwrap();
    assert!(
        !orphaned
            .iter()
            .any(|item| item.id == "session-with-shadow-branch"),
        "session with matching shadow branch should not be orphaned: {orphaned:?}"
    );
}

fn checkpoint_id_path(id: &str) -> String {
    let (a, b) = checkpoint_dir_parts(id);
    if b.is_empty() { a } else { format!("{a}/{b}") }
}

fn read_checkpoint_session_metadata_from_branch(
    repo_root: &Path,
    checkpoint_id: &str,
) -> serde_json::Value {
    let (a, b) = checkpoint_dir_parts(checkpoint_id);
    let raw = run_git(
        repo_root,
        &[
            "show",
            &format!(
                "{}:{a}/{b}/0/{}",
                paths::METADATA_BRANCH_NAME,
                paths::METADATA_FILE_NAME
            ),
        ],
    )
    .unwrap();
    serde_json::from_str(&raw).unwrap()
}

fn read_checkpoint_top_metadata_from_branch(
    repo_root: &Path,
    checkpoint_id: &str,
) -> serde_json::Value {
    let (a, b) = checkpoint_dir_parts(checkpoint_id);
    let raw = run_git(
        repo_root,
        &[
            "show",
            &format!(
                "{}:{a}/{b}/{}",
                paths::METADATA_BRANCH_NAME,
                paths::METADATA_FILE_NAME
            ),
        ],
    )
    .unwrap();
    serde_json::from_str(&raw).unwrap()
}

#[test]
fn checkpoint_id_methods() {
    let id = "a1b2c3d4e5f6".to_string();
    assert_eq!(id, "a1b2c3d4e5f6");
    assert!(
        String::new().is_empty(),
        "empty checkpoint id should be empty"
    );
    assert!(
        !id.is_empty(),
        "non-empty checkpoint id should not be empty"
    );
    assert_eq!(checkpoint_id_path(&id), "a1/b2c3d4e5f6");
}

#[test]
fn new_checkpoint_id_validation_via_trailer_parser() {
    let cases = [
        ("a1b2c3d4e5f6", false),
        ("a1b2c3", true),
        ("a1b2c3d4e5f6789012", true),
        ("a1b2c3d4e5gg", true),
        ("A1B2C3D4E5F6", true),
        ("", true),
    ];
    for (input, want_err) in cases {
        let msg = format!("{CHECKPOINT_TRAILER_KEY}: {input}");
        let got = parse_checkpoint_id(&msg);
        if want_err {
            assert!(
                got.is_none(),
                "expected invalid checkpoint id for {input:?}"
            );
        } else {
            assert_eq!(got.as_deref(), Some(input), "valid checkpoint id mismatch");
        }
    }
}

#[test]
fn generate_checkpoint_id_properties() {
    let id = generate_checkpoint_id();
    assert!(
        !id.is_empty(),
        "generated checkpoint id should not be empty"
    );
    assert_eq!(id.len(), 12, "generated checkpoint id should be 12 chars");
    assert!(
        id.chars().all(|c| c.is_ascii_hexdigit()),
        "generated checkpoint id should be hex"
    );
}

#[test]
fn checkpoint_id_path_cases() {
    let cases = [
        ("a1b2c3d4e5f6", "a1/b2c3d4e5f6"),
        ("abcdef123456", "ab/cdef123456"),
        ("", ""),
        ("a", "a"),
        ("ab", "ab"),
        ("abc", "ab/c"),
    ];
    for (input, expected) in cases {
        assert_eq!(
            checkpoint_id_path(input),
            expected,
            "checkpoint path mismatch for {input:?}"
        );
    }
}

#[test]
fn checkpoint_type_values() {
    assert_ne!(
        CheckpointType::Temporary,
        CheckpointType::Committed,
        "temporary and committed checkpoint types should differ"
    );
    let default_type = CheckpointType::default();
    assert_eq!(
        default_type,
        CheckpointType::Temporary,
        "default checkpoint type should be temporary"
    );
}

#[test]
fn checkpoint_info_json_round_trip() {
    let original = CheckpointTopMetadata {
        cli_version: "0.0.3".to_string(),
        checkpoint_id: "a1b2c3d4e5f6".to_string(),
        strategy: "manual-commit".to_string(),
        branch: "main".to_string(),
        checkpoints_count: 2,
        files_touched: vec!["a.rs".to_string()],
        sessions: vec![
            CheckpointSessionRef {
                metadata: "/a1/b2c3d4e5f6/0/metadata.json".to_string(),
                transcript: "/a1/b2c3d4e5f6/0/full.jsonl".to_string(),
                context: "/a1/b2c3d4e5f6/0/context.md".to_string(),
                content_hash: "/a1/b2c3d4e5f6/0/content_hash.txt".to_string(),
                prompt: "/a1/b2c3d4e5f6/0/prompt.txt".to_string(),
            },
            CheckpointSessionRef {
                metadata: "/a1/b2c3d4e5f6/1/metadata.json".to_string(),
                transcript: "/a1/b2c3d4e5f6/1/full.jsonl".to_string(),
                context: "/a1/b2c3d4e5f6/1/context.md".to_string(),
                content_hash: "/a1/b2c3d4e5f6/1/content_hash.txt".to_string(),
                prompt: "/a1/b2c3d4e5f6/1/prompt.txt".to_string(),
            },
        ],
        token_usage: Some(TokenUsageMetadata {
            input_tokens: 150,
            output_tokens: 50,
            api_call_count: 3,
            ..Default::default()
        }),
    };

    let json = serde_json::to_string(&original).unwrap();
    let restored: CheckpointTopMetadata = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.cli_version, "0.0.3");
    assert_eq!(restored.checkpoint_id, "a1b2c3d4e5f6");
    assert_eq!(restored.strategy, "manual-commit");
    assert_eq!(restored.branch, "main");
    assert_eq!(restored.checkpoints_count, 2);
    assert_eq!(restored.files_touched, vec!["a.rs".to_string()]);
    assert_eq!(restored.sessions.len(), 2);
    assert_eq!(
        restored.sessions[0].prompt,
        "/a1/b2c3d4e5f6/0/prompt.txt".to_string()
    );
    assert_eq!(
        restored.sessions[0].content_hash,
        "/a1/b2c3d4e5f6/0/content_hash.txt".to_string()
    );
}

#[test]
fn read_committed_missing_token_usage() {
    let metadata_without_token_usage = serde_json::json!({
        "checkpoint_id": "def456abc123",
        "cli_version": env!("CARGO_PKG_VERSION"),
        "strategy": "manual-commit",
        "checkpoints_count": 1,
        "files_touched": [],
        "sessions": [{
            "metadata": "/de/f456abc123/0/metadata.json",
            "transcript": "/de/f456abc123/0/full.jsonl",
            "context": "/de/f456abc123/0/context.md",
            "content_hash": "/de/f456abc123/0/content_hash.txt",
            "prompt": "/de/f456abc123/0/prompt.txt"
        }]
    })
    .to_string();

    let summary: CheckpointTopMetadata =
        serde_json::from_str(&metadata_without_token_usage).unwrap();
    assert_eq!(summary.checkpoint_id, "def456abc123");
    assert!(summary.token_usage.is_none());
}

#[cfg(unix)]
#[test]
fn write_session_metadata_skips_symlink_transcript() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let sensitive = dir.path().join("sensitive.jsonl");
    fs::write(&sensitive, "SECRET DATA").unwrap();
    let linked = dir.path().join("linked.jsonl");
    symlink(&sensitive, &linked).unwrap();

    let result = write_session_metadata(
        dir.path(),
        "symlink-session",
        linked.to_string_lossy().as_ref(),
    );
    assert!(
        result.is_err(),
        "symlink transcript should be rejected to avoid symlink traversal"
    );
}

#[test]
fn write_committed_agent_field() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let checkpoint_id = "ab1234567890";
    write_committed(
        dir.path(),
        WriteCommittedOptions {
            checkpoint_id: checkpoint_id.to_string(),
            session_id: "agent-field".to_string(),
            strategy: "manual-commit".to_string(),
            agent: AGENT_TYPE_CLAUDE_CODE.to_string(),
            transcript: b"{\"type\":\"assistant\",\"message\":{\"content\":\"agent\"}}\n".to_vec(),
            prompts: Some(vec!["agent prompt".to_string()]),
            context: None,
            checkpoints_count: 1,
            files_touched: vec![],
            token_usage_input: None,
            token_usage_output: None,
            token_usage_api_call_count: None,
            turn_id: String::new(),
            transcript_identifier_at_start: String::new(),
            checkpoint_transcript_start: 0,
            token_usage: None,
            initial_attribution: None,
            author_name: "Test Author".to_string(),
            author_email: "test@example.com".to_string(),
            summary: None,
            is_task: false,
            tool_use_id: String::new(),
            agent_id: String::new(),
            transcript_path: String::new(),
            subagent_transcript_path: String::new(),
        },
    )
    .unwrap();

    let metadata = read_checkpoint_session_metadata_from_branch(dir.path(), checkpoint_id);
    assert_eq!(metadata["agent"], AGENT_TYPE_CLAUDE_CODE);
    assert!(
        metadata.get("checkpoint_number").is_none(),
        "metadata schema does not include checkpoint_number"
    );
    assert!(
        metadata.get("turn_id").is_none(),
        "metadata omits empty turn_id"
    );
    assert!(
        metadata.get("transcript_identifier_at_start").is_none(),
        "metadata omits empty transcript_identifier_at_start"
    );

    let commit_msg = run_git(
        dir.path(),
        &["log", "-1", "--format=%B", paths::METADATA_BRANCH_NAME],
    )
    .unwrap();
    assert!(
        commit_msg.contains(&format!("Bitloops-Agent: {}", AGENT_TYPE_CLAUDE_CODE)),
        "metadata branch commit should contain Bitloops-Agent trailer, got:\n{commit_msg}"
    );
}

#[test]
fn write_temporary_deduplication() {
    with_git_env_cleared(|| {
        let dir = tempfile::tempdir().unwrap();
        let head = setup_git_repo(&dir);
        fs::write(dir.path().join("test.rs"), "package main\n").unwrap();

        let backend = LocalFileBackend::new(dir.path());
        backend
            .save_session(&SessionState {
                session_id: "dedup-session".to_string(),
                phase: crate::engine::session::phase::SessionPhase::Active,
                base_commit: head.clone(),
                ..Default::default()
            })
            .unwrap();

        let strategy = ManualCommitStrategy::new(dir.path());
        let ctx = StepContext {
            session_id: "dedup-session".to_string(),
            modified_files: vec!["test.rs".to_string()],
            new_files: vec![],
            deleted_files: vec![],
            metadata_dir: String::new(),
            metadata_dir_abs: String::new(),
            commit_message: "Checkpoint 1".to_string(),
            transcript_path: String::new(),
            author_name: "Test".to_string(),
            author_email: "test@test.com".to_string(),
            agent_type: "claude-code".to_string(),
            step_transcript_identifier: String::new(),
            step_transcript_start: 0,
            token_usage: None,
        };

        strategy.save_step(&ctx).unwrap();
        let shadow = shadow_branch_ref(&head, "");
        let first_hash = run_git(dir.path(), &["rev-parse", &shadow]).unwrap();

        strategy.save_step(&ctx).unwrap();
        let second_hash = run_git(dir.path(), &["rev-parse", &shadow]).unwrap();
        assert_eq!(
            second_hash, first_hash,
            "identical content should keep the same temporary checkpoint commit hash"
        );

        fs::write(
            dir.path().join("test.rs"),
            "package main\n\nfunc main() {}\n",
        )
        .unwrap();
        strategy.save_step(&ctx).unwrap();
        let third_hash = run_git(dir.path(), &["rev-parse", &shadow]).unwrap();
        assert_ne!(
            third_hash, first_hash,
            "modified content should create a new temporary checkpoint commit"
        );
    });
}

#[test]
fn write_committed_branch_field() {
    // On branch: expect branch field persisted.
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    run_git(dir.path(), &["checkout", "-b", "feature/test-branch"]).unwrap();

    let cp_on = "bc1234567890";
    write_committed(
        dir.path(),
        WriteCommittedOptions {
            checkpoint_id: cp_on.to_string(),
            session_id: "branch-on".to_string(),
            strategy: "manual-commit".to_string(),
            agent: "claude-code".to_string(),
            transcript: b"{\"type\":\"assistant\",\"message\":{\"content\":\"branch\"}}\n".to_vec(),
            prompts: Some(vec!["branch prompt".to_string()]),
            context: None,
            checkpoints_count: 1,
            files_touched: vec![],
            token_usage_input: None,
            token_usage_output: None,
            token_usage_api_call_count: None,
            turn_id: String::new(),
            transcript_identifier_at_start: String::new(),
            checkpoint_transcript_start: 0,
            token_usage: None,
            initial_attribution: None,
            author_name: "Test Author".to_string(),
            author_email: "test@example.com".to_string(),
            summary: None,
            is_task: false,
            tool_use_id: String::new(),
            agent_id: String::new(),
            transcript_path: String::new(),
            subagent_transcript_path: String::new(),
        },
    )
    .unwrap();

    let on_meta = read_checkpoint_session_metadata_from_branch(dir.path(), cp_on);
    assert_eq!(
        on_meta["branch"], "feature/test-branch",
        "branch field should be captured while on branch"
    );

    // Detached HEAD: expect branch field omitted/empty.
    let detached = tempfile::tempdir().unwrap();
    let detached_head = setup_git_repo(&detached);
    run_git(detached.path(), &["checkout", &detached_head]).unwrap();

    let cp_detached = "cd1234567890";
    write_committed(
        detached.path(),
        WriteCommittedOptions {
            checkpoint_id: cp_detached.to_string(),
            session_id: "branch-detached".to_string(),
            strategy: "manual-commit".to_string(),
            agent: "claude-code".to_string(),
            transcript: b"{\"type\":\"assistant\",\"message\":{\"content\":\"detached\"}}\n"
                .to_vec(),
            prompts: Some(vec!["detached prompt".to_string()]),
            context: None,
            checkpoints_count: 1,
            files_touched: vec![],
            token_usage_input: None,
            token_usage_output: None,
            token_usage_api_call_count: None,
            turn_id: String::new(),
            transcript_identifier_at_start: String::new(),
            checkpoint_transcript_start: 0,
            token_usage: None,
            initial_attribution: None,
            author_name: "Test Author".to_string(),
            author_email: "test@example.com".to_string(),
            summary: None,
            is_task: false,
            tool_use_id: String::new(),
            agent_id: String::new(),
            transcript_path: String::new(),
            subagent_transcript_path: String::new(),
        },
    )
    .unwrap();

    let detached_meta = read_checkpoint_session_metadata_from_branch(detached.path(), cp_detached);
    assert!(
        detached_meta.get("branch").is_none() || detached_meta["branch"] == "",
        "branch should be absent/empty in detached HEAD metadata"
    );
}

fn write_session_transcript(repo_root: &Path, session_id: &str, transcript_jsonl: &str) {
    let meta_dir = repo_root.join(paths::session_metadata_dir_from_session_id(session_id));
    fs::create_dir_all(&meta_dir).unwrap();
    fs::write(meta_dir.join(paths::TRANSCRIPT_FILE_NAME), transcript_jsonl).unwrap();
}

fn idle_state(
    session_id: &str,
    base_commit: &str,
    files_touched: Vec<String>,
    step_count: u32,
) -> SessionState {
    SessionState {
        session_id: session_id.to_string(),
        phase: crate::engine::session::phase::SessionPhase::Idle,
        base_commit: base_commit.to_string(),
        files_touched,
        step_count,
        agent_type: "claude-code".to_string(),
        ..Default::default()
    }
}

fn condense_with_transcript(
    strategy: &ManualCommitStrategy,
    state: &mut SessionState,
    checkpoint_id: &str,
    new_head: &str,
    transcript_jsonl: &str,
) {
    write_session_transcript(&strategy.repo_root, &state.session_id, transcript_jsonl);
    strategy
        .condense_session(state, checkpoint_id, new_head)
        .unwrap();
}

#[test]
fn condense_session_files_touched_fallback_empty_state() {
    let dir = tempfile::tempdir().unwrap();
    let base_head = setup_git_repo(&dir);

    fs::write(dir.path().join("agent.rs"), "package main\n").unwrap();
    git_ok(dir.path(), &["add", "agent.rs"]);
    git_ok(dir.path(), &["commit", "-m", "Add agent.rs"]);
    let new_head = git_ok(dir.path(), &["rev-parse", "HEAD"]);

    let strategy = ManualCommitStrategy::new(dir.path());
    let session_id = "test-empty-files";
    let mut state = idle_state(session_id, &base_head, vec![], 1);
    write_session_transcript(
        dir.path(),
        session_id,
        r#"{"type":"human","message":{"content":"create agent.rs"}}
{"type":"assistant","message":{"content":"Done"}}"#,
    );

    let checkpoint_id = "fa11bac00001";
    strategy
        .condense_session(&mut state, checkpoint_id, &new_head)
        .unwrap();

    let metadata = read_checkpoint_session_metadata_from_branch(dir.path(), checkpoint_id);
    let files = metadata["files_touched"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|v| v.as_str().map(ToString::to_string))
        .collect::<Vec<_>>();
    assert_eq!(
        files,
        vec!["agent.rs".to_string()],
        "fallback should use committed files when state.files_touched is empty"
    );
}

#[test]
fn condense_session_files_touched_no_fallback_no_overlap() {
    let dir = tempfile::tempdir().unwrap();
    let base_head = setup_git_repo(&dir);

    fs::write(dir.path().join("session_file.rs"), "package session\n").unwrap();
    fs::write(dir.path().join("other_file.rs"), "package other\n").unwrap();
    git_ok(dir.path(), &["add", "other_file.rs"]);
    git_ok(dir.path(), &["commit", "-m", "Add other_file.rs"]);
    let new_head = git_ok(dir.path(), &["rev-parse", "HEAD"]);

    let strategy = ManualCommitStrategy::new(dir.path());
    let session_id = "test-no-overlap";
    let mut state = idle_state(
        session_id,
        &base_head,
        vec!["session_file.rs".to_string()],
        1,
    );
    write_session_transcript(
        dir.path(),
        session_id,
        r#"{"type":"human","message":{"content":"work on session_file.rs"}}
{"type":"assistant","message":{"content":"Done"}}"#,
    );

    let checkpoint_id = "00001a000001";
    strategy
        .condense_session(&mut state, checkpoint_id, &new_head)
        .unwrap();

    let metadata = read_checkpoint_session_metadata_from_branch(dir.path(), checkpoint_id);
    let files = metadata["files_touched"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|v| v.as_str().map(ToString::to_string))
        .collect::<Vec<_>>();
    assert!(
        files.is_empty(),
        "should not fallback to committed files when session already tracked non-overlapping files: {files:?}"
    );
}

// Committed session metadata keeps turn/transcript start fields and token usage.
#[test]
fn condense_session_writes_turn_and_transcript_start_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let base_head = setup_git_repo(&dir);

    fs::write(dir.path().join("agent.rs"), "package main\n").unwrap();
    git_ok(dir.path(), &["add", "agent.rs"]);
    git_ok(dir.path(), &["commit", "-m", "Add agent.rs"]);
    let new_head = git_ok(dir.path(), &["rev-parse", "HEAD"]);

    let strategy = ManualCommitStrategy::new(dir.path());
    let session_id = "test-turn-and-transcript-start";
    let mut state = idle_state(session_id, &base_head, vec!["agent.rs".to_string()], 1);
    state.turn_id = "turn-123".to_string();
    state.transcript_identifier_at_start = "user-1".to_string();
    state.checkpoint_transcript_start = 1;
    state.transcript_path = "/tmp/transcript-session.jsonl".to_string();
    write_session_transcript(
        dir.path(),
        session_id,
        r#"{"uuid":"user-1","type":"user","message":{"content":"create agent.rs"}}
{"uuid":"assistant-1","type":"assistant","message":{"id":"msg_1","usage":{"input_tokens":8,"output_tokens":5}}}
"#,
    );

    let checkpoint_id = "00aa11bb22cc";
    strategy
        .condense_session(&mut state, checkpoint_id, &new_head)
        .unwrap();

    let metadata = read_checkpoint_session_metadata_from_branch(dir.path(), checkpoint_id);
    assert_eq!(metadata["turn_id"], "turn-123");
    assert_eq!(metadata["transcript_identifier_at_start"], "user-1");
    assert_eq!(metadata["checkpoint_transcript_start"], 1);
    assert_eq!(metadata["transcript_lines_at_start"], 1);
    assert_eq!(metadata["token_usage"]["input_tokens"], 8);
    assert_eq!(metadata["token_usage"]["output_tokens"], 5);
    assert_eq!(metadata["token_usage"]["api_call_count"], 1);
    assert_eq!(metadata["transcript_path"], "/tmp/transcript-session.jsonl");
    assert!(
        metadata.get("initial_attribution").is_some(),
        "manual-commit session metadata should include initial_attribution"
    );
    assert!(metadata["initial_attribution"]["calculated_at"].is_string());
    assert!(
        metadata["initial_attribution"]["agent_lines"]
            .as_i64()
            .unwrap_or_default()
            > 0
    );
    assert!(
        metadata["initial_attribution"]["total_committed"]
            .as_i64()
            .unwrap_or_default()
            > 0
    );
}

#[test]
fn update_summary_updates_session_metadata() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let checkpoint_id = "f1e2d3c4b5a6";
    let session_id = "test-session-summary";
    write_committed(
        dir.path(),
        WriteCommittedOptions {
            checkpoint_id: checkpoint_id.to_string(),
            session_id: session_id.to_string(),
            strategy: "manual-commit".to_string(),
            agent: "claude-code".to_string(),
            transcript:
                b"{\"type\":\"assistant\",\"message\":{\"content\":\"test transcript content\"}}\n"
                    .to_vec(),
            prompts: Some(vec!["summary prompt".to_string()]),
            context: None,
            checkpoints_count: 1,
            files_touched: vec!["file1.rs".to_string(), "file2.rs".to_string()],
            token_usage_input: None,
            token_usage_output: None,
            token_usage_api_call_count: None,
            turn_id: String::new(),
            transcript_identifier_at_start: String::new(),
            checkpoint_transcript_start: 0,
            token_usage: None,
            initial_attribution: None,
            author_name: "Test Author".to_string(),
            author_email: "test@example.com".to_string(),
            summary: None,
            is_task: false,
            tool_use_id: String::new(),
            agent_id: String::new(),
            transcript_path: String::new(),
            subagent_transcript_path: String::new(),
        },
    )
    .unwrap();

    let before = read_checkpoint_session_metadata_from_branch(dir.path(), checkpoint_id);
    assert!(
        before.get("summary").is_none(),
        "initial checkpoint should not have summary field"
    );

    let summary = serde_json::json!({
        "intent": "Test intent",
        "outcome": "Test outcome",
        "learnings": {
            "repo": ["Repo learning 1"],
            "code": [{"path":"file1.rs","line":10,"finding":"Code finding"}],
            "workflow": ["Workflow learning"]
        },
        "friction": ["Some friction"],
        "open_items": ["Open item 1"]
    });

    let result = update_summary(dir.path(), checkpoint_id, summary.clone());
    assert!(
        result.is_ok(),
        "expected update_summary to persist summary into session metadata: {result:?}"
    );

    let after = read_checkpoint_session_metadata_from_branch(dir.path(), checkpoint_id);
    assert_eq!(after["summary"]["intent"], "Test intent");
    assert_eq!(after["summary"]["outcome"], "Test outcome");
    assert_eq!(after["session_id"], session_id);
    assert_eq!(after["files_touched"].as_array().map(Vec::len), Some(2));
}

#[test]
fn update_summary_not_found() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let result = update_summary(
        dir.path(),
        "000000000000",
        serde_json::json!({"intent":"Test","outcome":"Test"}),
    );
    assert!(
        result.is_err(),
        "non-existent checkpoint should return error"
    );
    let msg = format!("{:#}", result.unwrap_err());
    assert!(
        msg.contains("checkpoint not found"),
        "expected checkpoint-not-found error, got: {msg}"
    );
}

#[test]
fn list_committed_falls_back_to_remote() {
    let remote = tempfile::tempdir().unwrap();
    setup_git_repo(&remote);
    let checkpoint_id = "abcdef123456";
    write_committed(
        remote.path(),
        WriteCommittedOptions {
            checkpoint_id: checkpoint_id.to_string(),
            session_id: "remote-session-id".to_string(),
            strategy: "manual-commit".to_string(),
            agent: "claude-code".to_string(),
            transcript:
                b"{\"type\":\"assistant\",\"message\":{\"content\":\"remote transcript\"}}\n"
                    .to_vec(),
            prompts: Some(vec!["remote prompt".to_string()]),
            context: None,
            checkpoints_count: 1,
            files_touched: vec![],
            token_usage_input: None,
            token_usage_output: None,
            token_usage_api_call_count: None,
            turn_id: String::new(),
            transcript_identifier_at_start: String::new(),
            checkpoint_transcript_start: 0,
            token_usage: None,
            initial_attribution: None,
            author_name: "Test Author".to_string(),
            author_email: "test@example.com".to_string(),
            summary: None,
            is_task: false,
            tool_use_id: String::new(),
            agent_id: String::new(),
            transcript_path: String::new(),
            subagent_transcript_path: String::new(),
        },
    )
    .unwrap();

    let local_parent = tempfile::tempdir().unwrap();
    let local_repo = local_parent.path().join("clone");
    let clone = git_command()
        .args([
            "clone",
            &remote.path().to_string_lossy(),
            &local_repo.to_string_lossy(),
        ])
        .output()
        .unwrap();
    assert!(
        clone.status.success(),
        "git clone failed: {}",
        String::from_utf8_lossy(&clone.stderr)
    );

    let fetch_refspec = format!(
        "+refs/heads/{0}:refs/remotes/origin/{0}",
        paths::METADATA_BRANCH_NAME
    );
    run_git(local_repo.as_path(), &["fetch", "origin", &fetch_refspec]).unwrap();

    assert!(
        run_git(
            local_repo.as_path(),
            &["rev-parse", paths::METADATA_BRANCH_NAME]
        )
        .is_err(),
        "local metadata branch should not exist"
    );
    assert!(
        run_git(
            local_repo.as_path(),
            &[
                "rev-parse",
                &format!("refs/remotes/origin/{}", paths::METADATA_BRANCH_NAME),
            ],
        )
        .is_ok(),
        "remote-tracking metadata branch should exist"
    );

    let result = list_committed(local_repo.as_path());
    assert!(
        result.is_ok(),
        "expected list_committed to fall back to remote metadata branch: {result:?}"
    );
    let checkpoints = result.unwrap();
    assert_eq!(checkpoints.len(), 1, "expected one remote checkpoint");
    assert_eq!(checkpoints[0].checkpoint_id, checkpoint_id);
}

#[test]
fn get_checkpoint_author_no_sessions_branch() {
    let dir = tempfile::tempdir().unwrap();
    let init = git_command()
        .args(["init"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(init.status.success());

    let result = get_checkpoint_author(dir.path(), "aabbccddeeff");
    assert!(
        result.is_ok(),
        "expected empty author (no error) when metadata branch is missing: {result:?}"
    );
    let author = result.unwrap();
    assert_eq!(author.name, "");
    assert_eq!(author.email, "");
}

#[test]
fn get_checkpoint_author_returns_author() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let checkpoint_id = "a1b2c3d4e5f6";
    write_committed(
        dir.path(),
        WriteCommittedOptions {
            checkpoint_id: checkpoint_id.to_string(),
            session_id: "author-session".to_string(),
            strategy: "manual-commit".to_string(),
            agent: "claude-code".to_string(),
            transcript:
                b"{\"type\":\"assistant\",\"message\":{\"content\":\"author transcript\"}}\n"
                    .to_vec(),
            prompts: Some(vec!["author prompt".to_string()]),
            context: None,
            checkpoints_count: 1,
            files_touched: vec!["main.rs".to_string()],
            token_usage_input: None,
            token_usage_output: None,
            token_usage_api_call_count: None,
            turn_id: String::new(),
            transcript_identifier_at_start: String::new(),
            checkpoint_transcript_start: 0,
            token_usage: None,
            initial_attribution: None,
            author_name: "Alice Developer".to_string(),
            author_email: "alice@example.com".to_string(),
            summary: None,
            is_task: false,
            tool_use_id: String::new(),
            agent_id: String::new(),
            transcript_path: String::new(),
            subagent_transcript_path: String::new(),
        },
    )
    .unwrap();

    let result = get_checkpoint_author(dir.path(), checkpoint_id);
    assert!(
        result.is_ok(),
        "expected checkpoint author lookup to succeed: {result:?}"
    );
    let author = result.unwrap();
    assert_eq!(author.name, "Alice Developer");
    assert_eq!(author.email, "alice@example.com");
}

#[test]
fn get_checkpoint_author_not_found() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let result = get_checkpoint_author(dir.path(), "ffffffffffff");
    assert!(
        result.is_ok(),
        "expected empty author (no error) for missing checkpoint: {result:?}"
    );
    let author = result.unwrap();
    assert_eq!(author.name, "");
    assert_eq!(author.email, "");
}

#[test]
fn write_committed_multiple_sessions_same_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let checkpoint_id = "a1a2a3a4a5a6";
    let strategy = ManualCommitStrategy::new(dir.path());

    let mut state_one = idle_state("session-one", &head, vec!["file1.rs".to_string()], 3);
    let mut state_two = idle_state("session-two", &head, vec!["file2.rs".to_string()], 2);

    condense_with_transcript(
        &strategy,
        &mut state_one,
        checkpoint_id,
        &head,
        r#"{"role":"assistant","content":"first session"}"#,
    );
    condense_with_transcript(
        &strategy,
        &mut state_two,
        checkpoint_id,
        &head,
        r#"{"role":"assistant","content":"second session"}"#,
    );

    let summary = read_committed(dir.path(), checkpoint_id)
        .unwrap()
        .expect("expected checkpoint summary");
    assert_eq!(summary.sessions.len(), 2, "expected 2 sessions in summary");
    assert!(summary.sessions[0].transcript.contains("/0/"));
    assert!(summary.sessions[1].transcript.contains("/1/"));

    let content0 = read_session_content(dir.path(), checkpoint_id, 0).unwrap();
    assert_eq!(content0.metadata["session_id"], "session-one");
    let content1 = read_session_content(dir.path(), checkpoint_id, 1).unwrap();
    assert_eq!(content1.metadata["session_id"], "session-two");
}

#[test]
fn read_committed_returns_checkpoint_summary() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let checkpoint_id = "c1c2c3c4c5c6";
    let strategy = ManualCommitStrategy::new(dir.path());

    let mut alpha = idle_state("session-alpha", &head, vec!["file0.rs".to_string()], 1);
    let mut beta = idle_state("session-beta", &head, vec!["file1.rs".to_string()], 2);
    condense_with_transcript(
        &strategy,
        &mut alpha,
        checkpoint_id,
        &head,
        r#"{"role":"assistant","content":"alpha"}"#,
    );
    condense_with_transcript(
        &strategy,
        &mut beta,
        checkpoint_id,
        &head,
        r#"{"role":"assistant","content":"beta"}"#,
    );

    let summary = read_committed(dir.path(), checkpoint_id)
        .unwrap()
        .expect("expected checkpoint summary");
    assert_eq!(summary.checkpoint_id, checkpoint_id);
    assert_eq!(summary.strategy, "manual-commit");
    assert_eq!(summary.sessions.len(), 2);
    assert!(summary.sessions[0].metadata.contains("/0/"));
    assert!(summary.sessions[1].metadata.contains("/1/"));
}

#[test]
fn write_committed_aggregation() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let checkpoint_id = "b1b2b3b4b5b6";
    write_committed(
        dir.path(),
        WriteCommittedOptions {
            checkpoint_id: checkpoint_id.to_string(),
            session_id: "session-one".to_string(),
            strategy: "manual-commit".to_string(),
            agent: "claude-code".to_string(),
            transcript: b"{\"message\":\"first\"}\n".to_vec(),
            prompts: Some(vec!["first prompt".to_string()]),
            context: None,
            checkpoints_count: 3,
            files_touched: vec!["a.rs".to_string(), "b.rs".to_string()],
            token_usage_input: Some(100),
            token_usage_output: Some(50),
            token_usage_api_call_count: Some(5),
            turn_id: String::new(),
            transcript_identifier_at_start: String::new(),
            checkpoint_transcript_start: 0,
            token_usage: None,
            initial_attribution: None,
            author_name: "Test Author".to_string(),
            author_email: "test@example.com".to_string(),
            summary: None,
            is_task: false,
            tool_use_id: String::new(),
            agent_id: String::new(),
            transcript_path: String::new(),
            subagent_transcript_path: String::new(),
        },
    )
    .unwrap();
    write_committed(
        dir.path(),
        WriteCommittedOptions {
            checkpoint_id: checkpoint_id.to_string(),
            session_id: "session-two".to_string(),
            strategy: "manual-commit".to_string(),
            agent: "claude-code".to_string(),
            transcript: b"{\"message\":\"second\"}\n".to_vec(),
            prompts: Some(vec!["second prompt".to_string()]),
            context: None,
            checkpoints_count: 2,
            files_touched: vec!["b.rs".to_string(), "c.rs".to_string()],
            token_usage_input: Some(50),
            token_usage_output: Some(25),
            token_usage_api_call_count: Some(3),
            turn_id: String::new(),
            transcript_identifier_at_start: String::new(),
            checkpoint_transcript_start: 0,
            token_usage: None,
            initial_attribution: None,
            author_name: "Test Author".to_string(),
            author_email: "test@example.com".to_string(),
            summary: None,
            is_task: false,
            tool_use_id: String::new(),
            agent_id: String::new(),
            transcript_path: String::new(),
            subagent_transcript_path: String::new(),
        },
    )
    .unwrap();

    let summary = read_committed(dir.path(), checkpoint_id)
        .unwrap()
        .expect("expected checkpoint summary");
    assert_eq!(summary.checkpoints_count, 5);
    assert_eq!(summary.files_touched, vec!["a.rs", "b.rs", "c.rs"]);

    let session_metadata = read_checkpoint_session_metadata_from_branch(dir.path(), checkpoint_id);
    assert!(
        session_metadata.get("token_usage").is_some(),
        "session metadata schema uses nested token_usage object"
    );
    assert!(
        session_metadata.get("token_usage_input").is_none()
            && session_metadata.get("token_usage_output").is_none()
            && session_metadata.get("token_usage_api_call_count").is_none(),
        "session metadata schema does not use flat token usage fields"
    );
    assert_eq!(session_metadata["token_usage"]["input_tokens"], 100);
    assert_eq!(session_metadata["token_usage"]["output_tokens"], 50);
    assert_eq!(session_metadata["token_usage"]["api_call_count"], 5);

    let top_metadata = read_checkpoint_top_metadata_from_branch(dir.path(), checkpoint_id);
    assert!(
        top_metadata.get("token_usage").is_some(),
        "summary schema uses nested token_usage object"
    );
    assert!(
        top_metadata.get("token_usage_input").is_none()
            && top_metadata.get("token_usage_output").is_none()
            && top_metadata.get("token_usage_api_call_count").is_none(),
        "summary schema does not use flat token usage fields"
    );
    assert_eq!(top_metadata["token_usage"]["input_tokens"], 150);
    assert_eq!(top_metadata["token_usage"]["output_tokens"], 75);
    assert_eq!(top_metadata["token_usage"]["api_call_count"], 8);
}

#[test]
fn read_session_content_by_index() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let checkpoint_id = "d1d2d3d4d5d6";
    let strategy = ManualCommitStrategy::new(dir.path());

    let mut first = idle_state("session-first", &head, vec![], 1);
    let mut second = idle_state("session-second", &head, vec![], 1);
    condense_with_transcript(
        &strategy,
        &mut first,
        checkpoint_id,
        &head,
        r#"{"role":"user","content":"First user prompt"}
{"role":"assistant","content":"first"}"#,
    );
    condense_with_transcript(
        &strategy,
        &mut second,
        checkpoint_id,
        &head,
        r#"{"role":"user","content":"Second user prompt"}
{"role":"assistant","content":"second"}"#,
    );

    let content0 = read_session_content(dir.path(), checkpoint_id, 0).unwrap();
    assert_eq!(content0.metadata["session_id"], "session-first");
    assert!(
        content0.transcript.contains("first"),
        "session 0 transcript should contain first"
    );
    assert!(
        content0.prompts.contains("First"),
        "session 0 prompts should contain First"
    );

    let content1 = read_session_content(dir.path(), checkpoint_id, 1).unwrap();
    assert_eq!(content1.metadata["session_id"], "session-second");
    assert!(
        content1.transcript.contains("second"),
        "session 1 transcript should contain second"
    );
}

#[test]
fn read_session_content_invalid_index() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let checkpoint_id = "e1e2e3e4e5e6";
    let strategy = ManualCommitStrategy::new(dir.path());

    let mut only = idle_state("only-session", &head, vec![], 1);
    condense_with_transcript(
        &strategy,
        &mut only,
        checkpoint_id,
        &head,
        r#"{"single": true}"#,
    );

    let err = read_session_content(dir.path(), checkpoint_id, 1).unwrap_err();
    let msg = format!("{:#}", err);
    assert!(
        msg.contains("session 1 not found"),
        "error should mention session not found, got: {msg}"
    );
}

#[test]
fn read_latest_session_content_returns_latest() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let checkpoint_id = "f1f2f3f4f5f6";
    let strategy = ManualCommitStrategy::new(dir.path());

    for i in 0..3 {
        let session_id = format!("session-{i}");
        let mut state = idle_state(&session_id, &head, vec![], 1);
        condense_with_transcript(
            &strategy,
            &mut state,
            checkpoint_id,
            &head,
            &format!(r#"{{"index": {i}}}"#),
        );
    }

    let content = read_latest_session_content(dir.path(), checkpoint_id).unwrap();
    assert_eq!(content.metadata["session_id"], "session-2");
    assert!(
        content.transcript.contains(r#""index": 2"#),
        "latest session transcript should contain index 2"
    );
}

#[test]
fn read_session_content_by_id_lookup() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let checkpoint_id = "010203040506";
    let strategy = ManualCommitStrategy::new(dir.path());

    for session_id in ["unique-id-alpha", "unique-id-beta"] {
        let mut state = idle_state(session_id, &head, vec![], 1);
        condense_with_transcript(
            &strategy,
            &mut state,
            checkpoint_id,
            &head,
            &format!(r#"{{"session_name": "{session_id}"}}"#),
        );
    }

    let content = read_session_content_by_id(dir.path(), checkpoint_id, "unique-id-beta").unwrap();
    assert_eq!(content.metadata["session_id"], "unique-id-beta");
    assert!(
        content.transcript.contains("unique-id-beta"),
        "transcript should contain the target session id"
    );
}

#[test]
fn read_session_content_by_id_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let checkpoint_id = "111213141516";
    let strategy = ManualCommitStrategy::new(dir.path());

    let mut existing = idle_state("existing-session", &head, vec![], 1);
    condense_with_transcript(
        &strategy,
        &mut existing,
        checkpoint_id,
        &head,
        r#"{"exists": true}"#,
    );

    let err =
        read_session_content_by_id(dir.path(), checkpoint_id, "nonexistent-session").unwrap_err();
    let msg = format!("{:#}", err);
    assert!(
        msg.contains("not found"),
        "error should mention not found, got: {msg}"
    );
}

#[test]
fn list_committed_multi_session_info() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let checkpoint_id = "212223242526";
    let strategy = ManualCommitStrategy::new(dir.path());

    let mut one = idle_state("list-session-1", &head, vec!["file0.rs".to_string()], 1);
    let mut two = idle_state("list-session-2", &head, vec!["file1.rs".to_string()], 2);
    two.agent_type = "gemini-cli".to_string();
    condense_with_transcript(&strategy, &mut one, checkpoint_id, &head, r#"{"i": 0}"#);
    condense_with_transcript(&strategy, &mut two, checkpoint_id, &head, r#"{"i": 1}"#);

    let checkpoints = list_committed(dir.path()).unwrap();
    let found = checkpoints
        .into_iter()
        .find(|cp| cp.checkpoint_id == checkpoint_id)
        .expect("checkpoint should be present in list");

    assert_eq!(found.session_count, 2, "SessionCount should be 2");
    assert_eq!(
        found.session_id, "list-session-2",
        "latest session id should be exposed"
    );
    assert_eq!(
        found.agent, "gemini-cli",
        "agent should come from latest session metadata"
    );
    assert_eq!(
        found.agents,
        vec![AGENT_TYPE_CLAUDE_CODE.to_string(), "gemini-cli".to_string()],
        "agents should include all unique session agents in order"
    );
}

#[test]
fn write_committed_session_with_no_prompts() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let checkpoint_id = "313233343536";

    let result = write_committed(
        dir.path(),
        WriteCommittedOptions {
            checkpoint_id: checkpoint_id.to_string(),
            session_id: "no-prompts-session".to_string(),
            strategy: "manual-commit".to_string(),
            agent: "claude-code".to_string(),
            transcript: br#"{"no_prompts": true}"#.to_vec(),
            prompts: None,
            context: Some(b"Some context".to_vec()),
            checkpoints_count: 1,
            files_touched: vec![],
            token_usage_input: None,
            token_usage_output: None,
            token_usage_api_call_count: None,
            turn_id: String::new(),
            transcript_identifier_at_start: String::new(),
            checkpoint_transcript_start: 0,
            token_usage: None,
            initial_attribution: None,
            author_name: "Test Author".to_string(),
            author_email: "test@example.com".to_string(),
            summary: None,
            is_task: false,
            tool_use_id: String::new(),
            agent_id: String::new(),
            transcript_path: String::new(),
            subagent_transcript_path: String::new(),
        },
    );
    assert!(
        result.is_ok(),
        "expected write_committed to succeed for no-prompts session: {result:?}"
    );

    let content = read_session_content(dir.path(), checkpoint_id, 0).unwrap();
    assert_eq!(content.metadata["session_id"], "no-prompts-session");
    assert!(
        !content.transcript.is_empty(),
        "Transcript should not be empty"
    );
    assert_eq!(content.prompts, "", "Prompts should be empty");
    assert_eq!(content.context, "Some context");
}

#[test]
fn write_committed_session_with_summary() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let checkpoint_id = "aabbccddeeff";

    let summary = serde_json::json!({
        "intent": "User wanted to fix a bug",
        "outcome": "Bug was fixed"
    });
    let update = write_committed(
        dir.path(),
        WriteCommittedOptions {
            checkpoint_id: checkpoint_id.to_string(),
            session_id: "summary-session".to_string(),
            strategy: "manual-commit".to_string(),
            agent: "claude-code".to_string(),
            transcript: br#"{"test": true}"#.to_vec(),
            prompts: None,
            context: None,
            checkpoints_count: 1,
            files_touched: vec![],
            token_usage_input: None,
            token_usage_output: None,
            token_usage_api_call_count: None,
            turn_id: String::new(),
            transcript_identifier_at_start: String::new(),
            checkpoint_transcript_start: 0,
            token_usage: None,
            initial_attribution: None,
            author_name: "Test Author".to_string(),
            author_email: "test@example.com".to_string(),
            summary: Some(summary),
            is_task: false,
            tool_use_id: String::new(),
            agent_id: String::new(),
            transcript_path: String::new(),
            subagent_transcript_path: String::new(),
        },
    );
    assert!(
        update.is_ok(),
        "expected write_committed to persist summary metadata: {update:?}"
    );

    let content = read_session_content(dir.path(), checkpoint_id, 0).unwrap();
    assert!(
        !content.metadata["summary"].is_null(),
        "summary should be present in session metadata"
    );
    assert_eq!(
        content.metadata["summary"]["intent"],
        "User wanted to fix a bug"
    );
    assert_eq!(content.metadata["summary"]["outcome"], "Bug was fixed");
}

#[test]
fn write_committed_session_with_no_context() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let checkpoint_id = "414243444546";

    let result = write_committed(
        dir.path(),
        WriteCommittedOptions {
            checkpoint_id: checkpoint_id.to_string(),
            session_id: "no-context-session".to_string(),
            strategy: "manual-commit".to_string(),
            agent: "claude-code".to_string(),
            transcript: br#"{"no_context": true}"#.to_vec(),
            prompts: Some(vec!["A prompt".to_string()]),
            context: None,
            checkpoints_count: 1,
            files_touched: vec![],
            token_usage_input: None,
            token_usage_output: None,
            token_usage_api_call_count: None,
            turn_id: String::new(),
            transcript_identifier_at_start: String::new(),
            checkpoint_transcript_start: 0,
            token_usage: None,
            initial_attribution: None,
            author_name: "Test Author".to_string(),
            author_email: "test@example.com".to_string(),
            summary: None,
            is_task: false,
            tool_use_id: String::new(),
            agent_id: String::new(),
            transcript_path: String::new(),
            subagent_transcript_path: String::new(),
        },
    );
    assert!(
        result.is_ok(),
        "expected write_committed to succeed for no-context session: {result:?}"
    );

    let content = read_session_content(dir.path(), checkpoint_id, 0).unwrap();
    assert_eq!(content.metadata["session_id"], "no-context-session");
    assert!(
        !content.transcript.is_empty(),
        "Transcript should not be empty"
    );
    assert!(
        content.prompts.contains("A prompt"),
        "Prompts should include the user prompt"
    );
    assert_eq!(content.context, "", "Context should be empty");
}

#[test]
fn write_committed_three_sessions() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let checkpoint_id = "515253545556";

    for i in 0..3 {
        let result = write_committed(
            dir.path(),
            WriteCommittedOptions {
                checkpoint_id: checkpoint_id.to_string(),
                session_id: format!("three-session-{i}"),
                strategy: "manual-commit".to_string(),
                agent: "claude-code".to_string(),
                transcript: format!(r#"{{"session_number": {i}}}"#).into_bytes(),
                prompts: None,
                context: None,
                checkpoints_count: (i + 1) as u32,
                files_touched: vec![format!("s{i}.rs")],
                token_usage_input: Some((i as u64 + 1) * 100),
                token_usage_output: Some((i as u64 + 1) * 50),
                token_usage_api_call_count: Some((i as u64 + 1) * 5),
                turn_id: String::new(),
                transcript_identifier_at_start: String::new(),
                checkpoint_transcript_start: 0,
                token_usage: None,
                initial_attribution: None,
                author_name: "Test Author".to_string(),
                author_email: "test@example.com".to_string(),
                summary: None,
                is_task: false,
                tool_use_id: String::new(),
                agent_id: String::new(),
                transcript_path: String::new(),
                subagent_transcript_path: String::new(),
            },
        );
        assert!(
            result.is_ok(),
            "expected write_committed to succeed for session {i}: {result:?}"
        );
    }

    let summary = read_committed(dir.path(), checkpoint_id)
        .unwrap()
        .expect("expected checkpoint summary");
    assert_eq!(summary.sessions.len(), 3, "expected 3 sessions");
    assert_eq!(
        summary.checkpoints_count, 6,
        "expected aggregated checkpoint count"
    );
    assert_eq!(
        summary.files_touched.len(),
        3,
        "expected aggregated files touched"
    );
    let top_metadata = read_checkpoint_top_metadata_from_branch(dir.path(), checkpoint_id);
    assert!(
        top_metadata.get("token_usage").is_some(),
        "summary schema uses nested token_usage object"
    );
    assert!(
        top_metadata.get("token_usage_input").is_none()
            && top_metadata.get("token_usage_output").is_none()
            && top_metadata.get("token_usage_api_call_count").is_none(),
        "summary schema does not use flat token usage fields"
    );
    assert_eq!(
        top_metadata["token_usage"]["input_tokens"], 600,
        "expected aggregated input tokens across sessions"
    );

    for i in 0..3 {
        let content = read_session_content(dir.path(), checkpoint_id, i).unwrap();
        assert_eq!(content.metadata["session_id"], format!("three-session-{i}"));
    }
}

#[test]
fn read_committed_nonexistent_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    run_git(
        dir.path(),
        &[
            "update-ref",
            &format!("refs/heads/{}", paths::METADATA_BRANCH_NAME),
            &head,
        ],
    )
    .unwrap();

    let summary = read_committed(dir.path(), "ffffffffffff").unwrap();
    assert!(
        summary.is_none(),
        "nonexistent checkpoint should return None, not an error"
    );
}

fn create_checkpoint_metadata_dir(repo_root: &Path, session_id: &str) -> String {
    let metadata_dir = repo_root
        .join(".bitloops")
        .join("metadata")
        .join(session_id);
    fs::create_dir_all(&metadata_dir).unwrap();
    fs::write(
        metadata_dir.join(paths::TRANSCRIPT_FILE_NAME),
        r#"{"test": true}"#,
    )
    .unwrap();
    metadata_dir.to_string_lossy().to_string()
}

fn first_checkpoint_opts(
    session_id: &str,
    base_commit: &str,
    metadata_dir_abs: &str,
) -> WriteTemporaryOptions {
    WriteTemporaryOptions {
        session_id: session_id.to_string(),
        base_commit: base_commit.to_string(),
        worktree_id: String::new(),
        modified_files: vec![],
        new_files: vec![],
        deleted_files: vec![],
        metadata_dir: format!(".bitloops/metadata/{session_id}"),
        metadata_dir_abs: metadata_dir_abs.to_string(),
        commit_message: "First checkpoint".to_string(),
        author_name: "Test".to_string(),
        author_email: "test@test.com".to_string(),
        is_first_checkpoint: true,
    }
}

fn default_write_committed_opts(
    checkpoint_id: &str,
    session_id: &str,
    transcript: &str,
) -> WriteCommittedOptions {
    WriteCommittedOptions {
        checkpoint_id: checkpoint_id.to_string(),
        session_id: session_id.to_string(),
        strategy: "manual-commit".to_string(),
        agent: "claude-code".to_string(),
        transcript: transcript.as_bytes().to_vec(),
        prompts: None,
        context: None,
        checkpoints_count: 1,
        files_touched: vec![],
        token_usage_input: None,
        token_usage_output: None,
        token_usage_api_call_count: None,
        turn_id: String::new(),
        transcript_identifier_at_start: String::new(),
        checkpoint_transcript_start: 0,
        token_usage: None,
        initial_attribution: None,
        author_name: "Test Author".to_string(),
        author_email: "test@example.com".to_string(),
        summary: None,
        is_task: false,
        tool_use_id: String::new(),
        agent_id: String::new(),
        transcript_path: String::new(),
        subagent_transcript_path: String::new(),
    }
}

#[test]
fn read_session_content_nonexistent_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    run_git(
        dir.path(),
        &[
            "update-ref",
            &format!("refs/heads/{}", paths::METADATA_BRANCH_NAME),
            &head,
        ],
    )
    .unwrap();

    let result = read_session_content(dir.path(), "eeeeeeeeeeee", 0);
    assert!(result.is_err(), "expected checkpoint-not-found error");
    let msg = format!("{:#}", result.unwrap_err());
    assert!(
        msg.contains("checkpoint not found"),
        "expected checkpoint-not-found error, got: {msg}"
    );
}

#[test]
fn write_temporary_first_checkpoint_captures_modified_tracked_files() {
    let dir = tempfile::tempdir().unwrap();
    let base_commit = setup_git_repo(&dir);
    let modified_content = "# Modified by User\n\nThis change was made before the agent started.\n";
    fs::write(dir.path().join("README.md"), modified_content).unwrap();
    let metadata_dir_abs = create_checkpoint_metadata_dir(dir.path(), "test-session");

    let result = write_temporary(
        dir.path(),
        first_checkpoint_opts("test-session", &base_commit, &metadata_dir_abs),
    )
    .unwrap();
    assert!(!result.skipped, "first checkpoint should not be skipped");

    let content = run_git(
        dir.path(),
        &["show", &format!("{}:README.md", result.commit_hash)],
    )
    .unwrap();
    assert_eq!(content, modified_content);
}

#[test]
fn write_temporary_first_checkpoint_captures_untracked_files() {
    let dir = tempfile::tempdir().unwrap();
    let base_commit = setup_git_repo(&dir);
    let untracked_content = r#"{"key": "secret_value"}"#;
    fs::write(dir.path().join("config.local.json"), untracked_content).unwrap();
    let metadata_dir_abs = create_checkpoint_metadata_dir(dir.path(), "test-session");

    let result = write_temporary(
        dir.path(),
        first_checkpoint_opts("test-session", &base_commit, &metadata_dir_abs),
    )
    .unwrap();
    assert!(
        !result.skipped,
        "first checkpoint with untracked files should not be skipped"
    );

    let content = run_git(
        dir.path(),
        &["show", &format!("{}:config.local.json", result.commit_hash)],
    )
    .unwrap();
    assert_eq!(content, untracked_content);
}

#[test]
fn write_temporary_first_checkpoint_excludes_gitignored_files() {
    let dir = tempfile::tempdir().unwrap();
    let _ = setup_git_repo(&dir);
    fs::write(dir.path().join(".gitignore"), "node_modules/\n").unwrap();
    run_git(dir.path(), &["add", ".gitignore"]).unwrap();
    run_git(dir.path(), &["commit", "-m", "add gitignore"]).unwrap();
    let base_commit = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();

    fs::create_dir_all(dir.path().join("node_modules")).unwrap();
    fs::write(
        dir.path().join("node_modules").join("some-package.js"),
        "module.exports = {}",
    )
    .unwrap();
    let metadata_dir_abs = create_checkpoint_metadata_dir(dir.path(), "test-session");

    let result = write_temporary(
        dir.path(),
        first_checkpoint_opts("test-session", &base_commit, &metadata_dir_abs),
    )
    .unwrap();
    assert!(!result.skipped);

    let ignored = run_git(
        dir.path(),
        &[
            "show",
            &format!("{}:node_modules/some-package.js", result.commit_hash),
        ],
    );
    assert!(
        ignored.is_err(),
        "gitignored file should not be present in checkpoint tree"
    );
}

#[test]
fn write_temporary_first_checkpoint_user_and_agent_changes() {
    with_git_env_cleared(|| {
        let dir = tempfile::tempdir().unwrap();
        let _ = setup_git_repo(&dir);
        fs::write(dir.path().join("main.rs"), "package main\n").unwrap();
        run_git(dir.path(), &["add", "main.rs"]).unwrap();
        run_git(dir.path(), &["commit", "-m", "add main.rs"]).unwrap();
        let base_commit = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();

        let user_modified = "# Modified by User\n";
        fs::write(dir.path().join("README.md"), user_modified).unwrap();
        let agent_modified = "package main\n\nfunc main() {\n\tprintln(\"Hello\")\n}\n";
        fs::write(dir.path().join("main.rs"), agent_modified).unwrap();
        let metadata_dir_abs = create_checkpoint_metadata_dir(dir.path(), "test-session");

        let mut opts = first_checkpoint_opts("test-session", &base_commit, &metadata_dir_abs);
        opts.modified_files = vec!["main.rs".to_string()];
        let result = write_temporary(dir.path(), opts).unwrap();
        assert!(!result.skipped);

        let readme = run_git(
            dir.path(),
            &["show", &format!("{}:README.md", result.commit_hash)],
        )
        .unwrap();
        assert_eq!(readme, user_modified);

        let main_go = run_git(
            dir.path(),
            &["show", &format!("{}:main.rs", result.commit_hash)],
        )
        .unwrap();
        assert_eq!(main_go, agent_modified);
    });
}

#[test]
fn write_temporary_first_checkpoint_captures_user_deleted_files() {
    let dir = tempfile::tempdir().unwrap();
    setup_empty_git_repo(&dir);
    fs::write(dir.path().join("keep.txt"), "keep this").unwrap();
    fs::write(dir.path().join("delete-me.txt"), "delete this").unwrap();
    run_git(dir.path(), &["add", "."]).unwrap();
    run_git(dir.path(), &["commit", "-m", "initial"]).unwrap();
    let base_commit = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();

    fs::remove_file(dir.path().join("delete-me.txt")).unwrap();
    let metadata_dir_abs = create_checkpoint_metadata_dir(dir.path(), "test-session");

    let result = write_temporary(
        dir.path(),
        first_checkpoint_opts("test-session", &base_commit, &metadata_dir_abs),
    )
    .unwrap();
    assert!(!result.skipped);

    let keep = run_git(
        dir.path(),
        &["show", &format!("{}:keep.txt", result.commit_hash)],
    )
    .unwrap();
    assert_eq!(keep, "keep this");

    let deleted = run_git(
        dir.path(),
        &["show", &format!("{}:delete-me.txt", result.commit_hash)],
    );
    assert!(
        deleted.is_err(),
        "user-deleted file should be absent from checkpoint tree"
    );
}

#[test]
fn write_temporary_first_checkpoint_captures_renamed_files() {
    let dir = tempfile::tempdir().unwrap();
    setup_empty_git_repo(&dir);
    fs::write(dir.path().join("old-name.txt"), "content").unwrap();
    run_git(dir.path(), &["add", "old-name.txt"]).unwrap();
    run_git(dir.path(), &["commit", "-m", "initial"]).unwrap();
    let base_commit = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();

    run_git(dir.path(), &["mv", "old-name.txt", "new-name.txt"]).unwrap();
    let metadata_dir_abs = create_checkpoint_metadata_dir(dir.path(), "test-session");

    let result = write_temporary(
        dir.path(),
        first_checkpoint_opts("test-session", &base_commit, &metadata_dir_abs),
    )
    .unwrap();
    assert!(!result.skipped);

    let renamed = run_git(
        dir.path(),
        &["show", &format!("{}:new-name.txt", result.commit_hash)],
    );
    assert!(
        renamed.is_ok(),
        "renamed file should exist in checkpoint tree"
    );

    let old = run_git(
        dir.path(),
        &["show", &format!("{}:old-name.txt", result.commit_hash)],
    );
    assert!(old.is_err(), "old file path should be absent after rename");
}

#[test]
fn write_temporary_first_checkpoint_filenames_with_spaces() {
    let dir = tempfile::tempdir().unwrap();
    setup_empty_git_repo(&dir);
    fs::write(dir.path().join("simple.txt"), "simple").unwrap();
    run_git(dir.path(), &["add", "simple.txt"]).unwrap();
    run_git(dir.path(), &["commit", "-m", "initial"]).unwrap();
    let base_commit = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();

    fs::write(
        dir.path().join("file with spaces.txt"),
        "content with spaces",
    )
    .unwrap();
    let metadata_dir_abs = create_checkpoint_metadata_dir(dir.path(), "test-session");

    let result = write_temporary(
        dir.path(),
        first_checkpoint_opts("test-session", &base_commit, &metadata_dir_abs),
    )
    .unwrap();
    assert!(!result.skipped);

    let with_spaces = run_git(
        dir.path(),
        &[
            "show",
            &format!("{}:file with spaces.txt", result.commit_hash),
        ],
    );
    assert!(
        with_spaces.is_ok(),
        "filename with spaces should be present in checkpoint tree"
    );
}

#[test]
fn write_committed_duplicate_session_id_updates_in_place() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let checkpoint_id = "deda01234567";

    let mut session_x_v1 =
        default_write_committed_opts(checkpoint_id, "session-X", r#"{"message":"session X v1"}"#);
    session_x_v1.files_touched = vec!["a.rs".to_string()];
    session_x_v1.checkpoints_count = 3;
    session_x_v1.token_usage_input = Some(100);
    session_x_v1.token_usage_output = Some(50);
    session_x_v1.token_usage_api_call_count = Some(5);
    let write_x_v1 = write_committed(dir.path(), session_x_v1);
    assert!(
        write_x_v1.is_ok(),
        "session X v1 write should succeed: {write_x_v1:?}"
    );

    let mut session_y =
        default_write_committed_opts(checkpoint_id, "session-Y", r#"{"message":"session Y"}"#);
    session_y.files_touched = vec!["b.rs".to_string()];
    session_y.checkpoints_count = 2;
    session_y.token_usage_input = Some(50);
    session_y.token_usage_output = Some(25);
    session_y.token_usage_api_call_count = Some(3);
    let write_y = write_committed(dir.path(), session_y);
    assert!(
        write_y.is_ok(),
        "session Y write should succeed: {write_y:?}"
    );

    let mut session_x_v2 =
        default_write_committed_opts(checkpoint_id, "session-X", r#"{"message":"session X v2"}"#);
    session_x_v2.files_touched = vec!["a.rs".to_string(), "c.rs".to_string()];
    session_x_v2.checkpoints_count = 5;
    session_x_v2.token_usage_input = Some(200);
    session_x_v2.token_usage_output = Some(100);
    session_x_v2.token_usage_api_call_count = Some(10);
    let write_x_v2 = write_committed(dir.path(), session_x_v2);
    assert!(
        write_x_v2.is_ok(),
        "session X overwrite should succeed: {write_x_v2:?}"
    );

    let summary = read_committed(dir.path(), checkpoint_id)
        .unwrap()
        .expect("summary should exist");
    assert_eq!(
        summary.sessions.len(),
        2,
        "duplicate session should update in place"
    );

    let content0 = read_session_content(dir.path(), checkpoint_id, 0).unwrap();
    assert_eq!(content0.metadata["session_id"], "session-X");
    assert!(content0.transcript.contains("session X v2"));

    let content1 = read_session_content(dir.path(), checkpoint_id, 1).unwrap();
    assert_eq!(content1.metadata["session_id"], "session-Y");

    assert_eq!(summary.checkpoints_count, 7);
    assert_eq!(summary.files_touched, vec!["a.rs", "b.rs", "c.rs"]);
    let top_metadata = read_checkpoint_top_metadata_from_branch(dir.path(), checkpoint_id);
    assert!(
        top_metadata.get("token_usage").is_some(),
        "summary schema uses nested token_usage object"
    );
    assert!(
        top_metadata.get("token_usage_input").is_none()
            && top_metadata.get("token_usage_output").is_none()
            && top_metadata.get("token_usage_api_call_count").is_none(),
        "summary schema does not use flat token usage fields"
    );
    assert_eq!(top_metadata["token_usage"]["input_tokens"], 250);
    assert_eq!(top_metadata["token_usage"]["output_tokens"], 125);
    assert_eq!(top_metadata["token_usage"]["api_call_count"], 13);
}

#[test]
fn write_committed_duplicate_session_id_single_session() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let checkpoint_id = "dedb07654321";

    let mut v1 = default_write_committed_opts(checkpoint_id, "session-X", r#"{"message":"v1"}"#);
    v1.files_touched = vec!["old.rs".to_string()];
    v1.checkpoints_count = 1;
    let write_v1 = write_committed(dir.path(), v1);
    assert!(
        write_v1.is_ok(),
        "initial write should succeed: {write_v1:?}"
    );

    let mut v2 = default_write_committed_opts(checkpoint_id, "session-X", r#"{"message":"v2"}"#);
    v2.files_touched = vec!["new.rs".to_string()];
    v2.checkpoints_count = 5;
    let write_v2 = write_committed(dir.path(), v2);
    assert!(
        write_v2.is_ok(),
        "overwrite write should succeed: {write_v2:?}"
    );

    let summary = read_committed(dir.path(), checkpoint_id)
        .unwrap()
        .expect("summary should exist");
    assert_eq!(
        summary.sessions.len(),
        1,
        "duplicate single session should not append"
    );

    let content = read_session_content(dir.path(), checkpoint_id, 0).unwrap();
    assert_eq!(content.metadata["session_id"], "session-X");
    assert!(
        content.transcript.contains("v2"),
        "session transcript should be overwritten with latest content"
    );

    assert_eq!(summary.checkpoints_count, 5);
    assert_eq!(summary.files_touched, vec!["new.rs"]);
}

#[test]
fn write_committed_duplicate_session_id_reuses_index() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let checkpoint_id = "dedc0abcdef1";

    let mut session_a_v1 = default_write_committed_opts(checkpoint_id, "session-A", r#"{"v": 1}"#);
    session_a_v1.checkpoints_count = 1;
    let write_a_v1 = write_committed(dir.path(), session_a_v1);
    assert!(
        write_a_v1.is_ok(),
        "session A v1 write should succeed: {write_a_v1:?}"
    );

    let mut session_b = default_write_committed_opts(checkpoint_id, "session-B", r#"{"v": 2}"#);
    session_b.checkpoints_count = 1;
    let write_b = write_committed(dir.path(), session_b);
    assert!(
        write_b.is_ok(),
        "session B write should succeed: {write_b:?}"
    );

    let mut session_a_v2 = default_write_committed_opts(checkpoint_id, "session-A", r#"{"v": 3}"#);
    session_a_v2.checkpoints_count = 2;
    let write_a_v2 = write_committed(dir.path(), session_a_v2);
    assert!(
        write_a_v2.is_ok(),
        "session A v2 write should succeed: {write_a_v2:?}"
    );

    let summary = read_committed(dir.path(), checkpoint_id)
        .unwrap()
        .expect("summary should exist");
    assert_eq!(summary.sessions.len(), 2, "session count should remain 2");
    assert!(
        summary.sessions[0].transcript.contains("/0/"),
        "session A should keep index 0 transcript path, got {}",
        summary.sessions[0].transcript
    );
    assert!(
        summary.sessions[1].transcript.contains("/1/"),
        "session B should stay at index 1 transcript path, got {}",
        summary.sessions[1].transcript
    );

    let content0 = read_session_content(dir.path(), checkpoint_id, 0).unwrap();
    assert_eq!(content0.metadata["session_id"], "session-A");
    assert!(
        content0.transcript.contains(r#""v": 3"#),
        "session 0 should hold updated transcript, got {}",
        content0.transcript
    );
}

#[test]
fn write_committed_duplicate_session_id_clears_stale_files() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let checkpoint_id = "dedd0abcdef2";

    let mut session_a_v1 = default_write_committed_opts(checkpoint_id, "session-A", r#"{"v": 1}"#);
    session_a_v1.prompts = Some(vec!["original prompt".to_string()]);
    session_a_v1.context = Some(b"original context".to_vec());
    session_a_v1.checkpoints_count = 1;
    let write_a_v1 = write_committed(dir.path(), session_a_v1);
    assert!(
        write_a_v1.is_ok(),
        "session A v1 write should succeed: {write_a_v1:?}"
    );

    let mut session_b =
        default_write_committed_opts(checkpoint_id, "session-B", r#"{"session":"B"}"#);
    session_b.prompts = Some(vec!["B prompt".to_string()]);
    session_b.context = Some(b"B context".to_vec());
    session_b.checkpoints_count = 1;
    let write_b = write_committed(dir.path(), session_b);
    assert!(
        write_b.is_ok(),
        "session B write should succeed: {write_b:?}"
    );

    let mut session_a_v2 = default_write_committed_opts(checkpoint_id, "session-A", r#"{"v": 2}"#);
    session_a_v2.prompts = None;
    session_a_v2.context = None;
    session_a_v2.checkpoints_count = 2;
    let write_a_v2 = write_committed(dir.path(), session_a_v2);
    assert!(
        write_a_v2.is_ok(),
        "session A v2 write should succeed: {write_a_v2:?}"
    );

    let content_a = read_session_content(dir.path(), checkpoint_id, 0).unwrap();
    assert_eq!(
        content_a.prompts, "",
        "stale prompts should be cleared for overwritten session"
    );
    assert_eq!(
        content_a.context, "",
        "stale context should be cleared for overwritten session"
    );
    assert!(
        content_a.transcript.contains(r#""v": 2"#),
        "session A transcript should be updated, got {}",
        content_a.transcript
    );

    let content_b = read_session_content(dir.path(), checkpoint_id, 1).unwrap();
    assert_eq!(content_b.metadata["session_id"], "session-B");
    assert!(
        content_b.prompts.contains("B prompt"),
        "session B prompts should remain untouched, got {}",
        content_b.prompts
    );
    assert!(
        content_b.context.contains("B context"),
        "session B context should remain untouched, got {}",
        content_b.context
    );
}

#[test]
fn write_committed_redacts_prompt_secrets() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let checkpoint_id = "aabbccddeef2";

    let mut opts =
        default_write_committed_opts(checkpoint_id, "redact-prompt-session", r#"{"msg":"safe"}"#);
    opts.prompts = Some(vec![format!("Set API_KEY={HIGH_ENTROPY_SECRET}")]);
    opts.checkpoints_count = 1;
    let result = write_committed(dir.path(), opts);
    assert!(
        result.is_ok(),
        "write_committed should redact secrets in prompts: {result:?}"
    );

    let content = read_session_content(dir.path(), checkpoint_id, 0).unwrap();
    assert!(
        !content.prompts.contains(HIGH_ENTROPY_SECRET),
        "prompts should not contain secret after redaction"
    );
    assert!(
        content.prompts.contains("REDACTED"),
        "prompts should contain REDACTED placeholder"
    );
}

#[test]
fn write_committed_redacts_transcript_secrets() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let checkpoint_id = "aabbccddeef1";
    let transcript =
        format!(r#"{{"role":"assistant","content":"Here is your key: {HIGH_ENTROPY_SECRET}"}}"#);

    let mut opts =
        default_write_committed_opts(checkpoint_id, "redact-transcript-session", &transcript);
    opts.checkpoints_count = 1;
    let result = write_committed(dir.path(), opts);
    assert!(
        result.is_ok(),
        "write_committed should redact secrets in transcript: {result:?}"
    );

    let content = read_session_content(dir.path(), checkpoint_id, 0).unwrap();
    assert!(
        !content.transcript.contains(HIGH_ENTROPY_SECRET),
        "transcript should not contain secret after redaction"
    );
    assert!(
        content.transcript.contains("REDACTED"),
        "transcript should contain REDACTED placeholder"
    );
}

#[test]
fn write_committed_redacts_context_secrets() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let checkpoint_id = "aabbccddeef3";

    let mut opts =
        default_write_committed_opts(checkpoint_id, "redact-context-session", r#"{"msg":"safe"}"#);
    opts.context = Some(format!("DB_PASSWORD={HIGH_ENTROPY_SECRET}").into_bytes());
    opts.checkpoints_count = 1;
    let result = write_committed(dir.path(), opts);
    assert!(
        result.is_ok(),
        "write_committed should redact secrets in context: {result:?}"
    );

    let content = read_session_content(dir.path(), checkpoint_id, 0).unwrap();
    assert!(
        !content.context.contains(HIGH_ENTROPY_SECRET),
        "context should not contain secret after redaction"
    );
    assert!(
        content.context.contains("REDACTED"),
        "context should contain REDACTED placeholder"
    );
}

#[test]
fn write_committed_cli_version_field() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let checkpoint_id = "b1c2d3e4f5a6";

    let opts =
        default_write_committed_opts(checkpoint_id, "test-session-version", "test transcript");
    let result = write_committed(dir.path(), opts);
    assert!(
        result.is_ok(),
        "write_committed should persist cli_version in root and session metadata: {result:?}"
    );

    let top_meta = read_checkpoint_top_metadata_from_branch(dir.path(), checkpoint_id);
    assert_eq!(top_meta["cli_version"], env!("CARGO_PKG_VERSION"));

    let session_meta = read_checkpoint_session_metadata_from_branch(dir.path(), checkpoint_id);
    assert_eq!(session_meta["cli_version"], env!("CARGO_PKG_VERSION"));
}

#[test]
fn copy_metadata_dir_redacts_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let metadata_dir = dir.path().join("metadata");
    fs::create_dir_all(&metadata_dir).unwrap();
    fs::write(
        metadata_dir.join("agent.jsonl"),
        format!(r#"{{"content":"key={HIGH_ENTROPY_SECRET}"}}"#),
    )
    .unwrap();
    fs::write(
        metadata_dir.join("notes.txt"),
        format!("secret: {HIGH_ENTROPY_SECRET}"),
    )
    .unwrap();

    let result = copy_metadata_dir(&metadata_dir, "cp/");
    assert!(
        result.is_ok(),
        "copy_metadata_dir should redact secrets while copying: {result:?}"
    );
    let entries = result.unwrap();

    assert!(
        entries.contains_key("cp/agent.jsonl"),
        "agent.jsonl should be included in copied entries"
    );
    assert!(
        entries.contains_key("cp/notes.txt"),
        "notes.txt should be included in copied entries"
    );

    for (path, content) in entries {
        assert!(
            !content.contains(HIGH_ENTROPY_SECRET),
            "{path} should not contain the raw secret after redaction"
        );
        assert!(
            content.contains("REDACTED"),
            "{path} should contain REDACTED placeholder"
        );
    }
}

#[test]
fn redact_summary_nil() {
    let result = redact_summary(None).expect("redact_summary(nil) should not error");
    assert!(result.is_none(), "redact_summary(None) should return None");
}

#[test]
fn redact_summary_with_secrets() {
    let summary = Summary {
        intent: format!("Set API_KEY={HIGH_ENTROPY_SECRET}"),
        outcome: format!("Configured key {HIGH_ENTROPY_SECRET} successfully"),
        friction: vec![
            format!("Had to find {HIGH_ENTROPY_SECRET} in env"),
            "No issues here".to_string(),
        ],
        open_items: vec![format!("Rotate {HIGH_ENTROPY_SECRET}")],
        learnings: LearningsSummary {
            repo: vec![format!("Found secret {HIGH_ENTROPY_SECRET} in config")],
            workflow: vec![format!("Use vault for {HIGH_ENTROPY_SECRET}")],
            code: vec![CodeLearning {
                path: "config/secrets.rs".to_string(),
                line: 42,
                end_line: 50,
                finding: format!("Key {HIGH_ENTROPY_SECRET} is hardcoded"),
            }],
        },
    };

    let redacted = redact_summary(Some(&summary))
        .expect("redact_summary should not error")
        .expect("redact_summary should return Some for non-nil input");

    assert!(
        !redacted.intent.contains(HIGH_ENTROPY_SECRET),
        "intent should not contain the secret"
    );
    assert!(
        redacted.intent.contains("REDACTED"),
        "intent should contain REDACTED placeholder"
    );
    assert!(
        !redacted.outcome.contains(HIGH_ENTROPY_SECRET),
        "outcome should not contain the secret"
    );
    assert!(
        !redacted.friction[0].contains(HIGH_ENTROPY_SECRET),
        "friction[0] should not contain the secret"
    );
    assert_eq!(redacted.friction[1], "No issues here");
    assert!(
        !redacted.open_items[0].contains(HIGH_ENTROPY_SECRET),
        "open_items[0] should not contain the secret"
    );
    assert!(
        !redacted.learnings.repo[0].contains(HIGH_ENTROPY_SECRET),
        "learnings.repo[0] should not contain the secret"
    );
    assert!(
        !redacted.learnings.workflow[0].contains(HIGH_ENTROPY_SECRET),
        "learnings.workflow[0] should not contain the secret"
    );

    let code = &redacted.learnings.code[0];
    assert_eq!(code.path, "config/secrets.rs");
    assert_eq!(code.line, 42);
    assert_eq!(code.end_line, 50);
    assert!(
        !code.finding.contains(HIGH_ENTROPY_SECRET),
        "code learning finding should not contain the secret"
    );
    assert!(
        code.finding.contains("REDACTED"),
        "code learning finding should contain REDACTED placeholder"
    );

    assert!(
        summary.intent.contains(HIGH_ENTROPY_SECRET),
        "original summary should remain unmodified"
    );
}

#[test]
fn redact_summary_no_secrets() {
    let summary = Summary {
        intent: "Fix a bug".to_string(),
        outcome: "Bug fixed".to_string(),
        friction: vec!["None".to_string()],
        open_items: vec![],
        learnings: LearningsSummary {
            repo: vec!["Found the pattern".to_string()],
            workflow: vec!["Use TDD".to_string()],
            code: vec![CodeLearning {
                path: "main.rs".to_string(),
                line: 1,
                end_line: 0,
                finding: "Good code".to_string(),
            }],
        },
    };

    let redacted = redact_summary(Some(&summary))
        .expect("redact_summary should not error")
        .expect("redact_summary should return Some for non-nil input");

    assert_eq!(redacted.intent, "Fix a bug");
    assert_eq!(redacted.outcome, "Bug fixed");
    assert_eq!(redacted.learnings.code[0].finding, "Good code");
}

#[test]
fn redact_string_slice_nil_and_empty() {
    let nil_result = redact_string_slice(None).expect("redact_string_slice(nil) should not error");
    assert!(nil_result.is_none(), "nil input should return None");

    let empty: Vec<String> = vec![];
    let empty_result =
        redact_string_slice(Some(&empty)).expect("redact_string_slice(empty) should not error");
    assert!(
        empty_result.is_some(),
        "empty slice should return Some(empty), not None"
    );
    assert_eq!(
        empty_result.unwrap().len(),
        0,
        "empty slice should stay empty"
    );
}

#[test]
fn redact_code_learnings_nil_and_empty() {
    let nil_result =
        redact_code_learnings(None).expect("redact_code_learnings(nil) should not error");
    assert!(nil_result.is_none(), "nil input should return None");

    let empty: Vec<CodeLearning> = vec![];
    let empty_result =
        redact_code_learnings(Some(&empty)).expect("redact_code_learnings(empty) should not error");
    assert!(
        empty_result.is_some(),
        "empty slice should return Some(empty), not None"
    );
    assert_eq!(
        empty_result.unwrap().len(),
        0,
        "empty slice should stay empty"
    );
}

#[test]
fn write_committed_redacts_summary_secrets() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let checkpoint_id = "aabbccddeef7";

    let mut opts =
        default_write_committed_opts(checkpoint_id, "redact-summary-session", r#"{"msg":"safe"}"#);
    opts.checkpoints_count = 1;
    opts.summary = Some(serde_json::json!({
        "intent": format!("Used key {HIGH_ENTROPY_SECRET} to auth"),
        "outcome": format!("Authenticated with {HIGH_ENTROPY_SECRET}")
    }));

    let result = write_committed(dir.path(), opts);
    assert!(
        result.is_ok(),
        "write_committed should redact summary secrets: {result:?}"
    );

    let content = read_session_content(dir.path(), checkpoint_id, 0).unwrap();
    assert!(
        !content.metadata["summary"].is_null(),
        "summary should not be null"
    );
    let intent = content
        .metadata
        .pointer("/summary/intent")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let outcome = content
        .metadata
        .pointer("/summary/outcome")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    assert!(
        !intent.contains(HIGH_ENTROPY_SECRET),
        "summary intent should not contain secret after redaction"
    );
    assert!(
        intent.contains("REDACTED"),
        "summary intent should contain REDACTED placeholder"
    );
    assert!(
        !outcome.contains(HIGH_ENTROPY_SECRET),
        "summary outcome should not contain secret after redaction"
    );
}

#[test]
fn update_summary_redacts_secrets() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let checkpoint_id = "aabbccddeef8";

    let write_result = write_committed(
        dir.path(),
        default_write_committed_opts(checkpoint_id, "update-summary-session", r#"{"msg":"safe"}"#),
    );
    assert!(
        write_result.is_ok(),
        "initial write_committed should succeed before update_summary: {write_result:?}"
    );

    let update_result = update_summary(
        dir.path(),
        checkpoint_id,
        serde_json::json!({
            "intent": format!("Rotated key {HIGH_ENTROPY_SECRET}"),
            "outcome": "Done"
        }),
    );
    assert!(
        update_result.is_ok(),
        "update_summary should redact summary secrets: {update_result:?}"
    );

    let content = read_session_content(dir.path(), checkpoint_id, 0).unwrap();
    let intent = content
        .metadata
        .pointer("/summary/intent")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    assert!(
        !intent.contains(HIGH_ENTROPY_SECRET),
        "updated summary intent should not contain secret"
    );
    assert!(
        intent.contains("REDACTED"),
        "updated summary intent should contain REDACTED placeholder"
    );
}

#[test]
fn write_committed_subagent_transcript_jsonl_fallback() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let checkpoint_id = "aabbccddeef9";

    let transcript_path = dir.path().join("agent.jsonl");
    let invalid_jsonl =
        format!("this is not valid JSON but has a secret {HIGH_ENTROPY_SECRET} in it");
    fs::write(&transcript_path, invalid_jsonl).unwrap();

    let mut opts =
        default_write_committed_opts(checkpoint_id, "jsonl-fallback-session", r#"{"msg":"safe"}"#);
    opts.is_task = true;
    opts.tool_use_id = "toolu_test123".to_string();
    opts.agent_id = "agent1".to_string();
    opts.subagent_transcript_path = transcript_path.to_string_lossy().to_string();

    let result = write_committed(dir.path(), opts);
    assert!(
        result.is_ok(),
        "write_committed should keep subagent transcript and redact via fallback: {result:?}"
    );

    let (a, b) = checkpoint_dir_parts(checkpoint_id);
    let agent_path = format!("{a}/{b}/tasks/toolu_test123/agent-agent1.jsonl");
    let content = run_git(
        dir.path(),
        &[
            "show",
            &format!("{}:{agent_path}", paths::METADATA_BRANCH_NAME),
        ],
    )
    .unwrap();
    assert!(
        !content.is_empty(),
        "subagent transcript should not be empty"
    );
    assert!(
        !content.contains(HIGH_ENTROPY_SECRET),
        "subagent transcript should not contain secret after redaction"
    );
    assert!(
        content.contains("REDACTED"),
        "subagent transcript should contain REDACTED placeholder"
    );
}

#[test]
fn write_temporary_task_subagent_transcript_redacts_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let base_commit = setup_git_repo(&dir);

    let transcript_path = dir.path().join("agent-transcript.jsonl");
    let invalid_jsonl =
        format!("this is not valid JSON but has a secret {HIGH_ENTROPY_SECRET} in it");
    fs::write(&transcript_path, invalid_jsonl).unwrap();

    let result = write_temporary_task(
        dir.path(),
        WriteTemporaryTaskOptions {
            session_id: "test-session".to_string(),
            base_commit: base_commit.clone(),
            worktree_id: String::new(),
            tool_use_id: "toolu_test456".to_string(),
            agent_id: "agent1".to_string(),
            modified_files: vec![],
            new_files: vec![],
            deleted_files: vec![],
            transcript_path: String::new(),
            subagent_transcript_path: transcript_path.to_string_lossy().to_string(),
            checkpoint_uuid: "test-uuid".to_string(),
            is_incremental: false,
            incremental_sequence: 0,
            incremental_type: String::new(),
            incremental_data: String::new(),
            commit_message: "Task checkpoint".to_string(),
            author_name: "Test".to_string(),
            author_email: "test@test.com".to_string(),
        },
    );
    assert!(
        result.is_ok(),
        "write_temporary_task should redact subagent transcript secrets: {result:?}"
    );

    let shadow_branch = shadow_branch_ref(&base_commit, "");
    let agent_path =
        ".bitloops/metadata/test-session/tasks/toolu_test456/agent-agent1.jsonl".to_string();
    let content = run_git(
        dir.path(),
        &["show", &format!("{shadow_branch}:{agent_path}")],
    )
    .unwrap();
    assert!(
        !content.is_empty(),
        "subagent transcript should not be empty"
    );
    assert!(
        !content.contains(HIGH_ENTROPY_SECRET),
        "subagent transcript on shadow branch should not contain secret"
    );
    assert!(
        content.contains("REDACTED"),
        "subagent transcript on shadow branch should contain REDACTED"
    );
}

#[test]
fn add_directory_to_entries_path_traversal() {
    let dir = tempfile::tempdir().unwrap();
    let metadata_dir = dir.path().join("metadata");
    let sub_dir = metadata_dir.join("sub");
    fs::create_dir_all(&sub_dir).unwrap();
    fs::write(sub_dir.join("data.txt"), "safe content").unwrap();

    let result =
        add_directory_to_entries_with_abs_path(&metadata_dir, ".bitloops/metadata/session");
    assert!(
        result.is_ok(),
        "add_directory_to_entries_with_abs_path should include regular files: {result:?}"
    );

    let entries = result.unwrap();
    let expected = ".bitloops/metadata/session/sub/data.txt";
    assert!(
        entries.contains_key(expected),
        "expected entry {expected}, got {entries:?}"
    );
}

#[cfg(unix)]
#[test]
fn add_directory_to_entries_skips_symlinks() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let metadata_dir = dir.path().join("metadata");
    fs::create_dir_all(&metadata_dir).unwrap();
    fs::write(metadata_dir.join("regular.txt"), "regular content").unwrap();
    let sensitive_file = dir.path().join("sensitive.txt");
    fs::write(&sensitive_file, "SECRET DATA").unwrap();
    symlink(&sensitive_file, metadata_dir.join("sneaky-link")).unwrap();

    let result = add_directory_to_entries_with_abs_path(&metadata_dir, "checkpoint/");
    assert!(
        result.is_ok(),
        "add_directory_to_entries_with_abs_path should not fail when symlinks exist: {result:?}"
    );
    let entries = result.unwrap();
    assert!(
        entries.contains_key("checkpoint/regular.txt"),
        "regular file should be included"
    );
    assert!(
        !entries.contains_key("checkpoint/sneaky-link"),
        "symlink should be skipped"
    );
    assert_eq!(entries.len(), 1, "only regular file should be present");
}

#[cfg(unix)]
#[test]
fn add_directory_to_entries_skips_symlinked_directories() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let metadata_dir = dir.path().join("metadata");
    fs::create_dir_all(&metadata_dir).unwrap();
    fs::write(metadata_dir.join("regular.txt"), "regular content").unwrap();

    let external_dir = dir.path().join("external-secrets");
    fs::create_dir_all(&external_dir).unwrap();
    fs::write(external_dir.join("secret.txt"), "SECRET DATA").unwrap();
    symlink(&external_dir, metadata_dir.join("evil-dir-link")).unwrap();

    let result = add_directory_to_entries_with_abs_path(&metadata_dir, "checkpoint/");
    assert!(
        result.is_ok(),
        "add_directory_to_entries_with_abs_path should skip symlinked directories: {result:?}"
    );
    let entries = result.unwrap();
    assert!(
        entries.contains_key("checkpoint/regular.txt"),
        "regular file should be included"
    );
    assert!(
        !entries.contains_key("checkpoint/evil-dir-link/secret.txt"),
        "files inside symlinked directories should be skipped"
    );
    assert_eq!(entries.len(), 1, "only regular file should be present");
}

fn setup_update_committed_fixture_with_sessions(
    dir: &TempDir,
    checkpoint_id: &str,
    session_ids: &[&str],
) {
    if !dir.path().join(".git").exists() {
        setup_git_repo(dir);
    }

    for session_id in session_ids {
        write_committed(
            dir.path(),
            WriteCommittedOptions {
                checkpoint_id: checkpoint_id.to_string(),
                session_id: (*session_id).to_string(),
                strategy: "manual-commit".to_string(),
                agent: AGENT_TYPE_CLAUDE_CODE.to_string(),
                transcript: format!("provisional transcript for {session_id}\n").into_bytes(),
                prompts: Some(vec![format!("initial prompt for {session_id}")]),
                context: Some(format!("initial context for {session_id}").into_bytes()),
                checkpoints_count: 1,
                files_touched: vec!["README.md".to_string()],
                token_usage_input: None,
                token_usage_output: None,
                token_usage_api_call_count: None,
                turn_id: "turn-001".to_string(),
                transcript_identifier_at_start: "transcript-start".to_string(),
                checkpoint_transcript_start: 0,
                token_usage: None,
                initial_attribution: None,
                author_name: "Test".to_string(),
                author_email: "test@test.com".to_string(),
                summary: None,
                is_task: false,
                tool_use_id: String::new(),
                agent_id: String::new(),
                transcript_path: String::new(),
                subagent_transcript_path: String::new(),
            },
        )
        .unwrap();
    }
}

fn setup_update_committed_fixture(dir: &TempDir) -> String {
    let cp = "a1b2c3d4e5f6".to_string();
    setup_update_committed_fixture_with_sessions(dir, &cp, &["session-001"]);
    cp
}

fn read_update_fixture_file(
    dir: &TempDir,
    checkpoint_id: &str,
    session_index: usize,
    file_name: &str,
) -> String {
    let (a, b) = checkpoint_dir_parts(checkpoint_id);
    run_git(
        dir.path(),
        &[
            "show",
            &format!(
                "refs/heads/{}:{a}/{b}/{session_index}/{file_name}",
                paths::METADATA_BRANCH_NAME
            ),
        ],
    )
    .unwrap()
}

#[test]
fn update_committed_replaces_transcript() {
    let dir = tempfile::tempdir().unwrap();
    let cp = setup_update_committed_fixture(&dir);
    let full_transcript =
        "full transcript line 1\nfull transcript line 2\nfull transcript line 3\n";
    let opts = UpdateCommittedOptions {
        checkpoint_id: cp.clone(),
        session_id: "session-001".to_string(),
        transcript: Some(full_transcript.as_bytes().to_vec()),
        prompts: None,
        context: None,
        agent: AGENT_TYPE_CLAUDE_CODE.to_string(),
    };
    update_committed(dir.path(), opts).unwrap();

    let content = read_session_content(dir.path(), &cp, 0).unwrap();
    assert_eq!(content.transcript, full_transcript);
}

#[test]
fn update_committed_replaces_prompts() {
    let dir = tempfile::tempdir().unwrap();
    let cp = setup_update_committed_fixture(&dir);
    let expected_prompts = "prompt 1\n\n---\n\nprompt 2\n\n---\n\nprompt 3";
    update_committed(
        dir.path(),
        UpdateCommittedOptions {
            checkpoint_id: cp.clone(),
            session_id: "session-001".to_string(),
            transcript: None,
            prompts: Some(vec![
                "prompt 1".to_string(),
                "prompt 2".to_string(),
                "prompt 3".to_string(),
            ]),
            context: None,
            agent: AGENT_TYPE_CLAUDE_CODE.to_string(),
        },
    )
    .unwrap();

    let content = read_session_content(dir.path(), &cp, 0).unwrap();
    assert_eq!(content.prompts, expected_prompts);
}

#[test]
fn update_committed_replaces_context() {
    let dir = tempfile::tempdir().unwrap();
    let cp = setup_update_committed_fixture(&dir);
    let expected_context = "updated context with full session info";
    update_committed(
        dir.path(),
        UpdateCommittedOptions {
            checkpoint_id: cp.clone(),
            session_id: "session-001".to_string(),
            transcript: None,
            prompts: None,
            context: Some(expected_context.as_bytes().to_vec()),
            agent: AGENT_TYPE_CLAUDE_CODE.to_string(),
        },
    )
    .unwrap();

    let content = read_session_content(dir.path(), &cp, 0).unwrap();
    assert_eq!(content.context, expected_context);
}

#[test]
fn update_committed_replaces_all_fields_together() {
    let dir = tempfile::tempdir().unwrap();
    let cp = setup_update_committed_fixture(&dir);
    let expected_transcript = "complete transcript\n";
    let expected_prompts = "final prompt";
    let expected_context = "final context";
    update_committed(
        dir.path(),
        UpdateCommittedOptions {
            checkpoint_id: cp.clone(),
            session_id: "session-001".to_string(),
            transcript: Some(expected_transcript.as_bytes().to_vec()),
            prompts: Some(vec!["final prompt".to_string()]),
            context: Some(expected_context.as_bytes().to_vec()),
            agent: AGENT_TYPE_CLAUDE_CODE.to_string(),
        },
    )
    .unwrap();

    let content = read_session_content(dir.path(), &cp, 0).unwrap();
    assert_eq!(content.transcript, expected_transcript);
    assert_eq!(content.prompts, expected_prompts);
    assert_eq!(content.context, expected_context);
}

#[test]
fn update_committed_nonexistent_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let result = update_committed(
        dir.path(),
        UpdateCommittedOptions {
            checkpoint_id: "deadbeef1234".to_string(),
            session_id: "session-001".to_string(),
            transcript: Some(b"should fail".to_vec()),
            prompts: None,
            context: None,
            agent: AGENT_TYPE_CLAUDE_CODE.to_string(),
        },
    );
    assert!(result.is_err(), "expected nonexistent checkpoint error");
}

#[test]
fn update_committed_preserves_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let cp = setup_update_committed_fixture(&dir);
    let before = read_session_content(dir.path(), &cp, 0).unwrap();

    update_committed(
        dir.path(),
        UpdateCommittedOptions {
            checkpoint_id: cp.clone(),
            session_id: "session-001".to_string(),
            transcript: Some(b"updated transcript\n".to_vec()),
            prompts: None,
            context: None,
            agent: AGENT_TYPE_CLAUDE_CODE.to_string(),
        },
    )
    .unwrap();

    let after = read_session_content(dir.path(), &cp, 0).unwrap();
    assert_eq!(after.metadata["session_id"], before.metadata["session_id"]);
    assert_eq!(after.metadata["strategy"], before.metadata["strategy"]);
}

#[test]
fn update_committed_multiple_checkpoints() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let cp1 = "a1b2c3d4e5f6".to_string();
    let cp2 = "b2c3d4e5f6a1".to_string();
    setup_update_committed_fixture_with_sessions(&dir, &cp1, &["session-001"]);
    setup_update_committed_fixture_with_sessions(&dir, &cp2, &["session-001"]);

    let full_transcript = "complete full transcript\n";
    for checkpoint_id in [&cp1, &cp2] {
        update_committed(
            dir.path(),
            UpdateCommittedOptions {
                checkpoint_id: checkpoint_id.to_string(),
                session_id: "session-001".to_string(),
                transcript: Some(full_transcript.as_bytes().to_vec()),
                prompts: Some(vec![
                    "final prompt 1".to_string(),
                    "final prompt 2".to_string(),
                ]),
                context: Some(b"final context".to_vec()),
                agent: AGENT_TYPE_CLAUDE_CODE.to_string(),
            },
        )
        .unwrap();
    }

    for checkpoint_id in [&cp1, &cp2] {
        let content = read_session_content(dir.path(), checkpoint_id, 0).unwrap();
        assert_eq!(content.transcript, full_transcript);
    }
}

#[test]
fn update_committed_updates_content_hash() {
    let dir = tempfile::tempdir().unwrap();
    let cp = setup_update_committed_fixture(&dir);
    let old_hash = read_update_fixture_file(&dir, &cp, 0, paths::CONTENT_HASH_FILE_NAME);
    let new_transcript = "new full transcript content\n";

    update_committed(
        dir.path(),
        UpdateCommittedOptions {
            checkpoint_id: cp.clone(),
            session_id: "session-001".to_string(),
            transcript: Some(new_transcript.as_bytes().to_vec()),
            prompts: None,
            context: None,
            agent: AGENT_TYPE_CLAUDE_CODE.to_string(),
        },
    )
    .unwrap();

    let new_hash = read_update_fixture_file(&dir, &cp, 0, paths::CONTENT_HASH_FILE_NAME);
    assert!(new_hash.starts_with("sha256:"));
    assert_ne!(new_hash, old_hash);
    assert_eq!(
        new_hash,
        format!("sha256:{}", sha256_hex(new_transcript.as_bytes()))
    );
}

#[test]
fn update_committed_empty_checkpoint_id() {
    let dir = tempfile::tempdir().unwrap();
    let result = update_committed(
        dir.path(),
        UpdateCommittedOptions {
            checkpoint_id: String::new(),
            session_id: "session-001".to_string(),
            transcript: Some(b"should fail".to_vec()),
            prompts: None,
            context: None,
            agent: AGENT_TYPE_CLAUDE_CODE.to_string(),
        },
    );
    assert!(result.is_err(), "expected error for empty checkpoint id");
}

#[test]
fn update_committed_falls_back_to_latest_session() {
    let dir = tempfile::tempdir().unwrap();
    let cp = "f1e2d3c4b5a6".to_string();
    setup_update_committed_fixture_with_sessions(&dir, &cp, &["session-001", "session-002"]);
    let session0_before = read_update_fixture_file(&dir, &cp, 0, paths::TRANSCRIPT_FILE_NAME);
    let updated = "updated via fallback\n";

    update_committed(
        dir.path(),
        UpdateCommittedOptions {
            checkpoint_id: cp.clone(),
            session_id: "nonexistent-session".to_string(),
            transcript: Some(updated.as_bytes().to_vec()),
            prompts: None,
            context: None,
            agent: AGENT_TYPE_CLAUDE_CODE.to_string(),
        },
    )
    .unwrap();

    assert_eq!(
        read_update_fixture_file(&dir, &cp, 1, paths::TRANSCRIPT_FILE_NAME),
        updated
    );
    assert_eq!(
        read_update_fixture_file(&dir, &cp, 0, paths::TRANSCRIPT_FILE_NAME),
        session0_before
    );
}

#[test]
fn update_committed_summary_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let cp = setup_update_committed_fixture(&dir);
    let before = read_committed(dir.path(), &cp).unwrap().unwrap();

    update_committed(
        dir.path(),
        UpdateCommittedOptions {
            checkpoint_id: cp.clone(),
            session_id: "session-001".to_string(),
            transcript: Some(b"updated\n".to_vec()),
            prompts: None,
            context: None,
            agent: AGENT_TYPE_CLAUDE_CODE.to_string(),
        },
    )
    .unwrap();

    let after = read_committed(dir.path(), &cp).unwrap().unwrap();
    assert_eq!(after.checkpoint_id, before.checkpoint_id);
    assert_eq!(after.sessions.len(), before.sessions.len());
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct StateSnippet {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    turn_checkpoint_ids: Vec<String>,
}

#[test]
fn state_turn_checkpoint_ids_json() {
    let original = StateSnippet {
        turn_checkpoint_ids: vec!["a1b2c3d4e5f6".to_string(), "b2c3d4e5f6a1".to_string()],
    };
    let data = serde_json::to_string(&original).unwrap();
    let decoded: StateSnippet = serde_json::from_str(&data).unwrap();
    assert_eq!(decoded.turn_checkpoint_ids.len(), 2);

    let empty = StateSnippet::default();
    let empty_data = serde_json::to_string(&empty).unwrap();
    assert_eq!(empty_data, "{}");
}

#[test]
fn update_committed_uses_correct_author() {
    struct Case<'a> {
        name: &'a str,
        local_name: Option<&'a str>,
        local_email: Option<&'a str>,
        global_name: Option<&'a str>,
        global_email: Option<&'a str>,
        want_name: &'a str,
        want_email: &'a str,
    }
    let cases = [
        Case {
            name: "global config only",
            local_name: None,
            local_email: None,
            global_name: Some("Global User"),
            global_email: Some("global@example.com"),
            want_name: "Global User",
            want_email: "global@example.com",
        },
        Case {
            name: "local config takes precedence",
            local_name: Some("Local User"),
            local_email: Some("local@example.com"),
            global_name: Some("Global User"),
            global_email: Some("global@example.com"),
            want_name: "Local User",
            want_email: "local@example.com",
        },
    ];

    for case in cases {
        let home = tempfile::tempdir().unwrap();
        with_env_var("HOME", Some(home.path().to_string_lossy().as_ref()), || {
            let mut global_cfg = String::from("[user]\n");
            if let Some(name) = case.global_name {
                global_cfg.push_str(&format!("\tname = {name}\n"));
            }
            if let Some(email) = case.global_email {
                global_cfg.push_str(&format!("\temail = {email}\n"));
            }
            fs::write(home.path().join(".gitconfig"), global_cfg).unwrap();

            let dir = tempfile::tempdir().unwrap();
            setup_git_repo(&dir);
            run_git(dir.path(), &["config", "--unset", "user.name"]).ok();
            run_git(dir.path(), &["config", "--unset", "user.email"]).ok();
            if let Some(name) = case.local_name {
                run_git(dir.path(), &["config", "user.name", name]).unwrap();
            }
            if let Some(email) = case.local_email {
                run_git(dir.path(), &["config", "user.email", email]).unwrap();
            }

            let cp = "a1b2c3d4e5f6";
            setup_update_committed_fixture_with_sessions(&dir, cp, &["session-001"]);

            let result = update_committed(
                dir.path(),
                UpdateCommittedOptions {
                    checkpoint_id: cp.to_string(),
                    session_id: "session-001".to_string(),
                    transcript: Some(b"full transcript\n".to_vec()),
                    prompts: None,
                    context: None,
                    agent: AGENT_TYPE_CLAUDE_CODE.to_string(),
                },
            );
            assert!(
                result.is_ok(),
                "expected update_committed to succeed for case '{}': {result:?}",
                case.name
            );

            let author_line = run_git(
                dir.path(),
                &[
                    "log",
                    "-1",
                    "--format=%an|%ae",
                    &format!("refs/heads/{}", paths::METADATA_BRANCH_NAME),
                ],
            )
            .unwrap();
            let mut parts = author_line.split('|');
            let got_name = parts.next().unwrap_or_default().trim();
            let got_email = parts.next().unwrap_or_default().trim();
            assert_eq!(
                got_name, case.want_name,
                "name mismatch for case '{}'",
                case.name
            );
            assert_eq!(
                got_email, case.want_email,
                "email mismatch for case '{}'",
                case.name
            );
        });
    }
}

#[test]
fn get_git_author_from_repo_global_fallback() {
    let home = tempfile::tempdir().unwrap();
    with_env_var("HOME", Some(home.path().to_string_lossy().as_ref()), || {
        fs::write(
            home.path().join(".gitconfig"),
            "[user]\n\tname = Global Author\n\temail = global@test.com\n",
        )
        .unwrap();

        let dir = tempfile::tempdir().unwrap();
        setup_git_repo(&dir);
        run_git(dir.path(), &["config", "--unset", "user.name"]).ok();
        run_git(dir.path(), &["config", "--unset", "user.email"]).ok();

        let author = get_git_author_from_repo(dir.path());
        assert!(
            author.is_ok(),
            "expected global git config fallback, got {author:?}"
        );
        let (name, email) = author.unwrap();
        assert_eq!(name, "Global Author");
        assert_eq!(email, "global@test.com");
    });
}

#[test]
fn get_git_author_from_repo_no_config() {
    let home = tempfile::tempdir().unwrap();
    with_env_var("HOME", Some(home.path().to_string_lossy().as_ref()), || {
        let dir = tempfile::tempdir().unwrap();
        setup_git_repo(&dir);
        run_git(dir.path(), &["config", "--unset", "user.name"]).ok();
        run_git(dir.path(), &["config", "--unset", "user.email"]).ok();

        let author = get_git_author_from_repo(dir.path());
        assert!(
            author.is_ok(),
            "expected defaults when no git config exists, got {author:?}"
        );
        let (name, email) = author.unwrap();
        assert_eq!(name, "Unknown");
        assert_eq!(email, "unknown@local");
    });
}

#[test]
fn prepare_commit_msg_adds_trailer() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    // Create an active session.
    let backend = LocalFileBackend::new(dir.path());
    let state = SessionState {
        session_id: "sa1".to_string(),
        phase: crate::engine::session::phase::SessionPhase::Active,
        base_commit: "abc1234".to_string(),
        ..Default::default()
    };
    backend.save_session(&state).unwrap();

    let strategy = ManualCommitStrategy::new(dir.path());
    let msg_file = dir.path().join("COMMIT_EDITMSG");
    fs::write(&msg_file, "fix: my change\n").unwrap();

    strategy.prepare_commit_msg(&msg_file, None).unwrap();

    let content = fs::read_to_string(&msg_file).unwrap();
    assert!(
        content.contains(CHECKPOINT_TRAILER_KEY),
        "trailer should be added: {content}"
    );
}

#[test]
fn add_checkpoint_trailer_no_comment() {
    let msg = "feat: implement parser\n";
    let out = add_checkpoint_trailer(msg, "abc123def456");
    assert!(out.contains("feat: implement parser"));
    assert!(out.contains("Bitloops-Checkpoint: abc123def456"));
}

#[test]
fn add_checkpoint_trailer_with_comment_has_comment() {
    let msg = "feat: implement parser\n\nDetailed body line\n";
    let out = add_checkpoint_trailer(msg, "abc123def456");
    assert!(out.contains("Detailed body line"));
    assert!(out.contains("Bitloops-Checkpoint: abc123def456"));
}

#[test]
fn add_checkpoint_trailer_with_comment_no_prompt() {
    let msg = "";
    let out = add_checkpoint_trailer(msg, "abc123def456");
    assert!(out.contains("Bitloops-Checkpoint: abc123def456"));
}

#[test]
fn add_checkpoint_trailer_conventional_commit_subject() {
    let msg = "fix(auth): handle nil token\n";
    let out = add_checkpoint_trailer(msg, "abc123def456");
    assert!(out.starts_with("fix(auth): handle nil token"));
    assert!(out.contains("Bitloops-Checkpoint: abc123def456"));
}

#[test]
fn add_checkpoint_trailer_existing_trailers() {
    let msg = "feat: update\n\nSigned-off-by: Dev <dev@test.com>\n";
    let out = add_checkpoint_trailer(msg, "abc123def456");
    assert!(out.contains("Signed-off-by: Dev <dev@test.com>"));
    assert!(out.contains("Bitloops-Checkpoint: abc123def456"));
}

#[test]
fn prepare_commit_msg_skips_merge() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    let strategy = ManualCommitStrategy::new(dir.path());
    let msg_file = dir.path().join("MERGE_MSG");
    let original = "Merge branch 'feature'\n";
    fs::write(&msg_file, original).unwrap();

    strategy
        .prepare_commit_msg(&msg_file, Some("merge"))
        .unwrap();

    let content = fs::read_to_string(&msg_file).unwrap();
    assert_eq!(
        content, original,
        "merge commit message should be unchanged"
    );
}

#[test]
fn prepare_commit_msg_amend_preserves_trailer() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    let strategy = ManualCommitStrategy::new(dir.path());
    let msg_file = dir.path().join("COMMIT_EDITMSG");
    let existing_msg = "fix: my change\n\nBitloops-Checkpoint: abcdef123456\n";
    fs::write(&msg_file, existing_msg).unwrap();

    strategy
        .prepare_commit_msg(&msg_file, Some("commit"))
        .unwrap();

    let content = fs::read_to_string(&msg_file).unwrap();
    assert_eq!(
        content, existing_msg,
        "existing trailer should be preserved on amend"
    );
}

#[test]
fn prepare_commit_msg_amend_restores_trailer_from_last_checkpoint_id() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    let backend = LocalFileBackend::new(dir.path());
    backend
        .save_session(&SessionState {
            session_id: "amend-restore".to_string(),
            phase: crate::engine::session::phase::SessionPhase::Active,
            last_checkpoint_id: "abc123def456".to_string(),
            ..Default::default()
        })
        .unwrap();

    let strategy = ManualCommitStrategy::new(dir.path());
    let msg_file = dir.path().join("COMMIT_EDITMSG");
    fs::write(&msg_file, "New amended message\n").unwrap();

    strategy
        .prepare_commit_msg(&msg_file, Some("commit"))
        .unwrap();

    let content = fs::read_to_string(&msg_file).unwrap();
    assert_eq!(
        parse_checkpoint_id(&content).as_deref(),
        Some("abc123def456"),
        "amend should restore trailer from last_checkpoint_id"
    );
}

#[test]
fn prepare_commit_msg_amend_no_trailer_no_last_checkpoint_id() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    let backend = LocalFileBackend::new(dir.path());
    backend
        .save_session(&SessionState {
            session_id: "amend-no-id".to_string(),
            phase: crate::engine::session::phase::SessionPhase::Active,
            ..Default::default()
        })
        .unwrap();

    let strategy = ManualCommitStrategy::new(dir.path());
    let msg_file = dir.path().join("COMMIT_EDITMSG");
    let original = "Amended without checkpoint context\n";
    fs::write(&msg_file, original).unwrap();

    strategy
        .prepare_commit_msg(&msg_file, Some("commit"))
        .unwrap();

    let content = fs::read_to_string(&msg_file).unwrap();
    assert_eq!(
        content, original,
        "amend should not add a trailer when last_checkpoint_id is empty"
    );
    assert!(
        parse_checkpoint_id(&content).is_none(),
        "checkpoint trailer should remain absent"
    );
}

#[test]
fn prepare_commit_msg_noop_no_session() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    // No sessions exist.
    let strategy = ManualCommitStrategy::new(dir.path());
    let msg_file = dir.path().join("COMMIT_EDITMSG");
    let original = "chore: no session active\n";
    fs::write(&msg_file, original).unwrap();

    strategy.prepare_commit_msg(&msg_file, None).unwrap();

    let content = fs::read_to_string(&msg_file).unwrap();
    assert_eq!(
        content, original,
        "message should be unchanged when no sessions exist"
    );
}

#[test]
fn prepare_commit_msg_skips_idle_sessions_without_pending_steps() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    let backend = LocalFileBackend::new(dir.path());
    backend
        .save_session(&SessionState {
            session_id: "idle-no-steps".to_string(),
            phase: crate::engine::session::phase::SessionPhase::Idle,
            step_count: 0,
            ..Default::default()
        })
        .unwrap();

    let strategy = ManualCommitStrategy::new(dir.path());
    let msg_file = dir.path().join("COMMIT_EDITMSG");
    let original = "docs: unrelated follow-up commit\n";
    fs::write(&msg_file, original).unwrap();

    strategy.prepare_commit_msg(&msg_file, None).unwrap();

    let content = fs::read_to_string(&msg_file).unwrap();
    assert_eq!(
        content, original,
        "idle sessions with no pending steps should not get new trailers"
    );
}

#[test]
fn commit_msg_strips_empty_commit() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    let strategy = ManualCommitStrategy::new(dir.path());
    let msg_file = dir.path().join("COMMIT_EDITMSG");
    // Only trailer, no user content.
    fs::write(&msg_file, "Bitloops-Checkpoint: abcdef123456\n").unwrap();

    strategy.commit_msg(&msg_file).unwrap();

    let content = fs::read_to_string(&msg_file).unwrap();
    assert!(
        !content.contains(CHECKPOINT_TRAILER_KEY),
        "trailer should be stripped from empty commit: {content}"
    );
}

#[test]
fn commit_msg_keeps_real_message() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    let strategy = ManualCommitStrategy::new(dir.path());
    let msg_file = dir.path().join("COMMIT_EDITMSG");
    let msg = "fix: real change\n\nBitloops-Checkpoint: abcdef123456\n";
    fs::write(&msg_file, msg).unwrap();

    strategy.commit_msg(&msg_file).unwrap();

    let content = fs::read_to_string(&msg_file).unwrap();
    assert!(
        content.contains("fix: real change"),
        "user message should be preserved: {content}"
    );
    assert!(
        content.contains(CHECKPOINT_TRAILER_KEY),
        "trailer should be preserved when user content exists: {content}"
    );
}

#[test]
fn post_commit_creates_checkpoint_branch() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);

    // Create a session with active state.
    let backend = LocalFileBackend::new(dir.path());
    let state = SessionState {
        session_id: "pc1".to_string(),
        phase: crate::engine::session::phase::SessionPhase::Idle,
        base_commit: head.clone(),
        agent_type: "claude-code".to_string(),
        first_prompt: "test prompt".to_string(),
        step_count: 1,
        files_touched: vec!["change.txt".to_string()],
        ..Default::default()
    };
    backend.save_session(&state).unwrap();

    // Make a commit with a Bitloops-Checkpoint trailer.
    fs::write(dir.path().join("change.txt"), "change").unwrap();
    git_command()
        .args(["add", "."])
        .current_dir(dir.path())
        .output()
        .unwrap();
    git_command()
        .args([
            "commit",
            "-m",
            "fix: something\n\nBitloops-Checkpoint: abc123456789",
        ])
        .current_dir(dir.path())
        .output()
        .unwrap();

    let strategy = ManualCommitStrategy::new(dir.path());
    strategy.post_commit().unwrap();

    // bitloops/checkpoints/v1 should now exist.
    let result = run_git(dir.path(), &["rev-parse", "bitloops/checkpoints/v1"]);
    assert!(
        result.is_ok(),
        "bitloops/checkpoints/v1 should exist after post_commit"
    );
}

// New test: post_commit creates full checkpoint structure.
#[test]
fn post_commit_creates_full_checkpoint_structure() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);

    let backend = LocalFileBackend::new(dir.path());
    let state = SessionState {
        session_id: "pc2".to_string(),
        phase: crate::engine::session::phase::SessionPhase::Idle,
        base_commit: head.clone(),
        agent_type: "claude-code".to_string(),
        files_touched: vec!["change2.txt".to_string()],
        ..Default::default()
    };
    backend.save_session(&state).unwrap();

    // Commit with known checkpoint ID so we can verify the sharded path.
    let checkpoint_id = "ab1234567890";
    fs::write(dir.path().join("change2.txt"), "change2").unwrap();
    git_command()
        .args(["add", "."])
        .current_dir(dir.path())
        .output()
        .unwrap();
    git_command()
        .args([
            "commit",
            "-m",
            &format!("fix\n\nBitloops-Checkpoint: {checkpoint_id}"),
        ])
        .current_dir(dir.path())
        .output()
        .unwrap();

    let strategy = ManualCommitStrategy::new(dir.path());
    strategy.post_commit().unwrap();

    // Check that both metadata files exist in the checkpoints branch tree.
    let top_meta = run_git(
        dir.path(),
        &[
            "show",
            "bitloops/checkpoints/v1:ab/1234567890/metadata.json",
        ],
    );
    assert!(
        top_meta.is_ok(),
        "top metadata.json should exist: {top_meta:?}"
    );

    let sess_meta = run_git(
        dir.path(),
        &[
            "show",
            "bitloops/checkpoints/v1:ab/1234567890/0/metadata.json",
        ],
    );
    assert!(
        sess_meta.is_ok(),
        "per-session metadata.json should exist: {sess_meta:?}"
    );

    // Verify top metadata content.
    let top_json: serde_json::Value = serde_json::from_str(&top_meta.unwrap()).unwrap();
    assert_eq!(top_json["checkpoint_id"], checkpoint_id);
    assert_eq!(top_json["strategy"], "manual-commit");
}

#[test]
fn post_commit_without_trailer_updates_active_base_commit() {
    let dir = tempfile::tempdir().unwrap();
    let head_before = setup_git_repo(&dir);
    let backend = LocalFileBackend::new(dir.path());
    backend
        .save_session(&SessionState {
            session_id: "pc-no-trailer".to_string(),
            phase: crate::engine::session::phase::SessionPhase::Active,
            base_commit: head_before.clone(),
            ..Default::default()
        })
        .unwrap();

    // Create a regular commit without Bitloops-Checkpoint trailer.
    fs::write(dir.path().join("plain.txt"), "plain").unwrap();
    git_command()
        .args(["add", "."])
        .current_dir(dir.path())
        .output()
        .unwrap();
    git_command()
        .args(["commit", "-m", "plain commit"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    let new_head = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();
    assert_ne!(head_before, new_head);

    let strategy = ManualCommitStrategy::new(dir.path());
    strategy.post_commit().unwrap();

    let loaded = backend.load_session("pc-no-trailer").unwrap().unwrap();
    assert_eq!(
        loaded.base_commit, new_head,
        "base_commit should advance when post-commit sees no trailer"
    );
    assert_eq!(
        loaded.phase,
        crate::engine::session::phase::SessionPhase::Active,
        "phase should remain active on no-trailer commits"
    );
}

#[test]
fn post_commit_without_trailer_updates_active_base_commit_during_rebase() {
    let dir = tempfile::tempdir().unwrap();
    let head_before = setup_git_repo(&dir);
    let backend = LocalFileBackend::new(dir.path());
    backend
        .save_session(&SessionState {
            session_id: "pc-no-trailer-rebase".to_string(),
            phase: SessionPhase::Active,
            base_commit: head_before.clone(),
            ..Default::default()
        })
        .unwrap();

    fs::create_dir_all(dir.path().join(".git").join("rebase-merge")).unwrap();

    // Create a regular commit without Bitloops-Checkpoint trailer.
    fs::write(dir.path().join("plain-rebase.txt"), "plain").unwrap();
    git_command()
        .args(["add", "."])
        .current_dir(dir.path())
        .output()
        .unwrap();
    git_command()
        .args(["commit", "-m", "plain commit during rebase"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    let new_head = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();
    assert_ne!(head_before, new_head);

    ManualCommitStrategy::new(dir.path()).post_commit().unwrap();

    let loaded = backend
        .load_session("pc-no-trailer-rebase")
        .unwrap()
        .unwrap();
    assert_eq!(
        loaded.base_commit, new_head,
        "base_commit should advance even when rebase markers are present"
    );
    assert_eq!(loaded.phase, SessionPhase::Active);
}

#[test]
fn extract_user_prompts_supports_nested_message_and_human_type() {
    let jsonl = r#"{"type":"user","message":{"content":[{"type":"text","text":"Create index.html"},{"type":"tool_result","tool_use_id":"x"}]}}
{"type":"assistant","message":{"content":[{"type":"text","text":"Done"}]}}
{"type":"human","message":{"content":"Add styles"}}
not-json"#;

    let prompts = extract_user_prompts_from_jsonl(jsonl);
    assert_eq!(prompts, vec!["Create index.html", "Add styles"]);
}

#[test]
fn extract_summary_supports_nested_message_content() {
    let jsonl = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"first summary"}]}}
{"type":"assistant","message":{"content":[{"type":"text","text":"final summary"},{"type":"tool_use","name":"Edit","input":{"file_path":"a.txt"}}]}}"#;

    let summary = extract_summary_from_jsonl(jsonl);
    assert_eq!(summary, "final summary");
}

#[test]
fn write_session_metadata_writes_prompt_and_summary_for_nested_claude_jsonl() {
    let dir = tempfile::tempdir().unwrap();
    let transcript_path = dir.path().join("transcript.jsonl");
    let jsonl = r#"{"type":"user","message":{"content":[{"type":"text","text":"Create test file"}]}}
{"type":"assistant","message":{"content":[{"type":"text","text":"Created test file"}]}}"#;
    fs::write(&transcript_path, jsonl).unwrap();

    let written = write_session_metadata(
        dir.path(),
        "session-nested",
        &transcript_path.to_string_lossy(),
    )
    .unwrap();
    assert!(
        written.contains(&".bitloops/metadata/session-nested/prompt.txt".to_string()),
        "prompt.txt should be part of written metadata files: {written:?}"
    );
    assert!(
        written.contains(&".bitloops/metadata/session-nested/summary.txt".to_string()),
        "summary.txt should be part of written metadata files: {written:?}"
    );

    let prompt = fs::read_to_string(
        dir.path()
            .join(".bitloops")
            .join("metadata")
            .join("session-nested")
            .join("prompt.txt"),
    )
    .unwrap();
    let summary = fs::read_to_string(
        dir.path()
            .join(".bitloops")
            .join("metadata")
            .join("session-nested")
            .join("summary.txt"),
    )
    .unwrap();

    assert_eq!(prompt.trim(), "Create test file");
    assert_eq!(summary.trim(), "Created test file");
}

#[test]
fn pre_push_pushes_checkpoints_branch_when_present() {
    let base = tempfile::tempdir().unwrap();
    let origin_dir = base.path().join("origin.git");
    let work_dir = base.path().join("work");
    fs::create_dir_all(&work_dir).unwrap();

    // Bare remote.
    let out = git_command()
        .args(["init", "--bare", origin_dir.to_string_lossy().as_ref()])
        .output()
        .unwrap();
    assert!(out.status.success(), "git init --bare failed");

    let work_temp = tempfile::TempDir::new_in(&work_dir).unwrap();
    let repo_dir = work_temp.path();
    let run = |args: &[&str]| {
        let out = git_command()
            .args(args)
            .current_dir(repo_dir)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {:?} failed", args);
    };

    run(&["init"]);
    run(&["config", "user.email", "t@t.com"]);
    run(&["config", "user.name", "Test"]);
    fs::write(repo_dir.join("README.md"), "init").unwrap();
    run(&["add", "."]);
    run(&["commit", "-m", "initial"]);
    run(&[
        "remote",
        "add",
        "origin",
        origin_dir.to_string_lossy().as_ref(),
    ]);

    // Create local checkpoints branch to push.
    let head = run_git(repo_dir, &["rev-parse", "HEAD"]).unwrap();
    run(&["update-ref", "refs/heads/bitloops/checkpoints/v1", &head]);

    let strategy = ManualCommitStrategy::new(repo_dir);
    strategy.pre_push("origin").unwrap();

    // Remote should now have bitloops/checkpoints/v1.
    let remote_ref = git_command()
        .args([
            "--git-dir",
            origin_dir.to_string_lossy().as_ref(),
            "show-ref",
            "--verify",
            "refs/heads/bitloops/checkpoints/v1",
        ])
        .output()
        .unwrap();
    assert!(
        remote_ref.status.success(),
        "remote should contain checkpoints branch after pre-push"
    );
}

fn commit_with_checkpoint_trailer(repo_root: &Path, checkpoint_id: &str, filename: &str) {
    fs::write(
        repo_root.join(filename),
        format!("content for {checkpoint_id}\n"),
    )
    .unwrap();
    git_ok(repo_root, &["add", filename]);
    git_ok(
        repo_root,
        &[
            "commit",
            "-m",
            &format!("test commit\n\nBitloops-Checkpoint: {checkpoint_id}"),
        ],
    );
}

#[test]
fn shadow_strategy_direct_instantiation() {
    let dir = tempfile::tempdir().unwrap();
    let strategy = ManualCommitStrategy::new(dir.path());
    assert_eq!(strategy.name(), "manual-commit");
}

#[test]
fn shadow_strategy_description() {
    let dir = tempfile::tempdir().unwrap();
    let strategy = ManualCommitStrategy::new(dir.path());
    assert_eq!(strategy.name(), "manual-commit");
}

#[test]
fn shadow_strategy_validate_repository() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    assert!(
        run_git(dir.path(), &["rev-parse", "--is-inside-work-tree"]).is_ok(),
        "expected git repo to validate"
    );
}

#[test]
fn shadow_strategy_validate_repository_not_git_repo() {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        run_git(dir.path(), &["rev-parse", "--is-inside-work-tree"]).is_err(),
        "non-git directory should fail validation"
    );
}

#[test]
fn post_commit_active_session_condenses_immediately() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let backend = LocalFileBackend::new(dir.path());
    backend
        .save_session(&SessionState {
            session_id: "pc-active".to_string(),
            phase: crate::engine::session::phase::SessionPhase::Active,
            base_commit: head,
            step_count: 2,
            files_touched: vec!["active.txt".to_string()],
            ..Default::default()
        })
        .unwrap();

    commit_with_checkpoint_trailer(dir.path(), "a1b2c3d4e5f6", "active.txt");
    ManualCommitStrategy::new(dir.path()).post_commit().unwrap();

    let loaded = backend.load_session("pc-active").unwrap().unwrap();
    assert_eq!(
        loaded.phase,
        crate::engine::session::phase::SessionPhase::Active
    );
    assert_eq!(loaded.step_count, 0, "active session should be condensed");
    assert!(
        run_git(dir.path(), &["rev-parse", "bitloops/checkpoints/v1"]).is_ok(),
        "condensation should materialize metadata branch"
    );
}

#[test]
fn post_commit_active_session_records_turn_checkpoint_ids() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let backend = LocalFileBackend::new(dir.path());
    backend
        .save_session(&SessionState {
            session_id: "pc-active-turn".to_string(),
            phase: SessionPhase::Active,
            base_commit: head,
            step_count: 1,
            files_touched: vec!["index.html".to_string()],
            ..Default::default()
        })
        .unwrap();

    commit_with_checkpoint_trailer(dir.path(), "a1b2c3d4e5f6", "index.html");
    ManualCommitStrategy::new(dir.path()).post_commit().unwrap();

    let loaded = backend.load_session("pc-active-turn").unwrap().unwrap();
    assert_eq!(
        loaded.turn_checkpoint_ids,
        vec!["a1b2c3d4e5f6".to_string()],
        "ACTIVE post-commit should record checkpoint IDs for stop-time finalization"
    );
}

#[test]
fn post_commit_idle_session_condenses() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let backend = LocalFileBackend::new(dir.path());
    backend
        .save_session(&SessionState {
            session_id: "pc-idle".to_string(),
            phase: crate::engine::session::phase::SessionPhase::Idle,
            base_commit: head,
            step_count: 1,
            files_touched: vec!["idle.txt".to_string()],
            ..Default::default()
        })
        .unwrap();

    commit_with_checkpoint_trailer(dir.path(), "b1c2d3e4f5a6", "idle.txt");
    ManualCommitStrategy::new(dir.path()).post_commit().unwrap();

    let loaded = backend.load_session("pc-idle").unwrap().unwrap();
    assert_eq!(loaded.step_count, 0);
    assert!(
        loaded.files_touched.is_empty(),
        "files_touched should be reset"
    );
}

#[test]
fn post_commit_rebase_during_active_skips_transition() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let backend = LocalFileBackend::new(dir.path());
    backend
        .save_session(&SessionState {
            session_id: "pc-rebase".to_string(),
            phase: crate::engine::session::phase::SessionPhase::Active,
            base_commit: head,
            step_count: 3,
            files_touched: vec!["rebase.txt".to_string()],
            ..Default::default()
        })
        .unwrap();

    fs::create_dir_all(dir.path().join(".git").join("rebase-merge")).unwrap();
    commit_with_checkpoint_trailer(dir.path(), "c1d2e3f4a5b6", "rebase.txt");

    ManualCommitStrategy::new(dir.path()).post_commit().unwrap();

    let loaded = backend.load_session("pc-rebase").unwrap().unwrap();
    assert_eq!(
        loaded.step_count, 3,
        "during rebase post-commit should be a no-op for session state"
    );
    assert!(
        run_git(dir.path(), &["rev-parse", "bitloops/checkpoints/v1"]).is_err(),
        "during rebase no condensation metadata branch should be written"
    );
}

#[test]
fn post_commit_files_touched_resets_after_condensation() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let backend = LocalFileBackend::new(dir.path());
    backend
        .save_session(&SessionState {
            session_id: "pc-files".to_string(),
            phase: crate::engine::session::phase::SessionPhase::Idle,
            base_commit: head,
            step_count: 1,
            files_touched: vec!["f1.rs".to_string(), "f2.rs".to_string()],
            ..Default::default()
        })
        .unwrap();

    fs::write(dir.path().join("f1.rs"), "f1").unwrap();
    fs::write(dir.path().join("f2.rs"), "f2").unwrap();
    git_ok(dir.path(), &["add", "f1.rs", "f2.rs"]);
    git_ok(
        dir.path(),
        &[
            "commit",
            "-m",
            "test commit\n\nBitloops-Checkpoint: d1e2f3a4b5c6",
        ],
    );
    ManualCommitStrategy::new(dir.path()).post_commit().unwrap();
    let loaded = backend.load_session("pc-files").unwrap().unwrap();
    assert!(loaded.files_touched.is_empty());
}

#[test]
fn handle_turn_end_finalizes_and_clears_turn_checkpoint_ids() {
    let dir = tempfile::tempdir().unwrap();
    let head = setup_git_repo(&dir);
    let backend = LocalFileBackend::new(dir.path());
    let transcript_path = dir.path().join("live-transcript.jsonl");
    fs::write(
        &transcript_path,
        "{\"type\":\"user\",\"message\":{\"content\":\"old prompt\"}}\n",
    )
    .unwrap();

    backend
        .save_session(&SessionState {
            session_id: "turn-end-session".to_string(),
            phase: SessionPhase::Active,
            base_commit: head,
            step_count: 1,
            files_touched: vec!["turn-end.txt".to_string()],
            transcript_path: transcript_path.to_string_lossy().to_string(),
            ..Default::default()
        })
        .unwrap();

    commit_with_checkpoint_trailer(dir.path(), "0aaabbbccdde", "turn-end.txt");
    let strategy = ManualCommitStrategy::new(dir.path());
    strategy.post_commit().unwrap();

    // Update the live transcript so turn-end finalization has richer content to persist.
    let new_transcript = "{\"type\":\"user\",\"message\":{\"content\":\"latest prompt\"}}\n\
{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"latest answer\"}]}}\n";
    fs::write(&transcript_path, new_transcript).unwrap();

    let mut state = backend.load_session("turn-end-session").unwrap().unwrap();
    assert_eq!(state.turn_checkpoint_ids.len(), 1);
    state
        .turn_checkpoint_ids
        .push("invalid-checkpoint".to_string());

    strategy.handle_turn_end(&mut state).unwrap();
    assert!(
        state.turn_checkpoint_ids.is_empty(),
        "turn checkpoint IDs should be cleared even if one update fails"
    );

    let (a, b) = checkpoint_dir_parts("0aaabbbccdde");
    let transcript_tree_path = format!("{a}/{b}/0/{}", paths::TRANSCRIPT_FILE_NAME);
    let committed = run_git(
        dir.path(),
        &[
            "show",
            &format!(
                "refs/heads/{}:{transcript_tree_path}",
                paths::METADATA_BRANCH_NAME
            ),
        ],
    )
    .unwrap();
    assert!(
        committed.contains("latest answer"),
        "turn-end should replace provisional transcript with full transcript"
    );
}

#[test]
fn subtract_files_compat() {
    let files_touched = vec!["a.rs".to_string(), "b.rs".to_string(), "c.rs".to_string()];
    let committed_files = std::collections::HashSet::from(["a.rs".to_string(), "c.rs".to_string()]);
    let remaining = subtract_files_by_name(&files_touched, &committed_files);
    assert_eq!(remaining, vec!["b.rs".to_string()]);
}

#[test]
fn files_changed_in_commit_compat() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    fs::write(dir.path().join("changed.rs"), "package changed").unwrap();
    git_ok(dir.path(), &["add", "changed.rs"]);
    git_ok(dir.path(), &["commit", "-m", "change tracked file"]);
    let head = git_ok(dir.path(), &["rev-parse", "HEAD"]);
    let changed = files_changed_in_commit(dir.path(), &head).unwrap();
    assert!(changed.contains("changed.rs"));
}

#[test]
fn files_changed_in_commit_initial_commit_compat() {
    let dir = tempfile::tempdir().unwrap();
    setup_empty_git_repo(&dir);
    fs::write(dir.path().join("initial.rs"), "package initial").unwrap();
    git_ok(dir.path(), &["add", "initial.rs"]);
    git_ok(dir.path(), &["commit", "-m", "initial commit"]);
    let head = git_ok(dir.path(), &["rev-parse", "HEAD"]);
    let changed = files_changed_in_commit(dir.path(), &head).unwrap();
    assert!(changed.contains("initial.rs"));
}

#[test]
fn save_step_empty_base_commit_recovery() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let backend = LocalFileBackend::new(dir.path());
    backend
        .save_session(&SessionState {
            session_id: "save-recovery".to_string(),
            base_commit: String::new(),
            ..Default::default()
        })
        .unwrap();

    let strategy = ManualCommitStrategy::new(dir.path());
    let ctx = StepContext {
        session_id: "save-recovery".to_string(),
        commit_message: "checkpoint".to_string(),
        metadata_dir: String::new(),
        metadata_dir_abs: String::new(),
        new_files: vec![],
        modified_files: vec![],
        deleted_files: vec![],
        author_name: "Test".to_string(),
        author_email: "test@test.com".to_string(),
        ..Default::default()
    };
    strategy.save_step(&ctx).unwrap();
    let loaded = backend.load_session("save-recovery").unwrap().unwrap();
    assert!(!loaded.base_commit.is_empty());
}

#[test]
fn save_step_uses_ctx_agent_type_when_no_session_state() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let strategy = ManualCommitStrategy::new(dir.path());
    let backend = LocalFileBackend::new(dir.path());

    strategy
        .save_step(&StepContext {
            session_id: "save-agent-none".to_string(),
            agent_type: "gemini-cli".to_string(),
            commit_message: "checkpoint".to_string(),
            ..Default::default()
        })
        .unwrap();

    let loaded = backend.load_session("save-agent-none").unwrap().unwrap();
    assert_eq!(loaded.agent_type, "gemini-cli");
    assert_eq!(loaded.turn_id.len(), 12);
    assert!(
        loaded
            .turn_id
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
        "turn_id should be 12-char lowercase hex: {}",
        loaded.turn_id
    );
}

#[test]
fn save_step_uses_ctx_agent_type_when_partial_state() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let backend = LocalFileBackend::new(dir.path());
    backend
        .save_session(&SessionState {
            session_id: "save-agent-partial".to_string(),
            base_commit: String::new(),
            agent_type: String::new(),
            ..Default::default()
        })
        .unwrap();

    let strategy = ManualCommitStrategy::new(dir.path());
    strategy
        .save_step(&StepContext {
            session_id: "save-agent-partial".to_string(),
            agent_type: "gemini-cli".to_string(),
            commit_message: "checkpoint".to_string(),
            ..Default::default()
        })
        .unwrap();

    let loaded = backend.load_session("save-agent-partial").unwrap().unwrap();
    assert_eq!(loaded.agent_type, "gemini-cli");
    assert_eq!(loaded.turn_id.len(), 12);
    assert!(
        loaded
            .turn_id
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
        "turn_id should be 12-char lowercase hex: {}",
        loaded.turn_id
    );
}

#[test]
fn post_commit_no_head_is_noop() {
    let dir = tempfile::tempdir().unwrap();
    setup_empty_git_repo(&dir);

    let strategy = ManualCommitStrategy::new(dir.path());
    let result = strategy.post_commit();
    assert!(
        result.is_ok(),
        "post_commit should no-op when HEAD is missing: {result:?}"
    );
}

#[test]
fn update_base_commit_no_head_is_noop() {
    let dir = tempfile::tempdir().unwrap();
    setup_empty_git_repo(&dir);

    let strategy = ManualCommitStrategy::new(dir.path());
    let backend = LocalFileBackend::new(dir.path());
    backend
        .save_session(&SessionState {
            session_id: "s_update_base_no_head".to_string(),
            phase: crate::engine::session::phase::SessionPhase::Active,
            base_commit: "deadbeef".to_string(),
            ..Default::default()
        })
        .unwrap();

    let result = strategy.update_base_commit_for_active_sessions();
    assert!(
        result.is_ok(),
        "update_base_commit_for_active_sessions should no-op when HEAD is missing: {result:?}"
    );

    let loaded = backend
        .load_session("s_update_base_no_head")
        .unwrap()
        .unwrap();
    assert_eq!(loaded.base_commit, "deadbeef");
}

use super::*;

#[test]
pub(crate) fn files_overlap_with_content_modified_file() {
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
pub(crate) fn files_overlap_with_content_new_file_content_match() {
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
pub(crate) fn files_overlap_with_content_new_file_content_mismatch() {
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
pub(crate) fn files_overlap_with_content_file_not_in_commit() {
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
pub(crate) fn files_overlap_with_content_deleted_file() {
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
pub(crate) fn files_overlap_with_content_no_shadow_branch() {
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
pub(crate) fn files_with_remaining_agent_changes_file_not_committed() {
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
pub(crate) fn files_with_remaining_agent_changes_fully_committed() {
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
pub(crate) fn files_with_remaining_agent_changes_partial_commit() {
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
pub(crate) fn files_with_remaining_agent_changes_no_shadow_branch() {
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
pub(crate) fn staged_files_overlap_with_content_modified_file() {
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
pub(crate) fn staged_files_overlap_with_content_new_file_content_match() {
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
pub(crate) fn staged_files_overlap_with_content_new_file_content_mismatch() {
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
pub(crate) fn staged_files_overlap_with_content_no_overlap() {
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
pub(crate) fn staged_files_overlap_with_content_deleted_file() {
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
pub(crate) fn test_extract_significant_lines() {
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
pub(crate) fn test_has_significant_content_overlap() {
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
pub(crate) fn test_trim_line() {
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
pub(crate) fn is_git_sequence_operation_no_operation() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    assert!(
        !is_git_sequence_operation(dir.path()),
        "clean repository should not be in sequence operation"
    );
}

#[test]
pub(crate) fn is_git_sequence_operation_rebase_merge() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    fs::create_dir_all(dir.path().join(".git").join("rebase-merge")).unwrap();
    assert!(
        is_git_sequence_operation(dir.path()),
        "rebase-merge should be detected as sequence operation"
    );
}

#[test]
pub(crate) fn is_git_sequence_operation_rebase_apply() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    fs::create_dir_all(dir.path().join(".git").join("rebase-apply")).unwrap();
    assert!(
        is_git_sequence_operation(dir.path()),
        "rebase-apply should be detected as sequence operation"
    );
}

#[test]
pub(crate) fn is_git_sequence_operation_cherry_pick() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    fs::write(dir.path().join(".git").join("CHERRY_PICK_HEAD"), "abc123").unwrap();
    assert!(
        is_git_sequence_operation(dir.path()),
        "CHERRY_PICK_HEAD should be detected as sequence operation"
    );
}

#[test]
pub(crate) fn is_git_sequence_operation_revert() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    fs::write(dir.path().join(".git").join("REVERT_HEAD"), "abc123").unwrap();
    assert!(
        is_git_sequence_operation(dir.path()),
        "REVERT_HEAD should be detected as sequence operation"
    );
}

#[test]
pub(crate) fn is_git_sequence_operation_worktree() {
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

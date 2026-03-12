use super::*;
use crate::test_support::process_state::{with_cwd, with_process_state};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn run_git_output(repo: &Path, args: &[&str]) -> Output {
    Command::new("git")
        .args(args)
        .current_dir(repo)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
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

fn init_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    run_git(dir.path(), &["init"]);
    run_git(dir.path(), &["config", "user.email", "test@example.com"]);
    run_git(dir.path(), &["config", "user.name", "Test"]);
    fs::write(dir.path().join("test.txt"), "initial\n").expect("write");
    run_git(dir.path(), &["add", "test.txt"]);
    run_git(dir.path(), &["commit", "-m", "initial commit"]);
    dir
}

fn with_repo_cwd<F: FnOnce(&Path)>(repo: &Path, f: F) {
    with_cwd(repo, || f(repo));
}

fn with_repo_cwd_and_temp_home<F: FnOnce(&Path)>(repo: &Path, home: &Path, f: F) {
    let home_owned = home.to_string_lossy().into_owned();
    with_process_state(
        Some(repo),
        &[
            ("HOME", Some(home_owned.as_str())),
            ("XDG_CONFIG_HOME", None),
        ],
        || f(repo),
    );
}

#[test]
fn test_get_current_branch() {
    let dir = init_repo();
    with_repo_cwd(dir.path(), |repo| {
        run_git(repo, &["checkout", "-b", "feature"]);
        let branch = get_current_branch().expect("expected branch");
        assert_eq!(branch, "feature");
    });
}

#[test]
fn test_get_current_branch_detached_head() {
    let dir = init_repo();
    with_repo_cwd(dir.path(), |repo| {
        let head = run_git(repo, &["rev-parse", "HEAD"]);
        run_git(repo, &["checkout", &head]);
        let err = get_current_branch().expect_err("expected detached HEAD error");
        assert!(err.to_string().to_lowercase().contains("detached"));
    });
}

#[test]
fn test_get_default_branch_name() {
    // returns main when main branch exists
    {
        let dir = init_repo();
        with_repo_cwd(dir.path(), |repo| {
            let current = run_git(repo, &["rev-parse", "--abbrev-ref", "HEAD"]);
            if current != "main" {
                let _ = run_git_output(repo, &["branch", "main"]);
            }
            let result = get_default_branch_name();
            assert_eq!(result, "main");
        });
    }

    // returns master when only master exists
    {
        let dir = init_repo();
        with_repo_cwd(dir.path(), |repo| {
            let current = run_git(repo, &["rev-parse", "--abbrev-ref", "HEAD"]);
            if current != "master" {
                run_git(repo, &["checkout", "-b", "master"]);
            }
            let _ = run_git_output(repo, &["branch", "-D", "main"]);

            let result = get_default_branch_name();
            assert_eq!(result, "master");
        });
    }

    // returns empty when no main or master branch exists
    {
        let dir = init_repo();
        with_repo_cwd(dir.path(), |repo| {
            run_git(repo, &["checkout", "-b", "develop"]);
            let _ = run_git_output(repo, &["branch", "-D", "main"]);
            let _ = run_git_output(repo, &["branch", "-D", "master"]);

            let result = get_default_branch_name();
            assert_eq!(result, "");
        });
    }

    // returns origin/HEAD target when present
    {
        let dir = init_repo();
        with_repo_cwd(dir.path(), |repo| {
            let head = run_git(repo, &["rev-parse", "HEAD"]);
            run_git(repo, &["update-ref", "refs/remotes/origin/trunk", &head]);
            run_git(
                repo,
                &[
                    "symbolic-ref",
                    "refs/remotes/origin/HEAD",
                    "refs/remotes/origin/trunk",
                ],
            );

            let result = get_default_branch_name();
            assert_eq!(result, "trunk");
        });
    }
}

#[test]
fn test_is_on_default_branch() {
    // returns true when on main
    {
        let dir = init_repo();
        with_repo_cwd(dir.path(), |repo| {
            let current = run_git(repo, &["rev-parse", "--abbrev-ref", "HEAD"]);
            if current != "main" {
                if run_git_output(
                    repo,
                    &["show-ref", "--verify", "--quiet", "refs/heads/main"],
                )
                .status
                .success()
                {
                    run_git(repo, &["checkout", "main"]);
                } else {
                    run_git(repo, &["checkout", "-b", "main"]);
                }
            }
            let (is_default, branch) = is_on_default_branch().expect("is_on_default_branch");
            assert!(is_default);
            assert_eq!(branch, "main");
        });
    }

    // returns true when on master
    {
        let dir = init_repo();
        with_repo_cwd(dir.path(), |repo| {
            let current = run_git(repo, &["rev-parse", "--abbrev-ref", "HEAD"]);
            if current != "master" {
                run_git(repo, &["checkout", "-b", "master"]);
            }
            let _ = run_git_output(repo, &["branch", "-D", "main"]);

            let (is_default, branch) = is_on_default_branch().expect("is_on_default_branch");
            assert!(is_default);
            assert_eq!(branch, "master");
        });
    }

    // returns false when on feature branch
    {
        let dir = init_repo();
        with_repo_cwd(dir.path(), |repo| {
            run_git(repo, &["checkout", "-b", "feature/test"]);

            let (is_default, branch) = is_on_default_branch().expect("is_on_default_branch");
            assert!(!is_default);
            assert_eq!(branch, "feature/test");
        });
    }

    // returns false for detached HEAD
    {
        let dir = init_repo();
        with_repo_cwd(dir.path(), |repo| {
            let head = run_git(repo, &["rev-parse", "HEAD"]);
            run_git(repo, &["checkout", &head]);

            let (is_default, branch) = is_on_default_branch().expect("is_on_default_branch");
            assert!(!is_default);
            assert_eq!(branch, "");
        });
    }
}

#[test]
fn test_get_merge_base() {
    let dir = init_repo();
    with_repo_cwd(dir.path(), |repo| {
        let base = run_git(repo, &["rev-parse", "HEAD"]);
        let current_branch = run_git(repo, &["rev-parse", "--abbrev-ref", "HEAD"]);
        if current_branch != "main" {
            run_git(repo, &["branch", "main", &base]);
        }
        run_git(repo, &["checkout", "-b", "feature", &base]);
        fs::write(repo.join("test.txt"), "feature change\n").expect("write feature");
        run_git(repo, &["add", "test.txt"]);
        run_git(repo, &["commit", "-m", "feature commit"]);

        let merge_base = get_merge_base("feature", "main").expect("merge base");
        assert_eq!(merge_base, base);
    });
}

#[test]
fn test_get_merge_base_non_existent_branch() {
    let dir = init_repo();
    with_repo_cwd(dir.path(), |_| {
        let err = get_merge_base("feature", "nonexistent").expect_err("expected error");
        assert!(!err.to_string().is_empty());
    });
}

#[test]
fn test_has_uncommitted_changes() {
    let dir = init_repo();
    with_repo_cwd(dir.path(), |repo| {
        let has_changes = has_uncommitted_changes().expect("status should work");
        assert!(!has_changes, "expected clean tree");

        fs::write(repo.join("test.txt"), "modified\n").expect("modify file");
        let has_changes = has_uncommitted_changes().expect("status should work");
        assert!(has_changes, "expected unstaged changes");

        run_git(repo, &["add", "test.txt"]);
        let has_changes = has_uncommitted_changes().expect("status should work");
        assert!(has_changes, "expected staged changes");

        run_git(repo, &["commit", "-m", "second commit"]);
        fs::write(repo.join("untracked.txt"), "new\n").expect("new file");
        let has_changes = has_uncommitted_changes().expect("status should work");
        assert!(has_changes, "expected untracked changes");

        fs::remove_file(repo.join("untracked.txt")).expect("remove file");
        let global_ignore_dir = tempfile::tempdir().expect("tempdir");
        let global_ignore_file = global_ignore_dir.path().join("global-gitignore");
        fs::write(&global_ignore_file, "*.globally-ignored\n").expect("write ignore");
        run_git(
            repo,
            &[
                "config",
                "core.excludesfile",
                global_ignore_file.to_string_lossy().as_ref(),
            ],
        );
        fs::write(repo.join("secret.globally-ignored"), "ignored\n").expect("ignored file");

        let has_changes = has_uncommitted_changes().expect("status should work");
        assert!(!has_changes, "expected globally ignored file to stay clean");
    });
}

#[test]
fn test_find_new_untracked_files() {
    type NewUntrackedCase<'a> = (&'a str, Vec<&'a str>, Vec<&'a str>, Vec<&'a str>);
    let cases: Vec<NewUntrackedCase<'_>> = vec![
        (
            "finds new files not in pre-existing list",
            vec!["file1.rs", "file2.rs", "file3.rs"],
            vec!["file1.rs"],
            vec!["file2.rs", "file3.rs"],
        ),
        (
            "returns empty when all files pre-exist",
            vec!["file1.rs", "file2.rs"],
            vec!["file1.rs", "file2.rs"],
            vec![],
        ),
        (
            "returns all files when pre-existing is empty",
            vec!["file1.rs", "file2.rs"],
            vec![],
            vec!["file1.rs", "file2.rs"],
        ),
        (
            "returns nil when current is empty",
            vec![],
            vec!["file1.rs"],
            vec![],
        ),
        (
            "handles nil current slice",
            vec![],
            vec!["file1.rs"],
            vec![],
        ),
        (
            "handles nil pre-existing slice",
            vec!["file1.rs", "file2.rs"],
            vec![],
            vec!["file1.rs", "file2.rs"],
        ),
        ("handles both nil slices", vec![], vec![], vec![]),
        (
            "handles files with paths",
            vec!["src/main.rs", "src/utils.rs", "test/main_test.rs"],
            vec!["src/main.rs"],
            vec!["src/utils.rs", "test/main_test.rs"],
        ),
        (
            "handles duplicate files in pre-existing",
            vec!["file1.rs", "file2.rs"],
            vec!["file1.rs", "file1.rs"],
            vec!["file2.rs"],
        ),
        (
            "is case-sensitive",
            vec!["File.rs", "file.rs"],
            vec!["file.rs"],
            vec!["File.rs"],
        ),
    ];

    for (_name, current, pre_existing, expected) in cases {
        let current = current
            .into_iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let pre_existing = pre_existing
            .into_iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let mut result = find_new_untracked_files(&current, &pre_existing);
        let mut expected = expected
            .into_iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();

        result.sort();
        expected.sort();
        assert_eq!(result, expected);
    }
}

#[test]
fn test_get_git_config_value() {
    let invalid = get_git_config_value("nonexistent.key.that.does.not.exist");
    assert_eq!(invalid, "");

    let _name = get_git_config_value("user.name");
}

#[test]
fn test_get_git_config_value_trims_whitespace() {
    let dir = init_repo();
    with_repo_cwd(dir.path(), |repo| {
        run_git(repo, &["config", "user.email", "  ws@example.com  "]);
        let email = get_git_config_value("user.email");
        assert_eq!(email, "ws@example.com");
    });
}

#[test]
fn test_get_git_author_returns_author() {
    let dir = init_repo();
    with_repo_cwd(dir.path(), |repo| {
        run_git(repo, &["config", "user.name", "Test Author"]);
        run_git(repo, &["config", "user.email", "test@example.com"]);

        let author = get_git_author().expect("author expected");
        assert_eq!(author.name, "Test Author");
        assert_eq!(author.email, "test@example.com");
    });
}

#[test]
fn test_get_git_author_falls_back_to_git_command() {
    let global_home = tempfile::tempdir().expect("tempdir");
    fs::write(
        global_home.path().join(".gitconfig"),
        "[user]\n\tname = Global User\n\temail = global@example.com\n",
    )
    .expect("write global gitconfig");

    let dir = tempfile::tempdir().expect("tempdir");
    run_git(dir.path(), &["init"]);

    with_repo_cwd_and_temp_home(dir.path(), global_home.path(), |_| {
        let author = get_git_author().expect("should not error");
        assert_eq!(author.name, "Global User");
        assert_eq!(author.email, "global@example.com");
    });
}

#[test]
fn test_get_git_author_returns_defaults_when_no_config() {
    let global_home = tempfile::tempdir().expect("tempdir");
    let dir = tempfile::tempdir().expect("tempdir");
    run_git(dir.path(), &["init"]);

    with_repo_cwd_and_temp_home(dir.path(), global_home.path(), |_| {
        let author = get_git_author().expect("should not error");
        assert_eq!(author.name, "Unknown");
        assert_eq!(author.email, "unknown@local");
    });
}

#[test]
fn test_get_git_author_from_repo() {
    struct Case {
        local_name: &'static str,
        local_email: &'static str,
        global_name: &'static str,
        global_email: &'static str,
        want_name: &'static str,
        want_email: &'static str,
    }

    let global_home = tempfile::tempdir().expect("tempdir");
    let cases = vec![
        Case {
            local_name: "Local User",
            local_email: "local@example.com",
            global_name: "",
            global_email: "",
            want_name: "Local User",
            want_email: "local@example.com",
        },
        Case {
            local_name: "Local User",
            local_email: "",
            global_name: "",
            global_email: "global@example.com",
            want_name: "Local User",
            want_email: "global@example.com",
        },
        Case {
            local_name: "",
            local_email: "local@example.com",
            global_name: "Global User",
            global_email: "",
            want_name: "Global User",
            want_email: "local@example.com",
        },
        Case {
            local_name: "",
            local_email: "",
            global_name: "Global User",
            global_email: "global@example.com",
            want_name: "Global User",
            want_email: "global@example.com",
        },
        Case {
            local_name: "",
            local_email: "",
            global_name: "",
            global_email: "",
            want_name: "Unknown",
            want_email: "unknown@local",
        },
        Case {
            local_name: "Local User",
            local_email: "local@example.com",
            global_name: "Global User",
            global_email: "global@example.com",
            want_name: "Local User",
            want_email: "local@example.com",
        },
    ];

    for case in cases {
        let global_cfg = global_home.path().join(".gitconfig");
        if case.global_name.is_empty() && case.global_email.is_empty() {
            let _ = fs::remove_file(&global_cfg);
        } else {
            let mut body = String::from("[user]\n");
            if !case.global_name.is_empty() {
                body.push_str(&format!("\tname = {}\n", case.global_name));
            }
            if !case.global_email.is_empty() {
                body.push_str(&format!("\temail = {}\n", case.global_email));
            }
            fs::write(&global_cfg, body).expect("write global gitconfig");
        }

        let dir = tempfile::tempdir().expect("tempdir");
        run_git(dir.path(), &["init"]);

        if !case.local_name.is_empty() {
            run_git(dir.path(), &["config", "user.name", case.local_name]);
        }
        if !case.local_email.is_empty() {
            run_git(dir.path(), &["config", "user.email", case.local_email]);
        }

        with_repo_cwd_and_temp_home(dir.path(), global_home.path(), |_| {
            let author = get_git_author().expect("author lookup should succeed");
            assert_eq!(author.name, case.want_name);
            assert_eq!(author.email, case.want_email);
        });
    }
}

#[test]
fn test_is_empty_repository() {
    // Empty repo returns true.
    {
        let dir = tempfile::tempdir().expect("tempdir");
        run_git(dir.path(), &["init"]);
        with_repo_cwd(dir.path(), |_| {
            let is_empty = is_empty_repository().expect("is_empty_repository");
            assert!(is_empty);
        });
    }

    // Repo with at least one commit returns false.
    {
        let dir = init_repo();
        with_repo_cwd(dir.path(), |_| {
            let is_empty = is_empty_repository().expect("is_empty_repository");
            assert!(!is_empty);
        });
    }
}

#[test]
fn test_hard_reset_with_protection_preserves_protected_dirs() {
    let dir = tempfile::tempdir().expect("tempdir");
    run_git(dir.path(), &["init"]);
    run_git(dir.path(), &["config", "user.email", "test@example.com"]);
    run_git(dir.path(), &["config", "user.name", "Test"]);

    fs::write(dir.path().join("initial.txt"), "initial content\n").expect("write initial");
    run_git(dir.path(), &["add", "initial.txt"]);
    run_git(dir.path(), &["commit", "-m", "Initial commit"]);
    let initial_hash = run_git(dir.path(), &["rev-parse", "HEAD"]);

    fs::write(dir.path().join("second.txt"), "second content\n").expect("write second");
    run_git(dir.path(), &["add", "second.txt"]);
    run_git(dir.path(), &["commit", "-m", "Second commit"]);

    fs::write(dir.path().join(".gitignore"), ".bitloops/\n.worktrees/\n").expect("write ignore");
    fs::create_dir_all(dir.path().join(".bitloops/metadata")).expect("create .bitloops");
    fs::write(
        dir.path().join(".bitloops/metadata/session.json"),
        "important session metadata",
    )
    .expect("write .bitloops content");
    fs::create_dir_all(dir.path().join(".worktrees/feature-branch")).expect("create .worktrees");
    fs::write(
        dir.path().join(".worktrees/feature-branch/config"),
        "worktree config",
    )
    .expect("write .worktrees content");

    with_repo_cwd(dir.path(), |_| {
        let short_id =
            hard_reset_with_protection(&initial_hash).expect("hard reset should succeed");
        assert_eq!(short_id.len(), 7);

        assert!(
            !dir.path().join("second.txt").exists(),
            "second commit file should be removed by reset"
        );

        assert!(
            dir.path().join(".bitloops").exists(),
            ".bitloops should not be deleted by hard reset"
        );
        let bitloops_content =
            fs::read_to_string(dir.path().join(".bitloops/metadata/session.json"))
                .expect("read .bitloops content");
        assert_eq!(bitloops_content, "important session metadata");

        assert!(
            dir.path().join(".worktrees").exists(),
            ".worktrees should not be deleted by hard reset"
        );
        let worktrees_content =
            fs::read_to_string(dir.path().join(".worktrees/feature-branch/config"))
                .expect("read .worktrees content");
        assert_eq!(worktrees_content, "worktree config");
    });
}

#[test]
fn test_branch_exists_on_remote() {
    let base_dir = tempfile::tempdir().expect("tempdir");
    let remote_dir = tempfile::tempdir().expect("remote tempdir");

    run_git(remote_dir.path(), &["init", "--bare"]);

    run_git(base_dir.path(), &["init"]);
    run_git(
        base_dir.path(),
        &["config", "user.email", "test@example.com"],
    );
    run_git(base_dir.path(), &["config", "user.name", "Test"]);
    run_git(
        base_dir.path(),
        &[
            "remote",
            "add",
            "origin",
            remote_dir.path().to_string_lossy().as_ref(),
        ],
    );

    fs::write(base_dir.path().join("test.txt"), "test\n").expect("write file");
    run_git(base_dir.path(), &["add", "test.txt"]);
    run_git(base_dir.path(), &["commit", "-m", "initial commit"]);
    run_git(base_dir.path(), &["checkout", "-b", "feature"]);
    run_git(base_dir.path(), &["push", "-u", "origin", "feature"]);

    let repo_path = PathBuf::from(base_dir.path());
    with_repo_cwd(&repo_path, |_| {
        let exists = branch_exists_on_remote("feature").expect("remote check");
        assert!(exists);

        let exists = branch_exists_on_remote("nonexistent").expect("remote check");
        assert!(!exists);
    });
}

use super::*;
use crate::test_support::process_state::with_cwd;
use std::collections::BTreeSet;
use tempfile::TempDir;

/// Creates an initialized git repository in `dir`.
fn setup_git_repo(dir: &TempDir) {
    let run = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(dir.path())
            .output()
            .unwrap();
    };
    run(&["init"]);
    run(&["config", "user.email", "t@t.com"]);
    run(&["config", "user.name", "Test"]);
    fs::write(dir.path().join("README.md"), "init").unwrap();
    run(&["add", "."]);
    run(&["commit", "-m", "initial"]);
}

fn run_git_checked(cwd: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {:?} failed", args);
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn init_hooks_worktree_repo() -> (TempDir, PathBuf, PathBuf) {
    let parent = tempfile::tempdir().unwrap();
    let main_repo = parent.path().join("main");
    let worktree_dir = parent.path().join("worktree");
    fs::create_dir_all(&main_repo).unwrap();

    run_git_checked(&main_repo, &["init"]);
    run_git_checked(&main_repo, &["config", "user.email", "t@t.com"]);
    run_git_checked(&main_repo, &["config", "user.name", "Test"]);
    fs::write(main_repo.join("README.md"), "init").unwrap();
    run_git_checked(&main_repo, &["add", "."]);
    run_git_checked(&main_repo, &["commit", "-m", "initial"]);
    run_git_checked(
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

fn with_repo_cwd<F: FnOnce()>(repo_root: &Path, f: F) {
    with_cwd(repo_root, f);
}

#[test]
fn test_detect_hook_managers_none() {
    let dir = tempfile::tempdir().unwrap();
    let managers = detect_hook_managers(dir.path());
    assert_eq!(managers.len(), 0);
}

#[test]
fn test_detect_hook_managers_husky() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join(".husky/_")).unwrap();

    let managers = detect_hook_managers(dir.path());
    assert_eq!(managers.len(), 1);
    assert_eq!(managers[0].name, "Husky");
    assert_eq!(managers[0].config_path, ".husky/");
    assert!(managers[0].overwrites_hooks);
}

#[test]
fn test_detect_hook_managers_lefthook() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("lefthook.yml"), "").unwrap();

    let managers = detect_hook_managers(dir.path());
    assert_eq!(managers.len(), 1);
    assert_eq!(managers[0].name, "Lefthook");
    assert_eq!(managers[0].config_path, "lefthook.yml");
    assert!(!managers[0].overwrites_hooks);
}

#[test]
fn test_detect_hook_managers_lefthook_dot_prefix() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join(".lefthook.yml"), "").unwrap();

    let managers = detect_hook_managers(dir.path());
    assert_eq!(managers.len(), 1);
    assert_eq!(managers[0].name, "Lefthook");
    assert_eq!(managers[0].config_path, ".lefthook.yml");
}

#[test]
fn test_detect_hook_managers_lefthook_toml() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("lefthook.toml"), "").unwrap();

    let managers = detect_hook_managers(dir.path());
    assert_eq!(managers.len(), 1);
    assert_eq!(managers[0].name, "Lefthook");
    assert_eq!(managers[0].config_path, "lefthook.toml");
}

#[test]
fn test_detect_hook_managers_lefthook_local() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("lefthook-local.yml"), "").unwrap();

    let managers = detect_hook_managers(dir.path());
    assert_eq!(managers.len(), 1);
    assert_eq!(managers[0].name, "Lefthook");
    assert_eq!(managers[0].config_path, "lefthook-local.yml");
}

#[test]
fn test_detect_hook_managers_lefthook_dedup() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("lefthook.yml"), "").unwrap();
    fs::write(dir.path().join(".lefthook.yml"), "").unwrap();

    let managers = detect_hook_managers(dir.path());
    assert_eq!(managers.len(), 1);
    assert_eq!(managers[0].name, "Lefthook");
}

#[test]
fn test_detect_hook_managers_pre_commit() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join(".pre-commit-config.yaml"), "").unwrap();

    let managers = detect_hook_managers(dir.path());
    assert_eq!(managers.len(), 1);
    assert_eq!(managers[0].name, "pre-commit");
    assert_eq!(managers[0].config_path, ".pre-commit-config.yaml");
    assert!(!managers[0].overwrites_hooks);
}

#[test]
fn test_detect_hook_managers_overcommit() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join(".overcommit.yml"), "").unwrap();

    let managers = detect_hook_managers(dir.path());
    assert_eq!(managers.len(), 1);
    assert_eq!(managers[0].name, "Overcommit");
    assert_eq!(managers[0].config_path, ".overcommit.yml");
    assert!(!managers[0].overwrites_hooks);
}

#[test]
fn test_detect_hook_managers_multiple() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join(".husky/_")).unwrap();
    fs::write(dir.path().join(".pre-commit-config.yaml"), "").unwrap();

    let managers = detect_hook_managers(dir.path());
    assert_eq!(managers.len(), 2);

    let names: BTreeSet<_> = managers.into_iter().map(|m| m.name).collect();
    assert!(names.contains("Husky"));
    assert!(names.contains("pre-commit"));
}

#[test]
fn test_hook_manager_warning_husky() {
    let managers = vec![HookManager {
        name: "Husky".to_string(),
        config_path: ".husky/".to_string(),
        overwrites_hooks: true,
    }];

    let warning = hook_manager_warning(&managers, "bitloops");

    for hook in HOOK_NAMES {
        assert!(warning.contains(&format!(".husky/{hook}:")));
    }

    for spec in build_hook_specs("bitloops") {
        let cmd_line = extract_command_line(&spec.content);
        assert!(!cmd_line.is_empty());
        assert!(warning.contains(&cmd_line));
    }

    assert!(warning.contains("Warning: Husky detected"));
    assert!(warning.contains("may overwrite hooks"));
}

#[test]
fn test_hook_manager_warning_git_hooks_manager() {
    let managers = vec![HookManager {
        name: "Lefthook".to_string(),
        config_path: "lefthook.yml".to_string(),
        overwrites_hooks: false,
    }];

    let warning = hook_manager_warning(&managers, "bitloops");
    assert!(warning.contains("Note: Lefthook detected"));
    assert!(warning.contains("run 'bitloops enable' to restore"));
    assert!(!warning.contains("prepare-commit-msg:"));
}

#[test]
fn test_hook_manager_warning_empty() {
    assert_eq!(hook_manager_warning(&[], "bitloops"), "");
}

#[test]
fn test_hook_manager_warning_local_dev() {
    let managers = vec![HookManager {
        name: "Husky".to_string(),
        config_path: ".husky/".to_string(),
        overwrites_hooks: true,
    }];

    let warning = hook_manager_warning(&managers, "bitloops-dev");
    assert!(warning.contains("bitloops-dev hooks git"));
}

#[test]
fn test_hook_manager_warning_multiple() {
    let managers = vec![
        HookManager {
            name: "Husky".to_string(),
            config_path: ".husky/".to_string(),
            overwrites_hooks: true,
        },
        HookManager {
            name: "Lefthook".to_string(),
            config_path: "lefthook.yml".to_string(),
            overwrites_hooks: false,
        },
    ];

    let warning = hook_manager_warning(&managers, "bitloops");
    assert!(warning.contains("Warning: Husky detected"));
    assert!(warning.contains("Note: Lefthook detected"));
}

#[test]
fn test_extract_command_line() {
    let cases = vec![
        (
            "standard hook",
            "#!/bin/sh\n# Bitloops CLI hooks\nbitloops hooks git post-commit 2>/dev/null || true\n",
            "bitloops hooks git post-commit 2>/dev/null || true",
        ),
        (
            "multiple comments",
            "#!/bin/sh\n# comment 1\n# comment 2\nbitloops hooks git pre-push \"$1\" || true\n",
            "bitloops hooks git pre-push \"$1\" || true",
        ),
        ("empty content", "", ""),
        ("only comments", "#!/bin/sh\n# just a comment\n", ""),
        (
            "whitespace around command",
            "#!/bin/sh\n# comment\n  bitloops hooks git commit-msg \"$1\" || exit 1  \n",
            "bitloops hooks git commit-msg \"$1\" || exit 1",
        ),
    ];

    for (_name, content, want) in cases {
        let got = extract_command_line(content);
        assert_eq!(got, want);
    }
}

#[test]
fn test_check_and_warn_hook_managers_no_managers() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    with_repo_cwd(dir.path(), || {
        let mut buf = Vec::new();
        check_and_warn_hook_managers(&mut buf, false);
        assert!(buf.is_empty(), "expected no warning output");
    });
}

#[test]
fn test_check_and_warn_hook_managers_with_husky() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    fs::create_dir_all(dir.path().join(".husky/_")).unwrap();

    with_repo_cwd(dir.path(), || {
        let mut buf = Vec::new();
        check_and_warn_hook_managers(&mut buf, false);
        let out = String::from_utf8(buf).expect("utf8");
        assert!(out.contains("Warning: Husky detected"), "output was: {out}");
    });
}

#[test]
fn get_git_dir_in_path_regular_repo() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    let result = get_git_dir(dir.path()).unwrap();
    let expected = dir.path().join(".git");

    assert_eq!(
        fs::canonicalize(result).unwrap(),
        fs::canonicalize(expected).unwrap()
    );
}

#[test]
fn get_git_dir_in_path_worktree() {
    let (_parent, main_repo, worktree_dir) = init_hooks_worktree_repo();

    let result = get_git_dir(&worktree_dir).unwrap();
    let result_resolved = fs::canonicalize(result).unwrap();
    let expected_prefix = fs::canonicalize(main_repo.join(".git").join("worktrees")).unwrap();

    assert!(
        result_resolved.starts_with(&expected_prefix),
        "expected git dir under {}, got {}",
        expected_prefix.display(),
        result_resolved.display()
    );
}

#[test]
fn get_git_dir_in_path_not_a_repo() {
    let dir = tempfile::tempdir().unwrap();
    let err = get_git_dir(dir.path()).unwrap_err();
    assert_eq!(err.to_string(), "not a git repository");
}

#[test]
fn get_hooks_dir_in_path_regular_repo() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    let result = get_hooks_dir(dir.path()).unwrap();
    let expected = dir.path().join(".git").join("hooks");

    assert_eq!(
        fs::canonicalize(result).unwrap(),
        fs::canonicalize(expected).unwrap()
    );
}

#[test]
fn get_hooks_dir_in_path_worktree() {
    let (_parent, main_repo, worktree_dir) = init_hooks_worktree_repo();

    let result = get_hooks_dir(&worktree_dir).unwrap();
    let expected = main_repo.join(".git").join("hooks");

    assert_eq!(
        fs::canonicalize(result).unwrap(),
        fs::canonicalize(expected).unwrap()
    );
}

#[test]
fn get_hooks_dir_in_path_core_hooks_path() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    run_git_checked(dir.path(), &["config", "core.hooksPath", ".githooks"]);
    let relative_result = get_hooks_dir(dir.path()).unwrap();
    let relative_expected = dir.path().join(".githooks");
    assert_eq!(relative_result, relative_expected);

    let abs_hooks_path = dir.path().join("abs-hooks");
    run_git_checked(
        dir.path(),
        &[
            "config",
            "core.hooksPath",
            abs_hooks_path.to_string_lossy().as_ref(),
        ],
    );
    let absolute_result = get_hooks_dir(dir.path()).unwrap();
    assert_eq!(absolute_result, abs_hooks_path);
}

#[test]
fn install_creates_four_scripts() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    let count = install_git_hooks(dir.path(), false).unwrap();
    assert_eq!(count, 4, "should install 4 hooks");

    let hooks_dir = get_hooks_dir(dir.path()).unwrap();
    for name in HOOK_NAMES {
        let path = hooks_dir.join(name);
        assert!(path.exists(), "{name} should exist");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::metadata(&path).unwrap().permissions();
            assert!(perms.mode() & 0o111 != 0, "{name} should be executable");
        }
    }
}

#[test]
fn install_scripts_contain_marker() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    install_git_hooks(dir.path(), false).unwrap();

    let hooks_dir = get_hooks_dir(dir.path()).unwrap();
    for name in HOOK_NAMES {
        let content = fs::read_to_string(hooks_dir.join(name)).unwrap();
        assert!(
            content.contains(HOOK_MARKER),
            "{name} should contain the Bitloops marker"
        );
    }
}

#[test]
fn install_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let first_count = install_git_hooks(dir.path(), false).unwrap();
    assert!(first_count > 0, "first install should install hooks");

    let hooks_dir = get_hooks_dir(dir.path()).unwrap();
    let mut first_contents = std::collections::BTreeMap::new();
    for &hook in HOOK_NAMES {
        let data = fs::read_to_string(hooks_dir.join(hook)).unwrap();
        assert!(
            data.contains(HOOK_MARKER),
            "{hook} should contain the Bitloops marker"
        );
        first_contents.insert(hook.to_string(), data);
    }

    let second_count = install_git_hooks(dir.path(), false).unwrap();
    assert_eq!(second_count, 0, "second install should report 0 new hooks");

    for &hook in HOOK_NAMES {
        let data = fs::read_to_string(hooks_dir.join(hook)).unwrap();
        assert_eq!(
            data, first_contents[hook],
            "{hook} content changed after idempotent reinstall"
        );
    }
}

#[test]
fn install_backs_up_existing_hook() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    let hooks_dir = get_hooks_dir(dir.path()).unwrap();
    fs::create_dir_all(&hooks_dir).unwrap();
    fs::write(
        hooks_dir.join("prepare-commit-msg"),
        "#!/bin/sh\necho existing\n",
    )
    .unwrap();

    install_git_hooks(dir.path(), false).unwrap();

    let backup = hooks_dir.join(format!("prepare-commit-msg{BACKUP_SUFFIX}"));
    assert!(backup.exists(), "backup file should exist");
    let backup_content = fs::read_to_string(&backup).unwrap();
    assert!(
        backup_content.contains("existing"),
        "backup should contain original content"
    );
}

#[test]
fn install_chains_to_backup() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    let hooks_dir = get_hooks_dir(dir.path()).unwrap();
    fs::create_dir_all(&hooks_dir).unwrap();
    fs::write(hooks_dir.join("post-commit"), "#!/bin/sh\necho existing\n").unwrap();

    install_git_hooks(dir.path(), false).unwrap();

    let content = fs::read_to_string(hooks_dir.join("post-commit")).unwrap();
    assert!(
        content.contains(BACKUP_SUFFIX),
        "installed script should chain to backup"
    );
}

#[test]
fn uninstall_removes_bitloops_hooks() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    install_git_hooks(dir.path(), false).unwrap();

    let removed = uninstall_git_hooks(dir.path()).unwrap();
    assert_eq!(removed, 4, "should remove 4 hooks");

    let hooks_dir = get_hooks_dir(dir.path()).unwrap();
    for name in HOOK_NAMES {
        assert!(
            !hooks_dir.join(name).exists(),
            "{name} should have been removed"
        );
    }
}

#[test]
fn uninstall_restores_backups() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    let hooks_dir = get_hooks_dir(dir.path()).unwrap();
    fs::create_dir_all(&hooks_dir).unwrap();
    fs::write(hooks_dir.join("commit-msg"), "#!/bin/sh\necho original\n").unwrap();

    install_git_hooks(dir.path(), false).unwrap();
    uninstall_git_hooks(dir.path()).unwrap();

    let restored = hooks_dir.join("commit-msg");
    assert!(restored.exists(), "original hook should be restored");
    let content = fs::read_to_string(&restored).unwrap();
    assert!(
        content.contains("original"),
        "restored hook should contain original content"
    );
}

#[test]
fn is_git_hook_installed_tracks_install_state() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    assert!(
        !is_git_hook_installed(dir.path()),
        "hooks should not be detected before install"
    );
    install_git_hooks(dir.path(), false).unwrap();
    assert!(
        is_git_hook_installed(dir.path()),
        "hooks should be detected after install"
    );
}

#[test]
fn install_respects_core_hookspath() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    run_git_checked(dir.path(), &["config", "core.hooksPath", ".husky/_"]);
    let count = install_git_hooks(dir.path(), false).unwrap();
    assert!(
        count > 0,
        "install should write hooks when core.hooksPath is set"
    );

    let configured_hooks_dir = dir.path().join(".husky").join("_");
    for &hook in HOOK_NAMES {
        let data = fs::read_to_string(configured_hooks_dir.join(hook)).unwrap();
        assert!(
            data.contains(HOOK_MARKER),
            "{hook} in core.hooksPath should contain Bitloops marker"
        );
    }

    let default_hooks_dir = dir.path().join(".git").join("hooks");
    for &hook in HOOK_NAMES {
        let default_hook_path = default_hooks_dir.join(hook);
        if let Ok(data) = fs::read_to_string(default_hook_path) {
            assert!(
                !data.contains(HOOK_MARKER),
                "default hook {hook} should not contain Bitloops marker when core.hooksPath is set"
            );
        }
    }

    assert!(
        is_git_hook_installed(dir.path()),
        "is_git_hook_installed should detect hooks in core.hooksPath"
    );
}

#[test]
fn install_from_worktree_writes_common_hooks_dir() {
    let (_parent, main_repo, worktree_dir) = init_hooks_worktree_repo();

    install_git_hooks(&worktree_dir, false).unwrap();

    let common_hooks_dir = main_repo.join(".git").join("hooks");
    for &name in HOOK_NAMES {
        let data = fs::read_to_string(common_hooks_dir.join(name)).unwrap();
        assert!(
            data.contains(HOOK_MARKER),
            "common hook {name} should contain Bitloops marker"
        );
    }

    let worktree_git_dir_raw = run_git_checked(&worktree_dir, &["rev-parse", "--git-dir"]);
    let worktree_git_dir = if Path::new(&worktree_git_dir_raw).is_absolute() {
        PathBuf::from(worktree_git_dir_raw)
    } else {
        worktree_dir.join(worktree_git_dir_raw)
    };

    for &name in HOOK_NAMES {
        let worktree_local_hook = worktree_git_dir.join("hooks").join(name);
        if let Ok(data) = fs::read_to_string(worktree_local_hook) {
            assert!(
                !data.contains(HOOK_MARKER),
                "worktree-local hook {name} should not contain marker"
            );
        }
    }

    assert!(
        is_git_hook_installed(&worktree_dir),
        "worktree should report hooks as installed via common hooks dir"
    );
}

#[test]
fn remove_from_core_hookspath_relative() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    run_git_checked(dir.path(), &["config", "core.hooksPath", ".husky/_"]);
    let install_count = install_git_hooks(dir.path(), false).unwrap();
    assert!(install_count > 0);

    let configured_hooks_dir = dir.path().join(".husky").join("_");
    for &hook in HOOK_NAMES {
        assert!(configured_hooks_dir.join(hook).exists());
    }

    let removed = uninstall_git_hooks(dir.path()).unwrap();
    assert_eq!(removed, install_count);
    for &hook in HOOK_NAMES {
        assert!(
            !configured_hooks_dir.join(hook).exists(),
            "{hook} should be removed from core.hooksPath"
        );
    }
    assert!(!is_git_hook_installed(dir.path()));
}

#[test]
fn remove_git_hook_no_hooks_installed() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let removed = uninstall_git_hooks(dir.path()).unwrap();
    assert_eq!(removed, 0);
}

#[test]
fn remove_git_hook_ignores_non_bitloops_hooks() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    let hooks_dir = get_hooks_dir(dir.path()).unwrap();
    fs::create_dir_all(&hooks_dir).unwrap();
    let custom_hook = hooks_dir.join("pre-commit");
    fs::write(&custom_hook, "#!/bin/sh\necho custom hook\n").unwrap();

    let removed = uninstall_git_hooks(dir.path()).unwrap();
    assert_eq!(removed, 0);
    assert!(custom_hook.exists());
}

#[test]
fn remove_git_hook_not_a_git_repo() {
    let dir = tempfile::tempdir().unwrap();
    let err = uninstall_git_hooks(dir.path()).unwrap_err();
    assert!(
        err.to_string().contains("not a git repository"),
        "unexpected error: {err}"
    );
}

#[test]
fn install_does_not_overwrite_existing_backup() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let hooks_dir = get_hooks_dir(dir.path()).unwrap();
    fs::create_dir_all(&hooks_dir).unwrap();

    let backup_path = hooks_dir.join(format!("prepare-commit-msg{BACKUP_SUFFIX}"));
    let first_backup = "#!/bin/sh\necho 'first custom hook'\n";
    fs::write(&backup_path, first_backup).unwrap();

    let hook_path = hooks_dir.join("prepare-commit-msg");
    fs::write(&hook_path, "#!/bin/sh\necho 'second custom hook'\n").unwrap();

    install_git_hooks(dir.path(), false).unwrap();

    let backup_after = fs::read_to_string(&backup_path).unwrap();
    assert_eq!(backup_after, first_backup);

    let hook_after = fs::read_to_string(&hook_path).unwrap();
    assert!(hook_after.contains(HOOK_MARKER));
    assert!(hook_after.contains("# Chain: run pre-existing hook"));
}

#[test]
fn install_idempotent_with_chaining() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let hooks_dir = get_hooks_dir(dir.path()).unwrap();
    fs::create_dir_all(&hooks_dir).unwrap();
    fs::write(
        hooks_dir.join("prepare-commit-msg"),
        "#!/bin/sh\necho custom\n",
    )
    .unwrap();

    let first_count = install_git_hooks(dir.path(), false).unwrap();
    assert!(first_count > 0);
    let second_count = install_git_hooks(dir.path(), false).unwrap();
    assert_eq!(second_count, 0);
}

#[test]
fn install_no_backup_when_no_existing_hook() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    install_git_hooks(dir.path(), false).unwrap();

    let hooks_dir = get_hooks_dir(dir.path()).unwrap();
    for &hook in HOOK_NAMES {
        assert!(
            !hooks_dir.join(format!("{hook}{BACKUP_SUFFIX}")).exists(),
            "fresh install should not create backup for {hook}"
        );
        let data = fs::read_to_string(hooks_dir.join(hook)).unwrap();
        assert!(
            !data.contains("# Chain: run pre-existing hook"),
            "{hook} should not contain chain call on fresh install"
        );
    }
}

#[test]
fn install_mixed_hooks() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let hooks_dir = get_hooks_dir(dir.path()).unwrap();
    fs::create_dir_all(&hooks_dir).unwrap();

    fs::write(
        hooks_dir.join("prepare-commit-msg"),
        "#!/bin/sh\necho 'custom pcm'\n",
    )
    .unwrap();
    fs::write(
        hooks_dir.join("pre-push"),
        "#!/bin/sh\necho 'custom prepush'\n",
    )
    .unwrap();

    install_git_hooks(dir.path(), false).unwrap();

    for name in ["prepare-commit-msg", "pre-push"] {
        assert!(
            hooks_dir.join(format!("{name}{BACKUP_SUFFIX}")).exists(),
            "backup for {name} should exist"
        );
        let data = fs::read_to_string(hooks_dir.join(name)).unwrap();
        assert!(data.contains("# Chain: run pre-existing hook"));
    }

    for name in ["commit-msg", "post-commit"] {
        assert!(
            !hooks_dir.join(format!("{name}{BACKUP_SUFFIX}")).exists(),
            "backup for {name} should not exist"
        );
        let data = fs::read_to_string(hooks_dir.join(name)).unwrap();
        assert!(!data.contains("# Chain: run pre-existing hook"));
    }
}

#[test]
fn remove_restores_backup_when_hook_already_gone() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    let hooks_dir = get_hooks_dir(dir.path()).unwrap();
    fs::create_dir_all(&hooks_dir).unwrap();
    let hook_path = hooks_dir.join("prepare-commit-msg");
    let custom_content = "#!/bin/sh\necho 'original'\n";
    fs::write(&hook_path, custom_content).unwrap();

    install_git_hooks(dir.path(), false).unwrap();
    fs::remove_file(&hook_path).unwrap();

    uninstall_git_hooks(dir.path()).unwrap();

    let restored = fs::read_to_string(&hook_path).unwrap();
    assert_eq!(restored, custom_content);
    assert!(
        !hooks_dir
            .join(format!("prepare-commit-msg{BACKUP_SUFFIX}"))
            .exists()
    );
}

#[test]
fn test_generate_chained_content() {
    let base = "#!/bin/sh\n# Bitloops git hooks\nbitloops hooks git pre-push \"$1\" || true\n";
    let result = generate_chained_content(base, "pre-push");

    assert!(result.starts_with(base));
    assert!(result.contains("# Chain: run pre-existing hook"));
    assert!(result.contains("_bitloops_hook_dir=\"$(dirname \"$0\")\""));
    assert!(result.contains(&format!(
        "[ -x \"$_bitloops_hook_dir/pre-push{BACKUP_SUFFIX}\" ]"
    )));
    assert!(result.contains(&format!(
        "\"$_bitloops_hook_dir/pre-push{BACKUP_SUFFIX}\" \"$@\""
    )));
}

#[test]
fn install_remove_reinstall_with_backup_cycle() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let hooks_dir = get_hooks_dir(dir.path()).unwrap();
    fs::create_dir_all(&hooks_dir).unwrap();

    let hook_path = hooks_dir.join("prepare-commit-msg");
    let custom_content = "#!/bin/sh\necho 'user hook'\n";
    fs::write(&hook_path, custom_content).unwrap();

    let first_count = install_git_hooks(dir.path(), false).unwrap();
    assert!(first_count > 0);
    let backup_path = hooks_dir.join(format!("prepare-commit-msg{BACKUP_SUFFIX}"));
    assert!(backup_path.exists());

    uninstall_git_hooks(dir.path()).unwrap();
    let restored = fs::read_to_string(&hook_path).unwrap();
    assert_eq!(restored, custom_content);
    assert!(!backup_path.exists());

    let reinstall_count = install_git_hooks(dir.path(), false).unwrap();
    assert!(reinstall_count > 0);
    assert!(backup_path.exists());
    let reinstalled = fs::read_to_string(&hook_path).unwrap();
    assert!(reinstalled.contains(HOOK_MARKER));
    assert!(reinstalled.contains("# Chain: run pre-existing hook"));
}

#[test]
fn remove_does_not_overwrite_replaced_hook() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let hooks_dir = get_hooks_dir(dir.path()).unwrap();
    fs::create_dir_all(&hooks_dir).unwrap();

    let hook_path = hooks_dir.join("prepare-commit-msg");
    let hook_a = "#!/bin/sh\necho 'hook A'\n";
    fs::write(&hook_path, hook_a).unwrap();
    install_git_hooks(dir.path(), false).unwrap();

    let hook_b = "#!/bin/sh\necho 'hook B'\n";
    fs::write(&hook_path, hook_b).unwrap();
    uninstall_git_hooks(dir.path()).unwrap();

    let data = fs::read_to_string(&hook_path).unwrap();
    assert_eq!(data, hook_b);
    assert!(
        hooks_dir
            .join(format!("prepare-commit-msg{BACKUP_SUFFIX}"))
            .exists(),
        "backup should be left in place when managed hook was replaced"
    );
}

#[cfg(unix)]
#[test]
fn remove_permission_denied() {
    let uid = std::process::Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse::<u32>().ok())
        .unwrap_or(1000);
    if uid == 0 {
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    install_git_hooks(dir.path(), false).unwrap();

    let hooks_dir = get_hooks_dir(dir.path()).unwrap();
    let mut perms = fs::metadata(&hooks_dir).unwrap().permissions();
    perms.set_mode(0o555);
    fs::set_permissions(&hooks_dir, perms).unwrap();

    let result = uninstall_git_hooks(dir.path());

    let mut restore = fs::metadata(&hooks_dir).unwrap().permissions();
    restore.set_mode(0o755);
    let _ = fs::set_permissions(&hooks_dir, restore);

    let err = result.expect_err("expected permission error when hooks dir is read-only");
    assert!(
        err.to_string().contains("removing hook"),
        "unexpected error: {err}"
    );
}

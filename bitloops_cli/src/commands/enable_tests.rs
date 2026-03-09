use super::*;
use crate::commands::{Cli, Commands};
use crate::engine::settings::{SETTINGS_DIR, settings_local_path, settings_path};
use crate::test_support::process_state::{with_cwd, with_env_var, with_env_vars};
use clap::Parser;
use tempfile::TempDir;

fn setup_settings(dir: &TempDir, content: &str) {
    let settings_dir = dir.path().join(SETTINGS_DIR);
    fs::create_dir_all(&settings_dir).unwrap();
    fs::write(settings_path(dir.path()), content).unwrap();
}

fn setup_local_settings(dir: &TempDir, content: &str) {
    let settings_dir = dir.path().join(SETTINGS_DIR);
    fs::create_dir_all(&settings_dir).unwrap();
    fs::write(settings_local_path(dir.path()), content).unwrap();
}

fn setup_git_repo(dir: &TempDir) {
    let status = Command::new("git")
        .args(["init", "-q"])
        .current_dir(dir.path())
        .status()
        .unwrap();
    assert!(status.success(), "git init should succeed");
}

fn with_repo_cwd<T>(path: &Path, f: impl FnOnce() -> T) -> T {
    with_cwd(path, f)
}

fn run_enable_command(args: EnableArgs) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    runtime.block_on(run(args))
}

/// Sets `enabled = true` in the project settings file and prints a confirmation.
fn run_enable(repo_root: &Path, out: &mut dyn Write) -> Result<()> {
    let path = settings_path(repo_root);
    let mut settings = load_from_file_or_default(&path);
    settings.enabled = true;
    save_settings(&settings, &path)?;
    writeln!(out, "Bitloops is enabled.")?;
    Ok(())
}

#[test]
fn run_enable_sets_enabled_true() {
    let dir = tempfile::tempdir().unwrap();
    setup_settings(&dir, r#"{"strategy": "manual-commit", "enabled": false}"#);

    let mut out = Vec::new();
    run_enable(dir.path(), &mut out).unwrap();

    let output = String::from_utf8(out).unwrap();
    assert!(
        output.contains("enabled"),
        "output should mention 'enabled': {output}"
    );

    let settings = load_settings(dir.path()).unwrap();
    assert!(
        settings.enabled,
        "Bitloops should be enabled after run_enable"
    );
}

#[test]
fn run_enable_already_enabled() {
    let dir = tempfile::tempdir().unwrap();
    setup_settings(&dir, r#"{"strategy": "manual-commit", "enabled": true}"#);

    let mut out = Vec::new();
    run_enable(dir.path(), &mut out).unwrap();

    let output = String::from_utf8(out).unwrap();
    assert!(
        output.contains("enabled"),
        "output should mention 'enabled': {output}"
    );
}

#[test]
fn run_disable_sets_enabled_false() {
    let dir = tempfile::tempdir().unwrap();
    setup_settings(&dir, r#"{"strategy": "manual-commit", "enabled": true}"#);

    let mut out = Vec::new();
    run_disable(dir.path(), &mut out, false).unwrap();

    let output = String::from_utf8(out).unwrap();
    assert!(
        output.contains("disabled"),
        "output should mention 'disabled': {output}"
    );

    assert!(
        !settings::is_enabled(dir.path()).unwrap(),
        "Bitloops should be disabled after run_disable"
    );
}

#[test]
fn run_disable_already_disabled() {
    let dir = tempfile::tempdir().unwrap();
    setup_settings(&dir, r#"{"strategy": "manual-commit", "enabled": false}"#);

    let mut out = Vec::new();
    run_disable(dir.path(), &mut out, false).unwrap();

    let output = String::from_utf8(out).unwrap();
    assert!(
        output.contains("disabled"),
        "output should mention 'disabled': {output}"
    );
}

#[test]
fn check_disabled_guard_test() {
    let dir = tempfile::tempdir().unwrap();

    // No settings file → not disabled (defaults to enabled)
    let mut out = Vec::new();
    assert!(
        !check_disabled_guard(dir.path(), &mut out),
        "should return false when no settings file"
    );
    assert!(
        String::from_utf8(out).unwrap().is_empty(),
        "should print nothing when enabled"
    );

    // Settings with enabled: true → not disabled
    setup_settings(&dir, r#"{"enabled": true}"#);
    let mut out = Vec::new();
    assert!(
        !check_disabled_guard(dir.path(), &mut out),
        "should return false when enabled"
    );

    // Settings with enabled: false → disabled
    setup_settings(&dir, r#"{"enabled": false}"#);
    let mut out = Vec::new();
    assert!(
        check_disabled_guard(dir.path(), &mut out),
        "should return true when disabled"
    );
    let output = String::from_utf8(out).unwrap();
    assert!(
        output.contains("Bitloops is disabled"),
        "should print disabled message: {output}"
    );
    assert!(
        output.contains("bitloops enable"),
        "should mention 'bitloops enable': {output}"
    );
}

#[test]
fn run_disable_with_local_settings() {
    let dir = tempfile::tempdir().unwrap();
    setup_settings(&dir, r#"{"strategy": "manual-commit", "enabled": true}"#);
    setup_local_settings(&dir, r#"{"enabled": true}"#);

    let mut out = Vec::new();
    run_disable(dir.path(), &mut out, false).unwrap();

    let merged = load_settings(dir.path()).unwrap();
    assert!(!merged.enabled, "merged settings should be disabled");

    let local_content = fs::read_to_string(settings_local_path(dir.path())).unwrap();
    assert!(
        local_content.contains("\"enabled\": false"),
        "local settings should be updated: {local_content}"
    );
}

#[test]
fn run_disable_with_project_flag() {
    let dir = tempfile::tempdir().unwrap();
    setup_settings(&dir, r#"{"strategy": "manual-commit", "enabled": true}"#);
    setup_local_settings(&dir, r#"{"enabled": true}"#);

    let mut out = Vec::new();
    run_disable(dir.path(), &mut out, true).unwrap();

    let project_content = fs::read_to_string(settings_path(dir.path())).unwrap();
    assert!(
        project_content.contains("\"enabled\": false"),
        "project settings should be disabled: {project_content}"
    );

    let local_content = fs::read_to_string(settings_local_path(dir.path())).unwrap();
    assert!(
        local_content.contains("\"enabled\": true"),
        "local settings should remain untouched: {local_content}"
    );
}

#[test]
fn run_disable_creates_local_settings_when_missing() {
    let dir = tempfile::tempdir().unwrap();
    setup_settings(&dir, r#"{"strategy": "manual-commit", "enabled": true}"#);

    let mut out = Vec::new();
    run_disable(dir.path(), &mut out, false).unwrap();

    let local_content = fs::read_to_string(settings_local_path(dir.path())).unwrap();
    assert!(
        local_content.contains("\"enabled\": false"),
        "local settings should be created and disabled: {local_content}"
    );

    let project_content = fs::read_to_string(settings_path(dir.path())).unwrap();
    assert!(
        project_content.contains("\"enabled\": true"),
        "project settings should remain enabled: {project_content}"
    );
}

#[test]
fn determine_settings_target_explicit_local_flag() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join(SETTINGS_DIR)).unwrap();
    fs::write(settings_path(dir.path()), "{}").unwrap();
    let (path, notify) = determine_settings_target(dir.path(), true, false);
    assert_eq!(path, settings_local_path(dir.path()));
    assert!(!notify);
}

#[test]
fn determine_settings_target_explicit_project_flag() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join(SETTINGS_DIR)).unwrap();
    fs::write(settings_path(dir.path()), "{}").unwrap();
    let (path, notify) = determine_settings_target(dir.path(), false, true);
    assert_eq!(path, settings_path(dir.path()));
    assert!(!notify);
}

#[test]
fn determine_settings_target_settings_exists_no_flags() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join(SETTINGS_DIR)).unwrap();
    fs::write(settings_path(dir.path()), "{}").unwrap();
    let (path, notify) = determine_settings_target(dir.path(), false, false);
    assert_eq!(path, settings_local_path(dir.path()));
    assert!(notify);
}

#[test]
fn determine_settings_target_settings_not_exists_no_flags() {
    let dir = tempfile::tempdir().unwrap();
    let (path, notify) = determine_settings_target(dir.path(), false, false);
    assert_eq!(path, settings_path(dir.path()));
    assert!(!notify);
}

#[test]
fn run_enable_with_strategy_preserves_existing_settings() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    setup_settings(
        &dir,
        r#"{"strategy":"manual-commit","enabled":true,"strategy_options":{"push":true,"some_other_option":"value"}}"#,
    );

    run_enable_with_strategy(dir.path(), "auto-commit", false, false).unwrap();

    let merged = load_settings(dir.path()).unwrap();
    assert_eq!(merged.strategy, "auto-commit");
    assert_eq!(
        merged
            .strategy_options
            .get("push")
            .and_then(|v| v.as_bool()),
        Some(true),
        "strategy_options.push should be preserved"
    );
    assert_eq!(
        merged
            .strategy_options
            .get("some_other_option")
            .and_then(|v| v.as_str()),
        Some("value"),
        "strategy_options.some_other_option should be preserved"
    );
}

#[test]
fn setup_bitloops_dir_writes_all_required_gitignore_entries() {
    let dir = tempfile::tempdir().unwrap();

    setup_bitloops_dir(dir.path()).unwrap();

    let gitignore = fs::read_to_string(dir.path().join(SETTINGS_DIR).join(".gitignore"))
        .expect("expected .bitloops/.gitignore to exist");

    for required in ["tmp/", "settings.local.json", "metadata/", "logs/"] {
        assert!(
            gitignore.contains(required),
            "missing required entry {required} in .bitloops/.gitignore:\n{gitignore}"
        );
    }
}

#[test]
fn setup_bitloops_dir_preserves_existing_gitignore_content() {
    let dir = tempfile::tempdir().unwrap();
    let bitloops_dir = dir.path().join(SETTINGS_DIR);
    fs::create_dir_all(&bitloops_dir).unwrap();
    fs::write(
        bitloops_dir.join(".gitignore"),
        "custom-entry/\nsettings.local.json\n",
    )
    .unwrap();

    setup_bitloops_dir(dir.path()).unwrap();

    let gitignore = fs::read_to_string(bitloops_dir.join(".gitignore")).unwrap();
    assert!(
        gitignore.contains("custom-entry/"),
        "existing content should be preserved:\n{gitignore}"
    );
    for required in ["tmp/", "settings.local.json", "metadata/", "logs/"] {
        assert!(
            gitignore.contains(required),
            "missing required entry {required} in .bitloops/.gitignore:\n{gitignore}"
        );
    }
}

#[test]
fn run_enable_with_strategy_preserves_local_settings() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    setup_settings(&dir, r#"{"strategy":"manual-commit","enabled":true}"#);
    setup_local_settings(&dir, r#"{"strategy_options":{"push":true}}"#);

    run_enable_with_strategy(dir.path(), "auto-commit", true, false).unwrap();

    let merged = load_settings(dir.path()).unwrap();
    assert_eq!(merged.strategy, "auto-commit");
    assert_eq!(
        merged
            .strategy_options
            .get("push")
            .and_then(|v| v.as_bool()),
        Some(true),
        "local strategy options should be preserved"
    );
}

#[test]
fn run_uninstall_force_nothing_installed() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    with_repo_cwd(dir.path(), || {
        let mut out = Vec::new();
        let mut err = Vec::new();
        run_uninstall(dir.path(), &mut out, &mut err, true).unwrap();

        let output = String::from_utf8(out).unwrap();
        assert!(output.contains("not installed"), "{output}");
    });
}

#[test]
fn run_uninstall_force_removes_bitloops_directory() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    setup_settings(&dir, r#"{"enabled":true}"#);
    assert!(dir.path().join(SETTINGS_DIR).exists());

    let mut out = Vec::new();
    let mut err = Vec::new();
    run_uninstall(dir.path(), &mut out, &mut err, true).unwrap();

    assert!(
        !dir.path().join(SETTINGS_DIR).exists(),
        ".bitloops directory should be removed"
    );
    let output = String::from_utf8(out).unwrap();
    assert!(output.contains("uninstalled successfully"), "{output}");
}

#[test]
fn run_uninstall_force_removes_git_hooks() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    setup_settings(&dir, r#"{"enabled":true}"#);
    git_hooks::install_git_hooks(dir.path(), false).unwrap();
    assert!(
        git_hooks::is_git_hook_installed(dir.path()),
        "git hooks should be installed before uninstall"
    );

    let mut out = Vec::new();
    let mut err = Vec::new();
    run_uninstall(dir.path(), &mut out, &mut err, true).unwrap();

    assert!(
        !git_hooks::is_git_hook_installed(dir.path()),
        "git hooks should be removed"
    );
    let output = String::from_utf8(out).unwrap();
    assert!(output.contains("Removed git hooks"), "{output}");
}

#[test]
fn run_uninstall_not_a_git_repo() {
    let dir = tempfile::tempdir().unwrap();
    let mut out = Vec::new();
    let mut err = Vec::new();
    let res = run_uninstall(dir.path(), &mut out, &mut err, true);
    assert!(res.is_err(), "should fail outside git repo");
    let stderr = String::from_utf8(err).unwrap();
    assert!(
        stderr.contains("Not a git repository"),
        "stderr should mention git repo: {stderr}"
    );
}

#[test]
fn test_check_bitloops_dir_exists() {
    let dir = tempfile::tempdir().unwrap();
    assert!(!check_bitloops_dir_exists(dir.path()));
    fs::create_dir_all(dir.path().join(SETTINGS_DIR)).unwrap();
    assert!(check_bitloops_dir_exists(dir.path()));
}

#[test]
fn is_fully_enabled_not_enabled() {
    let dir = tempfile::tempdir().unwrap();
    let (enabled, _, _) = is_fully_enabled(dir.path());
    assert!(!enabled, "should not be fully enabled");
}

#[test]
fn is_fully_enabled_settings_disabled() {
    let dir = tempfile::tempdir().unwrap();
    setup_settings(&dir, r#"{"enabled":false}"#);
    let (enabled, _, _) = is_fully_enabled(dir.path());
    assert!(!enabled, "disabled settings should not be fully enabled");
}

#[test]
fn count_session_states_test() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    assert_eq!(count_session_states(dir.path()), 0);
}

#[test]
fn count_shadow_branches_test() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    assert_eq!(count_shadow_branches(dir.path()), 0);
}

#[test]
fn test_remove_bitloops_directory() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join(SETTINGS_DIR).join("subdir")).unwrap();
    fs::write(dir.path().join(SETTINGS_DIR).join("test.txt"), "test").unwrap();
    remove_bitloops_directory(dir.path()).unwrap();
    assert!(
        !dir.path().join(SETTINGS_DIR).exists(),
        ".bitloops should be removed"
    );
}

#[test]
fn shell_completion_target_test() {
    struct Case<'a> {
        shell: &'a str,
        create_bash_profile: bool,
        want_shell: &'a str,
        want_rc_suffix: &'a str,
        want_completion: &'a str,
        unsupported: bool,
    }

    let cases = vec![
        Case {
            shell: "/bin/zsh",
            create_bash_profile: false,
            want_shell: "Zsh",
            want_rc_suffix: ".zshrc",
            want_completion: "autoload -Uz compinit && compinit && source <(bitloops completion zsh)",
            unsupported: false,
        },
        Case {
            shell: "/bin/bash",
            create_bash_profile: false,
            want_shell: "Bash",
            want_rc_suffix: ".bashrc",
            want_completion: "source <(bitloops completion bash)",
            unsupported: false,
        },
        Case {
            shell: "/bin/bash",
            create_bash_profile: true,
            want_shell: "Bash",
            want_rc_suffix: ".bash_profile",
            want_completion: "source <(bitloops completion bash)",
            unsupported: false,
        },
        Case {
            shell: "/usr/bin/fish",
            create_bash_profile: false,
            want_shell: "Fish",
            want_rc_suffix: ".config/fish/config.fish",
            want_completion: "bitloops completion fish | source",
            unsupported: false,
        },
        Case {
            shell: "",
            create_bash_profile: false,
            want_shell: "",
            want_rc_suffix: "",
            want_completion: "",
            unsupported: true,
        },
    ];

    for case in cases {
        with_env_var("SHELL", Some(case.shell), || {
            let home = tempfile::tempdir().unwrap();
            if case.create_bash_profile {
                fs::write(home.path().join(".bash_profile"), "").unwrap();
            }
            let got = shell_completion_target(home.path());
            if case.unsupported {
                assert!(got.is_err(), "unsupported shell should return error");
                assert!(
                    format!("{:#}", got.unwrap_err()).contains("unsupported shell"),
                    "error should mention unsupported shell"
                );
                return;
            }
            let (shell, rc, completion) = got.unwrap();
            assert_eq!(shell, case.want_shell);
            assert!(
                rc.ends_with(case.want_rc_suffix),
                "rc path mismatch: got={:?}",
                rc
            );
            assert_eq!(completion, case.want_completion);
        });
    }
}

#[test]
fn append_shell_completion_test() {
    struct Case<'a> {
        rc_file_rel: &'a str,
        completion: &'a str,
        pre_existing: &'a str,
        create_parent: bool,
    }

    let cases = vec![
        Case {
            rc_file_rel: ".zshrc",
            completion: "source <(bitloops completion zsh)",
            pre_existing: "",
            create_parent: true,
        },
        Case {
            rc_file_rel: ".zshrc",
            completion: "source <(bitloops completion zsh)",
            pre_existing: "# existing\n",
            create_parent: true,
        },
        Case {
            rc_file_rel: ".config/fish/config.fish",
            completion: "bitloops completion fish | source",
            pre_existing: "",
            create_parent: false,
        },
        Case {
            rc_file_rel: ".config/fish/config.fish",
            completion: "bitloops completion fish | source",
            pre_existing: "",
            create_parent: true,
        },
    ];

    for case in cases {
        let home = tempfile::tempdir().unwrap();
        let rc_file = home.path().join(case.rc_file_rel);
        if case.create_parent {
            fs::create_dir_all(rc_file.parent().unwrap()).unwrap();
        }
        if !case.pre_existing.is_empty() {
            fs::write(&rc_file, case.pre_existing).unwrap();
        }
        append_shell_completion(&rc_file, case.completion).unwrap();
        let content = fs::read_to_string(&rc_file).unwrap();
        assert!(content.contains(SHELL_COMPLETION_COMMENT), "{content}");
        assert!(content.contains(case.completion), "{content}");
        if !case.pre_existing.is_empty() {
            assert!(
                content.starts_with(case.pre_existing),
                "pre-existing content should be preserved"
            );
        }
        assert!(rc_file.parent().unwrap().is_dir());
    }
}

#[test]
fn run_post_install_shell_completion_with_io_yes_appends() {
    let home = tempfile::tempdir().unwrap();
    let home_value = home.path().to_str().unwrap().to_string();
    with_env_vars(
        &[
            ("SHELL", Some("/bin/zsh")),
            ("HOME", Some(home_value.as_str())),
        ],
        || {
            let mut out = Vec::new();
            let mut input = std::io::Cursor::new(b"yes\n".to_vec());
            run_post_install_shell_completion_with_io(&mut out, &mut input).unwrap();

            let rc_file = home.path().join(".zshrc");
            let content = fs::read_to_string(&rc_file).unwrap();
            assert!(content.contains(SHELL_COMPLETION_COMMENT), "{content}");
            assert!(content.contains("bitloops completion zsh"), "{content}");
        },
    );
}

#[test]
fn run_post_install_shell_completion_with_io_no_skips_append() {
    let home = tempfile::tempdir().unwrap();
    let home_value = home.path().to_str().unwrap().to_string();
    with_env_vars(
        &[
            ("SHELL", Some("/bin/zsh")),
            ("HOME", Some(home_value.as_str())),
        ],
        || {
            let mut out = Vec::new();
            let mut input = std::io::Cursor::new(b"no\n".to_vec());
            run_post_install_shell_completion_with_io(&mut out, &mut input).unwrap();

            assert!(
                !home.path().join(".zshrc").exists(),
                "answering no should not create shell rc file"
            );
        },
    );
}

#[test]
fn run_post_install_shell_completion_with_io_already_configured() {
    let home = tempfile::tempdir().unwrap();
    let home_value = home.path().to_str().unwrap().to_string();
    with_env_vars(
        &[
            ("SHELL", Some("/bin/zsh")),
            ("HOME", Some(home_value.as_str())),
        ],
        || {
            let rc_file = home.path().join(".zshrc");
            fs::write(
                &rc_file,
                format!(
                    "{}\nsource <(bitloops completion zsh)\n",
                    SHELL_COMPLETION_COMMENT
                ),
            )
            .unwrap();
            let before = fs::read_to_string(&rc_file).unwrap();

            let mut out = Vec::new();
            let mut input = std::io::Cursor::new(b"yes\n".to_vec());
            run_post_install_shell_completion_with_io(&mut out, &mut input).unwrap();

            let after = fs::read_to_string(&rc_file).unwrap();
            assert_eq!(before, after, "existing completion should remain unchanged");
        },
    );
}

#[test]
fn run_post_install_shell_completion_with_io_unsupported_shell_is_non_fatal() {
    let home = tempfile::tempdir().unwrap();
    let home_value = home.path().to_str().unwrap().to_string();
    with_env_vars(
        &[
            ("SHELL", Some("/bin/tcsh")),
            ("HOME", Some(home_value.as_str())),
        ],
        || {
            let mut out = Vec::new();
            let mut input = std::io::Cursor::new(Vec::<u8>::new());
            run_post_install_shell_completion_with_io(&mut out, &mut input).unwrap();

            let rendered = String::from_utf8(out).unwrap();
            assert!(
                rendered.contains("Shell completion not available for your shell"),
                "unsupported shell should produce informative note: {rendered}"
            );
            assert!(
                !home.path().join(".zshrc").exists(),
                "unsupported shell should not create rc files"
            );
        },
    );
}

#[test]
fn remove_bitloops_directory_not_exists() {
    let dir = tempfile::tempdir().unwrap();
    remove_bitloops_directory(dir.path()).unwrap();
}

#[test]
fn enable_args_accepts_legacy_agent_flag() {
    let parsed = Cli::try_parse_from(["bitloops", "enable", "--agent", "cursor"])
        .expect("enable with --agent should parse");
    let Some(Commands::Enable(args)) = parsed.command else {
        panic!("expected enable command");
    };
    assert_eq!(args.agent.as_deref(), Some("cursor"));
}

#[test]
fn run_enable_without_agent_does_not_initialize_agents() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    with_repo_cwd(dir.path(), || {
        run_enable_command(EnableArgs {
            local: false,
            project: false,
            force: false,
            agent: None,
        })
        .unwrap();

        assert!(!dir.path().join(".claude/settings.json").exists());
        assert!(!dir.path().join(".cursor/hooks.json").exists());
        assert!(!dir.path().join(".gemini/settings.json").exists());
        assert!(!dir.path().join(".opencode/plugins/bitloops.ts").exists());
        assert!(git_hooks::is_git_hook_installed(dir.path()));
    });
}

#[test]
fn run_enable_with_legacy_agent_flag_still_does_not_initialize_agents() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    with_repo_cwd(dir.path(), || {
        run_enable_command(EnableArgs {
            local: false,
            project: false,
            force: false,
            agent: Some("cursor".to_string()),
        })
        .unwrap();

        assert!(!dir.path().join(".cursor/hooks.json").exists());
        assert!(!dir.path().join(".claude/settings.json").exists());
        assert!(git_hooks::is_git_hook_installed(dir.path()));
    });
}

#[test]
fn initialized_agents_returns_empty_without_hooks() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    with_repo_cwd(dir.path(), || {
        let agents = initialized_agents(dir.path());
        assert!(agents.is_empty());
    });
}

#[test]
fn initialized_agents_detects_claude_and_cursor() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    with_repo_cwd(dir.path(), || {
        claude_hooks::install_hooks(dir.path(), false).unwrap();
        HookSupport::install_hooks(&CursorAgent, false, false).unwrap();

        let agents = initialized_agents(dir.path());
        assert!(agents.contains(&"claude-code".to_string()));
        assert!(agents.contains(&"cursor".to_string()));
    });
}

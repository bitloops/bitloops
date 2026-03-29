use super::*;
use crate::adapters::agents::HookSupport;
use crate::adapters::agents::claude_code::hooks as claude_hooks;
use crate::adapters::agents::codex::hooks as codex_hooks;
use crate::adapters::agents::copilot::agent::CopilotCliAgent;
use crate::adapters::agents::cursor::agent::CursorAgent;
use crate::cli::{Cli, Commands};
use crate::config::settings::{SETTINGS_DIR, save_settings, settings_local_path, settings_path};
use crate::test_support::process_state::{git_command, with_cwd, with_env_var, with_env_vars};
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
    let status = git_command()
        .args(["init", "-q"])
        .current_dir(dir.path())
        .status()
        .unwrap();
    assert!(status.success(), "git init should succeed");
}

fn with_repo_cwd<T>(path: &Path, f: impl FnOnce() -> T) -> T {
    with_cwd(path, f)
}

fn with_legacy_local_backend<T>(f: impl FnOnce() -> T) -> T {
    f()
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
    setup_settings(
        &dir,
        r#"[capture]
strategy = "manual-commit"
enabled = false
"#,
    );

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
    setup_settings(
        &dir,
        r#"[capture]
strategy = "manual-commit"
enabled = true
"#,
    );

    let mut out = Vec::new();
    run_enable(dir.path(), &mut out).unwrap();

    let output = String::from_utf8(out).unwrap();
    assert!(
        output.contains("enabled"),
        "output should mention 'enabled': {output}"
    );
}

#[test]
fn run_disable_removes_installed_hooks_without_editing_policy() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    setup_settings(
        &dir,
        r#"[capture]
strategy = "manual-commit"
enabled = true
"#,
    );
    git_hooks::install_git_hooks(dir.path(), false).unwrap();
    codex_hooks::install_hooks_at(dir.path(), false, false).unwrap();

    let mut out = Vec::new();
    run_disable(dir.path(), &mut out, false).unwrap();

    let output = String::from_utf8(out).unwrap();
    assert!(
        output.contains("disabled"),
        "output should mention 'disabled': {output}"
    );
    assert!(
        git_command()
            .arg("rev-parse")
            .current_dir(dir.path())
            .status()
            .is_ok(),
        "sanity check git command should still work"
    );
    assert!(!git_hooks::is_git_hook_installed(dir.path()));
    assert!(!codex_hooks::are_hooks_installed_at(dir.path()));
    assert!(settings::is_enabled(dir.path()).unwrap());
}

#[test]
fn run_disable_already_disabled() {
    let dir = tempfile::tempdir().unwrap();
    setup_settings(
        &dir,
        r#"[capture]
strategy = "manual-commit"
enabled = false
"#,
    );

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
    setup_settings(
        &dir,
        r#"[capture]
enabled = true
"#,
    );
    let mut out = Vec::new();
    assert!(
        !check_disabled_guard(dir.path(), &mut out),
        "should return false when enabled"
    );

    // Settings with enabled: false → disabled
    setup_settings(
        &dir,
        r#"[capture]
enabled = false
"#,
    );
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
fn run_disable_leaves_local_policy_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    setup_settings(
        &dir,
        r#"[capture]
strategy = "manual-commit"
enabled = true
"#,
    );
    setup_local_settings(
        &dir,
        r#"[capture]
enabled = true
"#,
    );

    let mut out = Vec::new();
    run_disable(dir.path(), &mut out, false).unwrap();

    let local_content = fs::read_to_string(settings_local_path(dir.path())).unwrap();
    assert!(
        local_content.contains("enabled = true"),
        "local policy should remain unchanged: {local_content}"
    );
}

#[test]
fn run_disable_with_project_flag_leaves_policy_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    setup_settings(
        &dir,
        r#"[capture]
strategy = "manual-commit"
enabled = true
"#,
    );
    setup_local_settings(
        &dir,
        r#"[capture]
enabled = true
"#,
    );

    let mut out = Vec::new();
    run_disable(dir.path(), &mut out, true).unwrap();

    let project_content = fs::read_to_string(settings_path(dir.path())).unwrap();
    assert!(
        project_content.contains("enabled = true"),
        "shared policy should remain unchanged: {project_content}"
    );

    let local_content = fs::read_to_string(settings_local_path(dir.path())).unwrap();
    assert!(
        local_content.contains("enabled = true"),
        "local settings should remain untouched: {local_content}"
    );
}

#[test]
fn run_disable_does_not_create_local_policy_when_missing() {
    let dir = tempfile::tempdir().unwrap();
    setup_settings(
        &dir,
        r#"[capture]
strategy = "manual-commit"
enabled = true
"#,
    );

    let mut out = Vec::new();
    run_disable(dir.path(), &mut out, false).unwrap();

    assert!(!settings_local_path(dir.path()).exists());

    let project_content = fs::read_to_string(settings_path(dir.path())).unwrap();
    assert!(
        project_content.contains("enabled = true"),
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
fn run_enable_with_strategy_rewrites_repo_policy() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    setup_settings(
        &dir,
        r#"[capture]
strategy = "manual-commit"
enabled = true
push = true
some_other_option = "value"
"#,
    );

    run_enable_with_strategy(dir.path(), "auto-commit", false, false).unwrap();

    let merged = load_settings(dir.path()).unwrap();
    assert_eq!(merged.strategy, "auto-commit");
    assert!(merged.enabled);
    assert!(merged.strategy_options.is_empty());
}

#[test]
fn setup_bitloops_dir_creates_directory() {
    let dir = tempfile::tempdir().unwrap();

    setup_bitloops_dir(dir.path()).unwrap();

    assert!(dir.path().join(SETTINGS_DIR).is_dir());
}

#[test]
fn setup_bitloops_dir_preserves_existing_files() {
    let dir = tempfile::tempdir().unwrap();
    let bitloops_dir = dir.path().join(SETTINGS_DIR);
    fs::create_dir_all(&bitloops_dir).unwrap();
    fs::write(bitloops_dir.join("marker.txt"), "marker").unwrap();

    setup_bitloops_dir(dir.path()).unwrap();

    assert_eq!(
        fs::read_to_string(bitloops_dir.join("marker.txt")).unwrap(),
        "marker"
    );
}

#[test]
fn run_enable_with_strategy_preserves_local_settings() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    setup_settings(
        &dir,
        r#"[capture]
strategy = "manual-commit"
enabled = true
"#,
    );
    setup_local_settings(
        &dir,
        r#"[capture]
push = true
"#,
    );

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
    setup_settings(
        &dir,
        r#"[capture]
enabled = true
"#,
    );
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
    setup_settings(
        &dir,
        r#"[capture]
enabled = true
"#,
    );
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
fn run_uninstall_force_removes_codex_hooks() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    setup_settings(
        &dir,
        r#"[capture]
enabled = true
"#,
    );

    codex_hooks::install_hooks_at(dir.path(), false, false).unwrap();
    assert!(
        codex_hooks::are_hooks_installed_at(dir.path()),
        "codex hooks should be installed before uninstall"
    );

    let mut out = Vec::new();
    let mut err = Vec::new();
    run_uninstall(dir.path(), &mut out, &mut err, true).unwrap();

    assert!(
        !codex_hooks::are_hooks_installed_at(dir.path()),
        "codex hooks should be removed"
    );

    let output = String::from_utf8(out).unwrap();
    assert!(output.contains("Removed Codex hooks"), "{output}");
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
    setup_settings(
        &dir,
        r#"[capture]
enabled = false
"#,
    );
    let (enabled, _, _) = is_fully_enabled(dir.path());
    assert!(!enabled, "disabled settings should not be fully enabled");
}

#[test]
fn count_session_states_test() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    with_legacy_local_backend(|| {
        assert_eq!(count_session_states(dir.path()), 0);
    });
}

#[test]
fn count_session_states_includes_legacy_invalid_json_file() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    let backend =
        crate::host::checkpoints::session::local_backend::LocalFileBackend::new(dir.path());
    let sessions_dir = backend.sessions_dir();
    fs::create_dir_all(&sessions_dir).unwrap();
    fs::write(sessions_dir.join("legacy-invalid.json"), "{not-json").unwrap();

    with_legacy_local_backend(|| {
        assert_eq!(count_session_states(dir.path()), 0);
    });
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
fn run_enable_without_agent_installs_default_agent_and_git_hooks() {
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

        assert!(dir.path().join(".claude/settings.json").exists());
        assert!(!dir.path().join(".codex/hooks.json").exists());
        assert!(!dir.path().join(".cursor/hooks.json").exists());
        assert!(!dir.path().join(".gemini/settings.json").exists());
        assert!(!dir.path().join(".opencode/plugins/bitloops.ts").exists());
        assert!(git_hooks::is_git_hook_installed(dir.path()));
        let exclude = fs::read_to_string(dir.path().join(".git/info/exclude")).unwrap();
        assert!(exclude.contains(".bitloops.local.toml"));
        assert!(exclude.contains(".bitloops/"));
    });
}

#[test]
fn run_enable_with_legacy_agent_flag_installs_requested_agent_hooks() {
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

        assert!(dir.path().join(".cursor/hooks.json").exists());
        assert!(!dir.path().join(".claude/settings.json").exists());
        assert!(!dir.path().join(".codex/hooks.json").exists());
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
        codex_hooks::install_hooks_at(dir.path(), false, false).unwrap();

        let agents = initialized_agents(dir.path());
        assert!(agents.contains(&"claude-code".to_string()));
        assert!(agents.contains(&"codex".to_string()));
        assert!(agents.contains(&"cursor".to_string()));
    });
}

#[test]
fn initialized_agents_detects_installed_hooks_without_repo_cwd() {
    let dir = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    with_repo_cwd(dir.path(), || {
        claude_hooks::install_hooks(dir.path(), false).unwrap();
        HookSupport::install_hooks(&CursorAgent, false, false).unwrap();
        codex_hooks::install_hooks_at(dir.path(), false, false).unwrap();
    });

    with_cwd(other.path(), || {
        let agents = initialized_agents(dir.path());
        assert!(agents.contains(&"claude-code".to_string()));
        assert!(agents.contains(&"codex".to_string()));
        assert!(agents.contains(&"cursor".to_string()));
    });
}

#[test]
fn initialized_agents_detects_copilot() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    with_repo_cwd(dir.path(), || {
        HookSupport::install_hooks(&CopilotCliAgent, false, false).unwrap();

        let agents = initialized_agents(dir.path());
        assert!(agents.contains(&"copilot".to_string()));
    });
}

// ── repo policy and exclude handling ──────────────────────────────────

#[test]
fn repo_local_policy_exclude_is_added_to_git_info_exclude() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    ensure_repo_local_policy_excluded(dir.path()).unwrap();

    let exclude = fs::read_to_string(dir.path().join(".git/info/exclude")).unwrap();
    assert!(exclude.contains(".bitloops.local.toml"));
    assert!(exclude.contains(".bitloops/"));
}

#[test]
fn repo_local_policy_exclude_does_not_add_legacy_names() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    ensure_repo_local_policy_excluded(dir.path()).unwrap();

    let gitignore = fs::read_to_string(dir.path().join(".git/info/exclude")).unwrap();
    assert!(
        !gitignore.contains("settings.local.json"),
        "git exclude must not include legacy settings.local.json:\n{gitignore}"
    );
}

#[test]
fn enable_does_not_create_shared_repo_policy_file() {
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
    });

    assert!(!settings_path(dir.path()).exists());
}

#[test]
fn enable_with_local_flag_does_not_create_local_repo_policy_file() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    with_repo_cwd(dir.path(), || {
        run_enable_command(EnableArgs {
            local: true,
            project: false,
            force: false,
            agent: None,
        })
        .unwrap();
    });

    assert!(!settings_local_path(dir.path()).exists());
}

#[test]
fn disable_does_not_create_local_repo_policy_file() {
    let dir = tempfile::tempdir().unwrap();
    setup_settings(
        &dir,
        r#"[capture]
strategy = "manual-commit"
enabled = true
"#,
    );

    let mut out = Vec::new();
    run_disable(dir.path(), &mut out, false).unwrap();

    assert!(!settings_local_path(dir.path()).exists());
}

#[test]
fn disable_with_project_flag_does_not_rewrite_shared_repo_policy() {
    let dir = tempfile::tempdir().unwrap();
    setup_settings(
        &dir,
        r#"[capture]
strategy = "manual-commit"
enabled = true
"#,
    );

    let mut out = Vec::new();
    run_disable(dir.path(), &mut out, true).unwrap();

    let content =
        fs::read_to_string(settings_path(dir.path())).expect("shared policy should still exist");
    assert!(
        content.contains("enabled = true"),
        "shared repo policy should remain unchanged, got: {content}"
    );
}

#[test]
fn repo_policy_determine_target_returns_toml_policy_paths() {
    let dir = tempfile::tempdir().unwrap();

    // No flags, no existing file → .bitloops.toml
    let (path, _) = determine_settings_target(dir.path(), false, false);
    let filename = path.file_name().unwrap().to_str().unwrap();
    assert_eq!(
        filename, ".bitloops.toml",
        "default target should be .bitloops.toml, got: {filename}"
    );

    // Explicit --local → .bitloops.local.toml
    let (path, _) = determine_settings_target(dir.path(), true, false);
    let filename = path.file_name().unwrap().to_str().unwrap();
    assert_eq!(
        filename, ".bitloops.local.toml",
        "--local target should be .bitloops.local.toml, got: {filename}"
    );

    // Explicit --project → .bitloops.toml
    let (path, _) = determine_settings_target(dir.path(), false, true);
    let filename = path.file_name().unwrap().to_str().unwrap();
    assert_eq!(
        filename, ".bitloops.toml",
        "--project target should be .bitloops.toml, got: {filename}"
    );
}

#[test]
fn unified_config_enable_help_references_config_not_settings() {
    let help_text = Cli::try_parse_from(["bitloops", "enable", "--help"])
        .err()
        .expect("--help should return a clap error")
        .to_string();

    assert!(
        !help_text.contains("settings.json"),
        "enable --help must not reference legacy 'settings.json':\n{help_text}"
    );
    assert!(
        !help_text.contains("settings.local.json"),
        "enable --help must not reference legacy 'settings.local.json':\n{help_text}"
    );
}

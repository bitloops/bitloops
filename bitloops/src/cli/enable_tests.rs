use super::*;
use crate::adapters::agents::HookSupport;
use crate::adapters::agents::claude_code::hooks as claude_hooks;
use crate::adapters::agents::codex::hooks as codex_hooks;
use crate::adapters::agents::copilot::agent::CopilotCliAgent;
use crate::adapters::agents::cursor::agent::CursorAgent;
use crate::cli::telemetry_consent::{
    NON_INTERACTIVE_TELEMETRY_ERROR, with_global_graphql_executor_hook,
};
use crate::cli::{Cli, Commands};
use crate::config::default_daemon_config_path;
use crate::config::settings::{SETTINGS_DIR, save_settings, settings_local_path, settings_path};
use crate::test_support::process_state::{
    git_command, with_cwd, with_env_var, with_env_vars, with_process_state,
};
use clap::Parser;
use std::io::Cursor;
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

fn test_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
}

fn with_isolated_daemon_config_process_state<T>(
    cwd: Option<&Path>,
    extra_env: &[(&str, Option<&str>)],
    f: impl FnOnce() -> T,
) -> T {
    let config_dir = tempfile::tempdir().unwrap();
    let config_dir_value = config_dir.path().to_string_lossy().into_owned();
    let mut env = Vec::with_capacity(extra_env.len() + 1);
    env.push((
        "BITLOOPS_TEST_CONFIG_DIR_OVERRIDE",
        Some(config_dir_value.as_str()),
    ));
    env.extend_from_slice(extra_env);
    with_process_state(cwd, &env, f)
}

fn with_isolated_daemon_config<T>(f: impl FnOnce() -> T) -> T {
    with_isolated_daemon_config_process_state(None, &[], f)
}

fn with_ready_daemon_and_repo_cwd<T>(path: &Path, f: impl FnOnce() -> T) -> T {
    with_isolated_daemon_config_process_state(
        Some(path),
        &[
            ("BITLOOPS_TEST_ASSUME_DAEMON_RUNNING", Some("1")),
            ("BITLOOPS_TEST_TTY", Some("0")),
        ],
        || {
            with_global_graphql_executor_hook(
                |_runtime_root, _query, _variables| {
                    Ok(serde_json::json!({
                        "updateCliTelemetryConsent": {
                            "telemetry": true,
                            "needsPrompt": false
                        }
                    }))
                },
                f,
            )
        },
    )
}

fn with_enable_test_process_state<T>(
    path: &Path,
    telemetry_response: serde_json::Value,
    f: impl FnOnce() -> T,
) -> T {
    with_isolated_daemon_config_process_state(
        Some(path),
        &[
            ("BITLOOPS_TEST_ASSUME_DAEMON_RUNNING", Some("1")),
            ("BITLOOPS_TEST_TTY", Some("0")),
        ],
        || {
            with_global_graphql_executor_hook(
                move |_runtime_root, _query, _variables| Ok(telemetry_response.clone()),
                f,
            )
        },
    )
}

#[cfg(unix)]
fn fake_runtime_command_and_args(repo_root: &Path) -> (String, Vec<String>) {
    use std::os::unix::fs::PermissionsExt;

    let script_path = repo_root.join(".bitloops/test-bin/fake-enable-embeddings-runtime.sh");
    if let Some(parent) = script_path.parent() {
        fs::create_dir_all(parent).expect("create fake runtime dir");
    }
    let script = r#"#!/bin/sh
profile_name=fake
while [ $# -gt 0 ]; do
  case "$1" in
    --profile)
      profile_name=$2
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done
while IFS= read -r line; do
  req_id=$(printf '%s\n' "$line" | sed -n 's/.*"request_id":"\([^"]*\)".*/\1/p')
  case "$line" in
    *'"type":"describe"'*)
      printf '{"type":"describe","request_id":"%s","protocol_version":1,"runtime":{"protocol_version":1,"runtime_name":"bitloops-embeddings","runtime_version":"test","profile_name":"%s","provider":{"kind":"local_fastembed","provider_name":"local_fastembed","model_name":"test-model","output_dimension":3,"cache_dir":null}}}\n' "$req_id" "$profile_name"
      ;;
    *'"type":"embed_batch"'*)
      printf '{"type":"embed_batch","request_id":"%s","protocol_version":1,"vectors":[{"index":0,"values":[0.1,0.2,0.3]}]}\n' "$req_id"
      ;;
    *'"type":"shutdown"'*)
      printf '{"type":"shutdown","request_id":"%s","protocol_version":1,"accepted":true}\n' "$req_id"
      exit 0
      ;;
    *)
      printf '{"type":"error","request_id":"%s","code":"runtime_error","message":"unexpected request"}\n' "$req_id"
      ;;
  esac
done
"#;
    fs::write(&script_path, script).expect("write fake runtime script");
    let mut permissions = fs::metadata(&script_path)
        .expect("stat fake runtime script")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script_path, permissions).expect("chmod fake runtime script");
    ("sh".to_string(), vec![script_path.display().to_string()])
}

#[cfg(windows)]
fn fake_runtime_command_and_args(repo_root: &Path) -> (String, Vec<String>) {
    let script_path = repo_root.join(".bitloops/test-bin/fake-enable-embeddings-runtime.ps1");
    if let Some(parent) = script_path.parent() {
        fs::create_dir_all(parent).expect("create fake runtime dir");
    }
    let script = r#"
$profileName = "fake"
for ($i = 0; $i -lt $args.Length; $i++) {
  if ($args[$i] -eq "--profile" -and ($i + 1) -lt $args.Length) {
    $profileName = $args[$i + 1]
    break
  }
}
$stdin = [Console]::In
while (($line = $stdin.ReadLine()) -ne $null) {
  if ([string]::IsNullOrWhiteSpace($line)) { continue }
  $request = $line | ConvertFrom-Json
  switch ($request.type) {
    "describe" {
      $response = @{
        type = "describe"
        request_id = $request.request_id
        protocol_version = 1
        runtime = @{
          protocol_version = 1
          runtime_name = "bitloops-embeddings"
          runtime_version = "test"
          profile_name = $profileName
          provider = @{
            kind = "local_fastembed"
            provider_name = "local_fastembed"
            model_name = "test-model"
            output_dimension = 3
            cache_dir = $null
          }
        }
      }
    }
    "embed_batch" {
      $response = @{
        type = "embed_batch"
        request_id = $request.request_id
        protocol_version = 1
        vectors = @(@{
          index = 0
          values = @(0.1, 0.2, 0.3)
        })
      }
    }
    "shutdown" {
      $response = @{
        type = "shutdown"
        request_id = $request.request_id
        protocol_version = 1
        accepted = $true
      }
      $response | ConvertTo-Json -Compress
      break
    }
    default {
      $response = @{
        type = "error"
        request_id = $request.request_id
        code = "runtime_error"
        message = "unexpected request"
      }
    }
  }
  $response | ConvertTo-Json -Compress
}
"#;
    fs::write(&script_path, script).expect("write fake runtime script");
    (
        "powershell".to_string(),
        vec![
            "-NoProfile".to_string(),
            "-ExecutionPolicy".to_string(),
            "Bypass".to_string(),
            "-File".to_string(),
            script_path.display().to_string(),
        ],
    )
}

fn write_runtime_only_daemon_config(command: &str, args: &[String]) {
    let runtime_args = args
        .iter()
        .map(|arg| format!("{arg:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    let config_path = default_daemon_config_path().expect("default daemon config path");
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).expect("create daemon config dir");
    }
    fs::write(
        &config_path,
        format!(
            r#"
[runtime]
local_dev = false

[embeddings.runtime]
command = {command:?}
args = [{runtime_args}]
startup_timeout_secs = 5
request_timeout_secs = 5
"#
        ),
    )
    .expect("write daemon config");
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
    with_isolated_daemon_config(|| {
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
    });
}

#[test]
fn run_enable_already_enabled() {
    with_isolated_daemon_config(|| {
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
    });
}

#[test]
fn run_disable_removes_installed_hooks_without_editing_policy() {
    with_isolated_daemon_config(|| {
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
        assert!(dir.path().join(".codex/config.toml").exists());

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
        assert!(git_hooks::is_git_hook_installed(dir.path()));
        assert!(codex_hooks::are_hooks_installed_at(dir.path()));
        assert!(dir.path().join(".codex/config.toml").exists());
        assert!(!settings::is_enabled(dir.path()).unwrap());
    });
}

#[test]
fn run_disable_already_disabled() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
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
    with_isolated_daemon_config(|| {
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
    });
}

#[test]
fn run_disable_leaves_local_policy_unchanged() {
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
enabled = true
"#,
    );

    let mut out = Vec::new();
    run_disable(dir.path(), &mut out, false).unwrap();

    let local_content = fs::read_to_string(settings_local_path(dir.path())).unwrap();
    assert!(
        local_content.contains("enabled = false"),
        "local policy should be disabled in place: {local_content}"
    );
}

#[test]
fn run_disable_with_project_flag_leaves_policy_unchanged() {
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
enabled = true
"#,
    );

    let mut out = Vec::new();
    run_disable(dir.path(), &mut out, true).unwrap();

    let project_content = fs::read_to_string(settings_path(dir.path())).unwrap();
    assert!(
        project_content.contains("enabled = true"),
        "shared policy should remain unchanged when local override exists: {project_content}"
    );

    let local_content = fs::read_to_string(settings_local_path(dir.path())).unwrap();
    assert!(
        local_content.contains("enabled = false"),
        "local settings should be toggled even when --project is passed: {local_content}"
    );
}

#[test]
fn run_disable_does_not_create_local_policy_when_missing() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
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
        project_content.contains("enabled = false"),
        "project settings should be disabled in place: {project_content}"
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
    with_isolated_daemon_config(|| {
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
        assert_eq!(
            merged
                .strategy_options
                .get("push")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
        assert_eq!(
            merged
                .strategy_options
                .get("some_other_option")
                .and_then(serde_json::Value::as_str),
            Some("value")
        );
    });
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
    with_isolated_daemon_config(|| {
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
    });
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
    with_isolated_daemon_config(|| {
        let dir = tempfile::tempdir().unwrap();
        let (enabled, _, _) = is_fully_enabled(dir.path());
        assert!(!enabled, "should not be fully enabled");
    });
}

#[test]
fn is_fully_enabled_settings_disabled() {
    with_isolated_daemon_config(|| {
        let dir = tempfile::tempdir().unwrap();
        setup_settings(
            &dir,
            r#"[capture]
enabled = false
"#,
        );
        let (enabled, _, _) = is_fully_enabled(dir.path());
        assert!(!enabled, "disabled settings should not be fully enabled");
    });
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
fn enable_args_support_telemetry_flags() {
    let parsed = Cli::try_parse_from(["bitloops", "enable", "--telemetry=false"])
        .expect("enable telemetry flag should parse");
    let Some(Commands::Enable(args)) = parsed.command else {
        panic!("expected enable command");
    };
    assert_eq!(args.telemetry, Some(false));

    let parsed = Cli::try_parse_from(["bitloops", "enable", "--no-telemetry"])
        .expect("enable no telemetry flag should parse");
    let Some(Commands::Enable(args)) = parsed.command else {
        panic!("expected enable command");
    };
    assert!(args.no_telemetry);
}

#[test]
fn enable_args_support_install_embeddings_flag() {
    let parsed = Cli::try_parse_from(["bitloops", "enable", "--install-embeddings"])
        .expect("enable install-embeddings flag should parse");
    let Some(Commands::Enable(args)) = parsed.command else {
        panic!("expected enable command");
    };
    assert!(args.install_embeddings);
}

#[test]
fn enable_prompts_for_embeddings_and_defaults_to_yes() {
    let repo = tempfile::tempdir().expect("repo tempdir");
    setup_git_repo(&repo);
    setup_settings(
        &repo,
        r#"[capture]
enabled = false
"#,
    );

    with_isolated_daemon_config_process_state(
        Some(repo.path()),
        &[
            ("BITLOOPS_TEST_ASSUME_DAEMON_RUNNING", Some("1")),
            ("BITLOOPS_TEST_TTY", Some("1")),
        ],
        || {
            let (command, args) = fake_runtime_command_and_args(repo.path());
            write_runtime_only_daemon_config(&command, &args);

            with_global_graphql_executor_hook(
                |_runtime_root, _query, _variables| {
                    Ok(serde_json::json!({
                        "updateCliTelemetryConsent": {
                            "telemetry": true,
                            "needsPrompt": false
                        }
                    }))
                },
                || {
                    let mut out = Vec::new();
                    let mut input = Cursor::new("\n");
                    let runtime = test_runtime();
                    runtime
                        .block_on(run_with_io(
                            EnableArgs {
                                local: false,
                                project: false,
                                force: false,
                                agent: None,
                                telemetry: None,
                                no_telemetry: false,
                                install_embeddings: false,
                            },
                            &mut out,
                            &mut input,
                        ))
                        .expect("run enable");

                    let rendered = String::from_utf8(out).expect("utf8 output");
                    assert!(rendered.contains("Install embeddings now? [Y/n]"));
                    assert!(rendered.contains("Pulled embedding profile `local`."));
                    let daemon_config = fs::read_to_string(
                        default_daemon_config_path().expect("daemon config path"),
                    )
                    .expect("read daemon config");
                    assert!(daemon_config.contains("embedding_profile = \"local\""));
                },
            );
        },
    );
}

#[test]
fn enable_install_embeddings_flag_skips_prompt_in_noninteractive_mode() {
    let repo = tempfile::tempdir().expect("repo tempdir");
    setup_git_repo(&repo);
    setup_settings(
        &repo,
        r#"[capture]
enabled = false
"#,
    );

    with_enable_test_process_state(
        repo.path(),
        serde_json::json!({
            "updateCliTelemetryConsent": {
                "telemetry": true,
                "needsPrompt": false
            }
        }),
        || {
            let (command, args) = fake_runtime_command_and_args(repo.path());
            write_runtime_only_daemon_config(&command, &args);

            let mut out = Vec::new();
            let mut input = Cursor::new("");
            let runtime = test_runtime();
            runtime
                .block_on(run_with_io(
                    EnableArgs {
                        local: false,
                        project: false,
                        force: false,
                        agent: None,
                        telemetry: None,
                        no_telemetry: false,
                        install_embeddings: true,
                    },
                    &mut out,
                    &mut input,
                ))
                .expect("run enable");

            let rendered = String::from_utf8(out).expect("utf8 output");
            assert!(!rendered.contains("Install embeddings now? [Y/n]"));
            assert!(rendered.contains("Pulled embedding profile `local`."));
        },
    );
}

#[test]
fn enable_does_not_prompt_when_embeddings_are_already_configured() {
    let repo = tempfile::tempdir().expect("repo tempdir");
    setup_git_repo(&repo);
    setup_settings(
        &repo,
        r#"[capture]
enabled = false
"#,
    );

    with_isolated_daemon_config_process_state(
        Some(repo.path()),
        &[
            ("BITLOOPS_TEST_ASSUME_DAEMON_RUNNING", Some("1")),
            ("BITLOOPS_TEST_TTY", Some("1")),
        ],
        || {
            let daemon_config_path = default_daemon_config_path().expect("daemon config path");
            if let Some(parent) = daemon_config_path.parent() {
                fs::create_dir_all(parent).expect("create daemon config dir");
            }
            fs::write(
                daemon_config_path,
                r#"
[runtime]
local_dev = false

[semantic_clones]
embedding_profile = "openai"

[embeddings.profiles.openai]
kind = "openai"
model = "text-embedding-3-large"
"#,
            )
            .expect("write daemon config");

            with_global_graphql_executor_hook(
                |_runtime_root, _query, _variables| {
                    Ok(serde_json::json!({
                        "updateCliTelemetryConsent": {
                            "telemetry": true,
                            "needsPrompt": false
                        }
                    }))
                },
                || {
                    let mut out = Vec::new();
                    let mut input = Cursor::new("");
                    let runtime = test_runtime();
                    runtime
                        .block_on(run_with_io(
                            EnableArgs {
                                local: false,
                                project: false,
                                force: false,
                                agent: None,
                                telemetry: None,
                                no_telemetry: false,
                                install_embeddings: false,
                            },
                            &mut out,
                            &mut input,
                        ))
                        .expect("run enable");

                    let rendered = String::from_utf8(out).expect("utf8 output");
                    assert!(!rendered.contains("Install embeddings now? [Y/n]"));
                },
            );
        },
    );
}

#[test]
fn run_enable_without_agent_installs_default_agent_and_git_hooks() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    with_ready_daemon_and_repo_cwd(dir.path(), || {
        let err = run_enable_command(EnableArgs {
            local: false,
            project: false,
            force: false,
            agent: None,
            telemetry: None,
            no_telemetry: false,
            install_embeddings: false,
        })
        .unwrap_err();

        assert!(format!("{err:#}").contains("bitloops init"));
        assert!(!dir.path().join(".claude/settings.json").exists());
        assert!(!git_hooks::is_git_hook_installed(dir.path()));
    });
}

#[test]
fn run_enable_with_legacy_agent_flag_installs_requested_agent_hooks() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    with_ready_daemon_and_repo_cwd(dir.path(), || {
        let err = run_enable_command(EnableArgs {
            local: false,
            project: false,
            force: false,
            agent: Some("cursor".to_string()),
            telemetry: None,
            no_telemetry: false,
            install_embeddings: false,
        })
        .unwrap_err();

        assert!(format!("{err:#}").contains("bitloops init"));
        assert!(!dir.path().join(".cursor/hooks.json").exists());
        assert!(!git_hooks::is_git_hook_installed(dir.path()));
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
    ensure_repo_local_policy_excluded(dir.path(), dir.path()).unwrap();

    let exclude = fs::read_to_string(dir.path().join(".git/info/exclude")).unwrap();
    assert!(exclude.contains(".bitloops.local.toml"));
    assert!(!exclude.contains(".bitloops/"));
}

#[test]
fn repo_local_policy_exclude_does_not_add_legacy_names() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    ensure_repo_local_policy_excluded(dir.path(), dir.path()).unwrap();

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

    with_ready_daemon_and_repo_cwd(dir.path(), || {
        let err = run_enable_command(EnableArgs {
            local: false,
            project: false,
            force: false,
            agent: None,
            telemetry: None,
            no_telemetry: false,
            install_embeddings: false,
        })
        .unwrap_err();
        assert!(format!("{err:#}").contains("bitloops init"));
    });

    assert!(!settings_path(dir.path()).exists());
}

#[test]
fn enable_with_local_flag_does_not_create_local_repo_policy_file() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);

    with_ready_daemon_and_repo_cwd(dir.path(), || {
        let err = run_enable_command(EnableArgs {
            local: true,
            project: false,
            force: false,
            agent: None,
            telemetry: None,
            no_telemetry: false,
            install_embeddings: false,
        })
        .unwrap_err();
        assert!(format!("{err:#}").contains("bitloops init"));
    });

    assert!(!settings_local_path(dir.path()).exists());
}

#[test]
fn run_enable_noninteractive_requires_explicit_telemetry_when_unresolved() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    setup_settings(
        &dir,
        r#"[capture]
strategy = "manual-commit"
enabled = false
"#,
    );

    with_enable_test_process_state(
        dir.path(),
        serde_json::json!({
            "updateCliTelemetryConsent": {
                "telemetry": serde_json::Value::Null,
                "needsPrompt": true
            }
        }),
        || {
            let err = run_enable_command(EnableArgs {
                local: false,
                project: false,
                force: false,
                agent: None,
                telemetry: None,
                no_telemetry: false,
                install_embeddings: false,
            })
            .unwrap_err();

            assert_eq!(err.to_string(), NON_INTERACTIVE_TELEMETRY_ERROR);
            let content = fs::read_to_string(settings_path(dir.path())).unwrap();
            assert!(content.contains("enabled = false"));
        },
    );
}

#[test]
fn run_enable_with_explicit_no_telemetry_updates_project_config() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    setup_settings(
        &dir,
        r#"[capture]
strategy = "manual-commit"
enabled = false
"#,
    );

    with_isolated_daemon_config_process_state(
        Some(dir.path()),
        &[
            ("BITLOOPS_TEST_ASSUME_DAEMON_RUNNING", Some("1")),
            ("BITLOOPS_TEST_TTY", Some("0")),
        ],
        || {
            with_global_graphql_executor_hook(
                |_runtime_root, _query, variables| {
                    assert_eq!(variables["telemetry"], serde_json::json!(false));
                    Ok(serde_json::json!({
                        "updateCliTelemetryConsent": {
                            "telemetry": false,
                            "needsPrompt": false
                        }
                    }))
                },
                || {
                    run_enable_command(EnableArgs {
                        local: false,
                        project: false,
                        force: false,
                        agent: None,
                        telemetry: Some(false),
                        no_telemetry: false,
                        install_embeddings: false,
                    })
                    .expect("enable should succeed");

                    let settings = load_settings(dir.path()).unwrap();
                    assert!(settings.enabled);
                },
            )
        },
    );
}

#[test]
fn disable_does_not_create_local_repo_policy_file() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
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
    let content = fs::read_to_string(settings_path(dir.path())).unwrap();
    assert!(content.contains("enabled = false"));
}

#[test]
fn disable_with_project_flag_does_not_rewrite_shared_repo_policy() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
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
        content.contains("enabled = false"),
        "shared repo policy should be toggled when it is the nearest config, got: {content}"
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

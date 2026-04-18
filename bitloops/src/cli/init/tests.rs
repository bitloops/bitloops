use super::agent_hooks::{
    AGENT_CLAUDE_CODE, AGENT_CODEX, AGENT_CURSOR, AGENT_GEMINI, DEFAULT_AGENT,
};
use super::*;
use crate::adapters::agents::{AGENT_NAME_COPILOT, AGENT_NAME_OPEN_CODE};
use crate::api::tls::with_test_mkcert_on_path;
use crate::cli::devql::graphql::{with_graphql_executor_hook, with_ingest_daemon_bootstrap_hook};
use crate::cli::embeddings::{
    ManagedEmbeddingsBinaryInstallOutcome, ManagedPlatformEmbeddingsBinaryInstallOutcome,
    with_managed_embeddings_install_hook, with_managed_platform_embeddings_install_hook,
};
use crate::cli::inference::{
    OllamaAvailability, with_ollama_probe_hook, with_summary_generation_configured_hook,
};
use crate::cli::telemetry_consent::{
    NON_INTERACTIVE_TELEMETRY_ERROR, prompt_telemetry_consent, with_global_graphql_executor_hook,
    with_test_assume_daemon_running, with_test_tty_override,
};
use crate::cli::terminal_picker::with_single_select_hook;
use crate::cli::{Cli, Commands};
use crate::config::{BITLOOPS_CONFIG_RELATIVE_PATH, ensure_daemon_config_exists};
use crate::test_support::process_state::{with_env_vars, with_process_state};
use crate::utils::platform_dirs::{TestPlatformDirOverrides, with_test_platform_dir_overrides};

use clap::Parser;
use serde_json::json;
use std::io::Cursor;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

fn setup_git_repo(dir: &TempDir) {
    std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(dir.path())
        .status()
        .expect("git init");
}

fn write_current_daemon_runtime_state(config_root: &std::path::Path) {
    let runtime_path = crate::daemon::runtime_state_path(config_root);
    if let Some(parent) = runtime_path.parent() {
        std::fs::create_dir_all(parent).expect("create runtime parent");
    }
    let config_path = config_root.join(crate::config::BITLOOPS_CONFIG_RELATIVE_PATH);
    let runtime_state = crate::daemon::DaemonRuntimeState {
        version: 1,
        config_path,
        config_root: config_root.to_path_buf(),
        pid: std::process::id(),
        mode: crate::daemon::DaemonMode::Detached,
        service_name: None,
        url: "http://127.0.0.1:5667".to_string(),
        host: "127.0.0.1".to_string(),
        port: 5667,
        bundle_dir: config_root.join("bundle"),
        relational_db_path: config_root.join("relational.db"),
        events_db_path: config_root.join("events.duckdb"),
        blob_store_path: config_root.join("blob"),
        repo_registry_path: config_root.join("repo-registry.json"),
        binary_fingerprint: crate::daemon::current_binary_fingerprint().unwrap_or_default(),
        updated_at_unix: 0,
    };
    let mut bytes = serde_json::to_vec_pretty(&runtime_state).expect("serialize runtime state");
    bytes.push(b'\n');
    std::fs::write(&runtime_path, bytes).expect("write runtime state");
}

fn app_dir_overrides(temp: &TempDir) -> TestPlatformDirOverrides {
    TestPlatformDirOverrides {
        config_root: Some(temp.path().join("config-root")),
        data_root: Some(temp.path().join("data-root")),
        cache_root: Some(temp.path().join("cache-root")),
        state_root: Some(temp.path().join("state-root")),
    }
}

fn with_temp_app_dirs<T>(
    temp: &TempDir,
    tty: bool,
    assume_daemon_running: bool,
    f: impl FnOnce() -> T,
) -> T {
    with_temp_app_dirs_and_summary_configured(temp, tty, assume_daemon_running, true, f)
}

fn with_temp_app_dirs_and_summary_configured<T>(
    temp: &TempDir,
    tty: bool,
    assume_daemon_running: bool,
    summary_configured: bool,
    f: impl FnOnce() -> T,
) -> T {
    with_summary_generation_configured_hook(
        move |_| summary_configured,
        || {
            with_test_platform_dir_overrides(app_dir_overrides(temp), || {
                with_test_tty_override(tty, || {
                    with_test_assume_daemon_running(assume_daemon_running, f)
                })
            })
        },
    )
}

fn test_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
}

fn render_install_default_daemon_handoff_with_mkcert(
    repo: &TempDir,
    app_dirs: &TempDir,
    mkcert_on_path: bool,
) -> String {
    with_temp_app_dirs(app_dirs, false, true, || {
        with_test_mkcert_on_path(mkcert_on_path, || {
            with_install_default_daemon_hook(
                move |install_default_daemon| {
                    assert!(install_default_daemon);
                    let config_path =
                        ensure_daemon_config_exists().expect("create default daemon config");
                    write_runtime_only_daemon_config(
                        &config_path,
                        "bitloops-local-embeddings",
                        &[],
                    );
                    Ok(())
                },
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
                            let mut out = Vec::new();
                            let mut input = Cursor::new("");
                            let runtime = test_runtime();
                            runtime
                                .block_on(run_with_io_async_for_project_root(
                                    InitArgs {
                                        install_default_daemon: true,
                                        force: false,
                                        disable_bitloops_skill: false,
                                        agent: vec![DEFAULT_AGENT.to_string()],
                                        telemetry: Some(false),
                                        no_telemetry: false,
                                        skip_baseline: false,
                                        sync: Some(false),
                                        ingest: Some(false),
                                        backfill: None,
                                        exclude: Vec::new(),
                                        exclude_from: Vec::new(),
                                        embeddings_runtime: None,
                                        no_embeddings: true,
                                        embeddings_gateway_url: None,
                                        embeddings_api_key_env: "BITLOOPS_PLATFORM_GATEWAY_TOKEN"
                                            .to_string(),
                                    },
                                    repo.path(),
                                    &mut out,
                                    &mut input,
                                    None,
                                ))
                                .expect("run init");

                            String::from_utf8(out).expect("utf8 output")
                        },
                    )
                },
            )
        })
    })
}

#[cfg(unix)]
fn fake_runtime_command_and_args(repo_root: &Path) -> (String, Vec<String>) {
    use std::os::unix::fs::PermissionsExt;

    let script_path = repo_root.join(".bitloops/test-bin/fake-init-embeddings-runtime.sh");
    if let Some(parent) = script_path.parent() {
        std::fs::create_dir_all(parent).expect("create fake runtime dir");
    }
    let script = r#"#!/bin/sh
model_name="bge-m3"
printf '{"event":"ready","protocol":1,"capabilities":["embed","shutdown"]}\n'
while IFS= read -r line; do
  req_id=$(printf '%s\n' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  case "$line" in
    *'"cmd":"embed"'*)
      printf '{"id":"%s","ok":true,"vectors":[[0.1,0.2,0.3]],"model":"%s"}\n' "$req_id" "$model_name"
      ;;
    *'"cmd":"shutdown"'*)
      printf '{"id":"%s","ok":true,"model":"%s"}\n' "$req_id" "$model_name"
      exit 0
      ;;
    *)
      printf '{"id":"%s","ok":false,"error":{"message":"unexpected request"}}\n' "$req_id"
      ;;
  esac
done
"#;
    std::fs::write(&script_path, script).expect("write fake runtime script");
    let mut permissions = std::fs::metadata(&script_path)
        .expect("stat fake runtime script")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&script_path, permissions).expect("chmod fake runtime script");
    ("sh".to_string(), vec![script_path.display().to_string()])
}

#[cfg(unix)]
fn fake_managed_runtime_path(repo_root: &Path) -> std::path::PathBuf {
    let (_, args) = fake_runtime_command_and_args(repo_root);
    std::path::PathBuf::from(args[0].clone())
}

#[cfg(windows)]
fn fake_runtime_command_and_args(repo_root: &Path) -> (String, Vec<String>) {
    let script_path = repo_root.join(".bitloops/test-bin/fake-init-embeddings-runtime.ps1");
    if let Some(parent) = script_path.parent() {
        std::fs::create_dir_all(parent).expect("create fake runtime dir");
    }
    let script = r#"
$modelName = "bge-m3"
$ready = @{
  event = "ready"
  protocol = 1
  capabilities = @("embed", "shutdown")
}
$ready | ConvertTo-Json -Compress
$stdin = [Console]::In
while (($line = $stdin.ReadLine()) -ne $null) {
  if ([string]::IsNullOrWhiteSpace($line)) { continue }
  $request = $line | ConvertFrom-Json
  switch ($request.cmd) {
    "embed" {
      $response = @{
        id = $request.id
        ok = $true
        vectors = @(@(0.1, 0.2, 0.3))
        model = $modelName
      }
    }
    "shutdown" {
      $response = @{
        id = $request.id
        ok = $true
        model = $modelName
      }
      $response | ConvertTo-Json -Compress
      break
    }
    default {
      $response = @{
        id = $request.id
        ok = $false
        error = @{
          message = "unexpected request"
        }
      }
    }
  }
  $response | ConvertTo-Json -Compress
}
"#;
    std::fs::write(&script_path, script).expect("write fake runtime script");
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

#[cfg(windows)]
fn fake_managed_runtime_path(repo_root: &Path) -> std::path::PathBuf {
    let script_dir = repo_root.join(".bitloops/test-bin");
    std::fs::create_dir_all(&script_dir).expect("create managed runtime dir");
    let powershell_script = script_dir.join("fake-managed-init-embeddings-runtime.ps1");
    let launcher = script_dir.join("fake-managed-init-embeddings-runtime.cmd");
    let (_, args) = fake_runtime_command_and_args(repo_root);
    std::fs::copy(&args[4], &powershell_script).expect("copy managed powershell script");
    std::fs::write(
        &launcher,
        format!(
            "@echo off\r\npowershell -NoProfile -ExecutionPolicy Bypass -File \"{}\" %*\r\n",
            powershell_script.display()
        ),
    )
    .expect("write managed runtime launcher");
    launcher
}

fn write_runtime_only_daemon_config(config_path: &Path, command: &str, args: &[String]) {
    let runtime_args = args
        .iter()
        .map(|arg| format!("{arg:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    std::fs::write(
        config_path,
        format!(
            r#"
[runtime]
local_dev = false

[inference.runtimes.bitloops_local_embeddings]
command = {command:?}
args = [{runtime_args}]
startup_timeout_secs = 5
request_timeout_secs = 5
"#
        ),
    )
    .expect("write daemon config");
}

fn test_repo_id(repo_root: &Path) -> String {
    crate::host::devql::resolve_repo_identity(repo_root)
        .expect("resolve repo identity")
        .repo_id
}

#[derive(Debug, Clone)]
struct RuntimeSessionSnapshotFixture {
    status: &'static str,
    waiting_reason: Option<&'static str>,
    follow_up_sync_required: bool,
    run_sync: bool,
    run_ingest: bool,
    embeddings_selected: bool,
    summaries_selected: bool,
    terminal_error: Option<&'static str>,
    top_lane_status: &'static str,
    top_lane_waiting_reason: Option<&'static str>,
    top_lane_detail: Option<&'static str>,
    embeddings_lane_status: &'static str,
    embeddings_lane_waiting_reason: Option<&'static str>,
    summaries_lane_status: &'static str,
    summaries_lane_waiting_reason: Option<&'static str>,
}

impl Default for RuntimeSessionSnapshotFixture {
    fn default() -> Self {
        Self {
            status: "COMPLETED",
            waiting_reason: None,
            follow_up_sync_required: false,
            run_sync: false,
            run_ingest: false,
            embeddings_selected: false,
            summaries_selected: false,
            terminal_error: None,
            top_lane_status: "SKIPPED",
            top_lane_waiting_reason: None,
            top_lane_detail: None,
            embeddings_lane_status: "SKIPPED",
            embeddings_lane_waiting_reason: None,
            summaries_lane_status: "SKIPPED",
            summaries_lane_waiting_reason: None,
        }
    }
}

fn runtime_start_init_result_json(session_id: &str) -> serde_json::Value {
    json!({
        "startInit": {
            "initSessionId": session_id
        }
    })
}

fn runtime_snapshot_json(
    repo_id: &str,
    session_id: &str,
    fixture: RuntimeSessionSnapshotFixture,
) -> serde_json::Value {
    json!({
        "runtimeSnapshot": {
            "repoId": repo_id,
            "taskQueue": {
                "persisted": true,
                "queuedTasks": 0,
                "runningTasks": 0,
                "failedTasks": 0,
                "completedRecentTasks": 0,
                "byKind": [],
                "paused": false,
                "pausedReason": serde_json::Value::Null,
                "lastAction": "idle",
                "lastUpdatedUnix": 1,
                "currentRepoTasks": []
            },
            "currentStateConsumer": {
                "persisted": true,
                "pendingRuns": 0,
                "runningRuns": 0,
                "failedRuns": 0,
                "completedRecentRuns": 0,
                "lastAction": "idle",
                "lastUpdatedUnix": 1,
                "currentRepoRun": serde_json::Value::Null
            },
            "workplane": {
                "pendingJobs": 0,
                "runningJobs": 0,
                "failedJobs": 0,
                "completedRecentJobs": 0,
                "mailboxes": []
            },
            "blockedMailboxes": [],
            "embeddingsReadinessGate": serde_json::Value::Null,
            "summariesBootstrap": serde_json::Value::Null,
            "currentInitSession": {
                "initSessionId": session_id,
                "status": fixture.status,
                "waitingReason": fixture.waiting_reason,
                "followUpSyncRequired": fixture.follow_up_sync_required,
                "runSync": fixture.run_sync,
                "runIngest": fixture.run_ingest,
                "embeddingsSelected": fixture.embeddings_selected,
                "summariesSelected": fixture.summaries_selected,
                "initialSyncTaskId": serde_json::Value::Null,
                "ingestTaskId": serde_json::Value::Null,
                "followUpSyncTaskId": serde_json::Value::Null,
                "embeddingsBootstrapTaskId": serde_json::Value::Null,
                "summaryBootstrapRunId": serde_json::Value::Null,
                "terminalError": fixture.terminal_error,
                "topPipelineLane": {
                    "status": fixture.top_lane_status,
                    "waitingReason": fixture.top_lane_waiting_reason,
                    "detail": fixture.top_lane_detail,
                    "taskId": serde_json::Value::Null,
                    "runId": serde_json::Value::Null,
                    "pendingCount": 0,
                    "runningCount": 0,
                    "failedCount": 0,
                    "completedCount": if fixture.top_lane_status.eq_ignore_ascii_case("completed") { 1 } else { 0 }
                },
                "embeddingsLane": {
                    "status": fixture.embeddings_lane_status,
                    "waitingReason": fixture.embeddings_lane_waiting_reason,
                    "detail": serde_json::Value::Null,
                    "taskId": serde_json::Value::Null,
                    "runId": serde_json::Value::Null,
                    "pendingCount": 0,
                    "runningCount": 0,
                    "failedCount": 0,
                    "completedCount": if fixture.embeddings_lane_status.eq_ignore_ascii_case("completed") { 1 } else { 0 }
                },
                "summariesLane": {
                    "status": fixture.summaries_lane_status,
                    "waitingReason": fixture.summaries_lane_waiting_reason,
                    "detail": serde_json::Value::Null,
                    "taskId": serde_json::Value::Null,
                    "runId": serde_json::Value::Null,
                    "pendingCount": 0,
                    "runningCount": 0,
                    "failedCount": 0,
                    "completedCount": if fixture.summaries_lane_status.eq_ignore_ascii_case("completed") { 1 } else { 0 }
                }
            }
        }
    })
}

#[test]
fn init_args_supports_repeated_agent_flags() {
    let parsed = Cli::try_parse_from(["bitloops", "init", "--agent", "cursor", "--agent", "codex"])
        .expect("parse init");
    let Some(Commands::Init(args)) = parsed.command else {
        panic!("expected init command");
    };
    assert_eq!(args.agent, vec!["cursor".to_string(), "codex".to_string()]);
}

#[test]
fn init_args_supports_install_default_daemon_flag() {
    let parsed = Cli::try_parse_from(["bitloops", "init", "--install-default-daemon"])
        .expect("parse init install-default-daemon flag");
    let Some(Commands::Init(args)) = parsed.command else {
        panic!("expected init command");
    };
    assert!(args.install_default_daemon);
}

#[test]
fn init_args_leave_embeddings_choice_unset_when_flags_are_omitted() {
    let parsed = Cli::try_parse_from(["bitloops", "init"]).expect("parse init");
    let Some(Commands::Init(args)) = parsed.command else {
        panic!("expected init command");
    };

    assert_eq!(args.embeddings_runtime, None);
    assert!(!args.no_embeddings);
}

#[test]
fn init_args_support_explicit_platform_embeddings_runtime() {
    let parsed = Cli::try_parse_from(["bitloops", "init", "--embeddings-runtime", "platform"])
        .expect("parse init platform embeddings runtime");
    let Some(Commands::Init(args)) = parsed.command else {
        panic!("expected init command");
    };

    assert_eq!(
        args.embeddings_runtime,
        Some(crate::cli::embeddings::EmbeddingsRuntime::Platform)
    );
    assert!(!args.no_embeddings);
}

#[test]
fn init_args_support_no_embeddings_flag() {
    let parsed = Cli::try_parse_from(["bitloops", "init", "--no-embeddings"])
        .expect("parse init no-embeddings flag");
    let Some(Commands::Init(args)) = parsed.command else {
        panic!("expected init command");
    };

    assert!(args.no_embeddings);
    assert_eq!(args.embeddings_runtime, None);
}

#[test]
fn init_args_reject_conflicting_no_embeddings_and_runtime_flags() {
    let err = Cli::try_parse_from([
        "bitloops",
        "init",
        "--no-embeddings",
        "--embeddings-runtime",
        "platform",
    ])
    .err()
    .expect("conflicting embeddings flags should fail");

    assert!(err.to_string().contains("--no-embeddings"));
}

#[test]
fn init_args_supports_skip_baseline_flag() {
    let parsed = Cli::try_parse_from(["bitloops", "init", "--skip-baseline"]).expect("parse init");
    let Some(Commands::Init(args)) = parsed.command else {
        panic!("expected init command");
    };
    assert!(args.skip_baseline);
}

#[test]
fn init_args_support_sync_flag_variants() {
    let parsed = Cli::try_parse_from(["bitloops", "init", "--sync"]).expect("parse init --sync");
    let Some(Commands::Init(args)) = parsed.command else {
        panic!("expected init command");
    };
    assert_eq!(args.sync, Some(true));

    let parsed =
        Cli::try_parse_from(["bitloops", "init", "--sync=false"]).expect("parse init --sync=false");
    let Some(Commands::Init(args)) = parsed.command else {
        panic!("expected init command");
    };
    assert_eq!(args.sync, Some(false));
}

#[test]
fn init_args_support_ingest_flag_variants() {
    let parsed =
        Cli::try_parse_from(["bitloops", "init", "--ingest"]).expect("parse init --ingest");
    let Some(Commands::Init(args)) = parsed.command else {
        panic!("expected init command");
    };
    assert_eq!(args.ingest, Some(true));

    let parsed = Cli::try_parse_from(["bitloops", "init", "--ingest=false"])
        .expect("parse init --ingest=false");
    let Some(Commands::Init(args)) = parsed.command else {
        panic!("expected init command");
    };
    assert_eq!(args.ingest, Some(false));
}

#[test]
fn init_args_support_backfill_flag_variants() {
    let parsed =
        Cli::try_parse_from(["bitloops", "init", "--backfill"]).expect("parse init --backfill");
    let Some(Commands::Init(args)) = parsed.command else {
        panic!("expected init command");
    };
    assert_eq!(args.backfill, Some(50));

    let parsed = Cli::try_parse_from(["bitloops", "init", "--backfill=10"])
        .expect("parse init --backfill=10");
    let Some(Commands::Init(args)) = parsed.command else {
        panic!("expected init command");
    };
    assert_eq!(args.backfill, Some(10));
}

#[test]
fn init_args_support_repeated_exclusion_flags() {
    let parsed = Cli::try_parse_from([
        "bitloops",
        "init",
        "--exclude",
        "docs/**",
        "--exclude",
        "**/third_party/**",
        "--exclude-from",
        ".bitloopsignore",
        "--exclude-from",
        "configs/extra.ignore",
    ])
    .expect("parse init exclusion flags");
    let Some(Commands::Init(args)) = parsed.command else {
        panic!("expected init command");
    };

    assert_eq!(args.exclude, vec!["docs/**", "**/third_party/**"]);
    assert_eq!(
        args.exclude_from,
        vec![".bitloopsignore", "configs/extra.ignore"]
    );
}

#[test]
fn init_embeddings_prompt_defaults_to_cloud_in_picker_mode() {
    let mut out = Vec::new();
    let mut input = Cursor::new(Vec::<u8>::new());

    let selection = with_single_select_hook(
        |_options, default_index| Ok(default_index),
        || prompt_install_embeddings_setup_selection(&mut out, &mut input),
    )
    .expect("pick default embeddings selection");

    assert_eq!(selection, InitEmbeddingsSetupSelection::Cloud);
    let rendered = String::from_utf8(out).expect("utf8 output");
    assert!(rendered.contains("Configure embeddings"));
    assert!(rendered.contains("Embeddings power semantic search across your codebase"));
    assert!(rendered.contains("(e.g. “find where authentication is handled”)."));
    assert!(
        rendered.contains(
            "Choosing Bitloops cloud will open the Bitloops sign-in flow in your browser."
        )
    );
    assert!(rendered.contains("Bitloops Cloud (recommended)"));
    assert!(rendered.contains("Local embeddings"));
    assert!(rendered.contains("Skip for now"));
    assert!(
        rendered
            .find("Choosing Bitloops cloud will open the Bitloops sign-in flow in your browser.")
            < rendered.find("Bitloops Cloud (recommended)"),
        "expected cloud sign-in note before the choices: {rendered}"
    );
}

#[test]
fn init_embeddings_prompt_accepts_text_input_variants() {
    let mut out = Vec::new();
    let mut input = Cursor::new("2\n");

    let selection = prompt_install_embeddings_setup_selection(&mut out, &mut input)
        .expect("read text embeddings selection");

    assert_eq!(selection, InitEmbeddingsSetupSelection::Local);
    let rendered = String::from_utf8(out).expect("utf8 output");
    assert!(rendered.contains("Select an option [1/2/3]"));
}

#[test]
fn init_embeddings_prompt_reprompts_after_invalid_input() {
    let mut out = Vec::new();
    let mut input = Cursor::new("wat\n3\n");

    let selection = prompt_install_embeddings_setup_selection(&mut out, &mut input)
        .expect("read fallback embeddings selection");

    assert_eq!(selection, InitEmbeddingsSetupSelection::Skip);
    let rendered = String::from_utf8(out).expect("utf8 output");
    assert!(rendered.contains("Please choose 1, 2, or 3."));
}

#[test]
fn init_args_reject_zero_backfill() {
    let err = Cli::try_parse_from(["bitloops", "init", "--backfill=0"])
        .err()
        .expect("expected clap parsing error");
    let rendered = err.to_string();
    assert!(
        rendered.contains("1..")
            || rendered.contains("greater than or equal to 1")
            || rendered.contains("greater than zero"),
        "unexpected clap error: {rendered}"
    );
}

#[test]
fn init_cmd_agent_flag_no_value_errors() {
    let err = Cli::try_parse_from(["bitloops", "init", "--agent"])
        .err()
        .expect("expected clap parsing error");
    let rendered = err.to_string();
    assert!(
        rendered.contains("a value is required") || rendered.contains("requires a value"),
        "unexpected clap error: {rendered}"
    );
}

#[test]
fn run_init_creates_project_local_policy_and_installs_selected_agents() {
    let repo = tempfile::tempdir().expect("repo tempdir");
    let app_dirs = tempfile::tempdir().expect("app tempdir");
    setup_git_repo(&repo);

    with_temp_app_dirs(&app_dirs, false, true, || {
        let mut out = Vec::new();
        run_with_writer_for_project_root(
            InitArgs {
                install_default_daemon: false,
                force: false,
                disable_bitloops_skill: false,
                agent: vec![DEFAULT_AGENT.to_string()],
                telemetry: None,
                no_telemetry: false,
                skip_baseline: false,
                sync: Some(false),
                ingest: Some(false),
                backfill: None,
                exclude: Vec::new(),
                exclude_from: Vec::new(),
                embeddings_runtime: Some(crate::cli::embeddings::EmbeddingsRuntime::Local),
                no_embeddings: false,
                embeddings_gateway_url: None,
                embeddings_api_key_env: "BITLOOPS_PLATFORM_GATEWAY_TOKEN".to_string(),
            },
            repo.path(),
            &mut out,
            None,
        )
        .expect("run init");

        let rendered = String::from_utf8(out).expect("utf8 output");
        assert!(!rendered.contains("Initialising DevQL schema"));
        assert!(!rendered.contains("Bitloops project bootstrap is ready."));
        assert!(repo.path().join(".bitloops.local.toml").exists());
        assert_eq!(
            crate::cli::enable::initialized_agents(repo.path()),
            vec![DEFAULT_AGENT.to_string()]
        );
        let repo_skill = repo
            .path()
            .join(".claude/skills/bitloops/using-devql/SKILL.md");
        assert!(
            repo_skill.exists(),
            "expected repo-local DevQL skill to be installed at {}",
            repo_skill.display()
        );
        let exclude = std::fs::read_to_string(repo.path().join(".git/info/exclude"))
            .expect("read git exclude");
        assert!(exclude.contains(".bitloops.local.toml"));
        assert!(exclude.contains(".claude/skills/bitloops/using-devql/SKILL.md"));
        assert!(!exclude.contains(".bitloops/"));
        assert!(!exclude.contains("config.local.json"));
        assert!(!exclude.contains(".bitloops/config.local.json"));
    });
}

#[test]
fn run_init_with_repeated_agent_flags_normalizes_and_deduplicates_explicit_agents() {
    let repo = tempfile::tempdir().expect("repo tempdir");
    let app_dirs = tempfile::tempdir().expect("app tempdir");
    setup_git_repo(&repo);

    with_temp_app_dirs(&app_dirs, false, true, || {
        let mut out = Vec::new();
        let select = |_choices: &[String],
                      _enable_bitloops_skill: bool|
         -> std::result::Result<InitAgentSelection, String> {
            panic!("selector should not run when --agent is provided")
        };

        run_with_writer_for_project_root(
            InitArgs {
                install_default_daemon: false,
                force: true,
                disable_bitloops_skill: false,
                agent: vec![
                    "Cursor".to_string(),
                    AGENT_CURSOR.to_string(),
                    "Gemini".to_string(),
                ],
                telemetry: None,
                no_telemetry: false,
                skip_baseline: true,
                sync: Some(false),
                ingest: Some(false),
                backfill: None,
                exclude: Vec::new(),
                exclude_from: Vec::new(),
                embeddings_runtime: Some(crate::cli::embeddings::EmbeddingsRuntime::Local),
                no_embeddings: false,
                embeddings_gateway_url: None,
                embeddings_api_key_env: "BITLOOPS_PLATFORM_GATEWAY_TOKEN".to_string(),
            },
            repo.path(),
            &mut out,
            Some(&select),
        )
        .expect("run init");

        assert_eq!(
            crate::cli::enable::initialized_agents(repo.path()),
            vec![AGENT_CURSOR.to_string(), AGENT_GEMINI.to_string()]
        );
        assert!(repo.path().join(".cursor/hooks.json").exists());
        assert!(
            repo.path()
                .join(".gemini/skills/bitloops/using-devql/SKILL.md")
                .exists()
        );
    });
}

#[test]
fn run_init_persists_scope_exclusions_and_preserves_unrelated_local_settings() {
    let repo = tempfile::tempdir().expect("repo tempdir");
    let app_dirs = tempfile::tempdir().expect("app tempdir");
    setup_git_repo(&repo);
    std::fs::write(
        repo.path().join(".bitloops.local.toml"),
        r#"
[custom]
keep = true
"#,
    )
    .expect("seed local policy");

    with_temp_app_dirs(&app_dirs, false, true, || {
        let mut out = Vec::new();
        run_with_writer_for_project_root(
            InitArgs {
                install_default_daemon: false,
                force: false,
                disable_bitloops_skill: false,
                agent: Vec::new(),
                telemetry: None,
                no_telemetry: false,
                skip_baseline: false,
                sync: Some(false),
                ingest: Some(false),
                backfill: None,
                exclude: vec!["docs/**".to_string(), "**/third_party/**".to_string()],
                exclude_from: vec![".bitloopsignore".to_string()],
                embeddings_runtime: Some(crate::cli::embeddings::EmbeddingsRuntime::Local),
                no_embeddings: false,
                embeddings_gateway_url: None,
                embeddings_api_key_env: "BITLOOPS_PLATFORM_GATEWAY_TOKEN".to_string(),
            },
            repo.path(),
            &mut out,
            None,
        )
        .expect("run init");

        let local_policy = std::fs::read_to_string(repo.path().join(".bitloops.local.toml"))
            .expect("read local policy");
        assert!(
            local_policy.contains("exclude = [\"**/third_party/**\", \"docs/**\"]")
                || local_policy.contains("exclude = [\"docs/**\", \"**/third_party/**\"]"),
            "scope.exclude should be persisted, got:\n{local_policy}"
        );
        assert!(
            local_policy.contains("exclude_from = [\".bitloopsignore\"]"),
            "scope.exclude_from should be persisted, got:\n{local_policy}"
        );
        assert!(
            local_policy.contains("[custom]") && local_policy.contains("keep = true"),
            "init should preserve unrelated existing local policy settings, got:\n{local_policy}"
        );
    });
}

#[test]
fn run_init_binds_repo_to_running_daemon_config() {
    let repo = tempfile::tempdir().expect("repo tempdir");
    let app_dirs = tempfile::tempdir().expect("app tempdir");
    let daemon_root = tempfile::tempdir().expect("daemon tempdir");
    setup_git_repo(&repo);

    with_temp_app_dirs(&app_dirs, false, false, || {
        write_current_daemon_runtime_state(daemon_root.path());

        let mut out = Vec::new();
        run_with_writer_for_project_root(
            InitArgs {
                install_default_daemon: false,
                force: false,
                disable_bitloops_skill: false,
                agent: Vec::new(),
                telemetry: None,
                no_telemetry: false,
                skip_baseline: false,
                sync: Some(false),
                ingest: Some(false),
                backfill: None,
                exclude: Vec::new(),
                exclude_from: Vec::new(),
                embeddings_runtime: Some(crate::cli::embeddings::EmbeddingsRuntime::Local),
                no_embeddings: false,
                embeddings_gateway_url: None,
                embeddings_api_key_env: "BITLOOPS_PLATFORM_GATEWAY_TOKEN".to_string(),
            },
            repo.path(),
            &mut out,
            None,
        )
        .expect("run init");

        let local_policy = std::fs::read_to_string(repo.path().join(".bitloops.local.toml"))
            .expect("read local repo policy");
        assert!(
            local_policy.contains(
                daemon_root
                    .path()
                    .join(BITLOOPS_CONFIG_RELATIVE_PATH)
                    .to_string_lossy()
                    .as_ref()
            ),
            "expected daemon binding in local policy:\n{local_policy}"
        );
    });
}

#[test]
fn run_init_bootstraps_repo_watcher_when_capture_is_enabled() {
    let repo = tempfile::tempdir().expect("repo tempdir");
    let app_dirs = tempfile::tempdir().expect("app tempdir");
    let repo_root = repo.path().to_path_buf();
    setup_git_repo(&repo);
    let reconcile_count = std::rc::Rc::new(std::cell::RefCell::new(0usize));

    with_temp_app_dirs(&app_dirs, false, true, || {
        crate::cli::watcher_bootstrap::with_watcher_reconciliation_hook(
            {
                let reconcile_count = std::rc::Rc::clone(&reconcile_count);
                let repo_root = repo_root.clone();
                move |actual_repo_root, capture_enabled| {
                    assert_eq!(actual_repo_root, repo_root.as_path());
                    assert!(
                        capture_enabled,
                        "init should only reconcile watchers when capture is enabled"
                    );
                    *reconcile_count.borrow_mut() += 1;
                    Ok(())
                }
            },
            || {
                let mut out = Vec::new();
                run_with_writer_for_project_root(
                    InitArgs {
                        install_default_daemon: false,
                        force: false,
                        disable_bitloops_skill: false,
                        agent: Vec::new(),
                        telemetry: None,
                        no_telemetry: false,
                        skip_baseline: false,
                        sync: Some(false),
                        ingest: Some(false),
                        backfill: None,
                        exclude: Vec::new(),
                        exclude_from: Vec::new(),
                        embeddings_runtime: Some(crate::cli::embeddings::EmbeddingsRuntime::Local),
                        no_embeddings: false,
                        embeddings_gateway_url: None,
                        embeddings_api_key_env: "BITLOOPS_PLATFORM_GATEWAY_TOKEN".to_string(),
                    },
                    repo_root.as_path(),
                    &mut out,
                    None,
                )
                .expect("run init");
            },
        );

        assert_eq!(
            *reconcile_count.borrow(),
            1,
            "successful init should reconcile the watcher exactly once"
        );
        assert!(
            crate::config::settings::is_enabled(repo_root.as_path())
                .expect("repo capture settings"),
            "successful init should leave capture enabled in repo settings"
        );
    });
}

#[test]
fn run_init_surfaces_repo_watcher_reconcile_failures() {
    let repo = tempfile::tempdir().expect("repo tempdir");
    let app_dirs = tempfile::tempdir().expect("app tempdir");
    let repo_root = repo.path().to_path_buf();
    setup_git_repo(&repo);

    with_temp_app_dirs(&app_dirs, false, true, || {
        crate::cli::watcher_bootstrap::with_watcher_reconciliation_hook(
            {
                let repo_root = repo_root.clone();
                move |actual_repo_root, capture_enabled| {
                    assert_eq!(actual_repo_root, repo_root.as_path());
                    assert!(
                        capture_enabled,
                        "init should only reconcile watchers when capture is enabled"
                    );
                    anyhow::bail!("watcher reconcile exploded");
                }
            },
            || {
                let mut out = Vec::new();
                let err = run_with_writer_for_project_root(
                    InitArgs {
                        install_default_daemon: false,
                        force: false,
                        disable_bitloops_skill: false,
                        agent: Vec::new(),
                        telemetry: None,
                        no_telemetry: false,
                        skip_baseline: false,
                        sync: Some(false),
                        ingest: Some(false),
                        backfill: None,
                        exclude: Vec::new(),
                        exclude_from: Vec::new(),
                        embeddings_runtime: Some(crate::cli::embeddings::EmbeddingsRuntime::Local),
                        no_embeddings: false,
                        embeddings_gateway_url: None,
                        embeddings_api_key_env: "BITLOOPS_PLATFORM_GATEWAY_TOKEN".to_string(),
                    },
                    repo_root.as_path(),
                    &mut out,
                    None,
                )
                .expect_err("init should surface watcher reconciliation failures");

                let rendered = format!("{err:#}");
                assert!(
                    rendered.contains("watcher reconcile exploded"),
                    "unexpected init error: {rendered}"
                );
            },
        );
    });
}

#[test]
fn run_init_bootstraps_repo_watcher_from_nested_project_root() {
    let repo = tempfile::tempdir().expect("repo tempdir");
    let app_dirs = tempfile::tempdir().expect("app tempdir");
    let project_root = repo.path().join("apps/nested-project");
    std::fs::create_dir_all(&project_root).expect("create nested project root");
    setup_git_repo(&repo);
    let reconcile_count = std::rc::Rc::new(std::cell::RefCell::new(0usize));

    with_temp_app_dirs(&app_dirs, false, true, || {
        crate::cli::watcher_bootstrap::with_watcher_reconciliation_hook(
            {
                let reconcile_count = std::rc::Rc::clone(&reconcile_count);
                let project_root = project_root.clone();
                move |actual_repo_root, capture_enabled| {
                    assert_eq!(actual_repo_root, project_root.as_path());
                    assert!(
                        capture_enabled,
                        "nested init should reconcile watchers when nested-project capture is enabled"
                    );
                    *reconcile_count.borrow_mut() += 1;
                    Ok(())
                }
            },
            || {
                let mut out = Vec::new();
                run_with_writer_for_project_root(
                    InitArgs {
                        install_default_daemon: false,
                        force: false,
                        disable_bitloops_skill: false,
                        agent: Vec::new(),
                        telemetry: None,
                        no_telemetry: false,
                        skip_baseline: false,
                        sync: Some(false),
                        ingest: Some(false),
                        backfill: None,
                        exclude: Vec::new(),
                        exclude_from: Vec::new(),
                        embeddings_runtime: Some(crate::cli::embeddings::EmbeddingsRuntime::Local),
                        no_embeddings: false,
                        embeddings_gateway_url: None,
                        embeddings_api_key_env: "BITLOOPS_PLATFORM_GATEWAY_TOKEN".to_string(),
                    },
                    project_root.as_path(),
                    &mut out,
                    None,
                )
                .expect("run init for nested project");
            },
        );

        assert_eq!(
            *reconcile_count.borrow(),
            1,
            "successful nested init should reconcile the watcher exactly once"
        );
        assert!(
            crate::config::settings::is_enabled(project_root.as_path())
                .expect("nested project capture settings"),
            "successful nested init should leave capture enabled in nested project settings"
        );
    });
}

#[test]
fn run_init_does_not_bootstrap_repo_watcher_when_repo_setup_fails() {
    let repo = tempfile::tempdir().expect("repo tempdir");
    let app_dirs = tempfile::tempdir().expect("app tempdir");
    let repo_root = repo.path().to_path_buf();
    setup_git_repo(&repo);
    let reconcile_count = std::rc::Rc::new(std::cell::RefCell::new(0usize));
    let select_fn = |_available: &[String],
                     _enable_bitloops_skill: bool|
     -> std::result::Result<InitAgentSelection, String> {
        Err("selector refused to choose an agent".to_string())
    };

    with_temp_app_dirs(&app_dirs, true, true, || {
        crate::cli::watcher_bootstrap::with_watcher_reconciliation_hook(
            {
                let reconcile_count = std::rc::Rc::clone(&reconcile_count);
                move |_repo_root, _capture_enabled| {
                    *reconcile_count.borrow_mut() += 1;
                    Ok(())
                }
            },
            || {
                let mut out = Vec::new();
                let err = run_with_writer_for_project_root(
                    InitArgs {
                        install_default_daemon: false,
                        force: false,
                        disable_bitloops_skill: false,
                        agent: Vec::new(),
                        telemetry: None,
                        no_telemetry: false,
                        skip_baseline: false,
                        sync: Some(false),
                        ingest: Some(false),
                        backfill: None,
                        exclude: Vec::new(),
                        exclude_from: Vec::new(),
                        embeddings_runtime: Some(crate::cli::embeddings::EmbeddingsRuntime::Local),
                        no_embeddings: false,
                        embeddings_gateway_url: None,
                        embeddings_api_key_env: "BITLOOPS_PLATFORM_GATEWAY_TOKEN".to_string(),
                    },
                    repo_root.as_path(),
                    &mut out,
                    Some(&select_fn),
                )
                .expect_err("init should fail before watcher reconciliation");

                let rendered = format!("{err:#}");
                assert!(
                    rendered.contains("selector refused to choose an agent"),
                    "unexpected init error: {rendered}"
                );
            },
        );
    });

    assert_eq!(
        *reconcile_count.borrow(),
        0,
        "watcher reconciliation should not run when init exits early"
    );
}

#[test]
fn run_init_rejects_exclude_from_paths_outside_repo_policy_root() {
    let repo = tempfile::tempdir().expect("repo tempdir");
    let app_dirs = tempfile::tempdir().expect("app tempdir");
    let outside = tempfile::tempdir().expect("outside tempdir");
    setup_git_repo(&repo);
    let outside_path = outside.path().join("outside.ignore");
    std::fs::write(&outside_path, "vendor/**\n").expect("write outside ignore file");

    with_temp_app_dirs(&app_dirs, false, true, || {
        let mut out = Vec::new();
        let err = run_with_writer_for_project_root(
            InitArgs {
                install_default_daemon: false,
                force: false,
                disable_bitloops_skill: false,
                agent: Vec::new(),
                telemetry: None,
                no_telemetry: false,
                skip_baseline: false,
                sync: Some(false),
                ingest: Some(false),
                backfill: None,
                exclude: Vec::new(),
                exclude_from: vec![outside_path.display().to_string()],
                embeddings_runtime: Some(crate::cli::embeddings::EmbeddingsRuntime::Local),
                no_embeddings: false,
                embeddings_gateway_url: None,
                embeddings_api_key_env: "BITLOOPS_PLATFORM_GATEWAY_TOKEN".to_string(),
            },
            repo.path(),
            &mut out,
            None,
        )
        .expect_err("outside-root --exclude-from path should fail");
        let rendered = format!("{err:#}");
        assert!(
            rendered.contains("must be under repo-policy root"),
            "unexpected error for outside-root --exclude-from path: {rendered}"
        );
    });
}

#[test]
fn run_init_rewrites_existing_daemon_binding() {
    let repo = tempfile::tempdir().expect("repo tempdir");
    let app_dirs = tempfile::tempdir().expect("app tempdir");
    let old_daemon_root = tempfile::tempdir().expect("old daemon tempdir");
    let new_daemon_root = tempfile::tempdir().expect("new daemon tempdir");
    setup_git_repo(&repo);

    crate::config::settings::write_repo_daemon_binding(
        &repo.path().join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
        &old_daemon_root.path().join(BITLOOPS_CONFIG_RELATIVE_PATH),
    )
    .expect("write initial repo daemon binding");

    with_temp_app_dirs(&app_dirs, false, false, || {
        write_current_daemon_runtime_state(new_daemon_root.path());

        let mut out = Vec::new();
        run_with_writer_for_project_root(
            InitArgs {
                install_default_daemon: false,
                force: false,
                disable_bitloops_skill: false,
                agent: Vec::new(),
                telemetry: None,
                no_telemetry: false,
                skip_baseline: false,
                sync: Some(false),
                ingest: Some(false),
                backfill: None,
                exclude: Vec::new(),
                exclude_from: Vec::new(),
                embeddings_runtime: Some(crate::cli::embeddings::EmbeddingsRuntime::Local),
                no_embeddings: false,
                embeddings_gateway_url: None,
                embeddings_api_key_env: "BITLOOPS_PLATFORM_GATEWAY_TOKEN".to_string(),
            },
            repo.path(),
            &mut out,
            None,
        )
        .expect("run init");

        let local_policy = std::fs::read_to_string(repo.path().join(".bitloops.local.toml"))
            .expect("read local repo policy");
        assert!(
            local_policy.contains(
                new_daemon_root
                    .path()
                    .join(BITLOOPS_CONFIG_RELATIVE_PATH)
                    .to_string_lossy()
                    .as_ref()
            ),
            "expected updated daemon binding in local policy:\n{local_policy}"
        );
        assert!(
            !local_policy.contains(
                old_daemon_root
                    .path()
                    .join(BITLOOPS_CONFIG_RELATIVE_PATH)
                    .to_string_lossy()
                    .as_ref()
            ),
            "old daemon binding should be replaced:\n{local_policy}"
        );
    });
}

#[test]
fn run_init_with_agent_flag_installs_requested_hooks_when_skip_baseline_is_requested() {
    let repo = tempfile::tempdir().expect("repo tempdir");
    let app_dirs = tempfile::tempdir().expect("app tempdir");
    setup_git_repo(&repo);

    with_temp_app_dirs(&app_dirs, false, true, || {
        let mut out = Vec::new();
        run_with_writer_for_project_root(
            InitArgs {
                install_default_daemon: false,
                force: true,
                disable_bitloops_skill: false,
                agent: vec![AGENT_CURSOR.to_string()],
                telemetry: None,
                no_telemetry: false,
                skip_baseline: true,
                sync: Some(false),
                ingest: Some(false),
                backfill: None,
                exclude: Vec::new(),
                exclude_from: Vec::new(),
                embeddings_runtime: Some(crate::cli::embeddings::EmbeddingsRuntime::Local),
                no_embeddings: false,
                embeddings_gateway_url: None,
                embeddings_api_key_env: "BITLOOPS_PLATFORM_GATEWAY_TOKEN".to_string(),
            },
            repo.path(),
            &mut out,
            None,
        )
        .expect("run init");

        let rendered = String::from_utf8(out).expect("utf8 output");
        assert!(!rendered.contains("Initialised agents: cursor"));
        assert!(!rendered.contains("Initialising DevQL schema"));
        assert!(repo.path().join(".cursor/hooks.json").exists());
        assert!(
            repo.path()
                .join(".cursor/rules/bitloops-using-devql.mdc")
                .exists()
        );
        assert!(!repo.path().join(".claude/settings.json").exists());
    });
}

#[test]
fn run_init_with_codex_agent_writes_project_local_codex_config_and_hooks() {
    let repo = tempfile::tempdir().expect("repo tempdir");
    let app_dirs = tempfile::tempdir().expect("app tempdir");
    let codex_home = tempfile::tempdir().expect("codex home tempdir");
    setup_git_repo(&repo);
    let codex_home_path = codex_home.path().to_string_lossy().to_string();

    with_env_vars(
        &[
            ("HOME", Some(codex_home_path.as_str())),
            ("USERPROFILE", Some(codex_home_path.as_str())),
        ],
        || {
            with_temp_app_dirs(&app_dirs, false, true, || {
                let mut out = Vec::new();
                run_with_writer_for_project_root(
                    InitArgs {
                        install_default_daemon: false,
                        force: true,
                        disable_bitloops_skill: false,
                        agent: vec![AGENT_CODEX.to_string()],
                        telemetry: None,
                        no_telemetry: false,
                        skip_baseline: true,
                        sync: Some(false),
                        ingest: Some(false),
                        backfill: None,
                        exclude: Vec::new(),
                        exclude_from: Vec::new(),
                        embeddings_runtime: Some(crate::cli::embeddings::EmbeddingsRuntime::Local),
                        no_embeddings: false,
                        embeddings_gateway_url: None,
                        embeddings_api_key_env: "BITLOOPS_PLATFORM_GATEWAY_TOKEN".to_string(),
                    },
                    repo.path(),
                    &mut out,
                    None,
                )
                .expect("run init");

                assert!(repo.path().join(".codex/hooks.json").exists());
                let config = std::fs::read_to_string(repo.path().join(".codex/config.toml"))
                    .expect("read codex config");
                assert!(config.contains("codex_hooks = true"));
                let repo_skill = repo
                    .path()
                    .join(".agents/skills/bitloops/using-devql/SKILL.md");
                assert!(
                    repo_skill.exists(),
                    "expected Codex repo-local skill to be installed"
                );
                let exclude = std::fs::read_to_string(repo.path().join(".git/info/exclude"))
                    .expect("read git exclude");
                assert!(exclude.contains(".agents/skills/bitloops/using-devql/SKILL.md"));
                assert!(!repo.path().join(".claude/settings.json").exists());
            });
        },
    );
}

#[test]
fn run_init_with_gemini_agent_installs_repo_skill_and_root_import() {
    let repo = tempfile::tempdir().expect("repo tempdir");
    let app_dirs = tempfile::tempdir().expect("app tempdir");
    setup_git_repo(&repo);

    with_temp_app_dirs(&app_dirs, false, true, || {
        let mut out = Vec::new();
        run_with_writer_for_project_root(
            InitArgs {
                install_default_daemon: false,
                force: true,
                disable_bitloops_skill: false,
                agent: vec![AGENT_GEMINI.to_string()],
                telemetry: None,
                no_telemetry: false,
                skip_baseline: true,
                sync: Some(false),
                ingest: Some(false),
                backfill: None,
                exclude: Vec::new(),
                exclude_from: Vec::new(),
                embeddings_runtime: Some(crate::cli::embeddings::EmbeddingsRuntime::Local),
                no_embeddings: false,
                embeddings_gateway_url: None,
                embeddings_api_key_env: "BITLOOPS_PLATFORM_GATEWAY_TOKEN".to_string(),
            },
            repo.path(),
            &mut out,
            None,
        )
        .expect("run init");

        let gemini_md =
            std::fs::read_to_string(repo.path().join("GEMINI.md")).expect("read GEMINI.md");
        assert!(gemini_md.contains("@./.gemini/skills/bitloops/using-devql/SKILL.md"));
        assert!(
            repo.path()
                .join(".gemini/skills/bitloops/using-devql/SKILL.md")
                .exists()
        );
        let exclude =
            std::fs::read_to_string(repo.path().join(".git/info/exclude")).expect("read exclude");
        assert!(exclude.contains(".gemini/skills/bitloops/using-devql/SKILL.md"));
    });
}

#[test]
fn run_init_with_copilot_agent_installs_hooks_and_repo_skill() {
    let repo = tempfile::tempdir().expect("repo tempdir");
    let app_dirs = tempfile::tempdir().expect("app tempdir");
    setup_git_repo(&repo);

    with_temp_app_dirs(&app_dirs, false, true, || {
        let mut out = Vec::new();
        run_with_writer_for_project_root(
            InitArgs {
                install_default_daemon: false,
                force: true,
                disable_bitloops_skill: false,
                agent: vec![AGENT_NAME_COPILOT.to_string()],
                telemetry: None,
                no_telemetry: false,
                skip_baseline: true,
                sync: Some(false),
                ingest: Some(false),
                backfill: None,
                exclude: Vec::new(),
                exclude_from: Vec::new(),
                embeddings_runtime: Some(crate::cli::embeddings::EmbeddingsRuntime::Local),
                no_embeddings: false,
                embeddings_gateway_url: None,
                embeddings_api_key_env: "BITLOOPS_PLATFORM_GATEWAY_TOKEN".to_string(),
            },
            repo.path(),
            &mut out,
            None,
        )
        .expect("run init");

        assert!(repo.path().join(".github/hooks/bitloops.json").exists());
        assert!(
            repo.path()
                .join(".github/skills/bitloops/using-devql/SKILL.md")
                .exists()
        );
        let exclude =
            std::fs::read_to_string(repo.path().join(".git/info/exclude")).expect("read exclude");
        assert!(exclude.contains(".github/skills/bitloops/using-devql/SKILL.md"));
    });
}

#[test]
fn run_init_with_opencode_agent_installs_plugin_and_repo_skill() {
    let repo = tempfile::tempdir().expect("repo tempdir");
    let app_dirs = tempfile::tempdir().expect("app tempdir");
    setup_git_repo(&repo);

    with_temp_app_dirs(&app_dirs, false, true, || {
        let mut out = Vec::new();
        run_with_writer_for_project_root(
            InitArgs {
                install_default_daemon: false,
                force: true,
                disable_bitloops_skill: false,
                agent: vec![AGENT_NAME_OPEN_CODE.to_string()],
                telemetry: None,
                no_telemetry: false,
                skip_baseline: true,
                sync: Some(false),
                ingest: Some(false),
                backfill: None,
                exclude: Vec::new(),
                exclude_from: Vec::new(),
                embeddings_runtime: Some(crate::cli::embeddings::EmbeddingsRuntime::Local),
                no_embeddings: false,
                embeddings_gateway_url: None,
                embeddings_api_key_env: "BITLOOPS_PLATFORM_GATEWAY_TOKEN".to_string(),
            },
            repo.path(),
            &mut out,
            None,
        )
        .expect("run init");

        assert!(repo.path().join(".opencode/plugins/bitloops.ts").exists());
        assert!(
            repo.path()
                .join(".opencode/skills/bitloops/using-devql/SKILL.md")
                .exists()
        );
        let exclude =
            std::fs::read_to_string(repo.path().join(".git/info/exclude")).expect("read exclude");
        assert!(exclude.contains(".opencode/skills/bitloops/using-devql/SKILL.md"));
    });
}

#[test]
fn run_init_with_disable_bitloops_skill_keeps_hooks_and_skips_repo_prompt_surfaces() {
    let repo = tempfile::tempdir().expect("repo tempdir");
    let app_dirs = tempfile::tempdir().expect("app tempdir");
    setup_git_repo(&repo);

    with_temp_app_dirs(&app_dirs, false, true, || {
        let mut out = Vec::new();
        run_with_writer_for_project_root(
            InitArgs {
                install_default_daemon: false,
                force: true,
                disable_bitloops_skill: true,
                agent: vec![
                    AGENT_CLAUDE_CODE.to_string(),
                    AGENT_CODEX.to_string(),
                    AGENT_CURSOR.to_string(),
                    AGENT_GEMINI.to_string(),
                    AGENT_NAME_COPILOT.to_string(),
                    AGENT_NAME_OPEN_CODE.to_string(),
                ],
                telemetry: None,
                no_telemetry: false,
                skip_baseline: true,
                sync: Some(false),
                ingest: Some(false),
                backfill: None,
                exclude: Vec::new(),
                exclude_from: Vec::new(),
                embeddings_runtime: Some(crate::cli::embeddings::EmbeddingsRuntime::Local),
                no_embeddings: false,
                embeddings_gateway_url: None,
                embeddings_api_key_env: "BITLOOPS_PLATFORM_GATEWAY_TOKEN".to_string(),
            },
            repo.path(),
            &mut out,
            None,
        )
        .expect("run init");

        assert!(repo.path().join(".claude/settings.json").exists());
        assert!(repo.path().join(".codex/hooks.json").exists());
        assert!(repo.path().join(".cursor/hooks.json").exists());
        assert!(repo.path().join(".gemini/settings.json").exists());
        assert!(repo.path().join(".github/hooks/bitloops.json").exists());
        assert!(repo.path().join(".opencode/plugins/bitloops.ts").exists());
        assert!(
            !repo
                .path()
                .join(".claude/skills/bitloops/using-devql/SKILL.md")
                .exists()
        );
        assert!(
            !repo
                .path()
                .join(".agents/skills/bitloops/using-devql/SKILL.md")
                .exists()
        );
        assert!(
            !repo
                .path()
                .join(".cursor/rules/bitloops-using-devql.mdc")
                .exists()
        );
        assert!(
            !repo
                .path()
                .join(".gemini/skills/bitloops/using-devql/SKILL.md")
                .exists()
        );
        assert!(!repo.path().join("GEMINI.md").exists());
        assert!(
            !repo
                .path()
                .join(".github/skills/bitloops/using-devql/SKILL.md")
                .exists()
        );
        assert!(
            !repo
                .path()
                .join(".opencode/skills/bitloops/using-devql/SKILL.md")
                .exists()
        );
    });
}

#[test]
fn run_init_with_invalid_explicit_agent_errors() {
    let repo = tempfile::tempdir().expect("repo tempdir");
    let app_dirs = tempfile::tempdir().expect("app tempdir");
    setup_git_repo(&repo);

    with_temp_app_dirs(&app_dirs, false, true, || {
        let mut out = Vec::new();
        let err = run_with_writer_for_project_root(
            InitArgs {
                install_default_daemon: false,
                force: false,
                disable_bitloops_skill: false,
                agent: vec![AGENT_CURSOR.to_string(), "not-a-real-agent".to_string()],
                telemetry: None,
                no_telemetry: false,
                skip_baseline: true,
                sync: Some(false),
                ingest: Some(false),
                backfill: None,
                exclude: Vec::new(),
                exclude_from: Vec::new(),
                embeddings_runtime: Some(crate::cli::embeddings::EmbeddingsRuntime::Local),
                no_embeddings: false,
                embeddings_gateway_url: None,
                embeddings_api_key_env: "BITLOOPS_PLATFORM_GATEWAY_TOKEN".to_string(),
            },
            repo.path(),
            &mut out,
            None,
        )
        .expect_err("invalid explicit agent should fail");
        let rendered = format!("{err:#}");
        assert!(
            rendered.contains("unknown agent name: not-a-real-agent"),
            "unexpected error: {rendered}"
        );
    });
}

#[test]
fn detect_or_select_agent_no_detection_no_tty_falls_back_to_default() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    with_process_state(
        Some(dir.path()),
        &[("BITLOOPS_TEST_TTY", Some("0"))],
        || {
            let mut out = Vec::new();
            let selected = detect_or_select_agent(dir.path(), &mut out, true, None).unwrap();
            assert_eq!(selected.agents, vec![DEFAULT_AGENT.to_string()]);
            assert!(selected.enable_bitloops_skill);
        },
    );
}

#[test]
fn detect_or_select_agent_agent_detected() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
    with_process_state(
        Some(dir.path()),
        &[("BITLOOPS_TEST_TTY", Some("0"))],
        || {
            let mut out = Vec::new();
            let selected = detect_or_select_agent(dir.path(), &mut out, true, None).unwrap();
            assert_eq!(selected.agents, vec![AGENT_CLAUDE_CODE.to_string()]);
            assert!(selected.enable_bitloops_skill);
        },
    );
}

#[test]
fn detect_or_select_agent_single_detected_with_tty_uses_selector() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    std::fs::create_dir_all(dir.path().join(".claude")).unwrap();

    let select = |_available: &[String],
                  enable_bitloops_skill: bool|
     -> std::result::Result<InitAgentSelection, String> {
        Ok(InitAgentSelection {
            agents: vec![AGENT_CURSOR.to_string()],
            enable_bitloops_skill,
        })
    };

    with_process_state(
        Some(dir.path()),
        &[("BITLOOPS_TEST_TTY", Some("1"))],
        || {
            let mut out = Vec::new();
            let selected =
                detect_or_select_agent(dir.path(), &mut out, true, Some(&select)).unwrap();
            assert_eq!(selected.agents, vec![AGENT_CURSOR.to_string()]);
            assert!(selected.enable_bitloops_skill);
        },
    );
}

#[test]
fn detect_or_select_agent_selection_cancelled() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let select =
        |_available: &[String], _enable_bitloops_skill: bool| Err("user cancelled".to_string());
    with_process_state(
        Some(dir.path()),
        &[("BITLOOPS_TEST_TTY", Some("1"))],
        || {
            let mut out = Vec::new();
            let err =
                detect_or_select_agent(dir.path(), &mut out, true, Some(&select)).unwrap_err();
            assert!(format!("{err:#}").contains("user cancelled"));
        },
    );
}

#[test]
fn detect_or_select_agent_none_selected_errors() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let select = |_available: &[String], _enable_bitloops_skill: bool| {
        Ok(InitAgentSelection {
            agents: vec![],
            enable_bitloops_skill: true,
        })
    };
    with_process_state(
        Some(dir.path()),
        &[("BITLOOPS_TEST_TTY", Some("1"))],
        || {
            let mut out = Vec::new();
            let err =
                detect_or_select_agent(dir.path(), &mut out, true, Some(&select)).unwrap_err();
            assert!(format!("{err:#}").contains("no agents selected"));
        },
    );
}

#[test]
fn detect_or_select_agent_no_tty_returns_all_detected() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
    std::fs::create_dir_all(dir.path().join(".gemini")).unwrap();
    with_process_state(
        Some(dir.path()),
        &[("BITLOOPS_TEST_TTY", Some("0"))],
        || {
            let mut out = Vec::new();
            let selected = detect_or_select_agent(dir.path(), &mut out, true, None).unwrap();
            assert_eq!(selected.agents.len(), 2);
            assert!(selected.agents.contains(&AGENT_CLAUDE_CODE.to_string()));
            assert!(selected.agents.contains(&AGENT_GEMINI.to_string()));
            assert!(selected.enable_bitloops_skill);
        },
    );
}

#[test]
fn detect_or_select_agent_multiple_with_selector() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
    std::fs::create_dir_all(dir.path().join(".gemini")).unwrap();
    let select = |_available: &[String],
                  _enable_bitloops_skill: bool|
     -> std::result::Result<InitAgentSelection, String> {
        Ok(InitAgentSelection {
            agents: vec![
                AGENT_GEMINI.to_string(),
                AGENT_CODEX.to_string(),
                AGENT_CLAUDE_CODE.to_string(),
            ],
            enable_bitloops_skill: false,
        })
    };
    with_process_state(
        Some(dir.path()),
        &[("BITLOOPS_TEST_TTY", Some("1"))],
        || {
            let mut out = Vec::new();
            let selected =
                detect_or_select_agent(dir.path(), &mut out, true, Some(&select)).unwrap();
            assert_eq!(
                selected.agents,
                vec![
                    AGENT_GEMINI.to_string(),
                    AGENT_CODEX.to_string(),
                    AGENT_CLAUDE_CODE.to_string()
                ]
            );
            assert!(!selected.enable_bitloops_skill);
        },
    );
}

#[test]
fn init_args_supports_telemetry_flag() {
    let parsed = Cli::try_parse_from(["bitloops", "init", "--telemetry=false"])
        .expect("parse init telemetry flag");
    let Some(Commands::Init(args)) = parsed.command else {
        panic!("expected init command");
    };
    assert_eq!(args.telemetry, Some(false));
}

#[test]
fn init_args_support_no_telemetry_flag() {
    let parsed = Cli::try_parse_from(["bitloops", "init", "--no-telemetry"])
        .expect("parse init no telemetry flag");
    let Some(Commands::Init(args)) = parsed.command else {
        panic!("expected init command");
    };
    assert!(args.no_telemetry);
}

#[test]
fn init_args_support_disable_bitloops_skill_flag() {
    let parsed = Cli::try_parse_from(["bitloops", "init", "--disable-bitloops-skill"])
        .expect("parse init disable Bitloops Skill flag");
    let Some(Commands::Init(args)) = parsed.command else {
        panic!("expected init command");
    };
    assert!(args.disable_bitloops_skill);
}

#[test]
fn prompt_telemetry_consent_defaults_yes() {
    let mut out = Vec::new();
    let mut input = Cursor::new("\n");
    let consent = prompt_telemetry_consent(&mut out, &mut input).expect("telemetry prompt");
    assert!(consent);
}

#[test]
fn prompt_telemetry_consent_accepts_no() {
    let mut out = Vec::new();
    let mut input = Cursor::new("no\n");
    let consent = prompt_telemetry_consent(&mut out, &mut input).expect("telemetry prompt");
    assert!(!consent);
}

#[test]
fn run_init_prompts_for_unresolved_existing_telemetry_consent() {
    let repo = tempfile::tempdir().unwrap();
    let app_dirs = tempfile::tempdir().unwrap();
    setup_git_repo(&repo);

    with_temp_app_dirs(&app_dirs, true, true, || {
        ensure_daemon_config_exists().expect("create default daemon config");

        with_global_graphql_executor_hook(
            |_runtime_root, _query, variables| {
                if variables["telemetry"].is_null() {
                    Ok(serde_json::json!({
                        "updateCliTelemetryConsent": {
                            "telemetry": serde_json::Value::Null,
                            "needsPrompt": true
                        }
                    }))
                } else {
                    assert_eq!(variables["telemetry"], serde_json::json!(true));
                    Ok(serde_json::json!({
                        "updateCliTelemetryConsent": {
                            "telemetry": true,
                            "needsPrompt": false
                        }
                    }))
                }
            },
            || {
                let mut out = Vec::new();
                let mut input = Cursor::new("n\n3\n");
                let select = |_items: &[String], enable_bitloops_skill: bool| {
                    Ok(InitAgentSelection {
                        agents: vec!["claude-code".to_string()],
                        enable_bitloops_skill,
                    })
                };
                let runtime = test_runtime();
                runtime
                    .block_on(run_with_io_async_for_project_root(
                        InitArgs {
                            install_default_daemon: false,
                            force: false,
                            disable_bitloops_skill: false,
                            agent: Vec::new(),
                            telemetry: None,
                            no_telemetry: false,
                            skip_baseline: false,
                            sync: Some(false),
                            ingest: Some(false),
                            backfill: None,
                            exclude: Vec::new(),
                            exclude_from: Vec::new(),
                            embeddings_runtime: Some(
                                crate::cli::embeddings::EmbeddingsRuntime::Local,
                            ),
                            no_embeddings: false,
                            embeddings_gateway_url: None,
                            embeddings_api_key_env: "BITLOOPS_PLATFORM_GATEWAY_TOKEN".to_string(),
                        },
                        repo.path(),
                        &mut out,
                        &mut input,
                        Some(&select),
                    ))
                    .expect("run init");

                let rendered = String::from_utf8(out).expect("utf8 output");
                assert!(rendered.contains("Final setup"));
                assert!(rendered.contains("Enable anonymous telemetry"));
                assert!(!rendered.contains("Bitloops project bootstrap is ready."));
            },
        );
    });
}

#[test]
fn run_init_noninteractive_existing_telemetry_requires_explicit_flag() {
    let repo = tempfile::tempdir().unwrap();
    let app_dirs = tempfile::tempdir().unwrap();
    setup_git_repo(&repo);

    with_temp_app_dirs(&app_dirs, false, true, || {
        ensure_daemon_config_exists().expect("create default daemon config");

        with_global_graphql_executor_hook(
            |_runtime_root, _query, _variables| {
                Ok(serde_json::json!({
                    "updateCliTelemetryConsent": {
                        "telemetry": serde_json::Value::Null,
                        "needsPrompt": true
                    }
                }))
            },
            || {
                let mut out = Vec::new();
                let mut input = Cursor::new("");
                let runtime = test_runtime();
                let err = runtime
                    .block_on(run_with_io_async_for_project_root(
                        InitArgs {
                            install_default_daemon: false,
                            force: false,
                            disable_bitloops_skill: false,
                            agent: Vec::new(),
                            telemetry: None,
                            no_telemetry: false,
                            skip_baseline: false,
                            sync: Some(false),
                            ingest: Some(false),
                            backfill: None,
                            exclude: Vec::new(),
                            exclude_from: Vec::new(),
                            embeddings_runtime: Some(
                                crate::cli::embeddings::EmbeddingsRuntime::Local,
                            ),
                            no_embeddings: false,
                            embeddings_gateway_url: None,
                            embeddings_api_key_env: "BITLOOPS_PLATFORM_GATEWAY_TOKEN".to_string(),
                        },
                        repo.path(),
                        &mut out,
                        &mut input,
                        None,
                    ))
                    .expect_err("init should fail without explicit telemetry");

                assert_eq!(err.to_string(), NON_INTERACTIVE_TELEMETRY_ERROR);
                assert!(!repo.path().join(".bitloops.local.toml").exists());
            },
        );
    });
}

#[test]
fn run_init_noninteractive_fresh_daemon_bootstrap_requires_explicit_telemetry_flag() {
    let repo = tempfile::tempdir().unwrap();
    let app_dirs = tempfile::tempdir().unwrap();
    setup_git_repo(&repo);

    with_temp_app_dirs(&app_dirs, false, false, || {
        let mut out = Vec::new();
        let mut input = Cursor::new("");
        let runtime = test_runtime();
        let err = runtime
            .block_on(run_with_io_async_for_project_root(
                InitArgs {
                    install_default_daemon: true,
                    force: false,
                    disable_bitloops_skill: false,
                    agent: Vec::new(),
                    telemetry: None,
                    no_telemetry: false,
                    skip_baseline: false,
                    sync: Some(false),
                    ingest: Some(false),
                    backfill: None,
                    exclude: Vec::new(),
                    exclude_from: Vec::new(),
                    embeddings_runtime: Some(crate::cli::embeddings::EmbeddingsRuntime::Local),
                    no_embeddings: false,
                    embeddings_gateway_url: None,
                    embeddings_api_key_env: "BITLOOPS_PLATFORM_GATEWAY_TOKEN".to_string(),
                },
                repo.path(),
                &mut out,
                &mut input,
                None,
            ))
            .expect_err("init should fail without explicit telemetry flag");

        assert_eq!(err.to_string(), NON_INTERACTIVE_TELEMETRY_ERROR);
    });
}

#[test]
fn run_init_with_install_default_daemon_shows_shell_escaped_config_path() {
    let repo = tempfile::tempdir().unwrap();
    let app_dirs = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    setup_git_repo(&repo);
    let home_value = home.path().to_string_lossy().to_string();

    with_env_vars(
        &[
            ("HOME", Some(home_value.as_str())),
            ("USERPROFILE", Some(home_value.as_str())),
        ],
        || {
            with_summary_generation_configured_hook(
                |_| true,
                || {
                    with_test_platform_dir_overrides(
                        TestPlatformDirOverrides {
                            config_root: Some(
                                home.path().join("Library").join("Application Support"),
                            ),
                            data_root: Some(app_dirs.path().join("data-root")),
                            cache_root: Some(app_dirs.path().join("cache-root")),
                            state_root: Some(app_dirs.path().join("state-root")),
                        },
                        || {
                            with_test_tty_override(false, || {
                                with_test_assume_daemon_running(true, || {
                                    with_install_default_daemon_hook(
                                        move |install_default_daemon| {
                                            assert!(install_default_daemon);
                                            let config_path = ensure_daemon_config_exists()
                                                .expect("create default daemon config");
                                            write_runtime_only_daemon_config(
                                                &config_path,
                                                "bitloops-local-embeddings",
                                                &[],
                                            );
                                            Ok(())
                                        },
                                        || {
                                            with_global_graphql_executor_hook(
                                                |_runtime_root, _query, variables| {
                                                    assert_eq!(
                                                        variables["telemetry"],
                                                        serde_json::json!(false)
                                                    );
                                                    Ok(serde_json::json!({
                                                        "updateCliTelemetryConsent": {
                                                            "telemetry": false,
                                                            "needsPrompt": false
                                                        }
                                                    }))
                                                },
                                                || {
                                                    let mut out = Vec::new();
                                                    let mut input = Cursor::new("");
                                                    let runtime = test_runtime();
                                                    runtime
                                                        .block_on(run_with_io_async_for_project_root(
                                                            InitArgs {
                                                                install_default_daemon: true,
                                                                force: false,
                                                                disable_bitloops_skill: false,
                                                                agent: vec![DEFAULT_AGENT.to_string()],
                                                                telemetry: Some(false),
                                                                no_telemetry: false,
                                                                skip_baseline: false,
                                                                sync: Some(false),
                                                                ingest: Some(false),
                                                                backfill: None,
                                                                exclude: Vec::new(),
                                                                exclude_from: Vec::new(),
                                                                embeddings_runtime: None,
                                                                no_embeddings: true,
                                                                embeddings_gateway_url: None,
                                                                embeddings_api_key_env: "BITLOOPS_PLATFORM_GATEWAY_TOKEN"
                                                                    .to_string(),
                                                            },
                                                            repo.path(),
                                                            &mut out,
                                                            &mut input,
                                                            None,
                                                        ))
                                                        .expect("run init");

                                                    let rendered = String::from_utf8(out)
                                                        .expect("utf8 output");
                                                    assert!(
                                                        rendered
                                                            .contains("Starting Bitloops daemon…")
                                                    );
                                                    assert!(rendered.contains(
                                                        "Library/Application\\ Support/bitloops/config.toml"
                                                    ));
                                                    assert!(rendered.contains("port:   5667"));
                                                },
                                            );
                                        },
                                    );
                                })
                            });
                        },
                    );
                },
            );
        },
    );
}

#[test]
fn run_init_with_install_default_daemon_shows_mkcert_notice_before_live_progress() {
    let repo = tempfile::tempdir().unwrap();
    let app_dirs = tempfile::tempdir().unwrap();
    setup_git_repo(&repo);

    let rendered = render_install_default_daemon_handoff_with_mkcert(&repo, &app_dirs, false);
    let notice =
        "Notice: local dashboard HTTPS is unavailable because `mkcert` is not on your PATH.";
    let progress_url = "  • View progress: http://127.0.0.1:5667";
    let live_progress = "Live progress";

    assert!(
        rendered.contains(progress_url),
        "expected HTTP fallback URL in init handoff:\n{rendered}"
    );
    assert!(
        rendered.contains(notice),
        "expected mkcert notice in init handoff:\n{rendered}"
    );
    assert!(
        rendered.contains("Guide: https://bitloops.com/docs/guides/dashboard-local-https-setup"),
        "expected dashboard HTTPS guide in init handoff:\n{rendered}"
    );

    let notice_index = rendered.find(notice).expect("mkcert notice position");
    let live_progress_index = rendered
        .find(live_progress)
        .expect("live progress position");
    assert!(
        notice_index < live_progress_index,
        "mkcert notice should appear before live progress:\n{rendered}"
    );
}

#[test]
fn run_init_with_install_default_daemon_prefers_https_fallback_when_mkcert_is_available() {
    let repo = tempfile::tempdir().unwrap();
    let app_dirs = tempfile::tempdir().unwrap();
    setup_git_repo(&repo);

    let rendered = render_install_default_daemon_handoff_with_mkcert(&repo, &app_dirs, true);

    assert!(
        rendered.contains("  • View progress: https://127.0.0.1:5667"),
        "expected HTTPS fallback URL in init handoff:\n{rendered}"
    );
    assert!(
        !rendered.contains(
            "Notice: local dashboard HTTPS is unavailable because `mkcert` is not on your PATH."
        ),
        "did not expect mkcert-missing notice when mkcert is available:\n{rendered}"
    );
}

#[test]
fn run_init_without_install_default_daemon_leaves_embeddings_unconfigured() {
    let repo = tempfile::tempdir().unwrap();
    let app_dirs = tempfile::tempdir().unwrap();
    setup_git_repo(&repo);

    with_temp_app_dirs(&app_dirs, false, true, || {
        let config_path = ensure_daemon_config_exists().expect("create default daemon config");
        let (command, args) = fake_runtime_command_and_args(repo.path());
        write_runtime_only_daemon_config(&config_path, &command, &args);

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
                let mut out = Vec::new();
                let mut input = Cursor::new("");
                let runtime = test_runtime();
                runtime
                    .block_on(run_with_io_async_for_project_root(
                        InitArgs {
                            install_default_daemon: false,
                            force: false,
                            disable_bitloops_skill: false,
                            agent: Vec::new(),
                            telemetry: Some(false),
                            no_telemetry: false,
                            skip_baseline: false,
                            sync: Some(false),
                            ingest: Some(false),
                            backfill: None,
                            exclude: Vec::new(),
                            exclude_from: Vec::new(),
                            embeddings_runtime: Some(
                                crate::cli::embeddings::EmbeddingsRuntime::Local,
                            ),
                            no_embeddings: false,
                            embeddings_gateway_url: None,
                            embeddings_api_key_env: "BITLOOPS_PLATFORM_GATEWAY_TOKEN".to_string(),
                        },
                        repo.path(),
                        &mut out,
                        &mut input,
                        None,
                    ))
                    .expect("run init");

                let config = std::fs::read_to_string(&config_path).expect("read config");
                assert!(
                    !config.contains("code_embeddings = \"local_code\""),
                    "plain init should not install embeddings:\n{config}"
                );
            },
        );
    });
}

#[test]
fn run_init_interactive_prompts_for_embeddings_and_installs_when_accepted() {
    let repo = tempfile::tempdir().unwrap();
    let app_dirs = tempfile::tempdir().unwrap();
    setup_git_repo(&repo);

    with_temp_app_dirs(&app_dirs, true, true, || {
        let config_path = ensure_daemon_config_exists().expect("create default daemon config");
        write_runtime_only_daemon_config(&config_path, "bitloops-local-embeddings", &[]);

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
                with_managed_embeddings_install_hook(
                    move |repo_root| {
                        Ok(ManagedEmbeddingsBinaryInstallOutcome {
                            version: "v0.1.0".to_string(),
                            binary_path: fake_managed_runtime_path(repo_root),
                            freshly_installed: true,
                        })
                    },
                    || {
                        let mut out = Vec::new();
                        let mut input = Cursor::new("\n");
                        let select = |_items: &[String], enable_bitloops_skill: bool| {
                            Ok(InitAgentSelection {
                                agents: vec!["claude-code".to_string()],
                                enable_bitloops_skill,
                            })
                        };
                        let runtime = test_runtime();
                        runtime
                            .block_on(run_with_io_async_for_project_root(
                                InitArgs {
                                    install_default_daemon: false,
                                    force: false,
                                    disable_bitloops_skill: false,
                                    agent: Vec::new(),
                                    telemetry: Some(false),
                                    no_telemetry: false,
                                    skip_baseline: false,
                                    sync: Some(false),
                                    ingest: Some(false),
                                    backfill: None,
                                    exclude: Vec::new(),
                                    exclude_from: Vec::new(),
                                    embeddings_runtime: Some(
                                        crate::cli::embeddings::EmbeddingsRuntime::Local,
                                    ),
                                    no_embeddings: false,
                                    embeddings_gateway_url: None,
                                    embeddings_api_key_env: "BITLOOPS_PLATFORM_GATEWAY_TOKEN"
                                        .to_string(),
                                },
                                repo.path(),
                                &mut out,
                                &mut input,
                                Some(&select),
                            ))
                            .expect("run init");

                        let rendered = String::from_utf8(out).expect("utf8 output");
                        assert!(rendered.contains("Install local embeddings as well?"));
                        assert!(rendered.contains("Install embeddings now? (Y/n)"));
                        assert!(rendered.contains("> "));
                        assert!(rendered.contains("Preparing local embeddings setup..."));
                        assert!(rendered.contains(
                            "This can take a moment if the managed runtime needs to be downloaded."
                        ));
                        assert!(rendered.contains("Installed managed standalone"));
                        assert!(rendered.contains("Pulled embedding profile `local_code`."));

                        let daemon_config = ensure_daemon_config_exists()
                            .expect("resolve daemon config after init");
                        let daemon_config =
                            std::fs::read_to_string(daemon_config).expect("read daemon config");
                        assert!(daemon_config.contains("code_embeddings = \"local_code\""));
                        assert!(daemon_config.contains("[inference.profiles.local_code]"));
                    },
                );
            },
        );
    });
}

#[test]
fn run_init_with_install_default_daemon_sends_summary_bootstrap_when_prompt_is_accepted() {
    let repo = tempfile::tempdir().unwrap();
    let app_dirs = tempfile::tempdir().unwrap();
    let repo_id = test_repo_id(repo.path());
    let session_id = "init-session-summary-bootstrap";
    let saw_start_init = std::rc::Rc::new(std::cell::RefCell::new(false));
    setup_git_repo(&repo);

    with_summary_generation_configured_hook(
        |_| false,
        || {
            with_test_platform_dir_overrides(app_dir_overrides(&app_dirs), || {
                with_test_tty_override(true, || {
                    with_test_assume_daemon_running(true, || {
                        with_install_default_daemon_hook(
                            move |install_default_daemon| {
                                assert!(install_default_daemon);
                                let config_path = ensure_daemon_config_exists()
                                    .expect("create default daemon config");
                                write_runtime_only_daemon_config(
                                    &config_path,
                                    "bitloops-local-embeddings",
                                    &[],
                                );
                                Ok(())
                            },
                            || {
                                with_global_graphql_executor_hook(
                                    |_runtime_root, _query, variables| {
                                        assert_eq!(
                                            variables["telemetry"],
                                            serde_json::json!(false)
                                        );
                                        Ok(serde_json::json!({
                                            "updateCliTelemetryConsent": {
                                                "telemetry": false,
                                                "needsPrompt": false
                                            }
                                        }))
                                    },
                                    || {
                                        with_ollama_probe_hook(
                                            || {
                                                Ok(OllamaAvailability::Running {
                                                    models: vec!["ministral-3:3b".to_string()],
                                                })
                                            },
                                            || {
                                                with_ingest_daemon_bootstrap_hook(
                                                    |_repo_root| Ok(()),
                                                    || {
                                                        with_graphql_executor_hook(
                                                            {
                                                                let repo_id = repo_id.clone();
                                                                let saw_start_init =
                                                                    std::rc::Rc::clone(
                                                                        &saw_start_init,
                                                                    );
                                                                move |_repo_root, query, variables| {
                                                                    if query.contains("startInit(") {
                                                                        *saw_start_init.borrow_mut() = true;
                                                                        assert_eq!(variables["repoId"], repo_id);
                                                                        assert_eq!(
                                                                            variables["input"]["runSync"],
                                                                            json!(false)
                                                                        );
                                                                        assert_eq!(
                                                                            variables["input"]["runIngest"],
                                                                            json!(false)
                                                                        );
                                                                        assert_eq!(
                                                                            variables["input"]["embeddingsBootstrap"]["profileName"],
                                                                            json!("local_code")
                                                                        );
                                                                        assert_eq!(
                                                                            variables["input"]["summariesBootstrap"]["action"],
                                                                            json!("CONFIGURE_LOCAL")
                                                                        );
                                                                        assert_eq!(
                                                                            variables["input"]["summariesBootstrap"]["modelName"],
                                                                            json!("ministral-3:3b")
                                                                        );
                                                                        return Ok(runtime_start_init_result_json(session_id));
                                                                    }

                                                                    if query.contains("runtimeSnapshot(") {
                                                                        return Ok(runtime_snapshot_json(
                                                                            repo_id.as_str(),
                                                                            session_id,
                                                                            RuntimeSessionSnapshotFixture {
                                                                                status: "COMPLETED",
                                                                                embeddings_selected: true,
                                                                                summaries_selected: true,
                                                                                embeddings_lane_status: "COMPLETED",
                                                                                summaries_lane_status: "COMPLETED",
                                                                                ..RuntimeSessionSnapshotFixture::default()
                                                                            },
                                                                        ));
                                                                    }

                                                                    panic!("unexpected repo-scoped query: {query}");
                                                                }
                                                            },
                                                            || {
                                                                let mut out = Vec::new();
                                                                let mut input =
                                                                    Cursor::new("2\n\n");
                                                                let select = |_items: &[String],
                                                                              enable_bitloops_skill: bool| {
                                                                    Ok(InitAgentSelection {
                                                                        agents: vec![
                                                                            "claude-code"
                                                                                .to_string(),
                                                                        ],
                                                                        enable_bitloops_skill,
                                                                    })
                                                                };
                                                                let runtime = test_runtime();
                                                                runtime
                                                                    .block_on(run_with_io_async_for_project_root(
                                                                        InitArgs {
                                                                            install_default_daemon: true,
                                                                            force: false,
                                                                            disable_bitloops_skill: false,
                                                                            agent: Vec::new(),
                                                                            telemetry: Some(false),
                                                                            no_telemetry: false,
                                                                            skip_baseline: false,
                                                                            sync: Some(false),
                                                                            ingest: Some(false),
                                                                            backfill: None,
                                                                            exclude: Vec::new(),
                                                                            exclude_from: Vec::new(),
                                                                            embeddings_runtime: Some(crate::cli::embeddings::EmbeddingsRuntime::Local),
                                                                            no_embeddings: false,
                                                                            embeddings_gateway_url: None,
                                                                            embeddings_api_key_env: "BITLOOPS_PLATFORM_GATEWAY_TOKEN".to_string(),
                                                                        },
                                                                        repo.path(),
                                                                        &mut out,
                                                                        &mut input,
                                                                        Some(&select),
                                                                    ))
                                                                    .expect("run init");

                                                                let rendered =
                                                                    String::from_utf8(out)
                                                                        .expect("utf8 output");
                                                                assert!(rendered.contains(
                                                                    "Configure semantic summaries"
                                                                ));
                                                                assert!(rendered.contains(
                                                                    "1. Bitloops Cloud (recommended)"
                                                                ));
                                                                assert!(
                                                                    rendered.contains(
                                                                        "2. Local (Ollama)"
                                                                    )
                                                                );
                                                                assert!(
                                                                    rendered.contains(
                                                                        "3. Skip for now"
                                                                    )
                                                                );
                                                                assert!(
                                                                    rendered.contains("Summaries")
                                                                );

                                                                let daemon_config_path =
                                                                    ensure_daemon_config_exists()
                                                                        .expect(
                                                                            "daemon config path",
                                                                        );
                                                                let daemon_config =
                                                                    std::fs::read_to_string(
                                                                        daemon_config_path,
                                                                    )
                                                                    .expect("read daemon config");
                                                                assert!(
                                                                    !daemon_config.contains(
                                                                        "summary_generation = \"summary_local\""
                                                                    ),
                                                                    "summary configuration should now be applied by the daemon bootstrap path:\n{daemon_config}"
                                                                );
                                                            },
                                                        )
                                                    },
                                                );
                                            },
                                        )
                                    },
                                );
                            },
                        );
                    })
                })
            })
        },
    );

    assert!(
        *saw_start_init.borrow(),
        "init should send a summary bootstrap request to the runtime API"
    );
}

#[test]
fn run_init_with_install_default_daemon_auto_installs_embeddings() {
    let repo = tempfile::tempdir().unwrap();
    let app_dirs = tempfile::tempdir().unwrap();
    let repo_id = test_repo_id(repo.path());
    let session_id = "init-session-auto-embeddings";
    let saw_start_init = std::rc::Rc::new(std::cell::RefCell::new(false));
    setup_git_repo(&repo);

    with_temp_app_dirs(&app_dirs, false, true, || {
        with_install_default_daemon_hook(
            move |install_default_daemon| {
                assert!(install_default_daemon);
                let config_path =
                    ensure_daemon_config_exists().expect("create default daemon config");
                write_runtime_only_daemon_config(&config_path, "bitloops-local-embeddings", &[]);
                Ok(())
            },
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
                        with_graphql_executor_hook(
                            {
                                let repo_id = repo_id.clone();
                                let saw_start_init = std::rc::Rc::clone(&saw_start_init);
                                move |_repo_root, query, variables| {
                                    if query.contains("startInit(") {
                                        *saw_start_init.borrow_mut() = true;
                                        assert_eq!(variables["repoId"], repo_id);
                                        assert_eq!(variables["input"]["runSync"], json!(false));
                                        assert_eq!(variables["input"]["runIngest"], json!(false));
                                        assert_eq!(
                                            variables["input"]["embeddingsBootstrap"]["profileName"],
                                            json!("local_code")
                                        );
                                        assert_eq!(
                                            variables["input"]["summariesBootstrap"],
                                            serde_json::Value::Null
                                        );
                                        return Ok(runtime_start_init_result_json(session_id));
                                    }

                                    if query.contains("runtimeSnapshot(") {
                                        return Ok(runtime_snapshot_json(
                                            repo_id.as_str(),
                                            session_id,
                                            RuntimeSessionSnapshotFixture {
                                                embeddings_selected: true,
                                                embeddings_lane_status: "COMPLETED",
                                                status: "COMPLETED",
                                                ..RuntimeSessionSnapshotFixture::default()
                                            },
                                        ));
                                    }

                                    panic!("unexpected repo-scoped query: {query}");
                                }
                            },
                            || {
                                let mut out = Vec::new();
                                let mut input = Cursor::new("");
                                let runtime = test_runtime();
                                runtime
                                    .block_on(run_with_io_async_for_project_root(
                                        InitArgs {
                                            install_default_daemon: true,
                                            force: false,
                                            disable_bitloops_skill: false,
                                            agent: Vec::new(),
                                            telemetry: Some(false),
                                            no_telemetry: false,
                                            skip_baseline: false,
                                            sync: Some(false),
                                            ingest: Some(false),
                                            backfill: None,
                                            exclude: Vec::new(),
                                            exclude_from: Vec::new(),
                                            embeddings_runtime: Some(
                                                crate::cli::embeddings::EmbeddingsRuntime::Local,
                                            ),
                                            no_embeddings: false,
                                            embeddings_gateway_url: None,
                                            embeddings_api_key_env:
                                                "BITLOOPS_PLATFORM_GATEWAY_TOKEN".to_string(),
                                        },
                                        repo.path(),
                                        &mut out,
                                        &mut input,
                                        None,
                                    ))
                                    .expect("run init");

                                let rendered = String::from_utf8(out).expect("utf8 output");
                                assert!(
                                    !rendered
                                        .contains("Queueing embeddings bootstrap in the daemon...")
                                );
                                assert!(!rendered.contains("Embeddings bootstrap task:"));
                                assert!(!rendered.contains("Embeddings bootstrap phase:"));
                                assert!(rendered.contains("✓ Setup complete"));
                                assert!(rendered.contains(
                                    "Bitloops is now building your project's Intelligence Layer in the background."
                                ));
                                assert!(rendered.contains("Live progress"));
                                assert!(rendered.contains("Embeddings"));
                                let config = std::fs::read_to_string(
                                    repo.path().join(BITLOOPS_CONFIG_RELATIVE_PATH),
                                )
                                .unwrap_or_else(|_| String::new());
                                assert!(
                                    config.is_empty(),
                                    "init with default daemon should use the daemon config, not repo-local config:\n{config}"
                                );
                                let daemon_config = ensure_daemon_config_exists()
                                    .expect("resolve daemon config after init");
                                let daemon_config = std::fs::read_to_string(daemon_config)
                                    .expect("read daemon config");
                                assert!(
                                    !daemon_config.contains("code_embeddings = \"local_code\""),
                                    "embeddings config should now be applied asynchronously by the daemon task:\n{daemon_config}"
                                );
                            },
                        );
                    },
                );
            },
        );
    });

    assert!(
        *saw_start_init.borrow(),
        "init should start a runtime session for embeddings bootstrap"
    );
}

#[test]
fn run_init_with_install_default_daemon_requires_explicit_embeddings_choice_when_noninteractive() {
    let repo = tempfile::tempdir().unwrap();
    let app_dirs = tempfile::tempdir().unwrap();
    setup_git_repo(&repo);

    with_temp_app_dirs_and_summary_configured(&app_dirs, false, true, true, || {
        with_install_default_daemon_hook(
            move |install_default_daemon| {
                assert!(install_default_daemon);
                let config_path =
                    ensure_daemon_config_exists().expect("create default daemon config");
                write_runtime_only_daemon_config(&config_path, "bitloops-local-embeddings", &[]);
                Ok(())
            },
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
                        let mut out = Vec::new();
                        let mut input = Cursor::new("");
                        let runtime = test_runtime();
                        let err = runtime
                            .block_on(run_with_io_async_for_project_root(
                                InitArgs {
                                    install_default_daemon: true,
                                    force: false,
                                    disable_bitloops_skill: false,
                                    agent: Vec::new(),
                                    telemetry: Some(false),
                                    no_telemetry: false,
                                    skip_baseline: false,
                                    sync: Some(false),
                                    ingest: Some(false),
                                    backfill: None,
                                    exclude: Vec::new(),
                                    exclude_from: Vec::new(),
                                    embeddings_runtime: None,
                                    no_embeddings: false,
                                    embeddings_gateway_url: None,
                                    embeddings_api_key_env: "BITLOOPS_PLATFORM_GATEWAY_TOKEN"
                                        .to_string(),
                                },
                                repo.path(),
                                &mut out,
                                &mut input,
                                None,
                            ))
                            .expect_err("non-interactive init should require an embeddings choice");

                        assert!(
                            format!("{err:#}")
                                .contains(NON_INTERACTIVE_INIT_EMBEDDINGS_SELECTION_ERROR)
                        );
                    },
                );
            },
        );
    });
}

#[test]
fn run_init_with_install_default_daemon_can_skip_embeddings_via_flag() {
    let repo = tempfile::tempdir().unwrap();
    let app_dirs = tempfile::tempdir().unwrap();
    setup_git_repo(&repo);

    with_temp_app_dirs_and_summary_configured(&app_dirs, false, true, true, || {
        with_install_default_daemon_hook(
            move |install_default_daemon| {
                assert!(install_default_daemon);
                let config_path =
                    ensure_daemon_config_exists().expect("create default daemon config");
                write_runtime_only_daemon_config(&config_path, "bitloops-local-embeddings", &[]);
                Ok(())
            },
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
                        let mut out = Vec::new();
                        let mut input = Cursor::new("");
                        let runtime = test_runtime();
                        let run_result = runtime.block_on(run_with_io_async_for_project_root(
                            InitArgs {
                                install_default_daemon: true,
                                force: false,
                                disable_bitloops_skill: false,
                                agent: vec![DEFAULT_AGENT.to_string()],
                                telemetry: Some(false),
                                no_telemetry: false,
                                skip_baseline: false,
                                sync: Some(false),
                                ingest: Some(false),
                                backfill: None,
                                exclude: Vec::new(),
                                exclude_from: Vec::new(),
                                embeddings_runtime: None,
                                no_embeddings: true,
                                embeddings_gateway_url: None,
                                embeddings_api_key_env: "BITLOOPS_PLATFORM_GATEWAY_TOKEN"
                                    .to_string(),
                            },
                            repo.path(),
                            &mut out,
                            &mut input,
                            None,
                        ));
                        std::mem::forget(runtime);
                        run_result.expect("run init without embeddings");

                        let _ = String::from_utf8(out).expect("utf8 output");

                        let daemon_config = ensure_daemon_config_exists()
                            .expect("resolve daemon config after init");
                        let daemon_config =
                            std::fs::read_to_string(daemon_config).expect("read daemon config");
                        assert!(
                            !daemon_config.contains("code_embeddings = "),
                            "skip should leave embeddings unconfigured:\n{daemon_config}"
                        );
                        assert!(
                            !daemon_config.contains("summary_embeddings = "),
                            "skip should leave summary embeddings unconfigured:\n{daemon_config}"
                        );

                        let config_root = ensure_daemon_config_exists()
                            .expect("resolve daemon config after init")
                            .parent()
                            .expect("daemon config parent")
                            .to_path_buf();
                        let store =
                            crate::host::runtime_store::RepoSqliteRuntimeStore::open_for_roots(
                                &config_root,
                                repo.path(),
                            )
                            .expect("open repo runtime store");
                        let status = store
                            .load_capability_workplane_mailbox_status(
                                crate::capability_packs::semantic_clones::SEMANTIC_CLONES_CAPABILITY_ID,
                                crate::capability_packs::semantic_clones::workplane::SEMANTIC_CLONES_DEFERRED_PIPELINE_MAILBOXES
                                    .iter()
                                    .copied(),
                            )
                            .expect("load mailbox status");
                        assert!(
                            !status
                                .get(crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX)
                                .is_some_and(|status| status.intent_active),
                            "skip should not activate code embeddings mailbox: {status:#?}"
                        );
                        assert!(
                            !status
                                .get(crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX)
                                .is_some_and(|status| status.intent_active),
                            "skip should not activate summary embeddings mailbox: {status:#?}"
                        );
                        assert!(
                            !status
                                .get(crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX)
                                .is_some_and(|status| status.intent_active),
                            "skip should not activate clone rebuild mailbox: {status:#?}"
                        );
                    },
                );
            },
        );
    });
}

#[test]
fn run_init_with_install_default_daemon_can_configure_cloud_embeddings_from_gateway_env() {
    let repo = tempfile::tempdir().unwrap();
    let app_dirs = tempfile::tempdir().unwrap();
    let repo_id = test_repo_id(repo.path());
    let session_id = "init-session-cloud-embeddings";
    setup_git_repo(&repo);

    with_temp_app_dirs_and_summary_configured(&app_dirs, true, true, false, || {
        with_install_default_daemon_hook(
            move |install_default_daemon| {
                assert!(install_default_daemon);
                let config_path =
                    ensure_daemon_config_exists().expect("create default daemon config");
                write_runtime_only_daemon_config(&config_path, "bitloops-local-embeddings", &[]);
                Ok(())
            },
            || {
                with_env_vars(
                    &[(
                        "BITLOOPS_PLATFORM_GATEWAY_URL",
                        Some("https://platform.example"),
                    )],
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
                                with_managed_platform_embeddings_install_hook(
                                    {
                                        let repo_root = repo.path().to_path_buf();
                                        move || {
                                            Ok(ManagedPlatformEmbeddingsBinaryInstallOutcome {
                                                version: "v0.2.0".to_string(),
                                                binary_path: repo_root
                                                    .join(".bitloops/test-bin/bitloops-platform-embeddings"),
                                                freshly_installed: true,
                                            })
                                        }
                                    },
                                    || {
                                        with_graphql_executor_hook(
                                            {
                                                let repo_id = repo_id.clone();
                                                move |_repo_root, query, variables| {
                                                    if query.contains("startInit(") {
                                                        assert_eq!(variables["repoId"], repo_id);
                                                        assert_eq!(
                                                            variables["input"]["embeddingsBootstrap"]
                                                                ["profileName"],
                                                            json!("platform_code")
                                                        );
                                                        assert_eq!(
                                                            variables["input"]["embeddingsBootstrap"]
                                                                ["mode"],
                                                            json!("PLATFORM")
                                                        );
                                                        assert_eq!(
                                                            variables["input"]["embeddingsBootstrap"]
                                                                ["gatewayUrlOverride"],
                                                            json!(
                                                                "https://platform.example/v1/embeddings"
                                                            )
                                                        );
                                                        assert_eq!(
                                                            variables["input"]["embeddingsBootstrap"]
                                                                ["apiKeyEnv"],
                                                            json!(
                                                                "BITLOOPS_PLATFORM_GATEWAY_TOKEN"
                                                            )
                                                        );
                                                        assert_eq!(
                                                            variables["input"]["summariesBootstrap"],
                                                            serde_json::Value::Null
                                                        );
                                                        return Ok(runtime_start_init_result_json(
                                                            session_id,
                                                        ));
                                                    }

                                                    if query.contains("runtimeSnapshot(") {
                                                        return Ok(runtime_snapshot_json(
                                                            repo_id.as_str(),
                                                            session_id,
                                                            RuntimeSessionSnapshotFixture {
                                                                status: "COMPLETED",
                                                                embeddings_selected: true,
                                                                embeddings_lane_status: "COMPLETED",
                                                                top_lane_status: "COMPLETED",
                                                                ..RuntimeSessionSnapshotFixture::default()
                                                            },
                                                        ));
                                                    }

                                                    panic!("unexpected repo-scoped query: {query}");
                                                }
                                            },
                                            || {
                                                let mut out = Vec::new();
                                                let mut input = Cursor::new("1\n3\n\n");
                                                let runtime = test_runtime();
                                                runtime
                                                    .block_on(run_with_io_async_for_project_root(
                                                        InitArgs {
                                                            install_default_daemon: true,
                                                            force: false,
                                                            disable_bitloops_skill: false,
                                                            agent: vec![DEFAULT_AGENT.to_string()],
                                                            telemetry: Some(false),
                                                            no_telemetry: false,
                                                            skip_baseline: false,
                                                            sync: Some(false),
                                                            ingest: Some(false),
                                                            backfill: None,
                                                            exclude: Vec::new(),
                                                            exclude_from: Vec::new(),
                                                            embeddings_runtime: None,
                                                            no_embeddings: false,
                                                            embeddings_gateway_url: None,
                                                            embeddings_api_key_env:
                                                                "BITLOOPS_PLATFORM_GATEWAY_TOKEN"
                                                                    .to_string(),
                                                        },
                                                        repo.path(),
                                                        &mut out,
                                                        &mut input,
                                                        None,
                                                    ))
                                                    .expect("run init with cloud embeddings");

                                                let rendered =
                                                    String::from_utf8(out).expect("utf8 output");
                                                assert!(rendered.contains("Configure embeddings"));
                                                assert!(rendered.contains("Embeddings"));
                                                assert!(
                                                    !rendered.contains(
                                                        "Configured platform embeddings in"
                                                    )
                                                );
                                                assert!(!rendered.contains(
                                                    "Installed managed standalone `bitloops-platform-embeddings` runtime"
                                                ));
                                            },
                                        );
                                    },
                                );
                            },
                        );
                    },
                );
            },
        );
    });
}

#[test]
fn run_init_with_install_default_daemon_can_configure_cloud_embeddings_without_gateway_override() {
    let repo = tempfile::tempdir().unwrap();
    let app_dirs = tempfile::tempdir().unwrap();
    let repo_id = test_repo_id(repo.path());
    let session_id = "init-session-cloud-embeddings-default-gateway";
    setup_git_repo(&repo);

    with_temp_app_dirs_and_summary_configured(&app_dirs, true, true, false, || {
        with_install_default_daemon_hook(
            move |install_default_daemon| {
                assert!(install_default_daemon);
                let config_path =
                    ensure_daemon_config_exists().expect("create default daemon config");
                write_runtime_only_daemon_config(&config_path, "bitloops-local-embeddings", &[]);
                Ok(())
            },
            || {
                with_env_vars(&[("BITLOOPS_PLATFORM_GATEWAY_URL", None)], || {
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
                            with_managed_platform_embeddings_install_hook(
                                {
                                    let repo_root = repo.path().to_path_buf();
                                    move || {
                                        Ok(ManagedPlatformEmbeddingsBinaryInstallOutcome {
                                            version: "v0.2.0".to_string(),
                                            binary_path: repo_root.join(
                                                ".bitloops/test-bin/bitloops-platform-embeddings",
                                            ),
                                            freshly_installed: true,
                                        })
                                    }
                                },
                                || {
                                    with_graphql_executor_hook(
                                        {
                                            let repo_id = repo_id.clone();
                                            move |_repo_root, query, variables| {
                                                if query.contains("startInit(") {
                                                    assert_eq!(variables["repoId"], repo_id);
                                                    assert_eq!(
                                                        variables["input"]["embeddingsBootstrap"]["profileName"],
                                                        json!("platform_code")
                                                    );
                                                    assert_eq!(
                                                        variables["input"]["embeddingsBootstrap"]["mode"],
                                                        json!("PLATFORM")
                                                    );
                                                    assert_eq!(
                                                        variables["input"]["embeddingsBootstrap"]["gatewayUrlOverride"],
                                                        serde_json::Value::Null
                                                    );
                                                    assert_eq!(
                                                        variables["input"]["embeddingsBootstrap"]["apiKeyEnv"],
                                                        json!("BITLOOPS_PLATFORM_GATEWAY_TOKEN")
                                                    );
                                                    assert_eq!(
                                                        variables["input"]["summariesBootstrap"],
                                                        serde_json::Value::Null
                                                    );
                                                    return Ok(runtime_start_init_result_json(
                                                        session_id,
                                                    ));
                                                }

                                                if query.contains("runtimeSnapshot(") {
                                                    return Ok(runtime_snapshot_json(
                                                        repo_id.as_str(),
                                                        session_id,
                                                        RuntimeSessionSnapshotFixture {
                                                            status: "COMPLETED",
                                                            embeddings_selected: true,
                                                            embeddings_lane_status: "COMPLETED",
                                                            top_lane_status: "COMPLETED",
                                                            ..RuntimeSessionSnapshotFixture::default(
                                                            )
                                                        },
                                                    ));
                                                }

                                                panic!("unexpected repo-scoped query: {query}");
                                            }
                                        },
                                        || {
                                            let mut out = Vec::new();
                                            let mut input = Cursor::new("1\n3\n\n");
                                            let runtime = test_runtime();
                                            runtime
                                                .block_on(run_with_io_async_for_project_root(
                                                    InitArgs {
                                                        install_default_daemon: true,
                                                        force: false,
                                                        disable_bitloops_skill: false,
                                                        agent: vec![DEFAULT_AGENT.to_string()],
                                                        telemetry: Some(false),
                                                        no_telemetry: false,
                                                        skip_baseline: false,
                                                        sync: Some(false),
                                                        ingest: Some(false),
                                                        backfill: None,
                                                        exclude: Vec::new(),
                                                        exclude_from: Vec::new(),
                                                        embeddings_runtime: None,
                                                        no_embeddings: false,
                                                        embeddings_gateway_url: None,
                                                        embeddings_api_key_env:
                                                            "BITLOOPS_PLATFORM_GATEWAY_TOKEN"
                                                                .to_string(),
                                                    },
                                                    repo.path(),
                                                    &mut out,
                                                    &mut input,
                                                    None,
                                                ))
                                                .expect(
                                                    "cloud embeddings without a gateway override should succeed",
                                                );
                                            let rendered =
                                                String::from_utf8(out).expect("utf8 output");
                                            assert!(rendered.contains("Embeddings"));
                                        },
                                    );
                                },
                            );
                        },
                    );
                });
            },
        );
    });
}

#[test]
fn run_init_with_install_default_daemon_starts_runtime_session_for_sync_ingest_and_embeddings() {
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let repo = tempfile::tempdir().unwrap();
    let app_dirs = tempfile::tempdir().unwrap();
    let repo_id = test_repo_id(repo.path());
    let session_id = "init-session-sync-ingest-embeddings";
    setup_git_repo(&repo);

    with_temp_app_dirs(&app_dirs, false, true, || {
        with_install_default_daemon_hook(
            move |install_default_daemon| {
                assert!(install_default_daemon);
                let config_path =
                    ensure_daemon_config_exists().expect("create default daemon config");
                write_runtime_only_daemon_config(&config_path, "bitloops-local-embeddings", &[]);
                Ok(())
            },
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
                        with_ingest_daemon_bootstrap_hook(
                            |_repo_root| Ok(()),
                            || {
                                with_graphql_executor_hook(
                                    {
                                        let events = std::rc::Rc::clone(&events);
                                        let repo_id = repo_id.clone();
                                        move |_repo_root, query, variables| {
                                            if query.contains("startInit(") {
                                                events.borrow_mut().push("start_init".to_string());
                                                assert_eq!(variables["repoId"], repo_id);
                                                assert_eq!(
                                                    variables["input"]["runSync"],
                                                    json!(true)
                                                );
                                                assert_eq!(
                                                    variables["input"]["runIngest"],
                                                    json!(true)
                                                );
                                                assert_eq!(
                                                    variables["input"]["ingestBackfill"],
                                                    json!(50)
                                                );
                                                assert_eq!(
                                                    variables["input"]["embeddingsBootstrap"]["profileName"],
                                                    json!("local_code")
                                                );
                                                return Ok(runtime_start_init_result_json(
                                                    session_id,
                                                ));
                                            }

                                            if query.contains("runtimeSnapshot(") {
                                                events.borrow_mut().push("snapshot".to_string());
                                                return Ok(runtime_snapshot_json(
                                                    repo_id.as_str(),
                                                    session_id,
                                                    RuntimeSessionSnapshotFixture {
                                                        status: "COMPLETED",
                                                        run_sync: true,
                                                        run_ingest: true,
                                                        embeddings_selected: true,
                                                        top_lane_status: "COMPLETED",
                                                        embeddings_lane_status: "COMPLETED",
                                                        ..RuntimeSessionSnapshotFixture::default()
                                                    },
                                                ));
                                            }

                                            panic!("unexpected repo-scoped query: {query}");
                                        }
                                    },
                                    || {
                                        let mut out = Vec::new();
                                        let mut input = Cursor::new("");
                                        let runtime = test_runtime();
                                        runtime
                                            .block_on(run_with_io_async_for_project_root(
                                                InitArgs {
                                                    install_default_daemon: true,
                                                    force: false,
                                                    disable_bitloops_skill: false,
                                                    agent: Vec::new(),
                                                    telemetry: Some(false),
                                                    no_telemetry: false,
                                                    skip_baseline: false,
                                                    sync: Some(true),
                                                    ingest: Some(true),
                                                    backfill: None,
                                                    exclude: Vec::new(),
                                                    exclude_from: Vec::new(),
                                                embeddings_runtime: Some(crate::cli::embeddings::EmbeddingsRuntime::Local),
                                                no_embeddings: false,
                                                embeddings_gateway_url: None,
                                                embeddings_api_key_env: "BITLOOPS_PLATFORM_GATEWAY_TOKEN".to_string(),
                                                },
                                                repo.path(),
                                                &mut out,
                                                &mut input,
                                                None,
                                            ))
                                            .expect("run init");

                                        let rendered = String::from_utf8(out).expect("utf8 output");
                                        assert!(rendered.contains("✓ Setup complete"));
                                        assert!(rendered.contains(
                                            "Bitloops is now building your project's Intelligence Layer in the background."
                                        ));
                                        assert!(rendered.contains("Live progress"));
                                        assert!(rendered.contains(
                                            "This may take a few minutes depending on your codebase size."
                                        ));
                                        assert!(!rendered.contains(
                                            "Queueing embeddings bootstrap in the daemon..."
                                        ));
                                        assert!(!rendered.contains("Embeddings bootstrap task:"));
                                        assert!(!rendered.contains("Embeddings bootstrap phase:"));
                                        assert!(
                                            !rendered.contains("Starting initial DevQL sync...")
                                        );
                                        assert!(rendered.contains("Embeddings"));
                                    },
                                )
                            },
                        );
                    },
                );
            },
        );
    });

    assert_eq!(
        &*events.borrow(),
        &["start_init".to_string(), "snapshot".to_string()]
    );
}

#[test]
fn run_init_with_install_default_daemon_renders_follow_up_sync_waiting_state() {
    let snapshot_count = std::rc::Rc::new(std::cell::RefCell::new(0usize));
    let repo = tempfile::tempdir().unwrap();
    let app_dirs = tempfile::tempdir().unwrap();
    let repo_id = test_repo_id(repo.path());
    let session_id = "init-session-follow-up-sync";
    setup_git_repo(&repo);

    with_temp_app_dirs(&app_dirs, false, true, || {
        with_install_default_daemon_hook(
            move |install_default_daemon| {
                assert!(install_default_daemon);
                let config_path =
                    ensure_daemon_config_exists().expect("create default daemon config");
                write_runtime_only_daemon_config(&config_path, "bitloops-embeddings", &[]);
                Ok(())
            },
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
                        with_ingest_daemon_bootstrap_hook(
                            |_repo_root| Ok(()),
                            || {
                                with_graphql_executor_hook(
                                    {
                                        let snapshot_count = std::rc::Rc::clone(&snapshot_count);
                                        let repo_id = repo_id.clone();
                                        move |_repo_root, query, variables| {
                                            if query.contains("startInit(") {
                                                assert_eq!(variables["repoId"], repo_id);
                                                assert_eq!(
                                                    variables["input"]["runSync"],
                                                    json!(true)
                                                );
                                                assert_eq!(
                                                    variables["input"]["runIngest"],
                                                    json!(false)
                                                );
                                                assert_eq!(
                                                    variables["input"]["embeddingsBootstrap"]["profileName"],
                                                    json!("local_code")
                                                );
                                                return Ok(runtime_start_init_result_json(
                                                    session_id,
                                                ));
                                            }

                                            if query.contains("runtimeSnapshot(") {
                                                let mut count = snapshot_count.borrow_mut();
                                                *count += 1;
                                                let fixture = if *count == 1 {
                                                    RuntimeSessionSnapshotFixture {
                                                        status: "RUNNING",
                                                        waiting_reason: Some(
                                                            "waiting_for_follow_up_sync",
                                                        ),
                                                        follow_up_sync_required: true,
                                                        run_sync: true,
                                                        embeddings_selected: true,
                                                        top_lane_status: "RUNNING",
                                                        top_lane_waiting_reason: Some(
                                                            "waiting_for_follow_up_sync",
                                                        ),
                                                        embeddings_lane_status: "COMPLETED",
                                                        ..RuntimeSessionSnapshotFixture::default()
                                                    }
                                                } else {
                                                    RuntimeSessionSnapshotFixture {
                                                        status: "COMPLETED",
                                                        follow_up_sync_required: true,
                                                        run_sync: true,
                                                        embeddings_selected: true,
                                                        top_lane_status: "COMPLETED",
                                                        embeddings_lane_status: "COMPLETED",
                                                        ..RuntimeSessionSnapshotFixture::default()
                                                    }
                                                };
                                                return Ok(runtime_snapshot_json(
                                                    repo_id.as_str(),
                                                    session_id,
                                                    fixture,
                                                ));
                                            }

                                            panic!("unexpected repo-scoped query: {query}");
                                        }
                                    },
                                    || {
                                        let mut out = Vec::new();
                                        let mut input = Cursor::new("");
                                        let runtime = test_runtime();
                                        runtime
                                            .block_on(run_with_io_async_for_project_root(
                                                InitArgs {
                                                    install_default_daemon: true,
                                                    force: false,
                                                    disable_bitloops_skill: false,
                                                    agent: Vec::new(),
                                                    telemetry: Some(false),
                                                    no_telemetry: false,
                                                    skip_baseline: false,
                                                    sync: Some(true),
                                                    ingest: Some(false),
                                                    backfill: None,
                                                    exclude: Vec::new(),
                                                    exclude_from: Vec::new(),
                                                    embeddings_runtime:
                                                        Some(crate::cli::embeddings::EmbeddingsRuntime::Local),
                                                    no_embeddings: false,
                                                    embeddings_gateway_url: None,
                                                    embeddings_api_key_env:
                                                        "BITLOOPS_PLATFORM_GATEWAY_TOKEN"
                                                            .to_string(),
                                                },
                                                repo.path(),
                                                &mut out,
                                                &mut input,
                                                None,
                                            ))
                                            .expect("run init");

                                        let rendered = String::from_utf8(out).expect("utf8 output");
                                        assert!(rendered.contains("follow-up sync"));
                                    },
                                );
                            },
                        );
                    },
                );
            },
        );
    });

    assert!(
        *snapshot_count.borrow() >= 2,
        "expected the renderer to observe a follow-up sync wait before completion"
    );
}

#[test]
fn run_init_with_install_default_daemon_does_not_mark_summaries_complete_while_waiting_for_follow_up_sync()
 {
    let snapshot_count = std::rc::Rc::new(std::cell::RefCell::new(0usize));
    let repo = tempfile::tempdir().unwrap();
    let app_dirs = tempfile::tempdir().unwrap();
    let repo_id = test_repo_id(repo.path());
    let session_id = "init-session-summary-follow-up-wait";
    setup_git_repo(&repo);

    with_temp_app_dirs(&app_dirs, false, true, || {
        with_install_default_daemon_hook(
            move |install_default_daemon| {
                assert!(install_default_daemon);
                let config_path =
                    ensure_daemon_config_exists().expect("create default daemon config");
                write_runtime_only_daemon_config(&config_path, "bitloops-embeddings", &[]);
                Ok(())
            },
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
                        with_ingest_daemon_bootstrap_hook(
                            |_repo_root| Ok(()),
                            || {
                                with_graphql_executor_hook(
                                    {
                                        let snapshot_count = std::rc::Rc::clone(&snapshot_count);
                                        let repo_id = repo_id.clone();
                                        move |_repo_root, query, variables| {
                                            if query.contains("startInit(") {
                                                assert_eq!(variables["repoId"], repo_id);
                                                assert_eq!(
                                                    variables["input"]["runSync"],
                                                    json!(true)
                                                );
                                                assert_eq!(
                                                    variables["input"]["runIngest"],
                                                    json!(false)
                                                );
                                                assert_eq!(
                                                    variables["input"]["embeddingsBootstrap"]["profileName"],
                                                    json!("local_code")
                                                );
                                                return Ok(runtime_start_init_result_json(
                                                    session_id,
                                                ));
                                            }

                                            if query.contains("runtimeSnapshot(") {
                                                let mut count = snapshot_count.borrow_mut();
                                                *count += 1;
                                                let fixture = if *count == 1 {
                                                    RuntimeSessionSnapshotFixture {
                                                        status: "RUNNING",
                                                        waiting_reason: Some(
                                                            "waiting_for_embeddings_bootstrap",
                                                        ),
                                                        follow_up_sync_required: true,
                                                        run_sync: true,
                                                        run_ingest: false,
                                                        embeddings_selected: true,
                                                        summaries_selected: true,
                                                        top_lane_status: "COMPLETED",
                                                        embeddings_lane_status: "RUNNING",
                                                        summaries_lane_status: "WAITING",
                                                        summaries_lane_waiting_reason: Some(
                                                            "waiting_for_follow_up_sync",
                                                        ),
                                                        ..RuntimeSessionSnapshotFixture::default()
                                                    }
                                                } else {
                                                    RuntimeSessionSnapshotFixture {
                                                        status: "COMPLETED",
                                                        follow_up_sync_required: true,
                                                        run_sync: true,
                                                        run_ingest: false,
                                                        embeddings_selected: true,
                                                        summaries_selected: true,
                                                        top_lane_status: "COMPLETED",
                                                        embeddings_lane_status: "COMPLETED",
                                                        summaries_lane_status: "COMPLETED",
                                                        ..RuntimeSessionSnapshotFixture::default()
                                                    }
                                                };
                                                return Ok(runtime_snapshot_json(
                                                    repo_id.as_str(),
                                                    session_id,
                                                    fixture,
                                                ));
                                            }

                                            panic!("unexpected repo-scoped query: {query}");
                                        }
                                    },
                                    || {
                                        let mut out = Vec::new();
                                        let mut input = Cursor::new("");
                                        let runtime = test_runtime();
                                        runtime
                                            .block_on(run_with_io_async_for_project_root(
                                                InitArgs {
                                                    install_default_daemon: true,
                                                    force: false,
                                                    disable_bitloops_skill: false,
                                                    agent: Vec::new(),
                                                    telemetry: Some(false),
                                                    no_telemetry: false,
                                                    skip_baseline: false,
                                                    sync: Some(true),
                                                    ingest: Some(false),
                                                    backfill: None,
                                                    exclude: Vec::new(),
                                                    exclude_from: Vec::new(),
                                                    embeddings_runtime:
                                                        Some(crate::cli::embeddings::EmbeddingsRuntime::Local),
                                                    no_embeddings: false,
                                                    embeddings_gateway_url: None,
                                                    embeddings_api_key_env:
                                                        "BITLOOPS_PLATFORM_GATEWAY_TOKEN"
                                                            .to_string(),
                                                },
                                                repo.path(),
                                                &mut out,
                                                &mut input,
                                                None,
                                            ))
                                            .expect("run init");

                                        let rendered = String::from_utf8(out).expect("utf8 output");
                                        assert!(rendered.contains("Summaries"));
                                        assert!(rendered.contains("follow-up sync"));
                                    },
                                );
                            },
                        );
                    },
                );
            },
        );
    });

    assert!(
        *snapshot_count.borrow() >= 2,
        "expected the renderer to observe the summaries lane waiting before completion"
    );
}

#[test]
fn run_init_with_install_default_daemon_renders_separate_summaries_lane() {
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let repo = tempfile::tempdir().unwrap();
    let app_dirs = tempfile::tempdir().unwrap();
    let repo_id = test_repo_id(repo.path());
    let session_id = "init-session-summary-lane";
    setup_git_repo(&repo);

    with_summary_generation_configured_hook(
        |_| false,
        || {
            with_test_platform_dir_overrides(app_dir_overrides(&app_dirs), || {
                with_test_tty_override(false, || {
                    with_test_assume_daemon_running(true, || {
                        with_install_default_daemon_hook(
                            move |install_default_daemon| {
                                assert!(install_default_daemon);
                                let config_path = ensure_daemon_config_exists()
                                    .expect("create default daemon config");
                                write_runtime_only_daemon_config(
                                    &config_path,
                                    "bitloops-local-embeddings",
                                    &[],
                                );
                                Ok(())
                            },
                            || {
                                with_global_graphql_executor_hook(
                                    |_runtime_root, _query, variables| {
                                        assert_eq!(
                                            variables["telemetry"],
                                            serde_json::json!(false)
                                        );
                                        Ok(serde_json::json!({
                                            "updateCliTelemetryConsent": {
                                                "telemetry": false,
                                                "needsPrompt": false
                                            }
                                        }))
                                    },
                                    || {
                                        with_ollama_probe_hook(
                                            || Ok(OllamaAvailability::MissingCli),
                                            || {
                                                with_ingest_daemon_bootstrap_hook(
                                                    |_repo_root| Ok(()),
                                                    || {
                                                        with_graphql_executor_hook(
                                                            {
                                                                let events = Arc::clone(&events);
                                                                let repo_id = repo_id.clone();
                                                                move |_repo_root, query, variables| {
                                                                    if query.contains("startInit(") {
                                                                        events
                                                                            .lock()
                                                                            .expect("lock events")
                                                                            .push("start_init".to_string());
                                                                        assert_eq!(variables["repoId"], repo_id);
                                                                        assert_eq!(variables["input"]["runSync"], json!(true));
                                                                        assert_eq!(variables["input"]["runIngest"], json!(false));
                                                                        assert_eq!(
                                                                            variables["input"]["summariesBootstrap"]["action"],
                                                                            json!("INSTALL_RUNTIME_ONLY")
                                                                        );
                                                                        return Ok(runtime_start_init_result_json(session_id));
                                                                    }

                                                                    if query.contains("runtimeSnapshot(") {
                                                                        events
                                                                            .lock()
                                                                            .expect("lock events")
                                                                            .push("snapshot".to_string());
                                                                        return Ok(runtime_snapshot_json(
                                                                            repo_id.as_str(),
                                                                            session_id,
                                                                            RuntimeSessionSnapshotFixture {
                                                                                status: "COMPLETED",
                                                                                run_sync: true,
                                                                                embeddings_selected: true,
                                                                                summaries_selected: true,
                                                                                top_lane_status: "COMPLETED",
                                                                                embeddings_lane_status: "COMPLETED",
                                                                                summaries_lane_status: "COMPLETED",
                                                                                ..RuntimeSessionSnapshotFixture::default()
                                                                            },
                                                                        ));
                                                                    }

                                                                    panic!("unexpected repo-scoped query: {query}");
                                                                }
                                                            },
                                                            || {
                                                                let mut out = Vec::new();
                                                                let mut input = Cursor::new("");
                                                                let runtime = test_runtime();
                                                                runtime
                                                                    .block_on(run_with_io_async_for_project_root(
                                                                        InitArgs {
                                                                            install_default_daemon: true,
                                                                            force: false,
                                                                            disable_bitloops_skill: false,
                                                                            agent: Vec::new(),
                                                                            telemetry: Some(false),
                                                                            no_telemetry: false,
                                                                            skip_baseline: false,
                                                                            sync: Some(true),
                                                                            ingest: Some(false),
                                                                            backfill: None,
                                                                            exclude: Vec::new(),
                                                                            exclude_from: Vec::new(),
                                                                            embeddings_runtime: Some(crate::cli::embeddings::EmbeddingsRuntime::Local),
                                                                            no_embeddings: false,
                                                                            embeddings_gateway_url: None,
                                                                            embeddings_api_key_env: "BITLOOPS_PLATFORM_GATEWAY_TOKEN".to_string(),
                                                                        },
                                                                        repo.path(),
                                                                        &mut out,
                                                                        &mut input,
                                                                        None,
                                                                    ))
                                                                    .expect("run init");

                                                                let rendered =
                                                                    String::from_utf8(out)
                                                                        .expect("utf8 output");
                                                                assert!(rendered.contains(
                                                                    "This may take a few minutes depending on your codebase size."
                                                                ));
                                                                assert!(
                                                                    rendered.contains("Summaries")
                                                                );
                                                                assert!(!rendered.contains(
                                                                    "Starting initial DevQL sync..."
                                                                ));
                                                            },
                                                        )
                                                    },
                                                );
                                            },
                                        );
                                    },
                                );
                            },
                        );
                    })
                })
            })
        },
    );

    let events = events.lock().expect("lock events");
    assert_eq!(
        &*events,
        &["start_init".to_string(), "snapshot".to_string()]
    );
}

#[test]
fn run_init_with_explicit_telemetry_choice_persists_without_prompt() {
    let repo = tempfile::tempdir().unwrap();
    let app_dirs = tempfile::tempdir().unwrap();
    setup_git_repo(&repo);

    with_temp_app_dirs(&app_dirs, false, true, || {
        ensure_daemon_config_exists().expect("create default daemon config");

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
                let mut out = Vec::new();
                let mut input = Cursor::new("");
                let runtime = test_runtime();
                runtime
                    .block_on(run_with_io_async_for_project_root(
                        InitArgs {
                            install_default_daemon: false,
                            force: false,
                            disable_bitloops_skill: false,
                            agent: Vec::new(),
                            telemetry: Some(false),
                            no_telemetry: false,
                            skip_baseline: false,
                            sync: Some(false),
                            ingest: Some(false),
                            backfill: None,
                            exclude: Vec::new(),
                            exclude_from: Vec::new(),
                            embeddings_runtime: Some(
                                crate::cli::embeddings::EmbeddingsRuntime::Local,
                            ),
                            no_embeddings: false,
                            embeddings_gateway_url: None,
                            embeddings_api_key_env: "BITLOOPS_PLATFORM_GATEWAY_TOKEN".to_string(),
                        },
                        repo.path(),
                        &mut out,
                        &mut input,
                        None,
                    ))
                    .expect("run init");

                let rendered = String::from_utf8(out).expect("utf8 output");
                assert!(!rendered.contains("Help us improve Bitloops"));
            },
        );
    });
}

#[test]
fn choose_final_setup_options_renders_final_setup_prompt() {
    with_test_tty_override(true, || {
        let mut out = Vec::new();
        let mut input = Cursor::new("\n");

        let selection = choose_final_setup_options(
            None,
            &mut out,
            &mut input,
            None,
            InitFinalSetupPromptOptions {
                show_telemetry: false,
                show_auto_start_daemon: false,
            },
        )
        .expect("render prompt");

        assert_eq!(
            selection,
            InitFinalSetupSelection {
                sync: true,
                ingest: true,
                telemetry: false,
                auto_start_daemon: false,
            }
        );
        let rendered = String::from_utf8(out).expect("utf8 output");
        assert!(rendered.contains("\nFinal setup\n"));
        assert!(rendered.contains("And we made it to the last setup options!:"));
        assert!(rendered.contains("Use space to select, enter to confirm."));
        assert!(rendered.contains("1. Sync codebase (selected)"));
        assert!(rendered.contains("2. Import commit history (selected)"));
    });
}

#[test]
fn choose_final_setup_options_preselects_telemetry_when_shown() {
    with_test_tty_override(true, || {
        let mut out = Vec::new();
        let mut input = Cursor::new("\n");

        let selection = choose_final_setup_options(
            Some(false),
            &mut out,
            &mut input,
            Some(false),
            InitFinalSetupPromptOptions {
                show_telemetry: true,
                show_auto_start_daemon: false,
            },
        )
        .expect("render telemetry prompt");

        assert_eq!(
            selection,
            InitFinalSetupSelection {
                sync: false,
                ingest: false,
                telemetry: true,
                auto_start_daemon: false,
            }
        );
        let rendered = String::from_utf8(out).expect("utf8 output");
        assert!(rendered.contains("Enable anonymous telemetry (selected)"));
    });
}

#[test]
fn choose_final_setup_options_defaults_auto_start_to_disabled_when_not_interactive() {
    with_test_tty_override(false, || {
        let mut out = Vec::new();
        let mut input = Cursor::new("");

        let selection = choose_final_setup_options(
            Some(false),
            &mut out,
            &mut input,
            Some(false),
            InitFinalSetupPromptOptions {
                show_telemetry: false,
                show_auto_start_daemon: true,
            },
        )
        .expect("default auto-start selection");

        assert_eq!(
            selection,
            InitFinalSetupSelection {
                sync: false,
                ingest: false,
                telemetry: false,
                auto_start_daemon: false,
            }
        );
        assert!(
            out.is_empty(),
            "non-interactive auto-start should not prompt"
        );
    });
}

#[test]
fn run_init_with_install_default_daemon_enables_auto_start_when_confirmed() {
    let repo = tempfile::tempdir().unwrap();
    let app_dirs = tempfile::tempdir().unwrap();
    let service_enabled = std::rc::Rc::new(std::cell::RefCell::new(false));
    setup_git_repo(&repo);

    with_temp_app_dirs(&app_dirs, true, true, || {
        with_install_default_daemon_hook(
            move |install_default_daemon| {
                assert!(install_default_daemon);
                let config_path =
                    ensure_daemon_config_exists().expect("create default daemon config");
                write_runtime_only_daemon_config(&config_path, "bitloops-local-embeddings", &[]);
                Ok(())
            },
            || {
                with_enable_default_daemon_service_hook(
                    {
                        let service_enabled = std::rc::Rc::clone(&service_enabled);
                        move |enable_default_daemon_service| {
                            assert!(enable_default_daemon_service);
                            *service_enabled.borrow_mut() = true;
                            Ok(())
                        }
                    },
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
                                let mut out = Vec::new();
                                let mut input = Cursor::new("\n");
                                let select = |_items: &[String], enable_bitloops_skill: bool| {
                                    Ok(InitAgentSelection {
                                        agents: vec!["claude-code".to_string()],
                                        enable_bitloops_skill,
                                    })
                                };
                                let runtime = test_runtime();
                                runtime
                                    .block_on(run_with_io_async_for_project_root(
                                        InitArgs {
                                            install_default_daemon: true,
                                            force: false,
                                            disable_bitloops_skill: false,
                                            agent: Vec::new(),
                                            telemetry: Some(false),
                                            no_telemetry: false,
                                            skip_baseline: false,
                                            sync: Some(false),
                                            ingest: Some(false),
                                            backfill: None,
                                            exclude: Vec::new(),
                                            exclude_from: Vec::new(),
                                            embeddings_runtime: None,
                                            no_embeddings: true,
                                            embeddings_gateway_url: None,
                                            embeddings_api_key_env:
                                                "BITLOOPS_PLATFORM_GATEWAY_TOKEN".to_string(),
                                        },
                                        repo.path(),
                                        &mut out,
                                        &mut input,
                                        Some(&select),
                                    ))
                                    .expect("run init");

                                let rendered = String::from_utf8(out).expect("utf8 output");
                                assert!(rendered.contains(
                                    "Start Bitloops daemon automatically when you sign in"
                                ));
                            },
                        )
                    },
                )
            },
        );
    });

    assert!(
        *service_enabled.borrow(),
        "expected init to enable the always-on daemon service"
    );
}

#[test]
fn run_init_with_install_default_daemon_can_skip_auto_start() {
    let repo = tempfile::tempdir().unwrap();
    let app_dirs = tempfile::tempdir().unwrap();
    let service_enabled = std::rc::Rc::new(std::cell::RefCell::new(false));
    setup_git_repo(&repo);

    with_temp_app_dirs(&app_dirs, true, true, || {
        with_install_default_daemon_hook(
            move |install_default_daemon| {
                assert!(install_default_daemon);
                let config_path =
                    ensure_daemon_config_exists().expect("create default daemon config");
                write_runtime_only_daemon_config(&config_path, "bitloops-local-embeddings", &[]);
                Ok(())
            },
            || {
                with_enable_default_daemon_service_hook(
                    {
                        let service_enabled = std::rc::Rc::clone(&service_enabled);
                        move |_enable_default_daemon_service| {
                            *service_enabled.borrow_mut() = true;
                            Ok(())
                        }
                    },
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
                                let mut out = Vec::new();
                                let mut input = Cursor::new("none\n");
                                let select = |_items: &[String], enable_bitloops_skill: bool| {
                                    Ok(InitAgentSelection {
                                        agents: vec!["claude-code".to_string()],
                                        enable_bitloops_skill,
                                    })
                                };
                                let runtime = test_runtime();
                                runtime
                                    .block_on(run_with_io_async_for_project_root(
                                        InitArgs {
                                            install_default_daemon: true,
                                            force: false,
                                            disable_bitloops_skill: false,
                                            agent: Vec::new(),
                                            telemetry: Some(false),
                                            no_telemetry: false,
                                            skip_baseline: false,
                                            sync: Some(false),
                                            ingest: Some(false),
                                            backfill: None,
                                            exclude: Vec::new(),
                                            exclude_from: Vec::new(),
                                            embeddings_runtime: None,
                                            no_embeddings: true,
                                            embeddings_gateway_url: None,
                                            embeddings_api_key_env:
                                                "BITLOOPS_PLATFORM_GATEWAY_TOKEN".to_string(),
                                        },
                                        repo.path(),
                                        &mut out,
                                        &mut input,
                                        Some(&select),
                                    ))
                                    .expect("run init");
                            },
                        )
                    },
                )
            },
        );
    });

    assert!(
        !*service_enabled.borrow(),
        "expected init to leave the daemon in detached mode when auto-start is skipped"
    );
}

#[test]
fn run_init_noninteractive_requires_explicit_sync_and_ingest_choices() {
    let repo = tempfile::tempdir().unwrap();
    let app_dirs = tempfile::tempdir().unwrap();
    setup_git_repo(&repo);

    with_temp_app_dirs(&app_dirs, false, true, || {
        let mut out = Vec::new();
        let mut input = Cursor::new("");
        let runtime = test_runtime();
        let err = runtime
            .block_on(run_with_io_async_for_project_root(
                InitArgs {
                    install_default_daemon: false,
                    force: false,
                    disable_bitloops_skill: false,
                    agent: Vec::new(),
                    telemetry: Some(false),
                    no_telemetry: false,
                    skip_baseline: false,
                    sync: None,
                    ingest: Some(false),
                    backfill: None,
                    exclude: Vec::new(),
                    exclude_from: Vec::new(),
                    embeddings_runtime: Some(crate::cli::embeddings::EmbeddingsRuntime::Local),
                    no_embeddings: false,
                    embeddings_gateway_url: None,
                    embeddings_api_key_env: "BITLOOPS_PLATFORM_GATEWAY_TOKEN".to_string(),
                },
                repo.path(),
                &mut out,
                &mut input,
                None,
            ))
            .expect_err("init should require explicit init actions");

        assert_eq!(
            err.to_string(),
            "`bitloops init` requires explicit `--sync=true|false` and `--ingest=true|false` choices when not running interactively."
        );
    });
}

#[test]
fn run_init_triggers_repo_scoped_ingest_when_enabled() {
    let saw_start_init = std::rc::Rc::new(std::cell::RefCell::new(false));
    let repo = tempfile::tempdir().unwrap();
    let repo_root = repo.path().to_path_buf();
    let app_dirs = tempfile::tempdir().unwrap();
    let repo_id = test_repo_id(repo.path());
    let session_id = "init-session-ingest";
    setup_git_repo(&repo);

    with_temp_app_dirs(&app_dirs, false, false, || {
        ensure_daemon_config_exists().expect("create default daemon config");
        write_current_daemon_runtime_state(repo.path());

        with_global_graphql_executor_hook(
            |_runtime_root, query, variables| {
                assert!(query.contains("updateCliTelemetryConsent"));
                assert_eq!(variables["telemetry"], serde_json::json!(false));
                Ok(serde_json::json!({
                    "updateCliTelemetryConsent": {
                        "telemetry": false,
                        "needsPrompt": false
                    }
                }))
            },
            || {
                with_ingest_daemon_bootstrap_hook(
                    |_repo_root| Ok(()),
                    || {
                        with_graphql_executor_hook(
                            {
                                let saw_start_init = std::rc::Rc::clone(&saw_start_init);
                                let repo_root = repo_root.clone();
                                let repo_id = repo_id.clone();
                                move |actual_repo_root: &std::path::Path,
                                  query: &str,
                                  variables: &serde_json::Value| {
                                let expected_repo_root =
                                    repo_root.canonicalize().unwrap_or_else(|_| repo_root.clone());
                                let actual_repo_root = actual_repo_root
                                    .canonicalize()
                                    .unwrap_or_else(|_| actual_repo_root.to_path_buf());
                                assert_eq!(actual_repo_root, expected_repo_root);

                                if query.contains("startInit(") {
                                    *saw_start_init.borrow_mut() = true;
                                    assert_eq!(variables["repoId"], repo_id);
                                    assert_eq!(variables["input"]["runSync"], json!(false));
                                    assert_eq!(variables["input"]["runIngest"], json!(true));
                                    assert_eq!(variables["input"]["ingestBackfill"], json!(50));
                                    assert_eq!(
                                        variables["input"]["embeddingsBootstrap"],
                                        serde_json::Value::Null
                                    );
                                    return Ok(runtime_start_init_result_json(session_id));
                                }

                                if query.contains("runtimeSnapshot(") {
                                    return Ok(runtime_snapshot_json(
                                        repo_id.as_str(),
                                        session_id,
                                        RuntimeSessionSnapshotFixture {
                                            status: "COMPLETED",
                                            run_ingest: true,
                                            top_lane_status: "COMPLETED",
                                            ..RuntimeSessionSnapshotFixture::default()
                                        },
                                    ));
                                }

                                panic!("unexpected repo-scoped query: {query}");
                            }
                            },
                            || {
                                let mut out = Vec::new();
                                let mut input = Cursor::new("");
                                let runtime = test_runtime();
                                runtime
                                    .block_on(run_with_io_async_for_project_root(
                                        InitArgs {
                                            install_default_daemon: false,
                                            force: false,
                                            disable_bitloops_skill: false,
                                            agent: Vec::new(),
                                            telemetry: Some(false),
                                            no_telemetry: false,
                                            skip_baseline: false,
                                            sync: Some(false),
                                            ingest: Some(true),
                                            backfill: None,
                                            exclude: Vec::new(),
                                            exclude_from: Vec::new(),
                                            embeddings_runtime: Some(
                                                crate::cli::embeddings::EmbeddingsRuntime::Local,
                                            ),
                                            no_embeddings: false,
                                            embeddings_gateway_url: None,
                                            embeddings_api_key_env:
                                                "BITLOOPS_PLATFORM_GATEWAY_TOKEN".to_string(),
                                        },
                                        repo.path(),
                                        &mut out,
                                        &mut input,
                                        None,
                                    ))
                                    .expect("run init");

                                let rendered = String::from_utf8(out).expect("utf8 output");
                                assert!(rendered.contains(
                                    "This may take a few minutes depending on your codebase size."
                                ));
                            },
                        )
                    },
                );
                assert!(
                    *saw_start_init.borrow(),
                    "init should invoke runtime startInit for repo-scoped ingest"
                );
            },
        );
    });
}

#[test]
fn run_init_uses_explicit_backfill_for_repo_scoped_ingest() {
    let saw_start_init = std::rc::Rc::new(std::cell::RefCell::new(false));
    let repo = tempfile::tempdir().unwrap();
    let repo_root = repo.path().to_path_buf();
    let app_dirs = tempfile::tempdir().unwrap();
    let repo_id = test_repo_id(repo.path());
    let session_id = "init-session-ingest-backfill";
    setup_git_repo(&repo);

    with_temp_app_dirs(&app_dirs, false, false, || {
        ensure_daemon_config_exists().expect("create default daemon config");
        write_current_daemon_runtime_state(repo.path());

        with_global_graphql_executor_hook(
            |_runtime_root, query, variables| {
                assert!(query.contains("updateCliTelemetryConsent"));
                assert_eq!(variables["telemetry"], serde_json::json!(false));
                Ok(serde_json::json!({
                    "updateCliTelemetryConsent": {
                        "telemetry": false,
                        "needsPrompt": false
                    }
                }))
            },
            || {
                with_ingest_daemon_bootstrap_hook(
                    |_repo_root| Ok(()),
                    || {
                        with_graphql_executor_hook(
                            {
                                let saw_start_init = std::rc::Rc::clone(&saw_start_init);
                                let repo_root = repo_root.clone();
                                let repo_id = repo_id.clone();
                                move |actual_repo_root: &std::path::Path,
                                          query: &str,
                                          variables: &serde_json::Value| {
                                        let expected_repo_root = repo_root
                                            .canonicalize()
                                            .unwrap_or_else(|_| repo_root.clone());
                                        let actual_repo_root = actual_repo_root
                                            .canonicalize()
                                            .unwrap_or_else(|_| actual_repo_root.to_path_buf());
                                        assert_eq!(actual_repo_root, expected_repo_root);

                                        if query.contains("startInit(") {
                                            *saw_start_init.borrow_mut() = true;
                                            assert_eq!(variables["repoId"], repo_id);
                                            assert_eq!(variables["input"]["runSync"], json!(false));
                                            assert_eq!(variables["input"]["runIngest"], json!(true));
                                            assert_eq!(
                                                variables["input"]["ingestBackfill"],
                                                json!(10)
                                            );
                                            return Ok(runtime_start_init_result_json(session_id));
                                        }

                                        if query.contains("runtimeSnapshot(") {
                                            return Ok(runtime_snapshot_json(
                                                repo_id.as_str(),
                                                session_id,
                                                RuntimeSessionSnapshotFixture {
                                                    status: "COMPLETED",
                                                    run_ingest: true,
                                                    top_lane_status: "COMPLETED",
                                                    ..RuntimeSessionSnapshotFixture::default()
                                                },
                                            ));
                                        }

                                        panic!("unexpected repo-scoped query: {query}");
                                    }
                            },
                            || {
                                let mut out = Vec::new();
                                let mut input = Cursor::new("");
                                let runtime = test_runtime();
                                runtime
                                    .block_on(run_with_io_async_for_project_root(
                                        InitArgs {
                                            install_default_daemon: false,
                                            force: false,
                                            disable_bitloops_skill: false,
                                            agent: Vec::new(),
                                            telemetry: Some(false),
                                            no_telemetry: false,
                                            skip_baseline: false,
                                            sync: Some(false),
                                            ingest: None,
                                            backfill: Some(10),
                                            exclude: Vec::new(),
                                            exclude_from: Vec::new(),
                                            embeddings_runtime: Some(
                                                crate::cli::embeddings::EmbeddingsRuntime::Local,
                                            ),
                                            no_embeddings: false,
                                            embeddings_gateway_url: None,
                                            embeddings_api_key_env:
                                                "BITLOOPS_PLATFORM_GATEWAY_TOKEN".to_string(),
                                        },
                                        repo.path(),
                                        &mut out,
                                        &mut input,
                                        None,
                                    ))
                                    .expect("run init");

                                let rendered = String::from_utf8(out).expect("utf8 output");
                                assert!(rendered.contains(
                                    "This may take a few minutes depending on your codebase size."
                                ));
                            },
                        )
                    },
                );
                assert!(
                    *saw_start_init.borrow(),
                    "init should invoke runtime startInit for repo-scoped ingest"
                );
            },
        );
    });
}

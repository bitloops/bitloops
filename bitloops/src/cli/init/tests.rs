use super::*;
use crate::cli::{Cli, Commands};
use crate::test_support::git_fixtures::init_test_repo;
use crate::test_support::process_state::with_process_state;
use crate::utils::platform_dirs::{TestPlatformDirOverrides, with_test_platform_dir_overrides};

use clap::Parser;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

fn completed_lane_json() -> serde_json::Value {
    serde_json::json!({
        "status": "COMPLETED",
        "waitingReason": serde_json::Value::Null,
        "detail": serde_json::Value::Null,
        "activityLabel": serde_json::Value::Null,
        "taskId": serde_json::Value::Null,
        "runId": serde_json::Value::Null,
        "pendingCount": 0,
        "runningCount": 0,
        "failedCount": 0,
        "completedCount": 1
    })
}

fn completed_runtime_snapshot_json(repo_id: &str, init_session_id: &str) -> serde_json::Value {
    serde_json::json!({
        "runtimeSnapshot": {
            "repoId": repo_id,
            "taskQueue": {
                "persisted": true,
                "queuedTasks": 0,
                "runningTasks": 0,
                "failedTasks": 0,
                "completedRecentTasks": 1,
                "byKind": [],
                "paused": false,
                "pausedReason": serde_json::Value::Null,
                "lastAction": "completed",
                "lastUpdatedUnix": 10,
                "currentRepoTasks": []
            },
            "currentStateConsumer": {
                "persisted": true,
                "pendingRuns": 0,
                "runningRuns": 0,
                "failedRuns": 0,
                "completedRecentRuns": 0,
                "lastAction": "idle",
                "lastUpdatedUnix": 10,
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
                "initSessionId": init_session_id,
                "status": "COMPLETED",
                "waitingReason": serde_json::Value::Null,
                "warningSummary": serde_json::Value::Null,
                "followUpSyncRequired": false,
                "runSync": true,
                "runIngest": true,
                "embeddingsSelected": true,
                "summariesSelected": true,
                "summaryEmbeddingsSelected": true,
                "initialSyncTaskId": serde_json::Value::Null,
                "ingestTaskId": serde_json::Value::Null,
                "followUpSyncTaskId": serde_json::Value::Null,
                "embeddingsBootstrapTaskId": serde_json::Value::Null,
                "summaryBootstrapTaskId": serde_json::Value::Null,
                "terminalError": serde_json::Value::Null,
                "syncLane": completed_lane_json(),
                "ingestLane": completed_lane_json(),
                "codeEmbeddingsLane": completed_lane_json(),
                "summariesLane": completed_lane_json(),
                "summaryEmbeddingsLane": completed_lane_json()
            }
        }
    })
}

fn app_dir_overrides(temp: &TempDir) -> TestPlatformDirOverrides {
    TestPlatformDirOverrides {
        config_root: Some(temp.path().join("config-root")),
        data_root: Some(temp.path().join("data-root")),
        cache_root: Some(temp.path().join("cache-root")),
        state_root: Some(temp.path().join("state-root")),
    }
}

fn fake_login_session() -> crate::daemon::WorkosSessionDetails {
    crate::daemon::WorkosSessionDetails {
        client_id: "client_test".to_string(),
        user_id: Some("user_123".to_string()),
        user_email: Some("cli@example.com".to_string()),
        user_first_name: Some("CLI".to_string()),
        user_last_name: Some("User".to_string()),
        organisation_id: Some("org_123".to_string()),
        authentication_method: Some("Test".to_string()),
        access_token_expires_at_unix: None,
        authenticated_at_unix: 0,
        updated_at_unix: 0,
    }
}

fn setup_git_repo(dir: &TempDir) {
    init_test_repo(dir.path(), "main", "Bitloops Test", "bitloops@example.com");
}

fn init_args() -> InitArgs {
    InitArgs {
        command: None,
        force: false,
        disable_devql_guidance: false,
        agent: vec!["claude-code".to_string()],
        sync: Some(false),
        ingest: Some(false),
        backfill: None,
        exclude: Vec::new(),
        exclude_from: Vec::new(),
        embeddings_runtime: None,
        summaries_runtime: None,
        summary_embeddings_mode: None,
    }
}

fn write_current_daemon_runtime_state(config_path: &Path) {
    let runtime_path = crate::daemon::runtime_state_path(Path::new("."));
    if let Some(parent) = runtime_path.parent() {
        std::fs::create_dir_all(parent).expect("create runtime parent");
    }
    let config_root = config_path
        .parent()
        .expect("config path should have a parent")
        .to_path_buf();
    let runtime_state = crate::daemon::DaemonRuntimeState {
        version: 1,
        config_path: config_path.to_path_buf(),
        config_root: config_root.clone(),
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

fn managed_inference_install_outcome(
    binary_path: PathBuf,
    freshly_installed: bool,
) -> crate::cli::inference::ManagedInferenceBinaryInstallOutcome {
    crate::cli::inference::ManagedInferenceBinaryInstallOutcome {
        version: "v1.2.3".to_string(),
        binary_path,
        freshly_installed,
    }
}

fn with_successful_managed_inference_install<T>(binary_path: PathBuf, f: impl FnOnce() -> T) -> T {
    crate::cli::inference::with_managed_inference_install_hook(
        move |_repo_root| Ok(managed_inference_install_outcome(binary_path.clone(), true)),
        f,
    )
}

fn set_default_daemon_bitloops_inference_command(config_path: &Path, command: &str) {
    let contents = std::fs::read_to_string(config_path).expect("read daemon config");
    let mut doc = contents
        .parse::<toml_edit::DocumentMut>()
        .expect("parse daemon config");
    doc["inference"]["runtimes"]["bitloops_inference"]["command"] = toml_edit::value(command);
    std::fs::write(config_path, doc.to_string()).expect("write daemon config");
}

#[test]
fn init_args_accept_repo_local_flags() {
    let parsed = Cli::try_parse_from([
        "bitloops",
        "init",
        "--agent",
        "codex",
        "--disable-devql-guidance",
        "--sync=false",
        "--ingest=true",
        "--backfill=12",
        "--exclude",
        "target/**",
        "--exclude-from",
        ".gitignore",
        "--force",
    ])
    .expect("repo init flags should parse");
    let Some(Commands::Init(args)) = parsed.command else {
        panic!("expected init command");
    };

    assert_eq!(args.agent, vec!["codex"]);
    assert!(args.disable_devql_guidance);
    assert_eq!(args.sync, Some(false));
    assert_eq!(args.ingest, Some(true));
    assert_eq!(args.backfill, Some(12));
    assert_eq!(args.exclude, vec!["target/**"]);
    assert_eq!(args.exclude_from, vec![".gitignore"]);
    assert!(args.force);
}

#[test]
fn init_args_bare_backfill_defaults_to_twenty_five_commits() {
    let parsed = Cli::try_parse_from([
        "bitloops",
        "init",
        "--agent",
        "codex",
        "--sync=false",
        "--ingest=true",
        "--backfill",
    ])
    .expect("bare init backfill flag should parse");
    let Some(Commands::Init(args)) = parsed.command else {
        panic!("expected init command");
    };

    assert_eq!(DEFAULT_INIT_INGEST_BACKFILL, 25);
    assert_eq!(args.backfill, Some(25));
}

#[test]
fn init_args_accept_embeddings_runtime_flag() {
    let parsed = Cli::try_parse_from([
        "bitloops",
        "init",
        "--agent",
        "codex",
        "--sync=false",
        "--ingest=false",
        "--embeddings-runtime",
        "platform",
    ])
    .expect("init embeddings runtime flag should parse");
    let Some(Commands::Init(args)) = parsed.command else {
        panic!("expected init command");
    };

    assert_eq!(
        args.embeddings_runtime,
        Some(crate::cli::embeddings::EmbeddingsRuntime::Platform)
    );
}

#[test]
fn init_args_accept_summaries_runtime_flag() {
    let parsed = Cli::try_parse_from([
        "bitloops",
        "init",
        "--agent",
        "codex",
        "--sync=false",
        "--ingest=false",
        "--summaries-runtime",
        "platform",
    ])
    .expect("init summaries runtime flag should parse");
    let Some(Commands::Init(args)) = parsed.command else {
        panic!("expected init command");
    };

    assert_eq!(
        args.summaries_runtime,
        Some(crate::cli::init::SummariesRuntime::Platform)
    );

    let parsed = Cli::try_parse_from([
        "bitloops",
        "init",
        "--sync=false",
        "--ingest=false",
        "--summaries-runtime",
        "local",
    ])
    .expect("init summaries runtime local should parse");
    let Some(Commands::Init(args)) = parsed.command else {
        panic!("expected init command");
    };
    assert_eq!(
        args.summaries_runtime,
        Some(crate::cli::init::SummariesRuntime::Local)
    );
}

#[test]
fn init_args_accept_summary_embeddings_mode_flag() {
    let parsed = Cli::try_parse_from([
        "bitloops",
        "init",
        "--sync=false",
        "--ingest=false",
        "--summary-embeddings-mode",
        "on",
    ])
    .expect("init summary embeddings mode on should parse");
    let Some(Commands::Init(args)) = parsed.command else {
        panic!("expected init command");
    };
    assert_eq!(
        args.summary_embeddings_mode,
        Some(crate::cli::init::SummaryEmbeddingsMode::On)
    );

    let parsed = Cli::try_parse_from([
        "bitloops",
        "init",
        "--sync=false",
        "--ingest=false",
        "--summary-embeddings-mode",
        "off",
    ])
    .expect("init summary embeddings mode off should parse");
    let Some(Commands::Init(args)) = parsed.command else {
        panic!("expected init command");
    };
    assert_eq!(
        args.summary_embeddings_mode,
        Some(crate::cli::init::SummaryEmbeddingsMode::Off)
    );
}

#[test]
fn init_summaries_runtime_is_ignored_when_summary_generation_is_already_configured() {
    let repo = TempDir::new().expect("repo");
    setup_git_repo(&repo);
    let local_policy_path = repo.path().join(crate::config::REPO_POLICY_LOCAL_FILE_NAME);
    crate::config::settings::set_repo_semantic_embedding_policy(
        &local_policy_path,
        &crate::config::RepoSemanticEmbeddingPolicy {
            present: true,
            summary_mode: Some(crate::config::SemanticSummaryMode::Auto),
            embedding_mode: Some(crate::config::SemanticCloneEmbeddingMode::SemanticAwareOnce),
            inference: crate::config::SemanticClonesInferenceBindings {
                summary_generation: Some("summary_llm".to_string()),
                code_embeddings: None,
                summary_embeddings: None,
            },
        },
    )
    .expect("seed summary generation policy");

    crate::cli::inference::with_summary_generation_configured_hook(
        |_| true,
        || {
            let mut out = Vec::new();
            let mut input = std::io::Cursor::new(Vec::<u8>::new());
            let selection = choose_summary_setup_during_init(
                repo.path(),
                true,
                Some(crate::cli::init::SummariesRuntime::Platform),
                &mut out,
                &mut input,
            )
            .expect("choose summary setup");

            assert_eq!(
                selection,
                crate::cli::inference::SummarySetupSelection::Skip
            );
        },
    );
}

#[test]
fn init_embeddings_runtime_platform_configures_semantic_policy_without_prompt() {
    let repo = TempDir::new().expect("repo");
    let app_dirs = TempDir::new().expect("app dirs");
    setup_git_repo(&repo);

    with_process_state(None, &[], || {
        with_test_platform_dir_overrides(app_dir_overrides(&app_dirs), || {
            crate::config::ensure_daemon_config_exists().expect("write default daemon config");
            let install_called = Arc::new(Mutex::new(false));
            let install_called_for_hook = Arc::clone(&install_called);
            let repo_root = repo.path().to_path_buf();

            crate::cli::embeddings::with_managed_platform_embeddings_install_hook(
                move || {
                    *install_called_for_hook.lock().expect("install called lock") = true;
                    Ok(
                        crate::cli::embeddings::ManagedPlatformEmbeddingsBinaryInstallOutcome {
                            version: "v1.2.3".to_string(),
                            binary_path: repo_root
                                .join(".bitloops/test-bin/bitloops-platform-embeddings"),
                            freshly_installed: true,
                        },
                    )
                },
                || {
                    crate::cli::login::with_ensure_logged_in_hook(
                        || Ok(fake_login_session()),
                        || {
                            let mut out = Vec::new();
                            let args = InitArgs {
                                embeddings_runtime: Some(
                                    crate::cli::embeddings::EmbeddingsRuntime::Platform,
                                ),
                                ..init_args()
                            };
                            run_with_writer_for_project_root(args, repo.path(), &mut out, None)
                                .expect("init should complete");

                            let rendered = String::from_utf8(out).expect("utf8 output");
                            assert!(
                                !rendered.contains("Select an option"),
                                "explicit embeddings runtime should not prompt for provider selection"
                            );
                        },
                    );
                },
            );

            assert!(
                *install_called.lock().expect("install called lock"),
                "platform embeddings install hook should be invoked"
            );
            let policy = std::fs::read_to_string(
                repo.path().join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
            )
            .expect("read repo policy");
            assert!(policy.contains("[semantic_clones]"));
            assert!(policy.contains("embedding_mode = \"semantic_aware_once\""));
            assert!(policy.contains("code_embeddings = \"platform_code\""));
        })
    });
}

#[test]
fn init_args_reject_daemon_and_inference_configuration_flags() {
    for flag in [
        "--install-default-daemon",
        "--telemetry",
        "--no-telemetry",
        "--no-embeddings",
        "--no-summaries",
        "--context-guidance-runtime",
        "--no-context-guidance",
        "--context-guidance-gateway-url",
        "--context-guidance-api-key-env",
        "--skip-baseline",
    ] {
        let mut args = vec!["bitloops", "init", flag];
        if matches!(flag, "--telemetry" | "--context-guidance-runtime") {
            args.push("platform");
        } else if matches!(
            flag,
            "--context-guidance-gateway-url" | "--context-guidance-api-key-env"
        ) {
            args.push("value");
        }
        let err = Cli::try_parse_from(args)
            .err()
            .unwrap_or_else(|| panic!("init should reject {flag}"));
        assert!(err.to_string().contains(flag));
    }
}

#[test]
fn init_writes_repo_policy_without_creating_daemon_config() {
    let repo = TempDir::new().expect("repo");
    let app_dirs = TempDir::new().expect("app dirs");
    setup_git_repo(&repo);

    with_process_state(None, &[], || {
        with_test_platform_dir_overrides(app_dir_overrides(&app_dirs), || {
            let default_config_path =
                crate::config::default_daemon_config_path().expect("default config path");
            assert!(!default_config_path.exists());

            let mut out = Vec::new();
            run_with_writer_for_project_root(init_args(), repo.path(), &mut out, None)
                .expect("init should complete");

            assert!(!default_config_path.exists());
            let policy = std::fs::read_to_string(
                repo.path().join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
            )
            .expect("read repo policy");
            assert!(policy.contains("[capture]"));
            assert!(policy.contains("strategy = \"manual-commit\""));
            assert!(policy.contains("supported = [\"claude-code\"]"));
            assert!(policy.contains("sync_enabled = false"));
            assert!(policy.contains("ingest_enabled = false"));
            assert!(!policy.contains("[daemon]"));

            let rendered = String::from_utf8(out).expect("utf8 output");
            assert!(!rendered.contains("telemetry"));
            assert!(!rendered.contains("embeddings"));
            assert!(!rendered.contains("Starting Bitloops daemon"));
        })
    });
}

#[test]
fn init_binds_existing_default_daemon_config_without_starting_it() {
    let repo = TempDir::new().expect("repo");
    let app_dirs = TempDir::new().expect("app dirs");
    setup_git_repo(&repo);

    with_process_state(None, &[], || {
        with_test_platform_dir_overrides(app_dir_overrides(&app_dirs), || {
            let config_path =
                crate::config::bootstrap_default_daemon_environment().expect("bootstrap config");

            let mut out = Vec::new();
            run_with_writer_for_project_root(init_args(), repo.path(), &mut out, None)
                .expect("init should complete");

            let policy = std::fs::read_to_string(
                repo.path().join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
            )
            .expect("read repo policy");
            let config_path = config_path.canonicalize().unwrap_or(config_path);
            assert!(policy.contains("[daemon]"));
            assert!(policy.contains(&format!(
                "config_path = \"{}\"",
                config_path.to_string_lossy()
            )));
        })
    });
}

#[test]
fn init_reconciles_repo_watcher_when_daemon_is_running() {
    let repo = TempDir::new().expect("repo");
    let app_dirs = TempDir::new().expect("app dirs");
    setup_git_repo(&repo);

    with_process_state(None, &[], || {
        with_test_platform_dir_overrides(app_dir_overrides(&app_dirs), || {
            let config_path =
                crate::config::bootstrap_default_daemon_environment().expect("bootstrap config");
            write_current_daemon_runtime_state(&config_path);
            let calls = Arc::new(Mutex::new(Vec::<(String, bool)>::new()));
            let calls_for_hook = Arc::clone(&calls);

            crate::cli::watcher_bootstrap::with_watcher_reconciliation_hook(
                move |repo_root, watcher_enabled| {
                    calls_for_hook
                        .lock()
                        .expect("calls lock")
                        .push((repo_root.display().to_string(), watcher_enabled));
                    Ok(())
                },
                || {
                    let mut out = Vec::new();
                    run_with_writer_for_project_root(init_args(), repo.path(), &mut out, None)
                        .expect("init should complete");
                },
            );

            let calls = calls.lock().expect("calls lock");
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].0, repo.path().display().to_string());
            assert!(!calls[0].1);
        })
    });
}

#[test]
fn init_runtime_start_installs_managed_inference_when_daemon_config_requires_it() {
    let repo = TempDir::new().expect("repo");
    let app_dirs = TempDir::new().expect("app dirs");
    setup_git_repo(&repo);

    with_process_state(None, &[], || {
        with_test_platform_dir_overrides(app_dir_overrides(&app_dirs), || {
            crate::config::ensure_daemon_config_exists().expect("write default daemon config");
            let install_called = Arc::new(Mutex::new(false));
            let install_called_for_hook = Arc::clone(&install_called);
            let start_called = Arc::new(Mutex::new(false));
            let start_called_for_hook = Arc::clone(&start_called);
            let binary_path = repo.path().join(".bitloops/test-bin/bitloops-inference");

            crate::cli::inference::with_managed_inference_install_hook(
                move |_repo_root| {
                    *install_called_for_hook.lock().expect("install called lock") = true;
                    Ok(managed_inference_install_outcome(binary_path.clone(), true))
                },
                || {
                    crate::cli::devql::graphql::with_graphql_executor_hook(
                        move |_, query, variables| {
                            if query.contains("mutation StartInit") {
                                *start_called_for_hook.lock().expect("start called lock") = true;
                                return Ok(serde_json::json!({
                                    "startInit": {
                                        "initSessionId": "init-managed-inference-test"
                                    }
                                }));
                            }
                            if query.contains("query RuntimeSnapshot") {
                                let repo_id = variables["repoId"].as_str().expect("repo id");
                                return Ok(completed_runtime_snapshot_json(
                                    repo_id,
                                    "init-managed-inference-test",
                                ));
                            }
                            panic!("unexpected GraphQL query: {query}");
                        },
                        || {
                            let mut out = Vec::new();
                            let args = InitArgs {
                                sync: Some(true),
                                ingest: Some(true),
                                ..init_args()
                            };
                            run_with_writer_for_project_root(args, repo.path(), &mut out, None)
                                .expect("init should complete");

                            let rendered = String::from_utf8(out).expect("utf8 output");
                            assert!(
                                rendered.contains(
                                    "Installed managed standalone `bitloops-inference` runtime"
                                ),
                                "init should print install lines only when installing:\n{rendered}"
                            );
                        },
                    );
                },
            );

            assert!(
                *install_called.lock().expect("install called lock"),
                "managed inference install hook should be invoked before runtime init"
            );
            assert!(
                *start_called.lock().expect("start called lock"),
                "runtime init should still start after a successful install"
            );
        })
    });
}

#[test]
fn init_runtime_start_fails_before_start_init_when_managed_inference_install_fails() {
    let repo = TempDir::new().expect("repo");
    let app_dirs = TempDir::new().expect("app dirs");
    setup_git_repo(&repo);

    with_process_state(None, &[], || {
        with_test_platform_dir_overrides(app_dir_overrides(&app_dirs), || {
            crate::config::ensure_daemon_config_exists().expect("write default daemon config");
            let start_called = Arc::new(Mutex::new(false));
            let start_called_for_hook = Arc::clone(&start_called);

            let result = crate::cli::inference::with_managed_inference_install_hook(
                move |_repo_root| Err(anyhow::anyhow!("download exploded")),
                || {
                    crate::cli::devql::graphql::with_graphql_executor_hook(
                        move |_, query, _| {
                            if query.contains("mutation StartInit") {
                                *start_called_for_hook.lock().expect("start called lock") = true;
                            }
                            panic!("StartInit should not be called after install failure");
                        },
                        || {
                            let mut out = Vec::new();
                            let args = InitArgs {
                                sync: Some(true),
                                ingest: Some(true),
                                ..init_args()
                            };
                            run_with_writer_for_project_root(args, repo.path(), &mut out, None)
                        },
                    )
                },
            );

            let err = result.expect_err("init should fail when managed inference install fails");
            let rendered = format!("{err:#}");
            assert!(
                rendered.contains(
                    "Bitloops init could not install the managed inference runtime required by daemon config"
                ),
                "unexpected error: {rendered}"
            );
            assert!(
                rendered.contains("download exploded"),
                "root cause should be preserved: {rendered}"
            );
            assert!(
                !*start_called.lock().expect("start called lock"),
                "StartInit must not be called when inference install fails"
            );
        })
    });
}

#[test]
fn init_config_only_run_does_not_install_managed_inference() {
    let repo = TempDir::new().expect("repo");
    let app_dirs = TempDir::new().expect("app dirs");
    setup_git_repo(&repo);

    with_process_state(None, &[], || {
        with_test_platform_dir_overrides(app_dir_overrides(&app_dirs), || {
            crate::config::ensure_daemon_config_exists().expect("write default daemon config");
            let install_called = Arc::new(Mutex::new(false));
            let install_called_for_hook = Arc::clone(&install_called);
            let binary_path = repo.path().join(".bitloops/test-bin/bitloops-inference");

            crate::cli::inference::with_managed_inference_install_hook(
                move |_repo_root| {
                    *install_called_for_hook.lock().expect("install called lock") = true;
                    Ok(managed_inference_install_outcome(binary_path.clone(), true))
                },
                || {
                    let mut out = Vec::new();
                    let args = InitArgs {
                        sync: Some(false),
                        ingest: Some(false),
                        ..init_args()
                    };
                    run_with_writer_for_project_root(args, repo.path(), &mut out, None)
                        .expect("init should complete");
                },
            );

            assert!(
                !*install_called.lock().expect("install called lock"),
                "config-only init should not install managed inference"
            );
        })
    });
}

#[test]
fn init_runtime_start_does_not_install_for_custom_managed_inference_command() {
    let repo = TempDir::new().expect("repo");
    let app_dirs = TempDir::new().expect("app dirs");
    setup_git_repo(&repo);

    with_process_state(None, &[], || {
        with_test_platform_dir_overrides(app_dir_overrides(&app_dirs), || {
            let config_path =
                crate::config::ensure_daemon_config_exists().expect("write default daemon config");
            set_default_daemon_bitloops_inference_command(
                &config_path,
                "/opt/custom/bitloops-inference",
            );
            let install_called = Arc::new(Mutex::new(false));
            let install_called_for_hook = Arc::clone(&install_called);
            let start_called = Arc::new(Mutex::new(false));
            let start_called_for_hook = Arc::clone(&start_called);
            let binary_path = repo.path().join(".bitloops/test-bin/bitloops-inference");

            crate::cli::inference::with_managed_inference_install_hook(
                move |_repo_root| {
                    *install_called_for_hook.lock().expect("install called lock") = true;
                    Ok(managed_inference_install_outcome(binary_path.clone(), true))
                },
                || {
                    crate::cli::devql::graphql::with_graphql_executor_hook(
                        move |_, query, variables| {
                            if query.contains("mutation StartInit") {
                                *start_called_for_hook.lock().expect("start called lock") = true;
                                return Ok(serde_json::json!({
                                    "startInit": {
                                        "initSessionId": "init-custom-inference-test"
                                    }
                                }));
                            }
                            if query.contains("query RuntimeSnapshot") {
                                let repo_id = variables["repoId"].as_str().expect("repo id");
                                return Ok(completed_runtime_snapshot_json(
                                    repo_id,
                                    "init-custom-inference-test",
                                ));
                            }
                            panic!("unexpected GraphQL query: {query}");
                        },
                        || {
                            let mut out = Vec::new();
                            let args = InitArgs {
                                sync: Some(true),
                                ingest: Some(true),
                                ..init_args()
                            };
                            run_with_writer_for_project_root(args, repo.path(), &mut out, None)
                                .expect("init should complete");
                        },
                    );
                },
            );

            assert!(
                !*install_called.lock().expect("install called lock"),
                "custom runtime commands should remain user-managed"
            );
            assert!(
                *start_called.lock().expect("start called lock"),
                "runtime init should still start for custom user-managed commands"
            );
        })
    });
}

#[test]
fn init_runtime_start_does_not_print_install_line_for_complete_managed_runtime() {
    let repo = TempDir::new().expect("repo");
    let app_dirs = TempDir::new().expect("app dirs");
    setup_git_repo(&repo);

    with_process_state(None, &[], || {
        with_test_platform_dir_overrides(app_dir_overrides(&app_dirs), || {
            let config_path =
                crate::config::ensure_daemon_config_exists().expect("write default daemon config");
            let binary_path =
                crate::cli::inference::managed_inference_binary_path().expect("binary path");
            set_default_daemon_bitloops_inference_command(
                &config_path,
                binary_path.to_string_lossy().as_ref(),
            );
            let install_called = Arc::new(Mutex::new(false));
            let install_called_for_hook = Arc::clone(&install_called);

            crate::cli::inference::with_managed_inference_install_hook(
                move |_repo_root| {
                    *install_called_for_hook.lock().expect("install called lock") = true;
                    Ok(managed_inference_install_outcome(
                        binary_path.clone(),
                        false,
                    ))
                },
                || {
                    crate::cli::devql::graphql::with_graphql_executor_hook(
                        move |_, query, variables| {
                            if query.contains("mutation StartInit") {
                                return Ok(serde_json::json!({
                                    "startInit": {
                                        "initSessionId": "init-complete-inference-test"
                                    }
                                }));
                            }
                            if query.contains("query RuntimeSnapshot") {
                                let repo_id = variables["repoId"].as_str().expect("repo id");
                                return Ok(completed_runtime_snapshot_json(
                                    repo_id,
                                    "init-complete-inference-test",
                                ));
                            }
                            panic!("unexpected GraphQL query: {query}");
                        },
                        || {
                            let mut out = Vec::new();
                            let args = InitArgs {
                                sync: Some(true),
                                ingest: Some(true),
                                ..init_args()
                            };
                            run_with_writer_for_project_root(args, repo.path(), &mut out, None)
                                .expect("init should complete");

                            let rendered = String::from_utf8(out).expect("utf8 output");
                            assert!(
                                !rendered.contains(
                                    "Installed managed standalone `bitloops-inference` runtime"
                                ),
                                "already-complete runtime should not print install line:\n{rendered}"
                            );
                            assert!(
                                !rendered.contains("Updated inference runtime command"),
                                "already-complete runtime should not print rewrite line:\n{rendered}"
                            );
                        },
                    );
                },
            );

            assert!(
                *install_called.lock().expect("install called lock"),
                "init should still verify a required managed runtime"
            );
        })
    });
}

#[test]
fn init_runtime_start_includes_semantic_lanes_and_repo_policy() {
    let repo = TempDir::new().expect("repo");
    let app_dirs = TempDir::new().expect("app dirs");
    setup_git_repo(&repo);

    with_process_state(None, &[], || {
        with_test_platform_dir_overrides(app_dir_overrides(&app_dirs), || {
            crate::config::ensure_daemon_config_exists().expect("write default daemon config");
            let captured_input = Arc::new(Mutex::new(None::<serde_json::Value>));
            let captured_input_for_hook = Arc::clone(&captured_input);
            let binary_path = repo.path().join(".bitloops/test-bin/bitloops-inference");

            with_successful_managed_inference_install(binary_path, || {
                crate::cli::devql::graphql::with_graphql_executor_hook(
                    move |_, query, variables| {
                        if query.contains("mutation StartInit") {
                            *captured_input_for_hook.lock().expect("captured input lock") =
                                Some(variables["input"].clone());
                            return Ok(serde_json::json!({
                                "startInit": {
                                    "initSessionId": "init-semantic-test"
                                }
                            }));
                        }
                        if query.contains("query RuntimeSnapshot") {
                            let repo_id = variables["repoId"].as_str().expect("repo id");
                            return Ok(completed_runtime_snapshot_json(
                                repo_id,
                                "init-semantic-test",
                            ));
                        }
                        panic!("unexpected GraphQL query: {query}");
                    },
                    || {
                        let mut out = Vec::new();
                        let args = InitArgs {
                            sync: Some(true),
                            ingest: Some(true),
                            ..init_args()
                        };
                        run_with_writer_for_project_root(args, repo.path(), &mut out, None)
                            .expect("init should complete");
                    },
                );
            });

            let input = captured_input
                .lock()
                .expect("captured input lock")
                .clone()
                .expect("start init input should be captured");
            assert_eq!(input["runSync"], serde_json::json!(true));
            assert_eq!(input["runIngest"], serde_json::json!(true));
            assert_eq!(input["runCodeEmbeddings"], serde_json::json!(true));
            assert_eq!(input["runSummaries"], serde_json::json!(true));
            assert_eq!(input["runSummaryEmbeddings"], serde_json::json!(true));

            let policy = std::fs::read_to_string(
                repo.path().join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
            )
            .expect("read repo policy");
            assert!(policy.contains("[semantic_clones]"));
            assert!(policy.contains("summary_mode = \"auto\""));
            assert!(policy.contains("embedding_mode = \"semantic_aware_once\""));
            assert!(policy.contains("summary_generation = \"summary_llm\""));
            assert!(policy.contains("code_embeddings = \"platform_code\""));
            assert!(policy.contains("summary_embeddings = \"platform_code\""));
        })
    });
}

#[test]
fn init_runtime_start_can_select_summary_embeddings_without_code_embeddings() {
    let repo = TempDir::new().expect("repo");
    let app_dirs = TempDir::new().expect("app dirs");
    setup_git_repo(&repo);

    with_process_state(None, &[], || {
        with_test_platform_dir_overrides(app_dir_overrides(&app_dirs), || {
            crate::config::ensure_daemon_config_exists().expect("write default daemon config");
            let local_policy_path = repo.path().join(crate::config::REPO_POLICY_LOCAL_FILE_NAME);
            crate::config::settings::set_repo_semantic_embedding_policy(
                &local_policy_path,
                &crate::config::RepoSemanticEmbeddingPolicy {
                    present: true,
                    summary_mode: Some(crate::config::SemanticSummaryMode::Auto),
                    embedding_mode: Some(
                        crate::config::SemanticCloneEmbeddingMode::SemanticAwareOnce,
                    ),
                    inference: crate::config::SemanticClonesInferenceBindings {
                        summary_generation: Some("summary_llm".to_string()),
                        code_embeddings: None,
                        summary_embeddings: Some("platform_code".to_string()),
                    },
                },
            )
            .expect("seed summary-only embeddings policy");

            let captured_input = Arc::new(Mutex::new(None::<serde_json::Value>));
            let captured_input_for_hook = Arc::clone(&captured_input);
            let binary_path = repo.path().join(".bitloops/test-bin/bitloops-inference");

            with_successful_managed_inference_install(binary_path, || {
                crate::cli::devql::graphql::with_graphql_executor_hook(
                    move |_, query, variables| {
                        if query.contains("mutation StartInit") {
                            *captured_input_for_hook.lock().expect("captured input lock") =
                                Some(variables["input"].clone());
                            return Ok(serde_json::json!({
                                "startInit": {
                                    "initSessionId": "init-summary-embeddings-test"
                                }
                            }));
                        }
                        if query.contains("query RuntimeSnapshot") {
                            let repo_id = variables["repoId"].as_str().expect("repo id");
                            return Ok(completed_runtime_snapshot_json(
                                repo_id,
                                "init-summary-embeddings-test",
                            ));
                        }
                        panic!("unexpected GraphQL query: {query}");
                    },
                    || {
                        let mut out = Vec::new();
                        let args = InitArgs {
                            sync: Some(true),
                            ingest: Some(false),
                            ..init_args()
                        };
                        run_with_writer_for_project_root(args, repo.path(), &mut out, None)
                            .expect("init should complete");
                    },
                );
            });

            let input = captured_input
                .lock()
                .expect("captured input lock")
                .clone()
                .expect("start init input should be captured");
            assert_eq!(input["runSync"], serde_json::json!(true));
            assert_eq!(input["runCodeEmbeddings"], serde_json::json!(false));
            assert_eq!(input["runSummaries"], serde_json::json!(true));
            assert_eq!(input["runSummaryEmbeddings"], serde_json::json!(true));
        })
    });
}

#[test]
fn init_prompts_for_embeddings_and_summary_provider_setup() {
    let repo = TempDir::new().expect("repo");
    let app_dirs = TempDir::new().expect("app dirs");
    setup_git_repo(&repo);

    with_process_state(None, &[], || {
        with_test_platform_dir_overrides(app_dir_overrides(&app_dirs), || {
            crate::cli::telemetry_consent::with_test_tty_override(true, || {
                let mut out = Vec::new();
                let mut input = std::io::Cursor::new(b"3\n1\n".to_vec());
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("runtime");
                let args = InitArgs {
                    sync: Some(false),
                    ingest: Some(false),
                    ..init_args()
                };

                runtime
                    .block_on(run_with_io_async_for_project_root(
                        args,
                        repo.path(),
                        &mut out,
                        &mut input,
                        None,
                    ))
                    .expect("init should complete");

                let rendered = String::from_utf8(out).expect("utf8 output");
                assert!(rendered.contains("Configure embeddings"));
                assert!(rendered.contains("Bitloops Cloud"));
                assert!(rendered.contains("Local embeddings"));
                assert!(rendered.contains("Skip for now"));
                assert!(rendered.contains("Configure semantic summaries"));
                assert!(rendered.contains("Local (Ollama)"));
            });
        })
    });
}

#[test]
fn init_prompts_for_provider_setup_when_existing_policy_profiles_are_unconfigured() {
    let repo = TempDir::new().expect("repo");
    let app_dirs = TempDir::new().expect("app dirs");
    setup_git_repo(&repo);

    with_process_state(None, &[], || {
        with_test_platform_dir_overrides(app_dir_overrides(&app_dirs), || {
            let local_policy_path = repo.path().join(crate::config::REPO_POLICY_LOCAL_FILE_NAME);
            crate::config::settings::set_repo_semantic_embedding_policy(
                &local_policy_path,
                &crate::config::RepoSemanticEmbeddingPolicy {
                    present: true,
                    summary_mode: Some(crate::config::SemanticSummaryMode::Auto),
                    embedding_mode: Some(
                        crate::config::SemanticCloneEmbeddingMode::SemanticAwareOnce,
                    ),
                    inference: crate::config::SemanticClonesInferenceBindings {
                        summary_generation: Some("summary_llm".to_string()),
                        code_embeddings: Some("platform_code".to_string()),
                        summary_embeddings: Some("platform_code".to_string()),
                    },
                },
            )
            .expect("seed semantic policy");

            crate::cli::telemetry_consent::with_test_tty_override(true, || {
                let mut out = Vec::new();
                let mut input = std::io::Cursor::new(b"3\n1\n".to_vec());
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("runtime");
                let args = InitArgs {
                    sync: Some(false),
                    ingest: Some(false),
                    ..init_args()
                };

                runtime
                    .block_on(run_with_io_async_for_project_root(
                        args,
                        repo.path(),
                        &mut out,
                        &mut input,
                        None,
                    ))
                    .expect("init should complete");

                let rendered = String::from_utf8(out).expect("utf8 output");
                assert!(rendered.contains("Configure embeddings"));
                assert!(rendered.contains("Configure semantic summaries"));
            });
        })
    });
}

#[test]
fn init_prompts_for_summaries_when_existing_policy_previously_skipped_them() {
    let repo = TempDir::new().expect("repo");
    let app_dirs = TempDir::new().expect("app dirs");
    setup_git_repo(&repo);

    with_process_state(None, &[], || {
        with_test_platform_dir_overrides(app_dir_overrides(&app_dirs), || {
            crate::config::ensure_daemon_config_exists().expect("write default daemon config");
            let local_policy_path = repo.path().join(crate::config::REPO_POLICY_LOCAL_FILE_NAME);
            crate::config::settings::set_repo_semantic_embedding_policy(
                &local_policy_path,
                &crate::config::RepoSemanticEmbeddingPolicy {
                    present: true,
                    summary_mode: Some(crate::config::SemanticSummaryMode::Off),
                    embedding_mode: Some(
                        crate::config::SemanticCloneEmbeddingMode::SemanticAwareOnce,
                    ),
                    inference: crate::config::SemanticClonesInferenceBindings {
                        summary_generation: None,
                        code_embeddings: Some("platform_code".to_string()),
                        summary_embeddings: None,
                    },
                },
            )
            .expect("seed skipped summaries policy");

            crate::cli::telemetry_consent::with_test_tty_override(true, || {
                let mut out = Vec::new();
                let mut input = std::io::Cursor::new(b"1\n".to_vec());
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("runtime");
                let args = InitArgs {
                    sync: Some(false),
                    ingest: Some(false),
                    ..init_args()
                };

                runtime
                    .block_on(run_with_io_async_for_project_root(
                        args,
                        repo.path(),
                        &mut out,
                        &mut input,
                        None,
                    ))
                    .expect("init should complete");

                let rendered = String::from_utf8(out).expect("utf8 output");
                assert!(rendered.contains("Configure semantic summaries"));
            });
        })
    });
}

#[test]
fn init_prompts_for_summary_embeddings_when_code_embeddings_are_skipped() {
    let repo = TempDir::new().expect("repo");
    let app_dirs = TempDir::new().expect("app dirs");
    setup_git_repo(&repo);

    with_process_state(None, &[], || {
        with_test_platform_dir_overrides(app_dir_overrides(&app_dirs), || {
            crate::config::ensure_daemon_config_exists().expect("write default daemon config");
            let local_policy_path = repo.path().join(crate::config::REPO_POLICY_LOCAL_FILE_NAME);
            crate::config::settings::set_repo_semantic_embedding_policy(
                &local_policy_path,
                &crate::config::RepoSemanticEmbeddingPolicy {
                    present: true,
                    summary_mode: Some(crate::config::SemanticSummaryMode::Auto),
                    embedding_mode: Some(crate::config::SemanticCloneEmbeddingMode::Off),
                    inference: crate::config::SemanticClonesInferenceBindings {
                        summary_generation: Some("summary_llm".to_string()),
                        code_embeddings: None,
                        summary_embeddings: None,
                    },
                },
            )
            .expect("seed summary-only policy");

            crate::cli::telemetry_consent::with_test_tty_override(true, || {
                let mut out = Vec::new();
                let mut input = std::io::Cursor::new(b"3\n3\n".to_vec());
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("runtime");
                let args = InitArgs {
                    sync: Some(false),
                    ingest: Some(false),
                    ..init_args()
                };

                runtime
                    .block_on(run_with_io_async_for_project_root(
                        args,
                        repo.path(),
                        &mut out,
                        &mut input,
                        None,
                    ))
                    .expect("init should complete");

                let rendered = String::from_utf8(out).expect("utf8 output");
                assert!(rendered.contains("Configure summary embeddings"));
                assert!(rendered.contains("Bitloops Cloud"));
                assert!(rendered.contains("Local embeddings"));
                assert!(rendered.contains("Skip for now"));
            });
        })
    });
}

#[test]
fn init_summary_embeddings_mode_off_unsets_summary_embeddings_without_prompt() {
    let repo = TempDir::new().expect("repo");
    let app_dirs = TempDir::new().expect("app dirs");
    setup_git_repo(&repo);

    with_process_state(None, &[], || {
        with_test_platform_dir_overrides(app_dir_overrides(&app_dirs), || {
            crate::config::ensure_daemon_config_exists().expect("write default daemon config");
            let local_policy_path = repo.path().join(crate::config::REPO_POLICY_LOCAL_FILE_NAME);
            crate::config::settings::set_repo_semantic_embedding_policy(
                &local_policy_path,
                &crate::config::RepoSemanticEmbeddingPolicy {
                    present: true,
                    summary_mode: Some(crate::config::SemanticSummaryMode::Auto),
                    embedding_mode: Some(
                        crate::config::SemanticCloneEmbeddingMode::SemanticAwareOnce,
                    ),
                    inference: crate::config::SemanticClonesInferenceBindings {
                        summary_generation: Some("summary_llm".to_string()),
                        code_embeddings: Some("platform_code".to_string()),
                        summary_embeddings: Some("platform_code".to_string()),
                    },
                },
            )
            .expect("seed semantic policy");

            crate::cli::inference::with_summary_generation_configured_hook(
                |_| true,
                || {
                    let mut out = Vec::new();
                    let args = InitArgs {
                        summary_embeddings_mode: Some(crate::cli::init::SummaryEmbeddingsMode::Off),
                        ..init_args()
                    };
                    run_with_writer_for_project_root(args, repo.path(), &mut out, None)
                        .expect("init should complete");

                    let rendered = String::from_utf8(out).expect("utf8 output");
                    assert!(
                        !rendered.contains("Configure summary embeddings"),
                        "summary embeddings mode off should bypass prompt"
                    );
                },
            );

            let policy = std::fs::read_to_string(
                repo.path().join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
            )
            .expect("read repo policy");
            assert!(!policy.contains("summary_embeddings ="));
            assert!(policy.contains("summary_generation = \"summary_llm\""));
        })
    });
}

#[test]
fn init_summary_embeddings_mode_on_uses_existing_embeddings_provider_without_prompt() {
    let repo = TempDir::new().expect("repo");
    let app_dirs = TempDir::new().expect("app dirs");
    setup_git_repo(&repo);

    with_process_state(None, &[], || {
        with_test_platform_dir_overrides(app_dir_overrides(&app_dirs), || {
            crate::config::ensure_daemon_config_exists().expect("write default daemon config");
            let local_policy_path = repo.path().join(crate::config::REPO_POLICY_LOCAL_FILE_NAME);
            crate::config::settings::set_repo_semantic_embedding_policy(
                &local_policy_path,
                &crate::config::RepoSemanticEmbeddingPolicy {
                    present: true,
                    summary_mode: Some(crate::config::SemanticSummaryMode::Auto),
                    embedding_mode: Some(
                        crate::config::SemanticCloneEmbeddingMode::SemanticAwareOnce,
                    ),
                    inference: crate::config::SemanticClonesInferenceBindings {
                        summary_generation: Some("summary_llm".to_string()),
                        code_embeddings: Some("platform_code".to_string()),
                        summary_embeddings: None,
                    },
                },
            )
            .expect("seed semantic policy");

            crate::cli::inference::with_summary_generation_configured_hook(
                |_| true,
                || {
                    let mut out = Vec::new();
                    let args = InitArgs {
                        summary_embeddings_mode: Some(crate::cli::init::SummaryEmbeddingsMode::On),
                        ..init_args()
                    };
                    run_with_writer_for_project_root(args, repo.path(), &mut out, None)
                        .expect("init should complete");

                    let rendered = String::from_utf8(out).expect("utf8 output");
                    assert!(
                        !rendered.contains("Configure summary embeddings"),
                        "summary embeddings mode on should not prompt when embeddings provider exists"
                    );
                },
            );

            let policy = std::fs::read_to_string(
                repo.path().join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
            )
            .expect("read repo policy");
            assert!(policy.contains("summary_embeddings = \"platform_code\""));
        })
    });
}

#[test]
fn init_summary_embeddings_mode_on_fails_without_provider_in_noninteractive_mode() {
    let repo = TempDir::new().expect("repo");
    setup_git_repo(&repo);
    let local_policy_path = repo.path().join(crate::config::REPO_POLICY_LOCAL_FILE_NAME);
    crate::config::settings::set_repo_semantic_embedding_policy(
        &local_policy_path,
        &crate::config::RepoSemanticEmbeddingPolicy {
            present: true,
            summary_mode: Some(crate::config::SemanticSummaryMode::Auto),
            embedding_mode: Some(crate::config::SemanticCloneEmbeddingMode::Off),
            inference: crate::config::SemanticClonesInferenceBindings {
                summary_generation: Some("summary_llm".to_string()),
                code_embeddings: None,
                summary_embeddings: None,
            },
        },
    )
    .expect("seed summary-only semantic policy");

    crate::cli::inference::with_summary_generation_configured_hook(
        |_| true,
        || {
            let mut out = Vec::new();
            let args = InitArgs {
                summary_embeddings_mode: Some(crate::cli::init::SummaryEmbeddingsMode::On),
                ..init_args()
            };
            let err = run_with_writer_for_project_root(args, repo.path(), &mut out, None)
                .expect_err("init should fail");
            assert!(
                err.to_string().contains("requires an embeddings provider"),
                "error should explain that summary embeddings mode on needs a provider"
            );
        },
    );
}

#[test]
fn init_summary_embeddings_mode_on_interactive_prompts_for_provider_selection() {
    let repo = TempDir::new().expect("repo");
    setup_git_repo(&repo);
    let local_policy_path = repo.path().join(crate::config::REPO_POLICY_LOCAL_FILE_NAME);
    crate::config::settings::set_repo_semantic_embedding_policy(
        &local_policy_path,
        &crate::config::RepoSemanticEmbeddingPolicy {
            present: true,
            summary_mode: Some(crate::config::SemanticSummaryMode::Auto),
            embedding_mode: Some(crate::config::SemanticCloneEmbeddingMode::Off),
            inference: crate::config::SemanticClonesInferenceBindings {
                summary_generation: Some("summary_llm".to_string()),
                code_embeddings: None,
                summary_embeddings: None,
            },
        },
    )
    .expect("seed summary-only semantic policy");

    crate::cli::inference::with_summary_generation_configured_hook(
        |_| true,
        || {
            crate::cli::telemetry_consent::with_test_tty_override(true, || {
                let mut out = Vec::new();
                let mut input = std::io::Cursor::new(b"3\n3\n".to_vec());
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("runtime");
                let args = InitArgs {
                    summary_embeddings_mode: Some(crate::cli::init::SummaryEmbeddingsMode::On),
                    ..init_args()
                };
                let err = runtime
                    .block_on(run_with_io_async_for_project_root(
                        args,
                        repo.path(),
                        &mut out,
                        &mut input,
                        None,
                    ))
                    .expect_err("init should fail when interactive provider selection is skipped");
                assert!(
                    err.to_string().contains("requires an embeddings provider")
                        || err
                            .to_string()
                            .contains("requires selecting an embeddings provider"),
                    "error should explain explicit on requires selecting a provider"
                );
                let rendered = String::from_utf8(out).expect("utf8 output");
                assert!(rendered.contains("Configure summary embeddings"));
            });
        },
    );
}

#[test]
fn init_summary_embeddings_mode_on_fails_when_summaries_are_not_enabled() {
    let repo = TempDir::new().expect("repo");
    setup_git_repo(&repo);
    let local_policy_path = repo.path().join(crate::config::REPO_POLICY_LOCAL_FILE_NAME);
    crate::config::settings::set_repo_semantic_embedding_policy(
        &local_policy_path,
        &crate::config::RepoSemanticEmbeddingPolicy {
            present: true,
            summary_mode: Some(crate::config::SemanticSummaryMode::Off),
            embedding_mode: Some(crate::config::SemanticCloneEmbeddingMode::SemanticAwareOnce),
            inference: crate::config::SemanticClonesInferenceBindings {
                summary_generation: None,
                code_embeddings: Some("platform_code".to_string()),
                summary_embeddings: None,
            },
        },
    )
    .expect("seed summaries-off semantic policy");

    let mut out = Vec::new();
    let args = InitArgs {
        summary_embeddings_mode: Some(crate::cli::init::SummaryEmbeddingsMode::On),
        ..init_args()
    };
    let err = run_with_writer_for_project_root(args, repo.path(), &mut out, None)
        .expect_err("init should fail");
    assert!(
        err.to_string()
            .contains("requires summaries to be enabled for this init run"),
        "error should explain summaries must be enabled"
    );
}

#[test]
fn init_status_command_args_remain_repo_scoped() {
    let args = InitArgs {
        command: Some(InitCommand::Status(InitStatusArgs {
            json: true,
            wait: false,
            watch: false,
            session_id: Some("init_123".to_string()),
        })),
        ..init_args()
    };

    let Some(InitCommand::Status(status)) = args.command else {
        panic!("expected status command");
    };
    assert!(status.json);
    assert_eq!(status.session_id.as_deref(), Some("init_123"));
}

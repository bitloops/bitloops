use super::*;
use crate::cli::{Cli, Commands};
use crate::test_support::git_fixtures::init_test_repo;
use crate::test_support::process_state::with_process_state;
use crate::utils::platform_dirs::{TestPlatformDirOverrides, with_test_platform_dir_overrides};

use clap::Parser;
use std::path::Path;
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
fn init_args_reject_daemon_and_inference_configuration_flags() {
    for flag in [
        "--install-default-daemon",
        "--telemetry",
        "--no-telemetry",
        "--embeddings-runtime",
        "--no-embeddings",
        "--no-summaries",
        "--context-guidance-runtime",
        "--no-context-guidance",
        "--context-guidance-gateway-url",
        "--context-guidance-api-key-env",
        "--skip-baseline",
    ] {
        let mut args = vec!["bitloops", "init", flag];
        if matches!(
            flag,
            "--telemetry" | "--embeddings-runtime" | "--context-guidance-runtime"
        ) {
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
fn init_runtime_start_includes_semantic_lanes_and_repo_policy() {
    let repo = TempDir::new().expect("repo");
    let app_dirs = TempDir::new().expect("app dirs");
    setup_git_repo(&repo);

    with_process_state(None, &[], || {
        with_test_platform_dir_overrides(app_dir_overrides(&app_dirs), || {
            crate::config::ensure_daemon_config_exists().expect("write default daemon config");
            let captured_input = Arc::new(Mutex::new(None::<serde_json::Value>));
            let captured_input_for_hook = Arc::clone(&captured_input);

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

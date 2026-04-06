use super::agent_hooks::{
    AGENT_CLAUDE_CODE, AGENT_CODEX, AGENT_CURSOR, AGENT_GEMINI, DEFAULT_AGENT,
};
use super::*;
use crate::cli::devql::graphql::{with_graphql_executor_hook, with_ingest_daemon_bootstrap_hook};
use crate::cli::telemetry_consent::{
    NON_INTERACTIVE_TELEMETRY_ERROR, prompt_telemetry_consent, with_global_graphql_executor_hook,
};
use crate::cli::{Cli, Commands};
use crate::config::ensure_daemon_config_exists;
use crate::test_support::process_state::with_process_state;

use clap::Parser;
use std::io::Cursor;
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

fn app_dir_env(temp: &TempDir) -> [(&'static str, Option<String>); 4] {
    [
        (
            "BITLOOPS_TEST_CONFIG_DIR_OVERRIDE",
            Some(temp.path().join("config-root").display().to_string()),
        ),
        (
            "BITLOOPS_TEST_DATA_DIR_OVERRIDE",
            Some(temp.path().join("data-root").display().to_string()),
        ),
        (
            "BITLOOPS_TEST_CACHE_DIR_OVERRIDE",
            Some(temp.path().join("cache-root").display().to_string()),
        ),
        (
            "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
            Some(temp.path().join("state-root").display().to_string()),
        ),
    ]
}

fn with_temp_app_dirs_and_env<T>(
    repo_root: &std::path::Path,
    temp: &TempDir,
    extra_env: &[(&str, Option<&str>)],
    f: impl FnOnce() -> T,
) -> T {
    let env_vars = app_dir_env(temp);
    let mut env_refs = env_vars
        .iter()
        .map(|(key, value)| (*key, value.as_deref()))
        .collect::<Vec<_>>();
    env_refs.extend_from_slice(extra_env);
    with_process_state(Some(repo_root), &env_refs, f)
}

#[test]
fn init_args_supports_agent_flag() {
    let parsed =
        Cli::try_parse_from(["bitloops", "init", "--agent", "cursor"]).expect("parse init");
    let Some(Commands::Init(args)) = parsed.command else {
        panic!("expected init command");
    };
    assert_eq!(args.agent.as_deref(), Some("cursor"));
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

    with_temp_app_dirs_and_env(
        repo.path(),
        &app_dirs,
        &[("BITLOOPS_TEST_ASSUME_DAEMON_RUNNING", Some("1"))],
        || {
            let mut out = Vec::new();
            run_with_writer(
                InitArgs {
                    install_default_daemon: false,
                    force: false,
                    agent: None,
                    telemetry: None,
                    no_telemetry: false,
                    skip_baseline: false,
                    sync: Some(false),
                    ingest: Some(false),
                },
                &mut out,
                None,
            )
            .expect("run init");

            let rendered = String::from_utf8(out).expect("utf8 output");
            assert!(!rendered.contains("Initialising DevQL schema"));
            assert!(!rendered.contains("Bitloops project bootstrap is ready."));
            assert!(repo.path().join(".bitloops.local.toml").exists());
            assert!(repo.path().join(".claude/settings.json").exists());
            let exclude = std::fs::read_to_string(repo.path().join(".git/info/exclude"))
                .expect("read git exclude");
            assert!(exclude.contains(".bitloops.local.toml"));
            assert!(!exclude.contains("config.local.json"));
            assert!(!exclude.contains(".bitloops/config.local.json"));
        },
    );
}

#[test]
fn run_init_with_agent_flag_installs_requested_hooks_when_skip_baseline_is_requested() {
    let repo = tempfile::tempdir().expect("repo tempdir");
    let app_dirs = tempfile::tempdir().expect("app tempdir");
    setup_git_repo(&repo);

    with_temp_app_dirs_and_env(
        repo.path(),
        &app_dirs,
        &[("BITLOOPS_TEST_ASSUME_DAEMON_RUNNING", Some("1"))],
        || {
            let mut out = Vec::new();
            run_with_writer(
                InitArgs {
                    install_default_daemon: false,
                    force: true,
                    agent: Some(AGENT_CURSOR.to_string()),
                    telemetry: None,
                    no_telemetry: false,
                    skip_baseline: true,
                    sync: Some(false),
                    ingest: Some(false),
                },
                &mut out,
                None,
            )
            .expect("run init");

            let rendered = String::from_utf8(out).expect("utf8 output");
            assert!(!rendered.contains("Initialised agents: cursor"));
            assert!(!rendered.contains("Initialising DevQL schema"));
            assert!(repo.path().join(".cursor/hooks.json").exists());
            assert!(!repo.path().join(".claude/settings.json").exists());
        },
    );
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
            let selected = detect_or_select_agent(dir.path(), &mut out, None).unwrap();
            assert_eq!(selected, vec![DEFAULT_AGENT.to_string()]);
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
            let selected = detect_or_select_agent(dir.path(), &mut out, None).unwrap();
            assert_eq!(selected, vec![AGENT_CLAUDE_CODE.to_string()]);
        },
    );
}

#[test]
fn detect_or_select_agent_single_detected_with_tty_uses_selector() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    std::fs::create_dir_all(dir.path().join(".claude")).unwrap();

    let select = |_available: &[String]| -> std::result::Result<Vec<String>, String> {
        Ok(vec![AGENT_CURSOR.to_string()])
    };

    with_process_state(
        Some(dir.path()),
        &[("BITLOOPS_TEST_TTY", Some("1"))],
        || {
            let mut out = Vec::new();
            let selected = detect_or_select_agent(dir.path(), &mut out, Some(&select)).unwrap();
            assert_eq!(selected, vec![AGENT_CURSOR.to_string()]);
        },
    );
}

#[test]
fn detect_or_select_agent_selection_cancelled() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let select = |_available: &[String]| -> std::result::Result<Vec<String>, String> {
        Err("user cancelled".to_string())
    };
    with_process_state(
        Some(dir.path()),
        &[("BITLOOPS_TEST_TTY", Some("1"))],
        || {
            let mut out = Vec::new();
            let err = detect_or_select_agent(dir.path(), &mut out, Some(&select)).unwrap_err();
            assert!(format!("{err:#}").contains("user cancelled"));
        },
    );
}

#[test]
fn detect_or_select_agent_none_selected_errors() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    let select = |_available: &[String]| -> std::result::Result<Vec<String>, String> { Ok(vec![]) };
    with_process_state(
        Some(dir.path()),
        &[("BITLOOPS_TEST_TTY", Some("1"))],
        || {
            let mut out = Vec::new();
            let err = detect_or_select_agent(dir.path(), &mut out, Some(&select)).unwrap_err();
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
            let selected = detect_or_select_agent(dir.path(), &mut out, None).unwrap();
            assert_eq!(selected.len(), 2);
            assert!(selected.contains(&AGENT_CLAUDE_CODE.to_string()));
            assert!(selected.contains(&AGENT_GEMINI.to_string()));
        },
    );
}

#[test]
fn detect_or_select_agent_multiple_with_selector() {
    let dir = tempfile::tempdir().unwrap();
    setup_git_repo(&dir);
    std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
    std::fs::create_dir_all(dir.path().join(".gemini")).unwrap();
    let select = |_available: &[String]| -> std::result::Result<Vec<String>, String> {
        Ok(vec![
            AGENT_GEMINI.to_string(),
            AGENT_CODEX.to_string(),
            AGENT_CLAUDE_CODE.to_string(),
        ])
    };
    with_process_state(
        Some(dir.path()),
        &[("BITLOOPS_TEST_TTY", Some("1"))],
        || {
            let mut out = Vec::new();
            let selected = detect_or_select_agent(dir.path(), &mut out, Some(&select)).unwrap();
            assert_eq!(
                selected,
                vec![
                    AGENT_GEMINI.to_string(),
                    AGENT_CODEX.to_string(),
                    AGENT_CLAUDE_CODE.to_string()
                ]
            );
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

    with_temp_app_dirs_and_env(
        repo.path(),
        &app_dirs,
        &[
            ("BITLOOPS_TEST_ASSUME_DAEMON_RUNNING", Some("1")),
            ("BITLOOPS_TEST_TTY", Some("1")),
        ],
        || {
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
                    let mut input = Cursor::new("\n");
                    let select = |_items: &[String]| Ok(vec!["claude-code".to_string()]);
                    let runtime = tokio::runtime::Runtime::new().expect("runtime");
                    runtime
                        .block_on(run_with_io_async(
                            InitArgs {
                                install_default_daemon: false,
                                force: false,
                                agent: None,
                                telemetry: None,
                                no_telemetry: false,
                                skip_baseline: false,
                                sync: Some(false),
                                ingest: Some(false),
                            },
                            &mut out,
                            &mut input,
                            Some(&select),
                        ))
                        .expect("run init");

                    let rendered = String::from_utf8(out).expect("utf8 output");
                    assert!(rendered.contains("Help us improve Bitloops"));
                    assert!(rendered.contains("Enable anonymous telemetry? [Y/n]"));
                    assert!(!rendered.contains("Bitloops project bootstrap is ready."));
                },
            );
        },
    );
}

#[test]
fn run_init_noninteractive_existing_telemetry_requires_explicit_flag() {
    let repo = tempfile::tempdir().unwrap();
    let app_dirs = tempfile::tempdir().unwrap();
    setup_git_repo(&repo);

    with_temp_app_dirs_and_env(
        repo.path(),
        &app_dirs,
        &[
            ("BITLOOPS_TEST_ASSUME_DAEMON_RUNNING", Some("1")),
            ("BITLOOPS_TEST_TTY", Some("0")),
        ],
        || {
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
                    let runtime = tokio::runtime::Runtime::new().expect("runtime");
                    let err = runtime
                        .block_on(run_with_io_async(
                            InitArgs {
                                install_default_daemon: false,
                                force: false,
                                agent: None,
                                telemetry: None,
                                no_telemetry: false,
                                skip_baseline: false,
                                sync: Some(false),
                                ingest: Some(false),
                            },
                            &mut out,
                            &mut input,
                            None,
                        ))
                        .expect_err("init should fail without explicit telemetry");

                    assert_eq!(err.to_string(), NON_INTERACTIVE_TELEMETRY_ERROR);
                    assert!(!repo.path().join(".bitloops.local.toml").exists());
                },
            );
        },
    );
}

#[test]
fn run_init_noninteractive_fresh_daemon_bootstrap_requires_explicit_telemetry_flag() {
    let repo = tempfile::tempdir().unwrap();
    let app_dirs = tempfile::tempdir().unwrap();
    setup_git_repo(&repo);

    with_temp_app_dirs_and_env(
        repo.path(),
        &app_dirs,
        &[("BITLOOPS_TEST_TTY", Some("0"))],
        || {
            let mut out = Vec::new();
            let mut input = Cursor::new("");
            let runtime = tokio::runtime::Runtime::new().expect("runtime");
            let err = runtime
                .block_on(run_with_io_async(
                    InitArgs {
                        install_default_daemon: true,
                        force: false,
                        agent: None,
                        telemetry: None,
                        no_telemetry: false,
                        skip_baseline: false,
                        sync: Some(false),
                        ingest: Some(false),
                    },
                    &mut out,
                    &mut input,
                    None,
                ))
                .expect_err("init should fail without explicit telemetry flag");

            assert_eq!(err.to_string(), NON_INTERACTIVE_TELEMETRY_ERROR);
        },
    );
}

#[test]
fn run_init_with_explicit_telemetry_choice_persists_without_prompt() {
    let repo = tempfile::tempdir().unwrap();
    let app_dirs = tempfile::tempdir().unwrap();
    setup_git_repo(&repo);

    with_temp_app_dirs_and_env(
        repo.path(),
        &app_dirs,
        &[
            ("BITLOOPS_TEST_ASSUME_DAEMON_RUNNING", Some("1")),
            ("BITLOOPS_TEST_TTY", Some("0")),
        ],
        || {
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
                    let runtime = tokio::runtime::Runtime::new().expect("runtime");
                    runtime
                        .block_on(run_with_io_async(
                            InitArgs {
                                install_default_daemon: false,
                                force: false,
                                agent: None,
                                telemetry: Some(false),
                                no_telemetry: false,
                                skip_baseline: false,
                                sync: Some(false),
                                ingest: Some(false),
                            },
                            &mut out,
                            &mut input,
                            None,
                        ))
                        .expect("run init");

                    let rendered = String::from_utf8(out).expect("utf8 output");
                    assert!(!rendered.contains("Help us improve Bitloops"));
                },
            );
        },
    );
}

#[test]
fn run_init_noninteractive_requires_explicit_sync_and_ingest_choices() {
    let repo = tempfile::tempdir().unwrap();
    let app_dirs = tempfile::tempdir().unwrap();
    setup_git_repo(&repo);

    with_temp_app_dirs_and_env(
        repo.path(),
        &app_dirs,
        &[
            ("BITLOOPS_TEST_ASSUME_DAEMON_RUNNING", Some("1")),
            ("BITLOOPS_TEST_TTY", Some("0")),
        ],
        || {
            let mut out = Vec::new();
            let mut input = Cursor::new("");
            let runtime = tokio::runtime::Runtime::new().expect("runtime");
            let err = runtime
                .block_on(run_with_io_async(
                    InitArgs {
                        install_default_daemon: false,
                        force: false,
                        agent: None,
                        telemetry: Some(false),
                        no_telemetry: false,
                        skip_baseline: false,
                        sync: None,
                        ingest: Some(false),
                    },
                    &mut out,
                    &mut input,
                    None,
                ))
                .expect_err("init should require explicit init actions");

            assert_eq!(
                err.to_string(),
                "`bitloops init` requires explicit `--sync=true|false` and `--ingest=true|false` choices when not running interactively."
            );
        },
    );
}

#[test]
fn run_init_triggers_repo_scoped_ingest_when_enabled() {
    let saw_ingest = std::rc::Rc::new(std::cell::RefCell::new(false));
    let repo = tempfile::tempdir().unwrap();
    let repo_root = repo.path().to_path_buf();
    let app_dirs = tempfile::tempdir().unwrap();
    setup_git_repo(&repo);

    with_temp_app_dirs_and_env(
        repo.path(),
        &app_dirs,
        &[("BITLOOPS_TEST_TTY", Some("0"))],
        || {
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
                                    let saw_ingest = std::rc::Rc::clone(&saw_ingest);
                                    let repo_root = repo_root.clone();
                                    move |actual_repo_root: &std::path::Path,
                                  query: &str,
                                  variables: &serde_json::Value| {
                                let expected_repo_root =
                                    repo_root.canonicalize().unwrap_or_else(|_| repo_root.clone());
                                let actual_repo_root = actual_repo_root
                                    .canonicalize()
                                    .unwrap_or_else(|_| actual_repo_root.to_path_buf());
                                assert_eq!(actual_repo_root, expected_repo_root);

                                if query.contains("enqueueSync") {
                                    panic!("init should not enqueue sync when sync=false");
                                }

                                if query.contains("ingest") {
                                    *saw_ingest.borrow_mut() = true;
                                    assert_eq!(variables, &serde_json::json!({}));
                                    return Ok(serde_json::json!({
                                        "ingest": {
                                            "success": true,
                                            "commitsProcessed": 1,
                                            "checkpointCompanionsProcessed": 0,
                                            "eventsInserted": 0,
                                            "artefactsUpserted": 1,
                                            "semanticFeatureRowsUpserted": 0,
                                            "semanticFeatureRowsSkipped": 0,
                                            "symbolEmbeddingRowsUpserted": 0,
                                            "symbolEmbeddingRowsSkipped": 0,
                                            "symbolCloneEdgesUpserted": 0,
                                            "symbolCloneSourcesScored": 0
                                        }
                                    }));
                                }

                                panic!("unexpected repo-scoped query: {query}");
                            }
                                },
                                || {
                                    let mut out = Vec::new();
                                    let mut input = Cursor::new("");
                                    let runtime = tokio::runtime::Runtime::new().expect("runtime");
                                    runtime
                                        .block_on(run_with_io_async(
                                            InitArgs {
                                                install_default_daemon: false,
                                                force: false,
                                                agent: None,
                                                telemetry: Some(false),
                                                no_telemetry: false,
                                                skip_baseline: false,
                                                sync: Some(false),
                                                ingest: Some(true),
                                            },
                                            &mut out,
                                            &mut input,
                                            None,
                                        ))
                                        .expect("run init");
                                },
                            )
                        },
                    );
                    assert!(
                        *saw_ingest.borrow(),
                        "init should invoke repo-scoped ingest"
                    );
                },
            );
        },
    );
}

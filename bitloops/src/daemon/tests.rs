use super::*;
use crate::config::BITLOOPS_CONFIG_RELATIVE_PATH;
use crate::config::{
    ENV_DAEMON_CONFIG_PATH_OVERRIDE, bootstrap_default_daemon_environment, load_daemon_settings,
    persist_dashboard_tls_hint, update_daemon_telemetry_consent,
};
use crate::devql_transport::{RepoPathRegistry, RepoPathRegistryEntry, persist_repo_path_registry};
use crate::host::runtime_store::RepoSqliteRuntimeStore;
use crate::test_support::git_fixtures::init_test_repo;
use crate::test_support::log_capture::capture_logs_async;
use crate::test_support::process_state::{enter_process_state, with_env_var};
use tempfile::TempDir;

fn write_daemon_test_config(config_root: &Path) -> PathBuf {
    let config_path = config_root.join(BITLOOPS_CONFIG_RELATIVE_PATH);
    let parent = config_path.parent().expect("config parent");
    fs::create_dir_all(parent).expect("create config parent");
    fs::write(
        &config_path,
        r#"[stores.relational]
sqlite_path = "stores/daemon.sqlite"

[stores.events]
duckdb_path = "stores/daemon.duckdb"

[stores.blob]
local_path = "blob-store"
"#,
    )
    .expect("write test config");
    config_path
}

fn write_daemon_config(config_root: &Path, content: &str) -> PathBuf {
    let config_path = config_root.join(BITLOOPS_CONFIG_RELATIVE_PATH);
    let parent = config_path.parent().expect("config parent");
    fs::create_dir_all(parent).expect("create config parent");
    fs::write(&config_path, content).expect("write daemon config");
    config_path
}

#[cfg(unix)]
fn spawn_detached_long_lived_process() -> u32 {
    let output = std::process::Command::new("sh")
        .args(["-c", "sleep 60 >/dev/null 2>&1 & echo $!"])
        .output()
        .expect("spawn detached long-lived process");
    assert!(
        output.status.success(),
        "failed to spawn detached long-lived process: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("detached pid stdout should be utf8")
        .trim()
        .parse()
        .expect("detached pid should parse")
}

#[cfg(windows)]
fn spawn_detached_long_lived_process() -> u32 {
    let output = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "$p = Start-Process -FilePath ping -ArgumentList '-n 60 127.0.0.1' -WindowStyle Hidden -PassThru; $p.Id",
        ])
        .output()
        .expect("spawn detached long-lived process");
    assert!(
        output.status.success(),
        "failed to spawn detached long-lived process: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("detached pid stdout should be utf8")
        .trim()
        .parse()
        .expect("detached pid should parse")
}

fn wait_for_pid_exit(pid: u32) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if !process_is_running(pid).expect("inspect detached watcher process state") {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "expected detached watcher process {pid} to exit during daemon shutdown cleanup"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[tokio::test]
async fn daemon_lifecycle_logs_terminal_start_failure() {
    let temp = TempDir::new().expect("temp dir");
    let missing_config = temp.path().join("missing-config.toml");
    let _guard = enter_process_state(Some(temp.path()), &[]);

    let (result, logs) = capture_logs_async(run_internal_process(InternalDaemonProcessArgs {
        config_path: missing_config,
        mode: DaemonProcessModeArg::Detached,
        service_name: None,
        host: None,
        port: crate::api::DEFAULT_DASHBOARD_PORT,
        http: false,
        recheck_local_dashboard_net: false,
        bundle_dir: None,
        telemetry: None,
    }))
    .await;

    assert!(
        result.is_err(),
        "missing config should fail daemon process start"
    );
    assert!(
        logs.iter().any(|entry| entry.level == log::Level::Error
            && entry.message.contains("daemon process start failed")),
        "expected lifecycle owner to log terminal start failure, got logs: {logs:?}"
    );
}

#[tokio::test]
async fn daemon_lifecycle_logs_terminal_restart_failure() {
    let temp = TempDir::new().expect("temp dir");
    let _guard = enter_process_state(Some(temp.path()), &[]);

    let (result, logs) = capture_logs_async(restart(None)).await;

    assert!(result.is_err(), "restart without runtime should fail");
    assert!(
        logs.iter().any(|entry| entry.level == log::Level::Error
            && entry.message.contains("daemon restart failed")),
        "expected lifecycle owner to log terminal restart failure, got logs: {logs:?}"
    );
}

#[test]
fn supervisor_service_name_is_global_and_stable() {
    assert_eq!(GLOBAL_SUPERVISOR_SERVICE_NAME, "com.bitloops.daemon");
}

#[test]
fn daemon_shutdown_tears_down_watchers_bound_to_the_same_daemon_config() {
    let temp = TempDir::new().expect("temp dir");
    let daemon_root = temp.path().join("daemon-root");
    let repo_root = temp.path().join("repo");
    fs::create_dir_all(&daemon_root).expect("create daemon root");
    fs::create_dir_all(&repo_root).expect("create repo root");
    init_test_repo(&repo_root, "main", "Bitloops Test", "bitloops@example.com");
    fs::write(
        repo_root.join(".bitloops.local.toml"),
        "[capture]\nenabled = true\nstrategy = \"manual-commit\"\n",
    )
    .expect("write repo-local watcher policy");

    let daemon_config_path = write_daemon_test_config(&daemon_root);
    crate::config::settings::write_repo_daemon_binding(
        &repo_root.join(".bitloops.local.toml"),
        &daemon_config_path,
    )
    .expect("write repo daemon binding");

    let mut daemon_config =
        resolve_daemon_config(Some(daemon_config_path.as_path())).expect("resolve daemon config");
    daemon_config.repo_registry_path = temp.path().join("repo-path-registry.json");

    let repo = crate::host::devql::resolve_repo_identity(&repo_root).expect("resolve repo");
    persist_repo_path_registry(
        &daemon_config.repo_registry_path,
        &RepoPathRegistry {
            version: 1,
            entries: vec![RepoPathRegistryEntry {
                repo_id: repo.repo_id,
                provider: repo.provider,
                organisation: repo.organization,
                name: repo.name,
                identity: repo.identity,
                repo_root: repo_root.clone(),
                last_branch: Some("main".to_string()),
                git_dir_relative_path: Some(".git".to_string()),
                updated_at_unix: 0,
            }],
        },
    )
    .expect("persist repo path registry");

    let watcher_pid = spawn_detached_long_lived_process();
    let runtime_store =
        RepoSqliteRuntimeStore::open_for_roots(&daemon_config.config_root, &repo_root)
            .expect("open repo runtime store");
    runtime_store
        .save_watcher_registration(
            watcher_pid,
            "shutdown-cleanup-token",
            &repo_root,
            crate::host::runtime_store::RepoWatcherRegistrationState::Ready,
        )
        .expect("seed watcher registration");

    stop_bound_repo_watchers_for_daemon_shutdown(&daemon_config);

    assert!(
        runtime_store
            .load_watcher_registration()
            .expect("load watcher registration after shutdown cleanup")
            .is_none(),
        "daemon shutdown cleanup should clear watcher registration for bound repo"
    );
    wait_for_pid_exit(watcher_pid);
}

#[test]
fn daemon_startup_rehydrates_enabled_bound_repo_watchers_and_continues_after_failures() {
    let temp = TempDir::new().expect("temp dir");
    let daemon_root = temp.path().join("daemon-root");
    let enabled_repo = temp.path().join("enabled-repo");
    let second_enabled_repo = temp.path().join("second-enabled-repo");
    let third_enabled_repo = temp.path().join("third-enabled-repo");
    let disabled_repo = temp.path().join("disabled-repo");
    let unbound_repo = temp.path().join("unbound-repo");
    for repo_root in [
        &enabled_repo,
        &second_enabled_repo,
        &third_enabled_repo,
        &disabled_repo,
        &unbound_repo,
    ] {
        fs::create_dir_all(repo_root).expect("create repo root");
        init_test_repo(repo_root, "main", "Bitloops Test", "bitloops@example.com");
    }
    fs::create_dir_all(&daemon_root).expect("create daemon root");

    let daemon_config_path = write_daemon_test_config(&daemon_root);
    let other_daemon_config_path = temp.path().join("other").join(".bitloops.toml");
    fs::create_dir_all(
        other_daemon_config_path
            .parent()
            .expect("other config parent"),
    )
    .expect("create other config parent");
    fs::write(&other_daemon_config_path, "").expect("write other config");

    for repo_root in [
        &enabled_repo,
        &second_enabled_repo,
        &third_enabled_repo,
        &unbound_repo,
    ] {
        fs::write(
            repo_root.join(".bitloops.local.toml"),
            "[capture]\nenabled = true\nstrategy = \"manual-commit\"\n",
        )
        .expect("write enabled repo policy");
    }
    fs::write(
        disabled_repo.join(".bitloops.local.toml"),
        "[capture]\nenabled = false\nstrategy = \"manual-commit\"\n",
    )
    .expect("write disabled repo policy");

    for repo_root in [
        &enabled_repo,
        &second_enabled_repo,
        &third_enabled_repo,
        &disabled_repo,
    ] {
        crate::config::settings::write_repo_daemon_binding(
            &repo_root.join(".bitloops.local.toml"),
            &daemon_config_path,
        )
        .expect("write repo daemon binding");
    }
    crate::config::settings::write_repo_daemon_binding(
        &unbound_repo.join(".bitloops.local.toml"),
        &other_daemon_config_path,
    )
    .expect("write other repo daemon binding");

    let mut daemon_config =
        resolve_daemon_config(Some(daemon_config_path.as_path())).expect("resolve daemon config");
    daemon_config.repo_registry_path = temp.path().join("repo-path-registry.json");
    let mut entries = Vec::new();
    for repo_root in [
        &enabled_repo,
        &second_enabled_repo,
        &third_enabled_repo,
        &disabled_repo,
        &unbound_repo,
    ] {
        let repo = crate::host::devql::resolve_repo_identity(repo_root).expect("resolve repo");
        entries.push(RepoPathRegistryEntry {
            repo_id: repo.repo_id,
            provider: repo.provider,
            organisation: repo.organization,
            name: repo.name,
            identity: repo.identity,
            repo_root: repo_root.clone(),
            last_branch: Some("main".to_string()),
            git_dir_relative_path: Some(".git".to_string()),
            updated_at_unix: 0,
        });
    }
    persist_repo_path_registry(
        &daemon_config.repo_registry_path,
        &RepoPathRegistry {
            version: 1,
            entries,
        },
    )
    .expect("persist repo path registry");

    let expected_enabled_repo = enabled_repo
        .canonicalize()
        .unwrap_or_else(|_| enabled_repo.clone());
    let expected_second_enabled_repo = second_enabled_repo
        .canonicalize()
        .unwrap_or_else(|_| second_enabled_repo.clone());
    let expected_third_enabled_repo = third_enabled_repo
        .canonicalize()
        .unwrap_or_else(|_| third_enabled_repo.clone());
    let mut attempted = Vec::new();
    let daemon_config_path_string = daemon_config_path.to_string_lossy().to_string();
    with_env_var(
        ENV_DAEMON_CONFIG_PATH_OVERRIDE,
        Some(daemon_config_path_string.as_str()),
        || {
            ensure_bound_repo_watchers_for_daemon_startup_with(
                &daemon_config,
                |repo_root, _config_root| {
                    attempted.push(repo_root.to_path_buf());
                    if repo_root == expected_second_enabled_repo {
                        anyhow::bail!("synthetic watcher startup failure");
                    }
                    Ok(())
                },
            );
        },
    );

    assert_eq!(
        attempted,
        vec![
            expected_enabled_repo,
            expected_second_enabled_repo,
            expected_third_enabled_repo
        ]
    );
}

#[test]
fn launchd_plist_includes_hidden_supervisor_command() {
    let rendered = render_launchd_plist(
        GLOBAL_SUPERVISOR_SERVICE_NAME,
        Path::new("/Users/test"),
        Path::new("/usr/local/bin/bitloops"),
        &[OsString::from(INTERNAL_SUPERVISOR_COMMAND_NAME)],
    );
    assert!(rendered.contains(INTERNAL_SUPERVISOR_COMMAND_NAME));
    assert!(rendered.contains(GLOBAL_SUPERVISOR_SERVICE_NAME));
}

#[test]
fn systemd_unit_includes_hidden_supervisor_command() {
    let rendered = render_systemd_unit(
        GLOBAL_SUPERVISOR_SERVICE_NAME,
        Path::new("/Users/test"),
        Path::new("/usr/local/bin/bitloops"),
        &[OsString::from(INTERNAL_SUPERVISOR_COMMAND_NAME)],
    );
    assert!(rendered.contains(INTERNAL_SUPERVISOR_COMMAND_NAME));
    assert!(rendered.contains("WorkingDirectory=/Users/test"));
}

#[test]
fn read_runtime_state_drops_stale_file() {
    let dir = TempDir::new().expect("temp dir");
    let repo_root = dir.path();
    let state_root = TempDir::new().expect("temp dir");
    let state_root_str = state_root.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        Some(repo_root),
        &[(
            "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
            Some(state_root_str.as_str()),
        )],
    );
    let runtime_path = runtime_state_path(repo_root);
    write_runtime_state(
        &runtime_path,
        &DaemonRuntimeState {
            version: 1,
            config_path: repo_root.join("config.toml"),
            config_root: repo_root.to_path_buf(),
            pid: 999_999,
            mode: DaemonMode::Detached,
            service_name: None,
            url: "http://127.0.0.1:5667".to_string(),
            host: "127.0.0.1".to_string(),
            port: 5667,
            bundle_dir: repo_root.join("bundle"),
            relational_db_path: repo_root.join("relational.db"),
            events_db_path: repo_root.join("events.duckdb"),
            blob_store_path: repo_root.join("blob"),
            repo_registry_path: repo_root.join("repo-path-registry.json"),
            binary_fingerprint: "test".to_string(),
            updated_at_unix: 0,
        },
    )
    .expect("write runtime state");

    let state = read_runtime_state(repo_root).expect("read runtime state");
    assert!(state.is_none());
    assert!(
        !runtime_path.exists(),
        "stale runtime state file should be cleaned up"
    );
}

#[test]
fn resolve_daemon_config_uses_explicit_config_path_independent_of_cwd() {
    let config_root = TempDir::new().expect("temp dir");
    let other_cwd = TempDir::new().expect("temp dir");
    let config_path = write_daemon_test_config(config_root.path());
    let _guard = enter_process_state(Some(other_cwd.path()), &[]);

    let resolved =
        resolve_daemon_config(Some(config_path.as_path())).expect("resolve daemon config");
    let canonical_root = config_root
        .path()
        .canonicalize()
        .unwrap_or_else(|_| config_root.path().to_path_buf());

    assert_eq!(resolved.config_root, canonical_root);
    assert_eq!(
        resolved.relational_db_path,
        canonical_root.join("stores/daemon.sqlite")
    );
    assert_eq!(
        resolved.events_db_path,
        canonical_root.join("stores/daemon.duckdb")
    );
    assert_eq!(resolved.blob_store_path, canonical_root.join("blob-store"));
    assert_eq!(
        resolved.repo_registry_path,
        global_daemon_dir_fallback().join("repo-path-registry.json")
    );
}

#[test]
fn resolve_daemon_config_uses_default_global_config_path() {
    let config_root = TempDir::new().expect("temp dir");
    let other_cwd = TempDir::new().expect("temp dir");
    let config_root_str = config_root.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        Some(other_cwd.path()),
        &[(
            "BITLOOPS_TEST_CONFIG_DIR_OVERRIDE",
            Some(config_root_str.as_str()),
        )],
    );
    let config_path = write_daemon_test_config(&config_root.path().join("bitloops"));

    let resolved = resolve_daemon_config(None).expect("resolve daemon config");
    let canonical_root = config_root
        .path()
        .join("bitloops")
        .canonicalize()
        .unwrap_or_else(|_| config_root.path().join("bitloops"));

    assert_eq!(
        resolved.config_path,
        config_path
            .canonicalize()
            .unwrap_or_else(|_| config_path.clone())
    );
    assert_eq!(resolved.config_root, canonical_root);
    assert_eq!(
        resolved.relational_db_path,
        canonical_root.join("stores/daemon.sqlite")
    );
}

#[test]
fn resolve_daemon_config_requires_bootstrapped_default_global_config() {
    let config_root = TempDir::new().expect("temp dir");
    let other_cwd = TempDir::new().expect("temp dir");
    let config_root_str = config_root.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        Some(other_cwd.path()),
        &[(
            "BITLOOPS_TEST_CONFIG_DIR_OVERRIDE",
            Some(config_root_str.as_str()),
        )],
    );

    let err = resolve_daemon_config(None).expect_err("missing default config should fail");
    let expected_path = config_root
        .path()
        .join("bitloops")
        .join(BITLOOPS_CONFIG_RELATIVE_PATH);
    let message = format!("{err:#}");

    assert!(message.contains(&expected_path.display().to_string()));
    assert!(message.contains("--create-default-config"));
}

#[test]
fn resolve_daemon_config_uses_env_override_when_explicit_path_is_missing() {
    let config_root = TempDir::new().expect("temp dir");
    let other_cwd = TempDir::new().expect("temp dir");
    let config_path = write_daemon_test_config(config_root.path());
    let config_path_str = config_path.to_string_lossy().to_string();
    let _guard = enter_process_state(
        Some(other_cwd.path()),
        &[(
            ENV_DAEMON_CONFIG_PATH_OVERRIDE,
            Some(config_path_str.as_str()),
        )],
    );

    let resolved = resolve_daemon_config(None).expect("resolve daemon config");
    let canonical_root = config_root
        .path()
        .canonicalize()
        .unwrap_or_else(|_| config_root.path().to_path_buf());

    assert_eq!(
        resolved.config_path,
        config_path
            .canonicalize()
            .unwrap_or_else(|_| config_path.clone())
    );
    assert_eq!(resolved.config_root, canonical_root);
    assert_eq!(
        resolved.relational_db_path,
        canonical_root.join("stores/daemon.sqlite")
    );
}

#[test]
fn bootstrap_default_daemon_environment_creates_config_and_local_store_files() {
    let config_root = TempDir::new().expect("temp dir");
    let data_root = TempDir::new().expect("temp dir");
    let cache_root = TempDir::new().expect("temp dir");
    let state_root = TempDir::new().expect("temp dir");
    let config_root_str = config_root.path().to_string_lossy().to_string();
    let data_root_str = data_root.path().to_string_lossy().to_string();
    let cache_root_str = cache_root.path().to_string_lossy().to_string();
    let state_root_str = state_root.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        None,
        &[
            (
                "BITLOOPS_TEST_CONFIG_DIR_OVERRIDE",
                Some(config_root_str.as_str()),
            ),
            (
                "BITLOOPS_TEST_DATA_DIR_OVERRIDE",
                Some(data_root_str.as_str()),
            ),
            (
                "BITLOOPS_TEST_CACHE_DIR_OVERRIDE",
                Some(cache_root_str.as_str()),
            ),
            (
                "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
                Some(state_root_str.as_str()),
            ),
        ],
    );

    let config_path = bootstrap_default_daemon_environment().expect("bootstrap default daemon");
    let rendered = fs::read_to_string(&config_path).expect("read daemon config");
    let resolved = resolve_daemon_config(Some(config_path.as_path())).expect("resolve config");

    assert!(rendered.contains("[stores.relational]"));
    assert!(rendered.contains("[stores.events]"));
    assert!(rendered.contains("[stores.blob]"));
    assert!(resolved.relational_db_path.is_file());
    assert!(resolved.events_db_path.is_file());
    assert!(resolved.blob_store_path.is_dir());
}

#[test]
fn update_daemon_telemetry_consent_persists_cli_version() {
    let config_root = TempDir::new().expect("temp dir");
    let config_root_str = config_root.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        None,
        &[(
            "BITLOOPS_TEST_CONFIG_DIR_OVERRIDE",
            Some(config_root_str.as_str()),
        )],
    );
    let config_path = write_daemon_config(
        &config_root.path().join("bitloops"),
        r#"[runtime]
local_dev = false
"#,
    );

    let state = update_daemon_telemetry_consent(Some(config_path.as_path()), "1.2.3", Some(true))
        .expect("update telemetry consent");
    let loaded = load_daemon_settings(Some(config_path.as_path())).expect("load daemon settings");

    assert_eq!(state.telemetry, Some(true));
    assert_eq!(state.cli_version, "1.2.3");
    assert_eq!(loaded.cli.cli_version, "1.2.3");
    assert_eq!(loaded.cli.telemetry, Some(true));
}

#[test]
fn update_daemon_telemetry_consent_clears_legacy_opt_out_without_version() {
    let config_root = TempDir::new().expect("temp dir");
    let config_root_str = config_root.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        None,
        &[(
            "BITLOOPS_TEST_CONFIG_DIR_OVERRIDE",
            Some(config_root_str.as_str()),
        )],
    );
    let config_path = write_daemon_config(
        &config_root.path().join("bitloops"),
        r#"[runtime]
local_dev = false

[telemetry]
enabled = false
"#,
    );

    let state = update_daemon_telemetry_consent(Some(config_path.as_path()), "1.2.3", None)
        .expect("update telemetry consent");
    let loaded = load_daemon_settings(Some(config_path.as_path())).expect("load daemon settings");

    assert_eq!(state.telemetry, None);
    assert!(state.needs_prompt);
    assert_eq!(loaded.cli.cli_version, "1.2.3");
    assert_eq!(loaded.cli.telemetry, None);
}

#[test]
fn update_daemon_telemetry_consent_clears_opt_out_on_newer_version() {
    let config_root = TempDir::new().expect("temp dir");
    let config_root_str = config_root.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        None,
        &[(
            "BITLOOPS_TEST_CONFIG_DIR_OVERRIDE",
            Some(config_root_str.as_str()),
        )],
    );
    let config_path = write_daemon_config(
        &config_root.path().join("bitloops"),
        r#"[runtime]
local_dev = false
cli_version = "1.2.2"

[telemetry]
enabled = false
"#,
    );

    let state = update_daemon_telemetry_consent(Some(config_path.as_path()), "1.2.3", None)
        .expect("update telemetry consent");
    let loaded = load_daemon_settings(Some(config_path.as_path())).expect("load daemon settings");

    assert_eq!(state.telemetry, None);
    assert!(state.needs_prompt);
    assert_eq!(loaded.cli.telemetry, None);
    assert_eq!(loaded.cli.cli_version, "1.2.3");
}

#[test]
fn update_daemon_telemetry_consent_preserves_opt_in_on_newer_version() {
    let config_root = TempDir::new().expect("temp dir");
    let config_root_str = config_root.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        None,
        &[(
            "BITLOOPS_TEST_CONFIG_DIR_OVERRIDE",
            Some(config_root_str.as_str()),
        )],
    );
    let config_path = write_daemon_config(
        &config_root.path().join("bitloops"),
        r#"[runtime]
local_dev = false
cli_version = "1.2.2"

[telemetry]
enabled = true
"#,
    );

    let state = update_daemon_telemetry_consent(Some(config_path.as_path()), "1.2.3", None)
        .expect("update telemetry consent");
    let loaded = load_daemon_settings(Some(config_path.as_path())).expect("load daemon settings");

    assert_eq!(state.telemetry, Some(true));
    assert!(!state.needs_prompt);
    assert_eq!(loaded.cli.telemetry, Some(true));
    assert_eq!(loaded.cli.cli_version, "1.2.3");
}

#[test]
fn persist_dashboard_tls_hint_creates_missing_dashboard_tables() {
    let config_root = TempDir::new().expect("temp dir");
    let config_root_str = config_root.path().to_string_lossy().to_string();
    let _guard = enter_process_state(
        None,
        &[(
            "BITLOOPS_TEST_CONFIG_DIR_OVERRIDE",
            Some(config_root_str.as_str()),
        )],
    );
    let config_path = write_daemon_config(
        &config_root.path().join("bitloops"),
        r#"[runtime]
local_dev = false
"#,
    );

    persist_dashboard_tls_hint(true).expect("persist dashboard tls hint");
    let contents = fs::read_to_string(&config_path).expect("read daemon config");

    assert!(contents.contains("[dashboard.local_dashboard]"));
    assert!(contents.contains("tls = true"));
}

use super::*;
use crate::config::BITLOOPS_CONFIG_RELATIVE_PATH;
use crate::config::{
    bootstrap_default_daemon_environment, load_daemon_settings, update_daemon_telemetry_consent,
};
use crate::test_support::process_state::enter_process_state;
use tempfile::TempDir;

fn write_daemon_test_config(config_root: &Path) -> PathBuf {
    let config_path = config_root.join(BITLOOPS_CONFIG_RELATIVE_PATH);
    let parent = config_path.parent().expect("config parent");
    fs::create_dir_all(parent).expect("create config parent");
    fs::write(
        &config_path,
        r#"[stores.relational]
sqlite_path = ".bitloops/stores/daemon.sqlite"

[stores.events]
duckdb_path = ".bitloops/stores/daemon.duckdb"

[stores.blob]
local_path = ".bitloops/blob-store"
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

#[test]
fn supervisor_service_name_is_global_and_stable() {
    assert_eq!(GLOBAL_SUPERVISOR_SERVICE_NAME, "com.bitloops.daemon");
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
        canonical_root.join(".bitloops/stores/daemon.sqlite")
    );
    assert_eq!(
        resolved.events_db_path,
        canonical_root.join(".bitloops/stores/daemon.duckdb")
    );
    assert_eq!(
        resolved.blob_store_path,
        canonical_root.join(".bitloops/blob-store")
    );
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
        canonical_root.join(".bitloops/stores/daemon.sqlite")
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

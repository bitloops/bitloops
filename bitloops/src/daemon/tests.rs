use super::*;
use crate::test_support::process_state::enter_process_state;
use serde_json::json;
use tempfile::TempDir;

fn write_daemon_test_config(config_root: &Path) -> PathBuf {
    let config_path = config_root.join(BITLOOPS_CONFIG_RELATIVE_PATH);
    let parent = config_path.parent().expect("config parent");
    fs::create_dir_all(parent).expect("create config parent");
    fs::write(
        &config_path,
        serde_json::to_vec_pretty(&json!({
            "version": "1.0",
            "scope": "project",
            "settings": {
                "stores": {
                    "relational": {
                        "sqlite_path": ".bitloops/stores/daemon.sqlite"
                    },
                    "events": {
                        "duckdb_path": ".bitloops/stores/daemon.duckdb"
                    },
                    "blob": {
                        "local_path": ".bitloops/blob-store"
                    }
                }
            }
        }))
        .expect("serialise test config"),
    )
    .expect("write test config");
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
    let runtime_path = runtime_state_path(repo_root);
    write_runtime_state(
        &runtime_path,
        &DaemonRuntimeState {
            version: 1,
            config_path: repo_root.join(".bitloops").join("config.json"),
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
fn resolve_daemon_config_uses_local_dot_bitloops_config_by_default() {
    let config_root = TempDir::new().expect("temp dir");
    let config_path = write_daemon_test_config(config_root.path());
    let _guard = enter_process_state(Some(config_root.path()), &[]);

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
        canonical_root.join(".bitloops/stores/daemon.sqlite")
    );
}

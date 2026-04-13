use std::fs;

use tempfile::NamedTempFile;

use super::*;

#[test]
fn load_daemon_settings_rejects_unknown_top_level_fields() {
    let config = NamedTempFile::new().expect("create temp config");
    fs::write(
        config.path(),
        r#"
cli_version = "0.0.3"

[runtime]
local_dev = true

[telemetry]
enabled = false

[logging]
level = "debug"
"#,
    )
    .expect("write temp config");

    let err = load_daemon_settings(Some(config.path())).expect_err("unknown top-level key");
    let message = format!("{err:#}");
    assert!(
        message.contains("unknown field `cli_version`"),
        "expected unknown field error, got: {message}"
    );
}

#[test]
fn load_daemon_settings_accepts_runtime_cli_version_field() {
    let config = NamedTempFile::new().expect("create temp config");
    fs::write(
        config.path(),
        r#"
[runtime]
local_dev = true
cli_version = "0.0.12"

[telemetry]
enabled = true

[logging]
level = "info"
"#,
    )
    .expect("write temp config");

    let loaded = load_daemon_settings(Some(config.path())).expect("load daemon settings");
    assert!(loaded.cli.local_dev, "runtime.local_dev should be parsed");
    assert_eq!(loaded.cli.telemetry, Some(true));
    assert_eq!(loaded.cli.log_level, "info");
}

#[test]
fn ensure_daemon_store_artifacts_creates_local_store_files_for_explicit_config() {
    let dir = tempfile::tempdir().expect("temp dir");
    let config_path = dir.path().join("config.toml");
    fs::write(
        &config_path,
        r#"
[runtime]
local_dev = false
cli_version = "0.0.12"

[stores.relational]
sqlite_path = "stores/relational/relational.db"

[stores.events]
duckdb_path = "stores/event/events.duckdb"

[stores.blob]
local_path = "stores/blob"
"#,
    )
    .expect("write daemon config");

    let returned_path =
        ensure_daemon_store_artifacts(Some(config_path.as_path())).expect("bootstrap stores");

    assert_eq!(
        returned_path,
        config_path
            .canonicalize()
            .unwrap_or_else(|_| config_path.clone())
    );
    assert!(dir.path().join("stores/relational/relational.db").is_file());
    assert!(dir.path().join("stores/event/events.duckdb").is_file());
    assert!(dir.path().join("stores/blob").is_dir());
}

#[test]
fn prepare_daemon_embeddings_install_applies_staged_runtime_args_cleanup() {
    let config = NamedTempFile::new().expect("create temp config");
    fs::write(
        config.path(),
        r#"
[runtime]
local_dev = false

[inference.runtimes.bitloops_embeddings]
command = "bitloops-embeddings"
args = ["-B", "-m", "bitloops_embeddings"]
startup_timeout_secs = 60
request_timeout_secs = 300
"#,
    )
    .expect("write temp config");

    let plan =
        prepare_daemon_embeddings_install(config.path()).expect("prepare embeddings install");
    assert_eq!(plan.mode, DaemonEmbeddingsInstallMode::Bootstrap);
    plan.apply().expect("apply staged embeddings config");

    let rendered = fs::read_to_string(config.path()).expect("read updated config");
    assert!(
        rendered.contains("args = []"),
        "expected args reset:\n{rendered}"
    );
    assert!(
        !rendered.contains("\"-B\""),
        "expected stale python-style args removed:\n{rendered}"
    );
}

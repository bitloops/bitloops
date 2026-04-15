use std::fs;
use std::path::Path;

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

[inference.runtimes.bitloops_local_embeddings]
command = "bitloops-local-embeddings"
args = ["-B", "-m", "bitloops_local_embeddings"]
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

#[test]
fn apply_with_managed_runtime_path_preserves_concurrent_summary_profile_updates() {
    let config = NamedTempFile::new().expect("create temp config");
    fs::write(
        config.path(),
        r#"
[runtime]
local_dev = false

[inference.runtimes.bitloops_local_embeddings]
command = "bitloops-local-embeddings"
args = []
startup_timeout_secs = 60
request_timeout_secs = 300
"#,
    )
    .expect("write temp config");

    let plan =
        prepare_daemon_embeddings_install(config.path()).expect("prepare embeddings install");
    assert_eq!(plan.mode, DaemonEmbeddingsInstallMode::Bootstrap);

    fs::write(
        config.path(),
        r#"
[runtime]
local_dev = false

[semantic_clones.inference]
summary_generation = "summary_local"

[inference.runtimes.bitloops_local_embeddings]
command = "bitloops-local-embeddings"
args = []
startup_timeout_secs = 60
request_timeout_secs = 300

[inference.profiles.summary_local]
task = "text_generation"
driver = "ollama_chat"
runtime = "bitloops_inference"
model = "ministral-3:3b"
base_url = "http://127.0.0.1:11434/api/chat"
temperature = "0.1"
max_output_tokens = 200
"#,
    )
    .expect("write concurrent summary config update");

    plan.apply_with_managed_runtime_path(Path::new("/tmp/bitloops-local-embeddings"))
        .expect("apply staged embeddings config");

    let rendered = fs::read_to_string(config.path()).expect("read updated config");
    assert!(
        rendered.contains("summary_generation = \"summary_local\""),
        "expected embeddings apply to preserve summary binding:\n{rendered}"
    );
    assert!(
        rendered.contains("[inference.profiles.summary_local]"),
        "expected embeddings apply to preserve summary profile:\n{rendered}"
    );
    assert!(
        rendered.contains("command = \"/tmp/bitloops-local-embeddings\""),
        "expected embeddings runtime command to be rewritten:\n{rendered}"
    );
}

#[test]
fn prepare_daemon_platform_embeddings_install_writes_platform_runtime_config() {
    let config = NamedTempFile::new().expect("create temp config");
    fs::write(
        config.path(),
        r#"
[runtime]
local_dev = false
"#,
    )
    .expect("write temp config");

    let plan = prepare_daemon_platform_embeddings_install(
        config.path(),
        "https://gateway.example/v1/embeddings",
        "BITLOOPS_PLATFORM_GATEWAY_TOKEN",
    )
    .expect("prepare platform embeddings install");
    assert_eq!(plan.mode, DaemonEmbeddingsInstallMode::Bootstrap);
    plan.apply()
        .expect("apply staged platform embeddings config");

    let rendered = fs::read_to_string(config.path()).expect("read updated config");
    assert!(
        rendered.contains("[inference.runtimes.bitloops_platform_embeddings]"),
        "expected platform runtime table:\n{rendered}"
    );
    assert!(
        rendered.contains("command = \"bitloops-platform-embeddings\""),
        "expected platform runtime command:\n{rendered}"
    );
    assert!(
        rendered.contains(
            "args = [\"--gateway-url\", \"https://gateway.example/v1/embeddings\", \"--api-key-env\", \"BITLOOPS_PLATFORM_GATEWAY_TOKEN\"]"
        ),
        "expected hosted runtime args:\n{rendered}"
    );
    assert!(
        rendered.contains("runtime = \"bitloops_platform_embeddings\""),
        "expected platform profile runtime binding:\n{rendered}"
    );
    assert!(
        rendered.contains("code_embeddings = \"platform_code\"")
            && rendered.contains("summary_embeddings = \"platform_code\""),
        "expected semantic clone bindings:\n{rendered}"
    );
}

#[test]
fn apply_with_managed_runtime_path_preserves_platform_runtime_args() {
    let config = NamedTempFile::new().expect("create temp config");

    let plan = prepare_daemon_platform_embeddings_install(
        config.path(),
        "https://gateway.example/v1/embeddings",
        "BITLOOPS_PLATFORM_GATEWAY_TOKEN",
    )
    .expect("prepare platform embeddings install");
    plan.apply_with_managed_runtime_path(Path::new("/tmp/bitloops-platform-embeddings"))
        .expect("apply managed platform runtime path");

    let rendered = fs::read_to_string(config.path()).expect("read updated config");
    assert!(
        rendered.contains("command = \"/tmp/bitloops-platform-embeddings\""),
        "expected managed platform runtime path:\n{rendered}"
    );
    assert!(
        rendered.contains(
            "args = [\"--gateway-url\", \"https://gateway.example/v1/embeddings\", \"--api-key-env\", \"BITLOOPS_PLATFORM_GATEWAY_TOKEN\"]"
        ),
        "expected platform runtime args to be preserved:\n{rendered}"
    );
}

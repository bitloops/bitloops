use super::*;
use crate::test_support::process_state::with_env_var;

use std::fs;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

fn write_alias_runtime_config(config_path: &Path) {
    fs::write(
        config_path,
        r#"
[runtime]
local_dev = false

[semantic_clones]
embedding_mode = "semantic_aware_once"

[semantic_clones.inference]
code_embeddings = "local_code"

[inference.runtimes.bitloops_local_embeddings]
command = "bitloops-local-embeddings"

[inference.profiles.local_code]
task = "embeddings"
driver = "bitloops_embeddings_ipc"
runtime = "bitloops_local_embeddings"
model = "bge-m3"
"#,
    )
    .expect("write daemon config");
}

fn write_fake_alias_runtime_command(bin_dir: &Path) {
    fs::create_dir_all(bin_dir).expect("create fake runtime bin dir");
    for file_name in ["bitloops-local-embeddings", "bitloops-local-embeddings.exe"] {
        let command_path = bin_dir.join(file_name);
        fs::write(&command_path, "#!/bin/sh\nexit 0\n").expect("write fake runtime command");
        #[cfg(unix)]
        {
            let mut permissions = fs::metadata(&command_path)
                .expect("fake runtime command metadata")
                .permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&command_path, permissions)
                .expect("mark fake runtime command executable");
        }
    }
}

fn with_path(path: &Path, f: impl FnOnce()) {
    let path = path.to_string_lossy().to_string();
    with_env_var("PATH", Some(path.as_str()), f);
}

#[test]
fn gate_status_allows_alias_managed_runtime_when_command_is_available_on_path() {
    let temp = tempfile::TempDir::new().expect("temp dir");
    let runtime_store = DaemonSqliteRuntimeStore::open_at(temp.path().join("runtime.sqlite"))
        .expect("open daemon runtime store");
    let config_path = temp.path().join("config.toml");
    let fake_bin_dir = temp.path().join("bin");

    write_alias_runtime_config(&config_path);
    write_fake_alias_runtime_command(&fake_bin_dir);

    with_path(&fake_bin_dir, || {
        let status = gate_status_for_config_path(&runtime_store, &config_path)
            .expect("load embeddings gate status");

        assert!(!status.blocked);
        assert_eq!(status.readiness, Some(EmbeddingsBootstrapReadiness::Ready));
        assert_eq!(status.active_task_id, None);
    });
}

#[test]
fn fresh_alias_managed_runtime_stays_blocked_without_available_command() {
    let temp = tempfile::TempDir::new().expect("temp dir");
    let runtime_store = DaemonSqliteRuntimeStore::open_at(temp.path().join("runtime.sqlite"))
        .expect("open daemon runtime store");
    let config_path = temp.path().join("config.toml");
    let empty_path_dir = temp.path().join("empty-path");

    write_alias_runtime_config(&config_path);
    fs::create_dir_all(&empty_path_dir).expect("create empty PATH dir");

    with_path(&empty_path_dir, || {
        let status = gate_status_for_config_path(&runtime_store, &config_path)
            .expect("load embeddings gate status");

        assert!(status.blocked);
        assert_eq!(
            status.readiness,
            Some(EmbeddingsBootstrapReadiness::Pending)
        );
        assert_eq!(status.active_task_id, None);
        assert_eq!(
            status.reason.as_deref(),
            Some("Managed embeddings runtime is not ready yet")
        );
    });
}

#[test]
fn stale_pending_gate_does_not_block_alias_managed_runtime_once_task_is_gone() {
    let temp = tempfile::TempDir::new().expect("temp dir");
    let runtime_store = DaemonSqliteRuntimeStore::open_at(temp.path().join("runtime.sqlite"))
        .expect("open daemon runtime store");
    let config_path = temp.path().join("config.toml");
    let fake_bin_dir = temp.path().join("bin");

    write_alias_runtime_config(&config_path);
    write_fake_alias_runtime_command(&fake_bin_dir);

    runtime_store
        .mutate_embeddings_bootstrap_state(|state| {
            state.entries.insert(
                config_path_key(&config_path),
                EmbeddingsBootstrapGateEntry {
                    config_path: config_path.clone(),
                    profile_name: "local_code".to_string(),
                    readiness: EmbeddingsBootstrapReadiness::Pending,
                    active_task_id: Some("bootstrap-task-1".to_string()),
                    last_error: None,
                    last_updated_unix: unix_timestamp_now(),
                },
            );
            Ok(())
        })
        .expect("persist stale bootstrap gate state");

    with_path(&fake_bin_dir, || {
        let status = gate_status_for_config_path(&runtime_store, &config_path)
            .expect("load embeddings gate status");

        assert!(!status.blocked);
        assert_eq!(status.readiness, Some(EmbeddingsBootstrapReadiness::Ready));
        assert_eq!(status.active_task_id, None);
    });
}

use super::*;

fn write_alias_runtime_config(config_path: &Path) {
    std::fs::write(
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

#[test]
fn gate_status_allows_alias_managed_runtime_without_active_bootstrap() {
    let temp = tempfile::TempDir::new().expect("temp dir");
    let runtime_store = DaemonSqliteRuntimeStore::open_at(temp.path().join("runtime.sqlite"))
        .expect("open daemon runtime store");
    let config_path = temp.path().join("config.toml");

    write_alias_runtime_config(&config_path);

    let status = gate_status_for_config_path(&runtime_store, &config_path)
        .expect("load embeddings gate status");

    assert!(!status.blocked);
    assert_eq!(status.readiness, Some(EmbeddingsBootstrapReadiness::Ready));
    assert_eq!(status.active_task_id, None);
}

#[test]
fn stale_pending_gate_does_not_block_alias_managed_runtime_once_task_is_gone() {
    let temp = tempfile::TempDir::new().expect("temp dir");
    let runtime_store = DaemonSqliteRuntimeStore::open_at(temp.path().join("runtime.sqlite"))
        .expect("open daemon runtime store");
    let config_path = temp.path().join("config.toml");

    write_alias_runtime_config(&config_path);

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

    let status = gate_status_for_config_path(&runtime_store, &config_path)
        .expect("load embeddings gate status");

    assert!(!status.blocked);
    assert_eq!(status.readiness, Some(EmbeddingsBootstrapReadiness::Ready));
    assert_eq!(status.active_task_id, None);
}

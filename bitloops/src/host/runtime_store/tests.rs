use std::fs;
use std::path::{Path, PathBuf};

use crate::config::{BITLOOPS_CONFIG_RELATIVE_PATH, ENV_DAEMON_CONFIG_PATH_OVERRIDE};
use crate::host::checkpoints::session::backend::SessionBackend;
use crate::host::checkpoints::session::state::SessionState;
use crate::host::interactions::store::InteractionSpool;
use crate::host::interactions::types::{InteractionSession, InteractionTurn};
use crate::test_support::git_fixtures::init_test_repo;
use crate::test_support::process_state::with_env_var;
use tempfile::TempDir;

use super::*;

fn write_test_daemon_config(config_root: &Path) -> PathBuf {
    let config_path = config_root.join(BITLOOPS_CONFIG_RELATIVE_PATH);
    fs::write(
        &config_path,
        r#"[runtime]
local_dev = false

[stores.relational]
sqlite_path = "stores/relational/relational.db"

[stores.events]
duckdb_path = "stores/event/events.duckdb"

[stores.blob]
local_path = "stores/blob"
"#,
    )
    .expect("write test daemon config");
    crate::config::settings::write_repo_daemon_binding(
        &config_root.join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
        &config_path,
    )
    .expect("write repo daemon binding");
    config_path
}

fn canonical_root(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

#[test]
fn repo_runtime_store_uses_config_root_runtime_sqlite_path() {
    let dir = TempDir::new().expect("tempdir");
    let repo_root = dir.path().join("bitloops");
    fs::create_dir_all(&repo_root).expect("create repo root");
    init_test_repo(&repo_root, "main", "Bitloops Test", "bitloops@example.com");
    let config_path = write_test_daemon_config(dir.path());
    let config_path_string = config_path.to_string_lossy().to_string();
    let expected = dir.path();
    let expected = canonical_root(expected)
        .join("stores")
        .join("runtime")
        .join("runtime.sqlite");
    with_env_var(
        ENV_DAEMON_CONFIG_PATH_OVERRIDE,
        Some(config_path_string.as_str()),
        || {
            let actual = RepoSqliteRuntimeStore::open(&repo_root)
                .expect("open runtime store")
                .db_path
                .clone();
            assert_eq!(actual, expected);
        },
    );
}

#[test]
fn repo_runtime_store_fails_without_daemon_config() {
    let dir = TempDir::new().expect("tempdir");
    init_test_repo(dir.path(), "main", "Bitloops Test", "bitloops@example.com");

    with_env_var(ENV_DAEMON_CONFIG_PATH_OVERRIDE, None, || {
        let err = RepoSqliteRuntimeStore::open(dir.path()).expect_err("runtime store should fail");
        let message = format!("{err:#}");
        assert!(
            message.contains("Bitloops repo daemon binding is missing"),
            "expected missing-config runtime store failure, got: {message}"
        );
    });
}

#[test]
fn repo_runtime_store_uses_repo_daemon_binding() {
    let dir = TempDir::new().expect("tempdir");
    let repo_root = dir.path().join("bitloops");
    fs::create_dir_all(&repo_root).expect("create repo root");
    init_test_repo(&repo_root, "main", "Bitloops Test", "bitloops@example.com");
    let expected = dir.path();
    let expected = canonical_root(expected)
        .join("stores")
        .join("runtime")
        .join("runtime.sqlite");

    write_test_daemon_config(dir.path());
    crate::config::settings::write_repo_daemon_binding(
        &repo_root.join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
        &dir.path().join(BITLOOPS_CONFIG_RELATIVE_PATH),
    )
    .expect("write repo daemon binding");

    with_env_var(ENV_DAEMON_CONFIG_PATH_OVERRIDE, None, || {
        let actual = RepoSqliteRuntimeStore::open(&repo_root)
            .expect("open runtime store")
            .db_path
            .clone();
        assert_eq!(actual, expected);
    });
}

#[test]
fn repo_runtime_store_shares_runtime_sqlite_and_fences_rows_by_repo() {
    let dir = TempDir::new().expect("tempdir");
    let repo_a = dir.path().join("repo-a");
    let repo_b = dir.path().join("repo-b");
    fs::create_dir_all(&repo_a).expect("create repo-a");
    fs::create_dir_all(&repo_b).expect("create repo-b");
    init_test_repo(&repo_a, "main", "Bitloops Test", "bitloops@example.com");
    init_test_repo(&repo_b, "main", "Bitloops Test", "bitloops@example.com");

    let config_path = write_test_daemon_config(dir.path());
    let config_path_string = config_path.to_string_lossy().to_string();
    let expected_db_path = dir.path();
    let expected_db_path = canonical_root(expected_db_path)
        .join("stores")
        .join("runtime")
        .join("runtime.sqlite");

    with_env_var(
        ENV_DAEMON_CONFIG_PATH_OVERRIDE,
        Some(config_path_string.as_str()),
        || {
            let store_a = RepoSqliteRuntimeStore::open(&repo_a).expect("open runtime store a");
            let store_b = RepoSqliteRuntimeStore::open(&repo_b).expect("open runtime store b");
            assert_eq!(store_a.db_path(), expected_db_path.as_path());
            assert_eq!(store_b.db_path(), expected_db_path.as_path());
            assert_ne!(store_a.repo_id(), store_b.repo_id());

            let shared_session_id = "shared-session";
            let session_backend_a = store_a.session_backend().expect("session backend a");
            let session_backend_b = store_b.session_backend().expect("session backend b");
            session_backend_a
                .save_session(&SessionState {
                    session_id: shared_session_id.to_string(),
                    worktree_path: repo_a.to_string_lossy().to_string(),
                    first_prompt: "repo a prompt".to_string(),
                    agent_type: "codex".to_string(),
                    ..SessionState::default()
                })
                .expect("save session for repo a");
            session_backend_b
                .save_session(&SessionState {
                    session_id: shared_session_id.to_string(),
                    worktree_path: repo_b.to_string_lossy().to_string(),
                    first_prompt: "repo b prompt".to_string(),
                    agent_type: "codex".to_string(),
                    ..SessionState::default()
                })
                .expect("save session for repo b");

            let loaded_a = session_backend_a
                .load_session(shared_session_id)
                .expect("load session for repo a")
                .expect("session for repo a should exist");
            let loaded_b = session_backend_b
                .load_session(shared_session_id)
                .expect("load session for repo b")
                .expect("session for repo b should exist");
            assert_eq!(loaded_a.worktree_path, repo_a.to_string_lossy());
            assert_eq!(loaded_b.worktree_path, repo_b.to_string_lossy());
            assert_eq!(loaded_a.first_prompt, "repo a prompt");
            assert_eq!(loaded_b.first_prompt, "repo b prompt");

            let spool_a = store_a.interaction_spool().expect("interaction spool a");
            let spool_b = store_b.interaction_spool().expect("interaction spool b");
            spool_a
                .record_session(&InteractionSession {
                    session_id: shared_session_id.to_string(),
                    repo_id: store_a.repo_id().to_string(),
                    agent_type: "codex".to_string(),
                    model: "gpt-5.4".to_string(),
                    first_prompt: "repo a prompt".to_string(),
                    transcript_path: repo_a
                        .join("transcript.jsonl")
                        .to_string_lossy()
                        .to_string(),
                    worktree_path: repo_a.to_string_lossy().to_string(),
                    worktree_id: "main".to_string(),
                    started_at: "2026-04-06T10:00:00Z".to_string(),
                    last_event_at: "2026-04-06T10:00:01Z".to_string(),
                    updated_at: "2026-04-06T10:00:01Z".to_string(),
                    ..InteractionSession::default()
                })
                .expect("record interaction session for repo a");
            spool_b
                .record_session(&InteractionSession {
                    session_id: shared_session_id.to_string(),
                    repo_id: store_b.repo_id().to_string(),
                    agent_type: "codex".to_string(),
                    model: "gpt-5.4".to_string(),
                    first_prompt: "repo b prompt".to_string(),
                    transcript_path: repo_b
                        .join("transcript.jsonl")
                        .to_string_lossy()
                        .to_string(),
                    worktree_path: repo_b.to_string_lossy().to_string(),
                    worktree_id: "main".to_string(),
                    started_at: "2026-04-06T10:00:00Z".to_string(),
                    last_event_at: "2026-04-06T10:00:02Z".to_string(),
                    updated_at: "2026-04-06T10:00:02Z".to_string(),
                    ..InteractionSession::default()
                })
                .expect("record interaction session for repo b");
            spool_a
                .record_turn(&InteractionTurn {
                    turn_id: "shared-turn".to_string(),
                    session_id: shared_session_id.to_string(),
                    repo_id: store_a.repo_id().to_string(),
                    turn_number: 1,
                    prompt: "repo a turn".to_string(),
                    agent_type: "codex".to_string(),
                    model: "gpt-5.4".to_string(),
                    started_at: "2026-04-06T10:00:01Z".to_string(),
                    updated_at: "2026-04-06T10:00:02Z".to_string(),
                    ..InteractionTurn::default()
                })
                .expect("record interaction turn for repo a");
            spool_b
                .record_turn(&InteractionTurn {
                    turn_id: "shared-turn".to_string(),
                    session_id: shared_session_id.to_string(),
                    repo_id: store_b.repo_id().to_string(),
                    turn_number: 1,
                    prompt: "repo b turn".to_string(),
                    agent_type: "codex".to_string(),
                    model: "gpt-5.4".to_string(),
                    started_at: "2026-04-06T10:00:01Z".to_string(),
                    updated_at: "2026-04-06T10:00:03Z".to_string(),
                    ..InteractionTurn::default()
                })
                .expect("record interaction turn for repo b");

            let sessions_a = spool_a.list_sessions(None, 10).expect("list sessions a");
            let sessions_b = spool_b.list_sessions(None, 10).expect("list sessions b");
            assert_eq!(sessions_a.len(), 1);
            assert_eq!(sessions_b.len(), 1);
            assert_eq!(sessions_a[0].worktree_path, repo_a.to_string_lossy());
            assert_eq!(sessions_b[0].worktree_path, repo_b.to_string_lossy());

            let turns_a = spool_a
                .list_turns_for_session(shared_session_id, 10)
                .expect("list turns a");
            let turns_b = spool_b
                .list_turns_for_session(shared_session_id, 10)
                .expect("list turns b");
            assert_eq!(turns_a.len(), 1);
            assert_eq!(turns_b.len(), 1);
            assert_eq!(turns_a[0].prompt, "repo a turn");
            assert_eq!(turns_b[0].prompt, "repo b turn");
        },
    );
}

#[test]
fn save_task_checkpoint_artefact_rejects_duplicate_ids_without_overwriting_blob() {
    let dir = TempDir::new().expect("tempdir");
    let repo_root = dir.path().join("bitloops");
    fs::create_dir_all(&repo_root).expect("create repo root");
    init_test_repo(&repo_root, "main", "Bitloops Test", "bitloops@example.com");
    let config_path = write_test_daemon_config(dir.path());
    let config_path_string = config_path.to_string_lossy().to_string();

    with_env_var(
        ENV_DAEMON_CONFIG_PATH_OVERRIDE,
        Some(config_path_string.as_str()),
        || {
            let store = RepoSqliteRuntimeStore::open(&repo_root).expect("open runtime store");
            let original_payload = br#"{"checkpoint":"original"}"#.to_vec();
            let duplicate_payload = br#"{"checkpoint":"duplicate"}"#.to_vec();

            let mut original = TaskCheckpointArtefact::new(
                "session-1",
                "tool-use-1",
                RuntimeMetadataBlobType::TaskCheckpoint,
                original_payload.clone(),
            );
            original.artefact_id = "artefact-1".to_string();
            store
                .save_task_checkpoint_artefact(&original)
                .expect("save original artefact");

            let mut duplicate = TaskCheckpointArtefact::new(
                "session-1",
                "tool-use-1",
                RuntimeMetadataBlobType::TaskCheckpoint,
                duplicate_payload,
            );
            duplicate.artefact_id = original.artefact_id.clone();
            let err = store
                .save_task_checkpoint_artefact(&duplicate)
                .expect_err("duplicate artefact should fail");
            let message = format!("{err:#}");
            assert!(
                message.contains("already exists"),
                "expected duplicate artefact failure, got: {message}"
            );

            let artefacts = store
                .load_task_checkpoint_artefacts("session-1", "tool-use-1")
                .expect("load saved artefacts");
            assert_eq!(artefacts.len(), 1);
            assert_eq!(artefacts[0].artefact_id, "artefact-1");
            assert_eq!(artefacts[0].payload, original_payload);
        },
    );
}

#[test]
fn daemon_runtime_store_persists_sync_state_in_sqlite() {
    let state_dir = TempDir::new().expect("tempdir");
    with_env_var(
        "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
        Some(state_dir.path().to_string_lossy().as_ref()),
        || {
            let store = DaemonSqliteRuntimeStore::open().expect("open daemon runtime store");
            let output = store
                .mutate_sync_queue_state(|state| {
                    state.version = 7;
                    Ok(state.version)
                })
                .expect("mutate sync queue state");
            assert_eq!(output, 7);
            let loaded = store
                .load_sync_queue_state()
                .expect("load sync queue state")
                .expect("state exists");
            assert_eq!(loaded.version, 7);
        },
    );
}

#[test]
fn daemon_runtime_store_persists_capability_event_queue_state_in_sqlite() {
    let state_dir = TempDir::new().expect("tempdir");
    with_env_var(
        "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
        Some(state_dir.path().to_string_lossy().as_ref()),
        || {
            let store = DaemonSqliteRuntimeStore::open().expect("open daemon runtime store");
            assert!(
                !store
                    .capability_event_state_exists()
                    .expect("check capability event state exists before save")
            );

            let output = store
                .mutate_capability_event_queue_state(|state| {
                    state.version = 3;
                    state.last_action = Some("enqueue".to_string());
                    state.runs.push(crate::daemon::CapabilityEventRunRecord {
                        run_id: "event-run-1".to_string(),
                        repo_id: "repo-1".to_string(),
                        capability_id: "test_harness".to_string(),
                        handler_id: "sync_completed".to_string(),
                        event_kind: "sync_completed".to_string(),
                        lane_key: "repo-1:test_harness:sync_completed".to_string(),
                        event_payload_json: serde_json::json!({
                            "repo_id": "repo-1",
                            "repo_root": "/tmp/repo",
                            "active_branch": "main",
                            "head_commit_sha": "abc123",
                            "sync_mode": "full",
                            "sync_completed_at": "2026-04-06T00:00:00Z",
                            "files": {},
                            "artefacts": {},
                        })
                        .to_string(),
                        status: crate::daemon::CapabilityEventRunStatus::Queued,
                        attempts: 0,
                        submitted_at_unix: 1,
                        started_at_unix: None,
                        updated_at_unix: 1,
                        completed_at_unix: None,
                        error: None,
                    });
                    Ok(state.version)
                })
                .expect("mutate capability event queue state");
            assert_eq!(output, 3);

            assert!(
                store
                    .capability_event_state_exists()
                    .expect("check capability event state exists after save")
            );

            let loaded = store
                .load_capability_event_queue_state()
                .expect("load capability event queue state")
                .expect("state exists");
            assert_eq!(loaded.version, 3);
            assert_eq!(loaded.last_action.as_deref(), Some("enqueue"));
            assert_eq!(loaded.runs.len(), 1);
            assert_eq!(loaded.runs[0].run_id, "event-run-1");
            assert_eq!(
                loaded.runs[0].lane_key,
                "repo-1:test_harness:sync_completed"
            );
        },
    );
}

#[test]
fn persisted_capability_event_queue_state_default_preserves_legacy_values() {
    let default = PersistedCapabilityEventQueueState::default();
    assert_eq!(default.version, 1);
    assert!(default.runs.is_empty());
    assert_eq!(default.last_action.as_deref(), Some("initialized"));
    assert_eq!(default.updated_at_unix, 0);
}

#[test]
fn daemon_runtime_store_uses_legacy_capability_event_defaults_when_state_is_missing() {
    let state_dir = TempDir::new().expect("tempdir");
    with_env_var(
        "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
        Some(state_dir.path().to_string_lossy().as_ref()),
        || {
            let store = DaemonSqliteRuntimeStore::open().expect("open daemon runtime store");
            let observed = store
                .mutate_capability_event_queue_state(|state| {
                    Ok((state.version, state.last_action.clone(), state.runs.len()))
                })
                .expect("load default capability event queue state");
            assert_eq!(observed.0, 1);
            assert_eq!(observed.1.as_deref(), Some("initialized"));
            assert_eq!(observed.2, 0);
        },
    );
}

#[test]
fn persisted_sync_queue_state_default_preserves_legacy_values() {
    let default = PersistedSyncQueueState::default();
    assert_eq!(default.version, 1);
    assert!(default.tasks.is_empty());
    assert_eq!(default.last_action.as_deref(), Some("initialized"));
    assert_eq!(default.updated_at_unix, 0);
}

#[test]
fn daemon_runtime_store_uses_legacy_sync_defaults_when_state_is_missing() {
    let state_dir = TempDir::new().expect("tempdir");
    with_env_var(
        "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
        Some(state_dir.path().to_string_lossy().as_ref()),
        || {
            let store = DaemonSqliteRuntimeStore::open().expect("open daemon runtime store");
            let observed = store
                .mutate_sync_queue_state(|state| Ok((state.version, state.last_action.clone())))
                .expect("load default sync queue state");
            assert_eq!(observed.0, 1);
            assert_eq!(observed.1.as_deref(), Some("initialized"));
        },
    );
}

#[test]
fn daemon_runtime_store_loads_legacy_sync_queue_state_with_config_root_field() {
    let state_dir = TempDir::new().expect("tempdir");
    with_env_var(
        "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
        Some(state_dir.path().to_string_lossy().as_ref()),
        || {
            let store = DaemonSqliteRuntimeStore::open().expect("open daemon runtime store");

            let task = crate::host::runtime_store::LegacySyncTaskRecord {
                task_id: "sync-task-legacy".to_string(),
                repo_id: "repo-1".to_string(),
                repo_name: "bitloops".to_string(),
                repo_provider: "local".to_string(),
                repo_organisation: "local".to_string(),
                repo_identity: "local/bitloops".to_string(),
                daemon_config_root: PathBuf::from("/tmp/legacy-config"),
                repo_root: PathBuf::from("/tmp/repo"),
                source: crate::daemon::DevqlTaskSource::ManualCli,
                mode: crate::daemon::SyncTaskMode::Full,
                status: crate::daemon::DevqlTaskStatus::Queued,
                submitted_at_unix: 1,
                started_at_unix: None,
                updated_at_unix: 1,
                completed_at_unix: None,
                queue_position: None,
                tasks_ahead: None,
                progress: crate::host::devql::SyncProgressUpdate::default(),
                error: None,
                summary: None,
            };
            let mut task_value = serde_json::to_value(&task).expect("serialise sync task");
            let daemon_config_root = task_value
                .as_object_mut()
                .expect("sync task should serialise as object")
                .remove("daemon_config_root")
                .expect("daemon_config_root should be present");
            task_value
                .as_object_mut()
                .expect("sync task should serialise as object")
                .insert("config_root".to_string(), daemon_config_root);
            let payload = serde_json::json!({
                "version": 1,
                "tasks": [task_value],
                "last_action": "enqueue",
                "updated_at_unix": 42
            })
            .to_string();

            let sqlite =
                crate::storage::SqliteConnectionPool::connect(store.db_path().to_path_buf())
                    .expect("connect runtime sqlite");
            sqlite
                .with_connection(|conn| {
                    conn.execute(
                        "INSERT INTO runtime_documents (document_kind, payload, updated_at)
                         VALUES (?1, ?2, datetime('now'))
                         ON CONFLICT(document_kind) DO UPDATE SET
                            payload = excluded.payload,
                            updated_at = excluded.updated_at",
                        rusqlite::params!["sync_queue_state", payload],
                    )?;
                    Ok::<_, anyhow::Error>(())
                })
                .expect("insert legacy sync queue payload");

            let loaded = store
                .load_sync_queue_state()
                .expect("load sync queue state")
                .expect("state should exist");
            assert_eq!(loaded.tasks.len(), 1);
            assert_eq!(
                loaded.tasks[0].daemon_config_root,
                PathBuf::from("/tmp/legacy-config")
            );

            let observed = store
                .mutate_sync_queue_state(|state| Ok(state.tasks[0].daemon_config_root.clone()))
                .expect("mutate legacy sync queue state");
            assert_eq!(observed, PathBuf::from("/tmp/legacy-config"));
        },
    );
}

#[test]
fn repo_runtime_store_persists_repo_watcher_registration() {
    let dir = TempDir::new().expect("tempdir");
    let repo_root = dir.path().join("repo");
    fs::create_dir_all(&repo_root).expect("create repo root");
    init_test_repo(&repo_root, "main", "Bitloops Test", "bitloops@example.com");

    let store = RepoSqliteRuntimeStore::open_for_roots(dir.path(), &repo_root)
        .expect("open repo runtime store");
    store
        .save_watcher_registration(4242, "restart-token", &repo_root)
        .expect("save watcher registration");

    let registration = store
        .load_watcher_registration()
        .expect("load watcher registration")
        .expect("watcher registration should exist");
    assert_eq!(registration.pid, 4242);
    assert_eq!(registration.restart_token, "restart-token");
    assert_eq!(registration.repo_root, repo_root);
}

fn collect_rust_files(root: &Path, out: &mut Vec<PathBuf>) {
    let mut entries = fs::read_dir(root)
        .expect("read source directory")
        .collect::<Result<Vec<_>, _>>()
        .expect("read source directory entries");
    entries.sort_by_key(|entry| entry.path());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_rust_files(&path, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

fn is_test_like(relative_path: &str) -> bool {
    relative_path.contains("/tests/")
        || relative_path.ends_with("/tests.rs")
        || relative_path.contains("_tests/")
        || relative_path.ends_with("_tests.rs")
        || relative_path.ends_with("_test.rs")
}

fn strip_inline_test_module(contents: &str) -> &str {
    contents
        .rfind("\n#[cfg(test)]")
        .map(|index| &contents[..index])
        .or_else(|| {
            contents
                .rfind("\r\n#[cfg(test)]")
                .map(|index| &contents[..index])
        })
        .unwrap_or(contents)
}

fn is_runtime_store_module(relative: &str) -> bool {
    relative == "host/runtime_store.rs" || relative.starts_with("host/runtime_store/")
}

#[test]
fn runtime_and_relational_store_boundaries_are_not_bypassed() {
    let src_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    collect_rust_files(&src_root, &mut files);
    files.sort();

    let allowed_temporary_path_shims =
        ["host/checkpoints/strategy/manual_commit/checkpoint_io/temporary.rs"];
    let allowed_spool_path_shims = [
        "host/interactions/db_store.rs",
        "host/checkpoints/lifecycle/adapters.rs",
        "host/checkpoints/strategy/manual_commit_tests/post_commit/helpers.rs",
    ];
    let banned_daemon_json_imports = [
        "super::super::state_store::{read_json, write_json}",
        "crate::daemon::state_store::{read_json, write_json}",
    ];
    let allowed_direct_sqlite_modules = [
        "host/runtime_store.rs",
        "host/relational_store.rs",
        "host/devql/types.rs",
        "host/devql/db_utils.rs",
        "host/devql/connection_status.rs",
        "host/devql/ingestion/schema/relational_initialisation.rs",
        "host/checkpoints/session/db_backend.rs",
        "host/checkpoints/strategy/manual_commit/checkpoint_io/temporary.rs",
        "host/interactions/db_store.rs",
        "api/db/sqlite.rs",
        "capability_packs/semantic_clones/stage_semantic_features.rs",
        "capability_packs/knowledge/storage/sqlite_relational.rs",
    ];
    let skipped_prefixes = ["config/", "storage/", "test_support/", "utils/"];
    let banned_direct_sqlite_patterns = [
        "resolve_sqlite_db_path_for_repo(",
        ".resolve_sqlite_db_path_for_repo(",
        "default_relational_db_path(",
        "SqliteConnectionPool::connect(",
        "SqliteConnectionPool::connect_existing(",
        "rusqlite::Connection::open_with_flags(",
    ];
    let allowed_relational_internal_modules = ["host/relational_store.rs", "host/devql/types.rs"];
    let banned_relational_internal_patterns = [".local.path", "RelationalStorage::local_only("];
    let mut violations = Vec::new();

    for file in files {
        let relative = file
            .strip_prefix(&src_root)
            .expect("strip source root prefix")
            .to_string_lossy()
            .replace('\\', "/");
        if is_runtime_store_module(&relative)
            || skipped_prefixes
                .iter()
                .any(|prefix| relative.starts_with(prefix))
        {
            continue;
        }
        if allowed_direct_sqlite_modules.contains(&relative.as_str()) || is_test_like(&relative) {
            continue;
        }
        let contents = fs::read_to_string(&file).expect("read source file");
        let production_contents = strip_inline_test_module(&contents);

        for banned_import in banned_daemon_json_imports {
            if production_contents.contains(banned_import) {
                violations.push(format!(
                    "legacy daemon JSON helpers are forbidden outside the runtime store: {}",
                    relative
                ));
            }
        }

        if production_contents.contains("resolve_temporary_checkpoint_sqlite_path(")
            && !allowed_temporary_path_shims.contains(&relative.as_str())
        {
            violations.push(format!(
                "runtime checkpoint path shim must stay local to the runtime-store compatibility layer: {}",
                relative
            ));
        }

        if production_contents.contains("interaction_spool_db_path(")
            && !allowed_spool_path_shims.contains(&relative.as_str())
        {
            violations.push(format!(
                "interaction spool path shim must stay local to the runtime-store compatibility layer: {}",
                relative
            ));
        }

        for banned_pattern in banned_direct_sqlite_patterns {
            if production_contents.contains(banned_pattern) {
                violations.push(format!(
                    "direct SQLite access must flow through RuntimeStore or RelationalStore: {} (matched `{}`)",
                    relative, banned_pattern
                ));
            }
        }

        if !allowed_relational_internal_modules.contains(&relative.as_str()) {
            for banned_pattern in banned_relational_internal_patterns {
                if production_contents.contains(banned_pattern) {
                    violations.push(format!(
                        "RelationalStorage internals must stay local to store implementation layers: {} (matched `{}`)",
                        relative, banned_pattern
                    ));
                }
            }
        }
    }

    if !violations.is_empty() {
        violations.sort();
        panic!(
            "Runtime/Relational store boundary violations:\n{}",
            violations.join("\n")
        );
    }
}

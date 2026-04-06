use std::fs;
use std::path::{Path, PathBuf};

use crate::host::interactions::db_store::legacy_interaction_spool_db_path;
use crate::host::interactions::store::InteractionSpool;
use crate::host::interactions::types::InteractionSession;
use crate::storage::SqliteConnectionPool;
use crate::test_support::git_fixtures::init_test_repo;
use crate::test_support::process_state::with_env_var;
use tempfile::TempDir;

use super::*;

#[test]
fn repo_runtime_store_uses_repo_scoped_runtime_sqlite_path() {
    let dir = TempDir::new().expect("tempdir");
    init_test_repo(dir.path(), "main", "Bitloops Test", "bitloops@example.com");
    let expected = dir
        .path()
        .join(".bitloops")
        .join("stores")
        .join("runtime")
        .join("runtime.sqlite");
    let actual = RepoSqliteRuntimeStore::open(dir.path())
        .expect("open runtime store")
        .db_path
        .clone();
    assert_eq!(actual, expected);
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
fn repo_runtime_store_imports_legacy_interaction_spool_from_standalone_sqlite() {
    let dir = TempDir::new().expect("tempdir");
    init_test_repo(dir.path(), "main", "Bitloops Test", "bitloops@example.com");
    let repo = crate::host::devql::resolve_repo_identity(dir.path()).expect("resolve repo");

    let legacy_path =
        legacy_interaction_spool_db_path(dir.path()).expect("resolve legacy spool path");
    fs::create_dir_all(legacy_path.parent().expect("legacy spool parent"))
        .expect("create legacy spool directory");

    let sqlite = SqliteConnectionPool::connect(legacy_path).expect("open legacy spool sqlite");
    let legacy_spool = crate::host::interactions::db_store::SqliteInteractionSpool::new(
        sqlite,
        repo.repo_id.clone(),
    )
    .expect("open legacy spool");
    legacy_spool
        .record_session(&InteractionSession {
            session_id: "session-1".into(),
            repo_id: repo.repo_id,
            agent_type: "codex".into(),
            model: "gpt-5.4".into(),
            first_prompt: "hello".into(),
            transcript_path: "/tmp/transcript.jsonl".into(),
            worktree_path: dir.path().display().to_string(),
            worktree_id: "main".into(),
            started_at: "2026-04-06T10:00:00Z".into(),
            last_event_at: "2026-04-06T10:00:00Z".into(),
            updated_at: "2026-04-06T10:00:00Z".into(),
            ..InteractionSession::default()
        })
        .expect("record session in legacy spool");

    let store = RepoSqliteRuntimeStore::open(dir.path()).expect("open repo runtime store");
    let sessions = store
        .interaction_spool()
        .expect("open runtime interaction spool")
        .list_sessions(None, 10)
        .expect("list imported sessions");

    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_id, "session-1");
}

#[test]
fn repo_runtime_store_imports_legacy_checkpoint_metadata_and_removes_files() {
    let dir = TempDir::new().expect("tempdir");
    init_test_repo(dir.path(), "main", "Bitloops Test", "bitloops@example.com");

    let session_dir = dir
        .path()
        .join(".bitloops")
        .join("metadata")
        .join("session-legacy");
    let task_dir = session_dir.join("tasks").join("toolu_legacy");
    let incremental_dir = task_dir.join("checkpoints");
    fs::create_dir_all(&incremental_dir).expect("create legacy metadata directories");

    fs::write(
        session_dir.join(crate::utils::paths::TRANSCRIPT_FILE_NAME),
        r#"{"type":"user","message":{"content":"Create foo"}}
{"type":"assistant","message":{"content":"Done"}}"#,
    )
    .expect("write legacy transcript");
    fs::write(
        session_dir.join(crate::utils::paths::PROMPT_FILE_NAME),
        "Create foo",
    )
    .expect("write legacy prompt");
    fs::write(
        session_dir.join(crate::utils::paths::SUMMARY_FILE_NAME),
        "Done",
    )
    .expect("write legacy summary");
    fs::write(
        session_dir.join(crate::utils::paths::CONTEXT_FILE_NAME),
        "# Session Context\n\nLegacy context",
    )
    .expect("write legacy context");
    fs::write(
        task_dir.join(crate::utils::paths::CHECKPOINT_FILE_NAME),
        r#"{"checkpoint_uuid":"legacy-checkpoint"}"#,
    )
    .expect("write legacy task checkpoint");
    fs::write(
        task_dir.join("agent-agent-1.jsonl"),
        r#"{"type":"assistant","message":{"content":"subagent"}}"#,
    )
    .expect("write legacy subagent transcript");
    fs::write(
        incremental_dir.join("003-toolu_legacy.json"),
        r#"{"type":"TodoWrite","data":{"todo":"document storage"}}"#,
    )
    .expect("write legacy incremental checkpoint");

    let store = RepoSqliteRuntimeStore::open(dir.path()).expect("open repo runtime store");
    let snapshot = store
        .load_latest_session_metadata_snapshot("session-legacy")
        .expect("load imported metadata snapshot")
        .expect("legacy metadata snapshot should be imported");
    assert_eq!(snapshot.bundle.prompts, vec!["Create foo".to_string()]);
    assert_eq!(snapshot.bundle.summary, "Done");
    assert!(
        String::from_utf8_lossy(&snapshot.bundle.context).contains("Legacy context"),
        "legacy context should be preserved during import"
    );

    let artefacts = store
        .load_task_checkpoint_artefacts("session-legacy", "toolu_legacy")
        .expect("load imported task artefacts");
    assert!(
        artefacts
            .iter()
            .any(|artefact| artefact.kind == RuntimeMetadataBlobType::TaskCheckpoint),
        "task checkpoint artefact should be imported"
    );
    assert!(
        artefacts
            .iter()
            .any(|artefact| artefact.kind == RuntimeMetadataBlobType::SubagentTranscript),
        "subagent transcript artefact should be imported"
    );
    assert!(
        artefacts.iter().any(|artefact| {
            artefact.kind == RuntimeMetadataBlobType::IncrementalCheckpoint
                && artefact.incremental_sequence == Some(3)
        }),
        "incremental checkpoint artefact should be imported with its sequence"
    );

    assert!(
        !session_dir.exists(),
        "legacy metadata directory should be removed after successful import"
    );
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

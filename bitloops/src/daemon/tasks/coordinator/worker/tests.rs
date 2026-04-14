use std::sync::{Arc, Mutex};

use tempfile::TempDir;
use tokio::sync::Notify;

use crate::config::resolve_store_backend_config_for_repo;
use crate::devql_transport::{RepoPathRegistry, RepoPathRegistryEntry, persist_repo_path_registry};
use crate::host::devql::{DevqlConfig, RelationalStorage};
use crate::host::runtime_store::DaemonSqliteRuntimeStore;

use super::super::DevqlTaskCoordinator;
use super::reconcile::persist_scope_exclusions_fingerprint;

fn coordinator(temp: &TempDir) -> DevqlTaskCoordinator {
    DevqlTaskCoordinator {
        runtime_store: DaemonSqliteRuntimeStore::open_at(temp.path().join("daemon-runtime.sqlite"))
            .expect("open daemon runtime store"),
        lock: Mutex::new(()),
        notify: Notify::new(),
        worker_started: std::sync::atomic::AtomicBool::new(false),
        subscription_hub: Mutex::new(None),
    }
}

#[test]
fn scope_exclusion_reconcile_skips_non_repo_config_root_and_keeps_matching_registry_repos() {
    let temp = TempDir::new().expect("temp dir");
    let config_root = temp.path().join("config-root");
    let repo_root = temp.path().join("repo-a");
    let other_config_root = temp.path().join("other-config-root");
    let other_repo_root = temp.path().join("repo-b");
    std::fs::create_dir_all(&config_root).expect("create config root");
    std::fs::create_dir_all(&other_config_root).expect("create other config root");
    std::fs::create_dir_all(&repo_root).expect("create repo root");
    std::fs::create_dir_all(&other_repo_root).expect("create other repo root");

    crate::test_support::git_fixtures::init_test_repo(
        &repo_root,
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );
    crate::test_support::git_fixtures::init_test_repo(
        &other_repo_root,
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );

    let config_path = crate::test_support::git_fixtures::write_test_daemon_config(&config_root);
    let other_config_path =
        crate::test_support::git_fixtures::write_test_daemon_config(&other_config_root);
    crate::config::settings::write_repo_daemon_binding(
        &repo_root.join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
        &config_path,
    )
    .expect("bind repo root to config");
    crate::config::settings::write_repo_daemon_binding(
        &other_repo_root.join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
        &other_config_path,
    )
    .expect("bind other repo root to config");

    let repo = crate::host::devql::resolve_repo_identity(&repo_root).expect("resolve repo");
    let other_repo =
        crate::host::devql::resolve_repo_identity(&other_repo_root).expect("resolve other repo");
    let registry_path = temp.path().join("repo-registry.json");
    persist_repo_path_registry(
        &registry_path,
        &RepoPathRegistry {
            version: 1,
            entries: vec![
                RepoPathRegistryEntry {
                    repo_id: repo.repo_id,
                    provider: repo.provider,
                    organisation: repo.organization,
                    name: repo.name,
                    identity: repo.identity,
                    repo_root: repo_root.clone(),
                    last_branch: Some("main".to_string()),
                    git_dir_relative_path: Some(".git".to_string()),
                    updated_at_unix: 1,
                },
                RepoPathRegistryEntry {
                    repo_id: other_repo.repo_id,
                    provider: other_repo.provider,
                    organisation: other_repo.organization,
                    name: other_repo.name,
                    identity: other_repo.identity,
                    repo_root: other_repo_root.clone(),
                    last_branch: Some("main".to_string()),
                    git_dir_relative_path: Some(".git".to_string()),
                    updated_at_unix: 2,
                },
            ],
        },
    )
    .expect("persist repo registry");

    let coordinator = coordinator(&temp);
    let repo_roots = coordinator
        .scope_exclusion_reconcile_repo_roots(&config_root, Some(&registry_path))
        .expect("resolve repo roots");

    assert_eq!(
        repo_roots,
        vec![repo_root.canonicalize().expect("canonicalise repo root")]
    );
}

#[test]
fn scope_exclusion_reconcile_keeps_repo_local_config_root() {
    let temp = TempDir::new().expect("temp dir");
    let repo_root = temp.path().join("repo");
    std::fs::create_dir_all(&repo_root).expect("create repo root");
    crate::test_support::git_fixtures::init_test_repo(
        &repo_root,
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );

    let coordinator = coordinator(&temp);
    let repo_roots = coordinator
        .scope_exclusion_reconcile_repo_roots(&repo_root, None)
        .expect("resolve repo roots");

    assert_eq!(
        repo_roots,
        vec![repo_root.canonicalize().expect("canonicalise repo root")]
    );
}

#[tokio::test]
async fn persist_scope_exclusions_fingerprint_writes_current_fingerprint_for_regular_syncs() {
    let temp = TempDir::new().expect("temp dir");
    let config_root = temp.path().join("config-root");
    let repo_root = temp.path().join("repo");
    std::fs::create_dir_all(&config_root).expect("create config root");
    std::fs::create_dir_all(&repo_root).expect("create repo root");
    crate::test_support::git_fixtures::init_test_repo(
        &repo_root,
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );

    let config_path = crate::test_support::git_fixtures::write_test_daemon_config(&config_root);
    crate::config::settings::write_repo_daemon_binding(
        &repo_root.join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
        &config_path,
    )
    .expect("bind repo root to config");

    let repo = crate::host::devql::resolve_repo_identity(&repo_root).expect("resolve repo");
    let cfg = DevqlConfig::from_roots(config_root.clone(), repo_root.clone(), repo)
        .expect("build devql config");
    let backends = resolve_store_backend_config_for_repo(&cfg.daemon_config_root)
        .expect("resolve store backends");
    let sqlite_path = backends
        .relational
        .resolve_sqlite_db_path_for_repo(&cfg.repo_root)
        .expect("resolve sqlite path");
    std::fs::create_dir_all(
        sqlite_path
            .parent()
            .expect("sqlite path should have a parent directory"),
    )
    .expect("create sqlite parent directory");
    let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite db");
    conn.execute_batch(
        "CREATE TABLE repo_sync_state (
            repo_id TEXT PRIMARY KEY,
            repo_root TEXT,
            active_branch TEXT,
            head_commit_sha TEXT,
            head_tree_sha TEXT,
            parser_version TEXT,
            extractor_version TEXT,
            last_sync_started_at TEXT,
            last_sync_completed_at TEXT,
            last_sync_status TEXT,
            last_sync_reason TEXT,
            scope_exclusions_fingerprint TEXT
        );",
    )
    .expect("create repo_sync_state table");
    drop(conn);
    let relational = RelationalStorage::local_only(sqlite_path);
    crate::host::devql::sync::state::write_sync_started(
        &relational,
        &cfg.repo.repo_id,
        cfg.repo_root.to_string_lossy().as_ref(),
        "full",
        "parser-v1",
        "extractor-v1",
    )
    .await
    .expect("write sync state");

    persist_scope_exclusions_fingerprint(&cfg, None, None)
        .await
        .expect("persist scope exclusion fingerprint");

    let stored = crate::host::devql::sync::state::read_scope_exclusions_fingerprint(
        &relational,
        &cfg.repo.repo_id,
    )
    .await
    .expect("read scope exclusion fingerprint");
    let expected = crate::host::devql::current_scope_exclusions_fingerprint(&cfg.repo_root)
        .expect("load current scope exclusions fingerprint");
    assert_eq!(stored.as_deref(), Some(expected.as_str()));
}

#[tokio::test]
async fn ensure_scope_exclusion_reconcile_for_repo_skips_first_run_repo_without_sync_state() {
    let temp = TempDir::new().expect("temp dir");
    let config_root = temp.path().join("config-root");
    let repo_root = temp.path().join("repo");
    std::fs::create_dir_all(&config_root).expect("create config root");
    std::fs::create_dir_all(&repo_root).expect("create repo root");
    crate::test_support::git_fixtures::init_test_repo(
        &repo_root,
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );

    let config_path = crate::test_support::git_fixtures::write_test_daemon_config(&config_root);
    crate::config::settings::write_repo_daemon_binding(
        &repo_root.join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
        &config_path,
    )
    .expect("bind repo root to config");

    let repo = crate::host::devql::resolve_repo_identity(&repo_root).expect("resolve repo");
    let cfg = DevqlConfig::from_roots(config_root.clone(), repo_root.clone(), repo)
        .expect("build devql config");
    let backends = resolve_store_backend_config_for_repo(&cfg.daemon_config_root)
        .expect("resolve store backends");
    let sqlite_path = backends
        .relational
        .resolve_sqlite_db_path_for_repo(&cfg.repo_root)
        .expect("resolve sqlite path");
    std::fs::create_dir_all(
        sqlite_path
            .parent()
            .expect("sqlite path should have a parent directory"),
    )
    .expect("create sqlite parent directory");
    let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite db");
    conn.execute_batch(
        "CREATE TABLE repo_sync_state (
            repo_id TEXT PRIMARY KEY,
            repo_root TEXT,
            active_branch TEXT,
            head_commit_sha TEXT,
            head_tree_sha TEXT,
            parser_version TEXT,
            extractor_version TEXT,
            last_sync_started_at TEXT,
            last_sync_completed_at TEXT,
            last_sync_status TEXT,
            last_sync_reason TEXT,
            scope_exclusions_fingerprint TEXT
        );",
    )
    .expect("create repo_sync_state table");

    let coordinator = Arc::new(coordinator(&temp));
    let blocked = coordinator
        .ensure_scope_exclusion_reconcile_for_repo(&repo_root)
        .await
        .expect("check first-run exclusion reconcile");

    assert!(
        !blocked,
        "first-run repos should not be blocked by an exclusion reconcile"
    );
    let tasks = coordinator
        .tasks(None, None, None, None)
        .expect("load queued tasks");
    assert!(
        tasks.is_empty(),
        "first-run repos should not enqueue a repo-policy sync before the first normal sync"
    );
}

#[tokio::test]
async fn ensure_scope_exclusion_reconcile_for_repo_skips_running_sync_without_fingerprint() {
    let temp = TempDir::new().expect("temp dir");
    let config_root = temp.path().join("config-root");
    let repo_root = temp.path().join("repo");
    std::fs::create_dir_all(&config_root).expect("create config root");
    std::fs::create_dir_all(&repo_root).expect("create repo root");
    crate::test_support::git_fixtures::init_test_repo(
        &repo_root,
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );

    let config_path = crate::test_support::git_fixtures::write_test_daemon_config(&config_root);
    crate::config::settings::write_repo_daemon_binding(
        &repo_root.join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
        &config_path,
    )
    .expect("bind repo root to config");

    let repo = crate::host::devql::resolve_repo_identity(&repo_root).expect("resolve repo");
    let cfg = DevqlConfig::from_roots(config_root.clone(), repo_root.clone(), repo)
        .expect("build devql config");
    let backends = resolve_store_backend_config_for_repo(&cfg.daemon_config_root)
        .expect("resolve store backends");
    let sqlite_path = backends
        .relational
        .resolve_sqlite_db_path_for_repo(&cfg.repo_root)
        .expect("resolve sqlite path");
    std::fs::create_dir_all(
        sqlite_path
            .parent()
            .expect("sqlite path should have a parent directory"),
    )
    .expect("create sqlite parent directory");
    let conn = rusqlite::Connection::open(&sqlite_path).expect("open sqlite db");
    conn.execute_batch(
        "CREATE TABLE repo_sync_state (
            repo_id TEXT PRIMARY KEY,
            repo_root TEXT,
            active_branch TEXT,
            head_commit_sha TEXT,
            head_tree_sha TEXT,
            parser_version TEXT,
            extractor_version TEXT,
            last_sync_started_at TEXT,
            last_sync_completed_at TEXT,
            last_sync_status TEXT,
            last_sync_reason TEXT,
            scope_exclusions_fingerprint TEXT
        );",
    )
    .expect("create repo_sync_state table");
    drop(conn);
    let relational = RelationalStorage::local_only(sqlite_path);
    crate::host::devql::sync::state::write_sync_started(
        &relational,
        &cfg.repo.repo_id,
        cfg.repo_root.to_string_lossy().as_ref(),
        "full",
        "parser-v1",
        "extractor-v1",
    )
    .await
    .expect("write running sync state");

    let coordinator = Arc::new(coordinator(&temp));
    let blocked = coordinator
        .ensure_scope_exclusion_reconcile_for_repo(&repo_root)
        .await
        .expect("check running-sync exclusion reconcile");

    assert!(
        !blocked,
        "repos with an in-flight sync and no stored fingerprint should not enqueue a blocking repo-policy sync"
    );
    let tasks = coordinator
        .tasks(None, None, None, None)
        .expect("load queued tasks");
    assert!(
        tasks.is_empty(),
        "running syncs should not have a hidden repo-policy sync queued behind them"
    );
}

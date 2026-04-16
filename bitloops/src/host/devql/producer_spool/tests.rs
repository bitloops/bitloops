use std::path::PathBuf;

use tempfile::TempDir;

use super::*;
use crate::config::REPO_POLICY_LOCAL_FILE_NAME;
use crate::daemon::SyncTaskMode;
use crate::host::runtime_store::RepoSqliteRuntimeStore;
use crate::test_support::git_fixtures::{init_test_repo, write_test_daemon_config};

fn seed_store() -> (
    TempDir,
    PathBuf,
    crate::host::devql::RepoIdentity,
    RepoSqliteRuntimeStore,
) {
    let dir = TempDir::new().expect("temp dir");
    let repo_root = dir.path().join("repo");
    std::fs::create_dir_all(&repo_root).expect("create repo dir");
    init_test_repo(
        &repo_root,
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );
    let config_path = write_test_daemon_config(dir.path());
    crate::config::settings::write_repo_daemon_binding(
        &repo_root.join(REPO_POLICY_LOCAL_FILE_NAME),
        &config_path,
    )
    .expect("write repo daemon binding");
    let repo = crate::host::devql::resolve_repo_identity(&repo_root).expect("resolve repo");
    let store = RepoSqliteRuntimeStore::open_for_roots(dir.path(), &repo_root)
        .expect("open repo runtime store");
    (dir, repo_root, repo, store)
}

#[test]
fn spooled_sync_paths_merge_into_existing_pending_job() {
    let (_dir, repo_root, repo, store) = seed_store();
    let cfg = crate::host::devql::DevqlConfig::from_roots(
        store.config_root.clone(),
        repo_root.clone(),
        repo.clone(),
    )
    .expect("build devql config");

    enqueue_spooled_sync_task(
        &cfg,
        DevqlTaskSource::Watcher,
        crate::host::devql::SyncMode::Paths(vec!["src/b.ts".to_string(), "src/a.ts".to_string()]),
    )
    .expect("enqueue first watcher sync");
    enqueue_spooled_sync_task(
        &cfg,
        DevqlTaskSource::Watcher,
        crate::host::devql::SyncMode::Paths(vec!["src/c.ts".to_string(), "src/a.ts".to_string()]),
    )
    .expect("enqueue second watcher sync");

    let claimed =
        claim_next_producer_spool_jobs(&store.config_root).expect("claim producer spool jobs");
    assert_eq!(claimed.len(), 1, "watcher path jobs should coalesce");
    match &claimed[0].payload {
        ProducerSpoolJobPayload::Task {
            source,
            spec:
                DevqlTaskSpec::Sync(crate::daemon::SyncTaskSpec {
                    mode: SyncTaskMode::Paths { paths },
                    ..
                }),
        } => {
            assert_eq!(*source, DevqlTaskSource::Watcher);
            assert_eq!(
                paths,
                &vec![
                    "src/a.ts".to_string(),
                    "src/b.ts".to_string(),
                    "src/c.ts".to_string(),
                ]
            );
        }
        other => panic!("unexpected payload: {other:?}"),
    }
}

#[test]
fn producer_spool_claims_at_most_one_running_job_per_repo() {
    let (_dir, repo_root, repo, store) = seed_store();
    let cfg = crate::host::devql::DevqlConfig::from_roots(
        store.config_root.clone(),
        repo_root.clone(),
        repo.clone(),
    )
    .expect("build devql config");

    enqueue_spooled_sync_task(
        &cfg,
        DevqlTaskSource::Watcher,
        crate::host::devql::SyncMode::Paths(vec!["src/a.ts".to_string()]),
    )
    .expect("enqueue first sync");
    enqueue_spooled_post_commit_refresh(&repo_root, "commit-a", &["src/a.ts".to_string()])
        .expect("enqueue post-commit refresh");

    let first = claim_next_producer_spool_jobs(&store.config_root)
        .expect("claim first producer spool batch");
    assert_eq!(first.len(), 1, "only one producer job should run per repo");

    let second = claim_next_producer_spool_jobs(&store.config_root)
        .expect("claim second producer spool batch");
    assert!(
        second.is_empty(),
        "pending jobs for the same repo should wait until the running job finishes"
    );
}

#[test]
fn running_producer_spool_jobs_recover_back_to_pending() {
    let (_dir, repo_root, repo, store) = seed_store();
    let cfg = crate::host::devql::DevqlConfig::from_roots(
        store.config_root.clone(),
        repo_root.clone(),
        repo.clone(),
    )
    .expect("build devql config");

    enqueue_spooled_sync_task(
        &cfg,
        DevqlTaskSource::Watcher,
        crate::host::devql::SyncMode::Paths(vec!["src/a.ts".to_string()]),
    )
    .expect("enqueue sync");
    let claimed =
        claim_next_producer_spool_jobs(&store.config_root).expect("claim producer spool job");
    assert_eq!(claimed.len(), 1);

    recover_running_producer_spool_jobs(&store.config_root)
        .expect("recover running producer spool jobs");
    let reclaimed = claim_next_producer_spool_jobs(&store.config_root)
        .expect("reclaim recovered producer spool job");
    assert_eq!(
        reclaimed.len(),
        1,
        "recovered job should be claimable again"
    );
}

#[test]
fn hook_enqueue_helpers_use_repo_binding_and_share_repo_runtime_store() {
    let (_dir, repo_root, _repo, store) = seed_store();

    crate::host::devql::enqueue_spooled_sync_task_for_repo_root(
        &repo_root,
        DevqlTaskSource::PostCheckout,
        crate::host::devql::SyncMode::Full,
    )
    .expect("enqueue post-checkout sync");
    crate::host::devql::enqueue_spooled_post_commit_refresh(
        &repo_root,
        "commit-head",
        &["src/lib.rs".to_string()],
    )
    .expect("enqueue post-commit refresh");
    crate::host::devql::enqueue_spooled_post_merge_refresh(
        &repo_root,
        "merge-head",
        &["src/lib.rs".to_string()],
    )
    .expect("enqueue post-merge refresh");
    crate::host::devql::enqueue_spooled_pre_push_sync(
        &repo_root,
        "origin",
        &["refs/heads/main abc refs/heads/main def".to_string()],
    )
    .expect("enqueue pre-push sync");

    let sqlite = store
        .connect_repo_sqlite()
        .expect("open repo runtime sqlite");
    let queued_rows = sqlite
        .with_connection(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM devql_producer_spool_jobs WHERE repo_id = ?1",
                rusqlite::params![store.repo_id()],
                |row| row.get::<_, i64>(0),
            )
            .map_err(anyhow::Error::from)
        })
        .expect("count queued producer spool jobs");
    assert_eq!(
        queued_rows, 4,
        "helper enqueues should target the bound repo runtime db"
    );
}

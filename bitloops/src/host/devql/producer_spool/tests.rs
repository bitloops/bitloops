use std::collections::HashSet;
use std::path::PathBuf;

use tempfile::TempDir;

use super::*;
use crate::config::REPO_POLICY_LOCAL_FILE_NAME;
use crate::daemon::{DevqlTaskKind, DevqlTaskSource, DevqlTaskSpec, SyncTaskMode};
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
fn spooled_ingest_task_preserves_explicit_commits() {
    let (_dir, repo_root, _repo, store) = seed_store();

    enqueue_spooled_ingest_task_for_repo_root(
        &repo_root,
        DevqlTaskSource::PostCommit,
        crate::daemon::IngestTaskSpec {
            commits: vec!["commit-a".to_string()],
            backfill: None,
        },
    )
    .expect("enqueue post-commit ingest task");

    let claimed =
        claim_next_producer_spool_jobs(&store.config_root).expect("claim producer spool jobs");
    assert_eq!(claimed.len(), 1, "expected one ingest producer job");
    match &claimed[0].payload {
        ProducerSpoolJobPayload::Task {
            source,
            spec: DevqlTaskSpec::Ingest(spec),
        } => {
            assert_eq!(*source, DevqlTaskSource::PostCommit);
            assert_eq!(spec.commits, vec!["commit-a".to_string()]);
            assert_eq!(spec.backfill, None);
        }
        other => panic!("unexpected payload: {other:?}"),
    }
}

#[test]
fn spooled_ingest_task_dedupe_key_preserves_explicit_commit_order() {
    let (_dir, repo_root, _repo, store) = seed_store();

    enqueue_spooled_ingest_task_for_repo_root(
        &repo_root,
        DevqlTaskSource::PostCommit,
        crate::daemon::IngestTaskSpec {
            commits: vec!["commit-b".to_string(), "commit-a".to_string()],
            backfill: None,
        },
    )
    .expect("enqueue first post-commit ingest task");
    enqueue_spooled_ingest_task_for_repo_root(
        &repo_root,
        DevqlTaskSource::PostCommit,
        crate::daemon::IngestTaskSpec {
            commits: vec!["commit-a".to_string(), "commit-b".to_string()],
            backfill: None,
        },
    )
    .expect("enqueue second post-commit ingest task");

    let jobs = list_recent_producer_spool_jobs(&store.config_root, store.repo_id(), 10)
        .expect("list producer spool jobs");
    let dedupe_keys = jobs
        .iter()
        .map(|job| job.dedupe_key.as_deref())
        .collect::<HashSet<_>>();

    assert_eq!(
        jobs.len(),
        2,
        "commit order should be part of the dedupe key"
    );
    assert!(dedupe_keys.contains(&Some("task:post_commit:ingest:commits:commit-b,commit-a")));
    assert!(dedupe_keys.contains(&Some("task:post_commit:ingest:commits:commit-a,commit-b")));
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
fn running_producer_spool_repo_ids_returns_running_repos() {
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

    let running =
        running_producer_spool_repo_ids(&store.config_root).expect("load running repo ids");

    assert_eq!(running, HashSet::from([store.repo_id().to_string()]));
}

#[test]
fn producer_spool_claim_allows_spooled_sync_while_ingest_task_running() {
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

    let running_tasks = vec![ProducerSpoolRunningTask::new(
        store.repo_id(),
        DevqlTaskKind::Ingest,
        DevqlTaskSource::PostMerge,
    )];
    let claimed = claim_next_producer_spool_jobs_excluding(
        &store.config_root,
        &HashSet::new(),
        &running_tasks,
        &PostCommitDerivationClaimGuards::default(),
    )
    .expect("claim producer spool jobs with running ingest");

    assert_eq!(
        claimed.len(),
        1,
        "watcher sync should be claimable while same-repo ingest is running"
    );
}

#[test]
fn producer_spool_claim_blocks_spooled_sync_while_sync_task_running() {
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

    let running_tasks = vec![ProducerSpoolRunningTask::new(
        store.repo_id(),
        DevqlTaskKind::Sync,
        DevqlTaskSource::Watcher,
    )];
    let blocked = claim_next_producer_spool_jobs_excluding(
        &store.config_root,
        &HashSet::new(),
        &running_tasks,
        &PostCommitDerivationClaimGuards::default(),
    )
    .expect("claim producer spool jobs with running sync");

    assert!(
        blocked.is_empty(),
        "spooled sync should wait while same-repo sync lane is running"
    );

    let unblocked =
        claim_next_producer_spool_jobs(&store.config_root).expect("claim unblocked job");
    assert_eq!(unblocked.len(), 1);
}

#[test]
fn producer_spool_claim_allows_spooled_ingest_while_sync_task_running() {
    let (_dir, repo_root, _repo, store) = seed_store();

    enqueue_spooled_ingest_task_for_repo_root(
        &repo_root,
        DevqlTaskSource::PostCommit,
        crate::daemon::IngestTaskSpec {
            commits: vec!["commit-a".to_string()],
            backfill: None,
        },
    )
    .expect("enqueue ingest");

    let running_tasks = vec![ProducerSpoolRunningTask::new(
        store.repo_id(),
        DevqlTaskKind::Sync,
        DevqlTaskSource::PostCommit,
    )];
    let claimed = claim_next_producer_spool_jobs_excluding(
        &store.config_root,
        &HashSet::new(),
        &running_tasks,
        &PostCommitDerivationClaimGuards::default(),
    )
    .expect("claim producer spool jobs with running sync");

    assert_eq!(
        claimed.len(),
        1,
        "spooled ingest should be claimable while same-repo sync is running"
    );
}

#[test]
fn producer_spool_claim_excludes_explicitly_blocked_repo_ids() {
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

    let blocked_repo_ids = HashSet::from([store.repo_id().to_string()]);
    let blocked_claim =
        claim_next_producer_spool_jobs_excluding_repo_ids(&store.config_root, &blocked_repo_ids)
            .expect("claim producer spool jobs with blocked repo");
    assert!(
        blocked_claim.is_empty(),
        "producer spool jobs should wait while a DevQL task is running for the same repo"
    );

    let unblocked_claim =
        claim_next_producer_spool_jobs(&store.config_root).expect("claim unblocked job");
    assert_eq!(
        unblocked_claim.len(),
        1,
        "blocked jobs should remain pending for a later unblocked claim"
    );
}

#[test]
fn producer_spool_claim_skips_blocked_post_commit_derivation_until_unblocked() {
    let (_dir, repo_root, _repo, store) = seed_store();
    let now = super::storage::unix_timestamp_now();
    let sqlite = store
        .connect_repo_sqlite()
        .expect("open repo runtime sqlite");
    sqlite
        .with_connection(|conn| {
            for (job_id, payload) in [
                (
                    "producer-spool-job-a-derivation",
                    ProducerSpoolJobPayload::PostCommitDerivation {
                        commit_sha: "commit-head".to_string(),
                        committed_files: vec!["src/lib.rs".to_string()],
                        is_rebase_in_progress: false,
                    },
                ),
                (
                    "producer-spool-job-b-refresh",
                    ProducerSpoolJobPayload::PostCommitRefresh {
                        commit_sha: "commit-head".to_string(),
                        changed_files: vec!["src/lib.rs".to_string()],
                    },
                ),
            ] {
                conn.execute(
                    "INSERT INTO devql_producer_spool_jobs (
                        job_id, repo_id, repo_root, config_root, repo_name, repo_provider,
                        repo_organisation, repo_identity, dedupe_key, payload, status, attempts,
                        available_at_unix, submitted_at_unix, updated_at_unix, last_error
                     ) VALUES (
                        ?1, ?2, ?3, ?4, 'repo', 'local',
                        'local', 'repo', NULL, ?5, ?6, 0,
                        ?7, ?8, ?9, NULL
                     )",
                    rusqlite::params![
                        job_id,
                        store.repo_id(),
                        repo_root.to_string_lossy().to_string(),
                        store.config_root.to_string_lossy().to_string(),
                        serde_json::to_string(&payload).expect("serialize payload"),
                        ProducerSpoolJobStatus::Pending.as_str(),
                        i64::try_from(now).expect("now fits i64"),
                        i64::try_from(now).expect("now fits i64"),
                        i64::try_from(now).expect("now fits i64"),
                    ],
                )
                .expect("insert producer spool job");
            }
            Ok(())
        })
        .expect("seed producer jobs");

    let blocked_derivations = PostCommitDerivationClaimGuards {
        blocked: HashSet::from([(store.repo_id().to_string(), "commit-head".to_string())]),
        abandoned: HashSet::new(),
    };
    let blocked_claim = claim_next_producer_spool_jobs_excluding(
        &store.config_root,
        &HashSet::new(),
        &[],
        &blocked_derivations,
    )
    .expect("claim producer spool jobs with blocked derivation");
    assert_eq!(blocked_claim.len(), 1);
    assert!(
        matches!(
            blocked_claim[0].payload,
            ProducerSpoolJobPayload::PostCommitRefresh { .. }
        ),
        "blocked derivation should remain pending while other jobs can claim"
    );

    delete_producer_spool_job(&store.config_root, &blocked_claim[0].job_id)
        .expect("delete claimed refresh");
    let still_blocked = claim_next_producer_spool_jobs_excluding(
        &store.config_root,
        &HashSet::new(),
        &[],
        &blocked_derivations,
    )
    .expect("claim producer spool jobs while derivation remains blocked");
    assert!(
        still_blocked.is_empty(),
        "matching derivation should not be claimed while blocked"
    );

    let unblocked =
        claim_next_producer_spool_jobs(&store.config_root).expect("claim unblocked derivation");
    assert_eq!(unblocked.len(), 1);
    assert!(
        matches!(
            unblocked[0].payload,
            ProducerSpoolJobPayload::PostCommitDerivation { .. }
        ),
        "derivation should become claimable once the matching sync task is completed"
    );
}

#[test]
fn producer_spool_claim_deletes_abandoned_post_commit_derivation() {
    let (_dir, repo_root, _repo, store) = seed_store();
    crate::host::devql::enqueue_spooled_post_commit_derivation(
        &repo_root,
        "commit-head",
        &["src/lib.rs".to_string()],
        false,
    )
    .expect("enqueue post-commit derivation");

    let guards = PostCommitDerivationClaimGuards {
        blocked: HashSet::new(),
        abandoned: HashSet::from([(store.repo_id().to_string(), "commit-head".to_string())]),
    };
    let abandoned_claim =
        claim_next_producer_spool_jobs_excluding(&store.config_root, &HashSet::new(), &[], &guards)
            .expect("claim producer spool jobs with abandoned derivation");
    assert!(
        abandoned_claim.is_empty(),
        "abandoned derivation should not be claimed"
    );

    let later_claim =
        claim_next_producer_spool_jobs(&store.config_root).expect("claim producer spool jobs");
    assert!(
        later_claim.is_empty(),
        "abandoned derivation should be deleted instead of becoming claimable later"
    );
}

#[test]
fn post_commit_refresh_claims_before_derivation_with_same_timestamp() {
    let (_dir, repo_root, _repo, store) = seed_store();
    let now = super::storage::unix_timestamp_now();
    let sqlite = store
        .connect_repo_sqlite()
        .expect("open repo runtime sqlite");
    sqlite
        .with_connection(|conn| {
            for (job_id, payload) in [
                (
                    "producer-spool-job-a-derivation",
                    ProducerSpoolJobPayload::PostCommitDerivation {
                        commit_sha: "commit-head".to_string(),
                        committed_files: vec!["src/lib.rs".to_string()],
                        is_rebase_in_progress: false,
                    },
                ),
                (
                    "producer-spool-job-z-refresh",
                    ProducerSpoolJobPayload::PostCommitRefresh {
                        commit_sha: "commit-head".to_string(),
                        changed_files: vec!["src/lib.rs".to_string()],
                    },
                ),
            ] {
                conn.execute(
                    "INSERT INTO devql_producer_spool_jobs (
                        job_id, repo_id, repo_root, config_root, repo_name, repo_provider,
                        repo_organisation, repo_identity, dedupe_key, payload, status, attempts,
                        available_at_unix, submitted_at_unix, updated_at_unix, last_error
                     ) VALUES (
                        ?1, ?2, ?3, ?4, 'repo', 'local',
                        'local', 'repo', NULL, ?5, ?6, 0,
                        ?7, ?8, ?9, NULL
                     )",
                    rusqlite::params![
                        job_id,
                        store.repo_id(),
                        repo_root.to_string_lossy().to_string(),
                        store.config_root.to_string_lossy().to_string(),
                        serde_json::to_string(&payload).expect("serialize payload"),
                        ProducerSpoolJobStatus::Pending.as_str(),
                        i64::try_from(now).expect("now fits i64"),
                        i64::try_from(now).expect("now fits i64"),
                        i64::try_from(now).expect("now fits i64"),
                    ],
                )
                .expect("insert producer spool job");
            }
            Ok(())
        })
        .expect("seed producer jobs");

    let claimed =
        claim_next_producer_spool_jobs(&store.config_root).expect("claim producer spool jobs");
    assert_eq!(claimed.len(), 1);
    assert!(
        matches!(
            claimed[0].payload,
            ProducerSpoolJobPayload::PostCommitRefresh { .. }
        ),
        "post-commit refresh must run before derivation when both jobs come from the same hook"
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
        false,
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
        queued_rows, 5,
        "helper enqueues should target the bound repo runtime db"
    );
}

#[test]
fn post_merge_enqueue_splits_sync_and_ingest_for_non_squash_merge() {
    let (_dir, repo_root, _repo, store) = seed_store();

    crate::host::devql::enqueue_spooled_post_merge_refresh(
        &repo_root,
        " merge-head ",
        &[
            "./src/lib.rs".to_string(),
            "src/main.rs".to_string(),
            "src/lib.rs".to_string(),
        ],
        false,
    )
    .expect("enqueue split post-merge refresh");

    let claimed = list_recent_producer_spool_jobs(&store.config_root, store.repo_id(), 10)
        .expect("list producer spool jobs");
    assert_eq!(
        claimed.len(),
        2,
        "non-squash post-merge should create two lane jobs"
    );

    let payloads = claimed.iter().map(|job| &job.payload).collect::<Vec<_>>();
    assert!(payloads.iter().any(|payload| matches!(
        payload,
        ProducerSpoolJobPayload::PostMergeSyncRefresh {
            merge_head_sha,
            changed_files,
            is_squash: false,
        } if merge_head_sha == "merge-head"
            && changed_files == &vec!["src/lib.rs".to_string(), "src/main.rs".to_string()]
    )));
    assert!(payloads.iter().any(|payload| matches!(
        payload,
        ProducerSpoolJobPayload::PostMergeIngestBackfill {
            merge_head_sha,
            is_squash: false,
        } if merge_head_sha == "merge-head"
    )));
}

#[test]
fn post_merge_enqueue_skips_ingest_for_squash_merge() {
    let (_dir, repo_root, _repo, store) = seed_store();

    crate::host::devql::enqueue_spooled_post_merge_refresh(
        &repo_root,
        "squash-head",
        &["src/lib.rs".to_string()],
        true,
    )
    .expect("enqueue squash post-merge refresh");

    let claimed = list_recent_producer_spool_jobs(&store.config_root, store.repo_id(), 10)
        .expect("list producer spool jobs");
    assert_eq!(
        claimed.len(),
        1,
        "squash post-merge should only create sync work"
    );
    match &claimed[0].payload {
        ProducerSpoolJobPayload::PostMergeSyncRefresh {
            merge_head_sha,
            changed_files,
            is_squash,
        } => {
            assert_eq!(merge_head_sha, "squash-head");
            assert_eq!(changed_files, &vec!["src/lib.rs".to_string()]);
            assert!(*is_squash);
        }
        other => panic!("unexpected squash post-merge payload: {other:?}"),
    }
}

#[test]
fn post_commit_derivation_enqueue_uses_repo_binding_and_preserves_payload() {
    let (_dir, repo_root, _repo, store) = seed_store();

    crate::host::devql::enqueue_spooled_post_commit_derivation(
        &repo_root,
        " commit-head ",
        &[
            "./src/lib.rs".to_string(),
            "src/main.rs".to_string(),
            "src/lib.rs".to_string(),
        ],
        true,
    )
    .expect("enqueue post-commit derivation");

    let claimed =
        claim_next_producer_spool_jobs(&store.config_root).expect("claim producer spool jobs");
    assert_eq!(claimed.len(), 1);
    assert_eq!(
        claimed[0].repo_id,
        store.repo_id(),
        "helper should target the bound repo runtime db"
    );
    match &claimed[0].payload {
        ProducerSpoolJobPayload::PostCommitDerivation {
            commit_sha,
            committed_files,
            is_rebase_in_progress,
        } => {
            assert_eq!(commit_sha, "commit-head");
            assert_eq!(
                committed_files,
                &vec!["src/lib.rs".to_string(), "src/main.rs".to_string()]
            );
            assert!(*is_rebase_in_progress);
        }
        other => panic!("unexpected payload: {other:?}"),
    }
}

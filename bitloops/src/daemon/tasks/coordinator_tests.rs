use super::helpers::{PROGRESS_PERSIST_INTERVAL, should_persist_embeddings_bootstrap_progress};
use super::*;
use crate::test_support::log_capture::capture_logs;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tempfile::TempDir;
use tokio::sync::Notify;

async fn run_all_producer_spool_jobs(
    coordinator: &Arc<DevqlTaskCoordinator>,
    config_root: &Path,
) -> anyhow::Result<usize> {
    let mut processed = 0;
    loop {
        let jobs = crate::host::devql::claim_next_producer_spool_jobs(config_root)?;
        if jobs.is_empty() {
            return Ok(processed);
        }
        for job in jobs {
            Arc::clone(coordinator).run_producer_spool_job(job).await?;
            processed += 1;
        }
    }
}

#[tokio::test]
async fn receive_embeddings_bootstrap_outcome_waits_for_result_after_progress_channel_closes() {
    let (progress_tx, progress_rx) = mpsc::unbounded_channel();
    let (result_tx, result_rx) = oneshot::channel();

    tokio::spawn(async move {
        progress_tx
            .send(EmbeddingsBootstrapProgress {
                phase: EmbeddingsBootstrapPhase::WarmingProfile,
                message: Some("warming".to_string()),
                ..Default::default()
            })
            .expect("send bootstrap progress");
        drop(progress_tx);
        tokio::task::yield_now().await;
        result_tx
            .send(Ok(EmbeddingsBootstrapResult {
                version: Some("v0.1.2".to_string()),
                binary_path: None,
                cache_dir: None,
                runtime_name: None,
                model_name: Some("local_code".to_string()),
                freshly_installed: false,
                message: "ok".to_string(),
            }))
            .expect("send bootstrap result");
    });

    let mut seen_phases = Vec::new();
    let outcome = receive_embeddings_bootstrap_outcome(progress_rx, result_rx, |progress| {
        seen_phases.push(progress.phase);
        Ok(())
    })
    .await
    .expect("receive bootstrap outcome")
    .expect("bootstrap result");

    assert_eq!(seen_phases, vec![EmbeddingsBootstrapPhase::WarmingProfile]);
    assert_eq!(outcome.message, "ok");
}

#[test]
fn embeddings_bootstrap_progress_persists_phase_changes_immediately() {
    let previous = EmbeddingsBootstrapProgress {
        phase: EmbeddingsBootstrapPhase::DownloadingRuntime,
        asset_name: Some("asset.tar.xz".to_string()),
        bytes_downloaded: 8,
        bytes_total: Some(16),
        version: Some("v0.1.2".to_string()),
        message: Some("Downloading".to_string()),
    };
    let update = EmbeddingsBootstrapProgress {
        phase: EmbeddingsBootstrapPhase::ExtractingRuntime,
        bytes_downloaded: 16,
        message: Some("Extracting".to_string()),
        ..previous.clone()
    };
    let persisted_at = Instant::now();

    assert!(should_persist_embeddings_bootstrap_progress(
        Some(&previous),
        &update,
        Some(persisted_at),
        persisted_at + Duration::from_millis(100),
    ));
}

#[test]
fn embeddings_bootstrap_progress_throttles_byte_only_updates() {
    let previous = EmbeddingsBootstrapProgress {
        phase: EmbeddingsBootstrapPhase::DownloadingRuntime,
        asset_name: Some("asset.tar.xz".to_string()),
        bytes_downloaded: 8,
        bytes_total: Some(16),
        version: Some("v0.1.2".to_string()),
        message: Some("Downloading".to_string()),
    };
    let update = EmbeddingsBootstrapProgress {
        bytes_downloaded: 12,
        ..previous.clone()
    };
    let persisted_at = Instant::now();

    assert!(!should_persist_embeddings_bootstrap_progress(
        Some(&previous),
        &update,
        Some(persisted_at),
        persisted_at + Duration::from_millis(100),
    ));
    assert!(should_persist_embeddings_bootstrap_progress(
        Some(&previous),
        &update,
        Some(persisted_at),
        persisted_at + PROGRESS_PERSIST_INTERVAL,
    ));
}

#[tokio::test]
async fn producer_spool_task_job_enqueues_devql_task_and_clears_spool_row() {
    let dir = TempDir::new().expect("temp dir");
    let config_root = dir.path().join("config");
    let repo_root = dir.path().join("repo");
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
    .expect("write repo daemon binding");

    let repo = crate::host::devql::resolve_repo_identity(&repo_root).expect("resolve repo");
    let cfg = DevqlConfig::from_roots(config_root.clone(), repo_root.clone(), repo.clone())
        .expect("build devql config");
    crate::host::devql::enqueue_spooled_sync_task(
        &cfg,
        DevqlTaskSource::Watcher,
        crate::host::devql::SyncMode::Paths(vec!["src/lib.rs".to_string()]),
    )
    .expect("enqueue watcher sync into producer spool");

    let jobs = crate::host::devql::claim_next_producer_spool_jobs(&config_root)
        .expect("claim producer spool jobs");
    assert_eq!(jobs.len(), 1, "expected one producer spool job");

    let coordinator = Arc::new(DevqlTaskCoordinator {
        runtime_store: DaemonSqliteRuntimeStore::open_at(dir.path().join("daemon-runtime.sqlite"))
            .expect("open daemon runtime store"),
        lock: Mutex::new(()),
        notify: Notify::new(),
        worker_started: AtomicBool::new(false),
        subscription_hub: Mutex::new(None),
    });

    Arc::clone(&coordinator)
        .run_producer_spool_job(jobs.into_iter().next().expect("producer spool job"))
        .await
        .expect("process producer spool job");

    let tasks = coordinator
        .tasks(
            Some(&cfg.repo.repo_id),
            Some(DevqlTaskKind::Sync),
            Some(DevqlTaskStatus::Queued),
            None,
        )
        .expect("load queued tasks");
    assert_eq!(
        tasks.len(),
        1,
        "producer spool task should enqueue one DevQL task"
    );
    assert_eq!(tasks[0].source, DevqlTaskSource::Watcher);
    assert_eq!(
        tasks[0].sync_spec().expect("sync task spec").mode,
        SyncTaskMode::Paths {
            paths: vec!["src/lib.rs".to_string()],
        }
    );

    let remaining_jobs = crate::host::devql::claim_next_producer_spool_jobs(&config_root)
        .expect("claim producer spool jobs after processing");
    assert!(
        remaining_jobs.is_empty(),
        "completed producer spool jobs should be removed from the repo runtime store"
    );
}

#[tokio::test]
async fn producer_spool_sync_task_job_skips_when_devql_sync_disabled() {
    let dir = TempDir::new().expect("temp dir");
    let config_root = dir.path().join("config");
    let repo_root = dir.path().join("repo");
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
    .expect("write repo daemon binding");
    crate::config::settings::set_devql_producer_settings(
        &repo_root.join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
        false,
        true,
    )
    .expect("disable producer sync");

    let repo = crate::host::devql::resolve_repo_identity(&repo_root).expect("resolve repo");
    let cfg = DevqlConfig::from_roots(config_root.clone(), repo_root.clone(), repo.clone())
        .expect("build devql config");
    crate::host::devql::enqueue_spooled_sync_task(
        &cfg,
        DevqlTaskSource::Watcher,
        crate::host::devql::SyncMode::Paths(vec!["src/lib.rs".to_string()]),
    )
    .expect("enqueue watcher sync into producer spool");

    let jobs = crate::host::devql::claim_next_producer_spool_jobs(&config_root)
        .expect("claim producer spool jobs");
    assert_eq!(jobs.len(), 1, "expected one producer spool job");

    let coordinator = Arc::new(DevqlTaskCoordinator {
        runtime_store: DaemonSqliteRuntimeStore::open_at(dir.path().join("daemon-runtime.sqlite"))
            .expect("open daemon runtime store"),
        lock: Mutex::new(()),
        notify: Notify::new(),
        worker_started: AtomicBool::new(false),
        subscription_hub: Mutex::new(None),
    });

    Arc::clone(&coordinator)
        .run_producer_spool_job(jobs.into_iter().next().expect("producer spool job"))
        .await
        .expect("process producer spool job");

    let tasks = coordinator
        .tasks(
            Some(&cfg.repo.repo_id),
            Some(DevqlTaskKind::Sync),
            Some(DevqlTaskStatus::Queued),
            None,
        )
        .expect("load queued tasks");
    assert!(
        tasks.is_empty(),
        "producer spool task must not enqueue sync when [devql].sync_enabled=false"
    );

    let remaining_jobs = crate::host::devql::claim_next_producer_spool_jobs(&config_root)
        .expect("claim producer spool jobs after processing");
    assert!(
        remaining_jobs.is_empty(),
        "policy-skipped producer spool jobs should be removed"
    );
}

#[tokio::test]
async fn post_merge_producer_spool_job_enqueues_visible_tasks_and_clears_spool_row() {
    let dir = TempDir::new().expect("temp dir");
    let config_root = dir.path().join("config");
    let repo_root = dir.path().join("repo");
    std::fs::create_dir_all(&config_root).expect("create config root");
    std::fs::create_dir_all(repo_root.join("src")).expect("create repo src dir");

    crate::test_support::git_fixtures::init_test_repo(
        &repo_root,
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );
    std::fs::write(
        repo_root.join("src/lib.rs"),
        "pub fn qa_merge() -> i32 { 7 }\n",
    )
    .expect("write source file");
    crate::test_support::git_fixtures::git_ok(&repo_root, &["add", "."]);
    crate::test_support::git_fixtures::git_ok(&repo_root, &["commit", "-m", "initial"]);
    let head_sha = crate::test_support::git_fixtures::git_ok(&repo_root, &["rev-parse", "HEAD"]);
    let config_path = crate::test_support::git_fixtures::write_test_daemon_config(&config_root);
    crate::config::settings::write_repo_daemon_binding(
        &repo_root.join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
        &config_path,
    )
    .expect("write repo daemon binding");

    let repo = crate::host::devql::resolve_repo_identity(&repo_root).expect("resolve repo");
    let cfg = DevqlConfig::from_roots(config_root.clone(), repo_root.clone(), repo.clone())
        .expect("build devql config");
    crate::host::devql::execute_init_schema(&cfg, "post-merge producer spool test")
        .await
        .expect("initialise devql schema");
    crate::host::devql::enqueue_spooled_post_merge_refresh(
        &repo_root,
        &head_sha,
        &["src/lib.rs".to_string()],
        false,
    )
    .expect("enqueue post-merge producer spool job");

    let coordinator = Arc::new(DevqlTaskCoordinator {
        runtime_store: DaemonSqliteRuntimeStore::open_at(dir.path().join("daemon-runtime.sqlite"))
            .expect("open daemon runtime store"),
        lock: Mutex::new(()),
        notify: Notify::new(),
        worker_started: AtomicBool::new(false),
        subscription_hub: Mutex::new(None),
    });

    let processed = run_all_producer_spool_jobs(&coordinator, &config_root)
        .await
        .expect("process producer spool jobs");
    assert_eq!(processed, 2, "expected split post-merge producer jobs");

    let ingest_tasks = coordinator
        .tasks(
            Some(&cfg.repo.repo_id),
            Some(DevqlTaskKind::Ingest),
            Some(DevqlTaskStatus::Queued),
            None,
        )
        .expect("load queued ingest tasks");
    assert_eq!(
        ingest_tasks.len(),
        1,
        "post-merge history catch-up should be a visible queued task"
    );
    let ingest_spec = ingest_tasks[0].ingest_spec().expect("ingest task spec");
    assert_eq!(ingest_spec.backfill, None);
    assert_eq!(
        ingest_spec.commits.last().map(String::as_str),
        Some(head_sha.as_str())
    );

    let sync_tasks = coordinator
        .tasks(
            Some(&cfg.repo.repo_id),
            Some(DevqlTaskKind::Sync),
            Some(DevqlTaskStatus::Queued),
            None,
        )
        .expect("load queued sync tasks");
    assert_eq!(
        sync_tasks.len(),
        1,
        "post-merge current-state refresh should be a visible queued task"
    );
    assert_eq!(sync_tasks[0].source, DevqlTaskSource::PostMerge);
    assert_eq!(
        sync_tasks[0].sync_spec().expect("sync task spec").mode,
        SyncTaskMode::Paths {
            paths: vec!["src/lib.rs".to_string()],
        }
    );

    let remaining_jobs = crate::host::devql::claim_next_producer_spool_jobs(&config_root)
        .expect("claim producer spool jobs after processing");
    assert!(
        remaining_jobs.is_empty(),
        "completed producer spool jobs should be removed from the repo runtime store"
    );
}

#[tokio::test]
async fn post_merge_ingest_backfill_uses_captured_merge_head_when_current_head_moves() {
    let dir = TempDir::new().expect("temp dir");
    let config_root = dir.path().join("config");
    let repo_root = dir.path().join("repo");
    std::fs::create_dir_all(&config_root).expect("create config root");
    std::fs::create_dir_all(repo_root.join("src")).expect("create repo src dir");

    crate::test_support::git_fixtures::init_test_repo(
        &repo_root,
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );
    std::fs::write(repo_root.join("src/lib.rs"), "pub fn base() {}\n").expect("write base file");
    crate::test_support::git_fixtures::git_ok(&repo_root, &["add", "."]);
    crate::test_support::git_fixtures::git_ok(&repo_root, &["commit", "-m", "base"]);

    crate::test_support::git_fixtures::git_ok(&repo_root, &["checkout", "-b", "feature"]);
    std::fs::write(repo_root.join("src/feature.rs"), "pub fn feature() {}\n")
        .expect("write feature file");
    crate::test_support::git_fixtures::git_ok(&repo_root, &["add", "."]);
    crate::test_support::git_fixtures::git_ok(&repo_root, &["commit", "-m", "feature"]);
    let merge_head_sha =
        crate::test_support::git_fixtures::git_ok(&repo_root, &["rev-parse", "HEAD"]);

    crate::test_support::git_fixtures::git_ok(&repo_root, &["checkout", "main"]);
    std::fs::write(
        repo_root.join("src/main_only.rs"),
        "pub fn main_only() {}\n",
    )
    .expect("write main-only file");
    crate::test_support::git_fixtures::git_ok(&repo_root, &["add", "."]);
    crate::test_support::git_fixtures::git_ok(&repo_root, &["commit", "-m", "main moved"]);

    let config_path = crate::test_support::git_fixtures::write_test_daemon_config(&config_root);
    crate::config::settings::write_repo_daemon_binding(
        &repo_root.join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
        &config_path,
    )
    .expect("write repo daemon binding");
    let repo = crate::host::devql::resolve_repo_identity(&repo_root).expect("resolve repo");
    let cfg = DevqlConfig::from_roots(config_root.clone(), repo_root.clone(), repo)
        .expect("build devql config");
    crate::host::devql::execute_init_schema(&cfg, "post-merge captured head test")
        .await
        .expect("initialise devql schema");

    crate::host::devql::enqueue_spooled_post_merge_refresh(
        &repo_root,
        &merge_head_sha,
        &["src/feature.rs".to_string()],
        false,
    )
    .expect("enqueue post-merge producer spool job");

    let coordinator = Arc::new(DevqlTaskCoordinator {
        runtime_store: DaemonSqliteRuntimeStore::open_at(dir.path().join("daemon-runtime.sqlite"))
            .expect("open daemon runtime store"),
        lock: Mutex::new(()),
        notify: Notify::new(),
        worker_started: AtomicBool::new(false),
        subscription_hub: Mutex::new(None),
    });

    let processed = run_all_producer_spool_jobs(&coordinator, &config_root)
        .await
        .expect("process producer spool jobs");
    assert_eq!(processed, 2, "expected split post-merge producer jobs");

    let ingest_tasks = coordinator
        .tasks(
            None,
            Some(DevqlTaskKind::Ingest),
            Some(DevqlTaskStatus::Queued),
            None,
        )
        .expect("load queued ingest tasks");
    assert_eq!(ingest_tasks.len(), 1);
    let spec = ingest_tasks[0].ingest_spec().expect("ingest spec");
    assert_eq!(
        spec.commits.last().map(String::as_str),
        Some(merge_head_sha.as_str())
    );
    assert_eq!(spec.backfill, None);
}

#[tokio::test]
async fn post_merge_producer_spool_job_skips_ingest_when_devql_ingest_disabled() {
    let dir = TempDir::new().expect("temp dir");
    let config_root = dir.path().join("config");
    let repo_root = dir.path().join("repo");
    std::fs::create_dir_all(&config_root).expect("create config root");
    std::fs::create_dir_all(repo_root.join("src")).expect("create repo src dir");

    crate::test_support::git_fixtures::init_test_repo(
        &repo_root,
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );
    std::fs::write(
        repo_root.join("src/lib.rs"),
        "pub fn qa_merge() -> i32 { 7 }\n",
    )
    .expect("write source file");
    crate::test_support::git_fixtures::git_ok(&repo_root, &["add", "."]);
    crate::test_support::git_fixtures::git_ok(&repo_root, &["commit", "-m", "initial"]);
    let head_sha = crate::test_support::git_fixtures::git_ok(&repo_root, &["rev-parse", "HEAD"]);
    let config_path = crate::test_support::git_fixtures::write_test_daemon_config(&config_root);
    crate::config::settings::write_repo_daemon_binding(
        &repo_root.join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
        &config_path,
    )
    .expect("write repo daemon binding");
    crate::config::settings::set_devql_producer_settings(
        &repo_root.join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
        true,
        false,
    )
    .expect("disable producer ingest");

    let repo = crate::host::devql::resolve_repo_identity(&repo_root).expect("resolve repo");
    let cfg = DevqlConfig::from_roots(config_root.clone(), repo_root.clone(), repo.clone())
        .expect("build devql config");
    crate::host::devql::enqueue_spooled_post_merge_refresh(
        &repo_root,
        &head_sha,
        &["src/lib.rs".to_string()],
        false,
    )
    .expect("enqueue post-merge producer spool job");

    let coordinator = Arc::new(DevqlTaskCoordinator {
        runtime_store: DaemonSqliteRuntimeStore::open_at(dir.path().join("daemon-runtime.sqlite"))
            .expect("open daemon runtime store"),
        lock: Mutex::new(()),
        notify: Notify::new(),
        worker_started: AtomicBool::new(false),
        subscription_hub: Mutex::new(None),
    });

    let processed = run_all_producer_spool_jobs(&coordinator, &config_root)
        .await
        .expect("process producer spool jobs");
    assert_eq!(processed, 2, "expected split post-merge producer jobs");

    let ingest_tasks = coordinator
        .tasks(
            Some(&cfg.repo.repo_id),
            Some(DevqlTaskKind::Ingest),
            Some(DevqlTaskStatus::Queued),
            None,
        )
        .expect("load queued ingest tasks");
    assert!(
        ingest_tasks.is_empty(),
        "post-merge must not enqueue ingest when [devql].ingest_enabled=false"
    );

    let sync_tasks = coordinator
        .tasks(
            Some(&cfg.repo.repo_id),
            Some(DevqlTaskKind::Sync),
            Some(DevqlTaskStatus::Queued),
            None,
        )
        .expect("load queued sync tasks");
    assert_eq!(
        sync_tasks.len(),
        1,
        "post-merge current-state refresh should still be queued"
    );
    assert_eq!(sync_tasks[0].source, DevqlTaskSource::PostMerge);
}

#[tokio::test]
async fn post_merge_producer_spool_job_skips_sync_when_devql_sync_disabled() {
    let dir = TempDir::new().expect("temp dir");
    let config_root = dir.path().join("config");
    let repo_root = dir.path().join("repo");
    std::fs::create_dir_all(&config_root).expect("create config root");
    std::fs::create_dir_all(repo_root.join("src")).expect("create repo src dir");

    crate::test_support::git_fixtures::init_test_repo(
        &repo_root,
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );
    std::fs::write(
        repo_root.join("src/lib.rs"),
        "pub fn qa_merge() -> i32 { 7 }\n",
    )
    .expect("write source file");
    crate::test_support::git_fixtures::git_ok(&repo_root, &["add", "."]);
    crate::test_support::git_fixtures::git_ok(&repo_root, &["commit", "-m", "initial"]);
    let head_sha = crate::test_support::git_fixtures::git_ok(&repo_root, &["rev-parse", "HEAD"]);
    let config_path = crate::test_support::git_fixtures::write_test_daemon_config(&config_root);
    crate::config::settings::write_repo_daemon_binding(
        &repo_root.join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
        &config_path,
    )
    .expect("write repo daemon binding");
    crate::config::settings::set_devql_producer_settings(
        &repo_root.join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
        false,
        true,
    )
    .expect("disable producer sync");

    let repo = crate::host::devql::resolve_repo_identity(&repo_root).expect("resolve repo");
    let cfg = DevqlConfig::from_roots(config_root.clone(), repo_root.clone(), repo.clone())
        .expect("build devql config");
    crate::host::devql::execute_init_schema(&cfg, "post-merge producer spool sync disabled test")
        .await
        .expect("initialise devql schema");
    crate::host::devql::enqueue_spooled_post_merge_refresh(
        &repo_root,
        &head_sha,
        &["src/lib.rs".to_string()],
        false,
    )
    .expect("enqueue post-merge producer spool job");

    let coordinator = Arc::new(DevqlTaskCoordinator {
        runtime_store: DaemonSqliteRuntimeStore::open_at(dir.path().join("daemon-runtime.sqlite"))
            .expect("open daemon runtime store"),
        lock: Mutex::new(()),
        notify: Notify::new(),
        worker_started: AtomicBool::new(false),
        subscription_hub: Mutex::new(None),
    });

    let processed = run_all_producer_spool_jobs(&coordinator, &config_root)
        .await
        .expect("process producer spool jobs");
    assert_eq!(processed, 2, "expected split post-merge producer jobs");

    let sync_tasks = coordinator
        .tasks(
            Some(&cfg.repo.repo_id),
            Some(DevqlTaskKind::Sync),
            Some(DevqlTaskStatus::Queued),
            None,
        )
        .expect("load queued sync tasks");
    assert!(
        sync_tasks.is_empty(),
        "post-merge must not enqueue sync when [devql].sync_enabled=false"
    );

    let ingest_tasks = coordinator
        .tasks(
            Some(&cfg.repo.repo_id),
            Some(DevqlTaskKind::Ingest),
            Some(DevqlTaskStatus::Queued),
            None,
        )
        .expect("load queued ingest tasks");
    assert_eq!(
        ingest_tasks.len(),
        1,
        "ingest_enabled=true should still allow post-merge ingest"
    );
}

#[tokio::test]
async fn post_commit_producer_spool_job_skips_sync_when_devql_sync_disabled() {
    let dir = TempDir::new().expect("temp dir");
    let config_root = dir.path().join("config");
    let repo_root = dir.path().join("repo");
    std::fs::create_dir_all(&config_root).expect("create config root");
    std::fs::create_dir_all(repo_root.join("src")).expect("create repo src dir");

    crate::test_support::git_fixtures::init_test_repo(
        &repo_root,
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );
    std::fs::write(repo_root.join("src/lib.rs"), "pub fn qa_commit() {}\n")
        .expect("write source file");
    crate::test_support::git_fixtures::git_ok(&repo_root, &["add", "."]);
    crate::test_support::git_fixtures::git_ok(&repo_root, &["commit", "-m", "initial"]);
    let head_sha = crate::test_support::git_fixtures::git_ok(&repo_root, &["rev-parse", "HEAD"]);
    let config_path = crate::test_support::git_fixtures::write_test_daemon_config(&config_root);
    crate::config::settings::write_repo_daemon_binding(
        &repo_root.join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
        &config_path,
    )
    .expect("write repo daemon binding");
    crate::config::settings::set_devql_producer_settings(
        &repo_root.join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
        false,
        true,
    )
    .expect("disable producer sync");

    crate::host::devql::enqueue_spooled_post_commit_refresh(
        &repo_root,
        &head_sha,
        &["src/lib.rs".to_string()],
    )
    .expect("enqueue post-commit producer spool job");

    let jobs = crate::host::devql::claim_next_producer_spool_jobs(&config_root)
        .expect("claim producer spool jobs");
    assert_eq!(jobs.len(), 1, "expected one post-commit producer job");
    std::fs::remove_dir_all(repo_root.join(".git")).expect("remove git dir after spooling");

    let coordinator = Arc::new(DevqlTaskCoordinator {
        runtime_store: DaemonSqliteRuntimeStore::open_at(dir.path().join("daemon-runtime.sqlite"))
            .expect("open daemon runtime store"),
        lock: Mutex::new(()),
        notify: Notify::new(),
        worker_started: AtomicBool::new(false),
        subscription_hub: Mutex::new(None),
    });

    Arc::clone(&coordinator)
        .run_producer_spool_job(jobs.into_iter().next().expect("producer spool job"))
        .await
        .expect("process producer spool job");

    let remaining_jobs = crate::host::devql::claim_next_producer_spool_jobs(&config_root)
        .expect("claim producer spool jobs after processing");
    assert!(
        remaining_jobs.is_empty(),
        "sync-disabled post-commit producer spool job should be dropped instead of retried"
    );
}

#[tokio::test]
async fn pre_push_producer_spool_job_skips_sync_when_devql_sync_disabled() {
    let dir = TempDir::new().expect("temp dir");
    let config_root = dir.path().join("config");
    let repo_root = dir.path().join("repo");
    std::fs::create_dir_all(&config_root).expect("create config root");
    std::fs::create_dir_all(repo_root.join("src")).expect("create repo src dir");

    crate::test_support::git_fixtures::init_test_repo(
        &repo_root,
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );
    std::fs::write(repo_root.join("src/lib.rs"), "pub fn qa_pre_push() {}\n")
        .expect("write source file");
    crate::test_support::git_fixtures::git_ok(&repo_root, &["add", "."]);
    crate::test_support::git_fixtures::git_ok(&repo_root, &["commit", "-m", "initial"]);
    let head_sha = crate::test_support::git_fixtures::git_ok(&repo_root, &["rev-parse", "HEAD"]);
    let config_path = crate::test_support::git_fixtures::write_test_daemon_config(&config_root);
    crate::config::settings::write_repo_daemon_binding(
        &repo_root.join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
        &config_path,
    )
    .expect("write repo daemon binding");
    crate::config::settings::set_devql_producer_settings(
        &repo_root.join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
        false,
        true,
    )
    .expect("disable producer sync");

    crate::host::devql::enqueue_spooled_pre_push_sync(
        &repo_root,
        "origin",
        &[format!(
            "refs/heads/main {head_sha} refs/heads/main 0000000000000000000000000000000000000000"
        )],
    )
    .expect("enqueue pre-push producer spool job");

    let jobs = crate::host::devql::claim_next_producer_spool_jobs(&config_root)
        .expect("claim producer spool jobs");
    assert_eq!(jobs.len(), 1, "expected one pre-push producer job");
    std::fs::remove_dir_all(repo_root.join(".git")).expect("remove git dir after spooling");

    let coordinator = Arc::new(DevqlTaskCoordinator {
        runtime_store: DaemonSqliteRuntimeStore::open_at(dir.path().join("daemon-runtime.sqlite"))
            .expect("open daemon runtime store"),
        lock: Mutex::new(()),
        notify: Notify::new(),
        worker_started: AtomicBool::new(false),
        subscription_hub: Mutex::new(None),
    });

    Arc::clone(&coordinator)
        .run_producer_spool_job(jobs.into_iter().next().expect("producer spool job"))
        .await
        .expect("process producer spool job");

    let remaining_jobs = crate::host::devql::claim_next_producer_spool_jobs(&config_root)
        .expect("claim producer spool jobs after processing");
    assert!(
        remaining_jobs.is_empty(),
        "sync-disabled pre-push producer spool job should be dropped instead of retried"
    );
}

#[tokio::test]
async fn producer_spool_claim_prunes_jobs_for_now_excluded_paths() {
    let dir = TempDir::new().expect("temp dir");
    let config_root = dir.path().join("config");
    let repo_root = dir.path().join("repo");
    std::fs::create_dir_all(&config_root).expect("create config root");
    std::fs::create_dir_all(&repo_root).expect("create repo root");

    crate::test_support::git_fixtures::init_test_repo(
        &repo_root,
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );
    std::fs::write(
        repo_root.join(crate::config::REPO_POLICY_FILE_NAME),
        "[scope]\nexclude = [\"src/**\"]\n",
    )
    .expect("write repo policy");

    let repo = crate::host::devql::resolve_repo_identity(&repo_root).expect("resolve repo");
    let cfg = DevqlConfig::from_roots(config_root.clone(), repo_root.clone(), repo)
        .expect("build devql config");
    crate::host::devql::enqueue_spooled_sync_task(
        &cfg,
        DevqlTaskSource::Watcher,
        crate::host::devql::SyncMode::Paths(vec!["src/lib.rs".to_string()]),
    )
    .expect("enqueue watcher sync into producer spool");

    let jobs = crate::host::devql::claim_next_producer_spool_jobs(&config_root)
        .expect("claim producer spool jobs");
    assert!(
        jobs.is_empty(),
        "excluded path-only producer spool jobs should be dropped before claim"
    );
}

#[test]
fn daemon_devql_task_execution_logs_terminal_failure() {
    let dir = TempDir::new().expect("temp dir");
    let config_root = dir.path().join("config");
    let repo_root = dir.path().join("repo");
    std::fs::create_dir_all(&config_root).expect("create config root");
    std::fs::create_dir_all(&repo_root).expect("create repo root");

    crate::test_support::git_fixtures::init_test_repo(
        &repo_root,
        "main",
        "Bitloops Test",
        "bitloops-test@example.com",
    );
    crate::test_support::git_fixtures::write_test_daemon_config(&config_root);

    let repo = crate::host::devql::resolve_repo_identity(&repo_root).expect("resolve repo");
    let cfg = DevqlConfig::from_roots(config_root.clone(), repo_root.clone(), repo)
        .expect("build devql config");
    let coordinator = Arc::new(DevqlTaskCoordinator {
        runtime_store: DaemonSqliteRuntimeStore::open_at(dir.path().join("daemon-runtime.sqlite"))
            .expect("open daemon runtime store"),
        lock: Mutex::new(()),
        notify: Notify::new(),
        worker_started: AtomicBool::new(false),
        subscription_hub: Mutex::new(None),
    });
    let task = coordinator
        .enqueue(
            &cfg,
            DevqlTaskSource::Watcher,
            DevqlTaskSpec::Sync(crate::daemon::SyncTaskSpec {
                mode: SyncTaskMode::Full,
                post_commit_snapshot: None,
            }),
        )
        .expect("enqueue sync task")
        .task;

    let (result, logs) = capture_logs(|| {
        coordinator.finish_task_failed(
            &task.task_id,
            anyhow::anyhow!("simulated DevQL task execution failure"),
        )
    });

    result.expect("finish task failed should update task state");
    assert!(
        logs.iter()
            .any(|entry| entry.level == log::Level::Error
                && entry.message.contains("DevQL task failed")),
        "expected terminal task failure log, got logs: {logs:?}"
    );
}

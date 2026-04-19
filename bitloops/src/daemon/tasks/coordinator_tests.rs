use super::helpers::{PROGRESS_PERSIST_INTERVAL, should_persist_embeddings_bootstrap_progress};
use super::*;
use crate::test_support::log_capture::capture_logs;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tempfile::TempDir;
use tokio::sync::Notify;

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

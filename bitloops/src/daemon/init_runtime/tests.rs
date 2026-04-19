use rusqlite::Connection;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Mutex;
use uuid::Uuid;

use crate::capability_packs::semantic_clones::types::{
    SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX, SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX,
    SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX, SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX,
};
use crate::daemon::{
    DevqlTaskKind, DevqlTaskRecord, DevqlTaskSource, DevqlTaskStatus, EmbeddingsBootstrapTaskSpec,
    InitEmbeddingsBootstrapRequest, InitSessionRecord, StartInitSessionSelections,
    SummaryBootstrapAction, SummaryBootstrapProgress, SummaryBootstrapRequest,
    SummaryBootstrapRunRecord, SummaryBootstrapStatus, SyncTaskMode, SyncTaskSpec,
};
use crate::host::runtime_store::DaemonSqliteRuntimeStore;

use super::coordinator::InitRuntimeCoordinator;
use super::lanes::{
    derive_embeddings_completed_count, derive_embeddings_lane, derive_session_status,
    derive_summaries_lane, derive_top_pipeline_lane,
};
use super::orchestration::{semantic_bootstrap_waiting_reason, semantic_follow_up_ready_for_sync};
use super::session_stats::{
    load_semantic_embedding_session_mailbox_counts, load_semantic_summary_session_mailbox_counts,
    summary_effective_work_item_count,
};
use super::stats::{
    SessionMailboxStats, SessionWorkplaneStats, StatusCounts, SummaryFreshnessState,
    is_summary_mailbox,
};
use super::types::InitRuntimeLaneProgressView;

fn completed_sync_task(task_id: &str, completed_at_unix: u64) -> DevqlTaskRecord {
    DevqlTaskRecord {
        task_id: task_id.to_string(),
        repo_id: "repo-1".to_string(),
        repo_name: "repo".to_string(),
        repo_provider: "local".to_string(),
        repo_organisation: "local".to_string(),
        repo_identity: "repo".to_string(),
        daemon_config_root: PathBuf::from("/tmp/config-1"),
        repo_root: PathBuf::from("/tmp/repo-1"),
        init_session_id: Some("init-session-1".to_string()),
        kind: DevqlTaskKind::Sync,
        source: DevqlTaskSource::Init,
        spec: crate::daemon::DevqlTaskSpec::Sync(SyncTaskSpec {
            mode: SyncTaskMode::Full,
            post_commit_snapshot: None,
        }),
        status: DevqlTaskStatus::Completed,
        submitted_at_unix: 1,
        started_at_unix: Some(1),
        updated_at_unix: completed_at_unix,
        completed_at_unix: Some(completed_at_unix),
        queue_position: None,
        tasks_ahead: None,
        error: None,
        progress: crate::daemon::DevqlTaskProgress::Sync(Default::default()),
        result: None,
    }
}

fn embeddings_only_session() -> InitSessionRecord {
    InitSessionRecord {
        init_session_id: "init-session-1".to_string(),
        repo_id: "repo-1".to_string(),
        repo_root: PathBuf::from("/tmp/repo-1"),
        daemon_config_root: PathBuf::from("/tmp/config-1"),
        selections: StartInitSessionSelections {
            run_sync: true,
            run_ingest: false,
            ingest_backfill: None,
            embeddings_bootstrap: Some(InitEmbeddingsBootstrapRequest {
                config_path: PathBuf::from("/tmp/config-1/config.toml"),
                profile_name: "local_code".to_string(),
                mode: crate::daemon::EmbeddingsBootstrapMode::Local,
                gateway_url_override: None,
                api_key_env: None,
            }),
            summaries_bootstrap: None,
        },
        initial_sync_task_id: None,
        ingest_task_id: None,
        embeddings_bootstrap_task_id: Some("bootstrap-task-1".to_string()),
        summary_bootstrap_task_id: None,
        follow_up_sync_required: false,
        follow_up_sync_task_id: None,
        next_completion_seq: 0,
        initial_sync_completion_seq: None,
        embeddings_bootstrap_completion_seq: None,
        summary_bootstrap_completion_seq: None,
        follow_up_sync_completion_seq: None,
        submitted_at_unix: 1,
        updated_at_unix: 1,
        terminal_status: None,
        terminal_error: None,
    }
}

fn test_init_runtime_coordinator() -> InitRuntimeCoordinator {
    let db_path = std::env::temp_dir().join(format!(
        "bitloops-init-runtime-test-{}.sqlite",
        Uuid::new_v4()
    ));
    InitRuntimeCoordinator {
        runtime_store: DaemonSqliteRuntimeStore::open_at(db_path)
            .expect("opening test init runtime store"),
        subscription_hub: Mutex::new(None),
        summary_in_memory_batches: Mutex::new(BTreeMap::new()),
    }
}

#[test]
fn summary_in_memory_progress_dedupes_artefacts_across_active_batches() {
    let coordinator = test_init_runtime_coordinator();
    let session_ids = BTreeSet::from(["init-session-1".to_string()]);

    coordinator.record_summary_in_memory_artefact("repo-1", "lease-1", "artefact-a", &session_ids);
    coordinator.record_summary_in_memory_artefact("repo-1", "lease-1", "artefact-a", &session_ids);
    coordinator.record_summary_in_memory_artefact("repo-1", "lease-2", "artefact-a", &session_ids);
    coordinator.record_summary_in_memory_artefact("repo-1", "lease-2", "artefact-b", &session_ids);

    assert_eq!(
        coordinator.summary_in_memory_completed("repo-1", "init-session-1"),
        2
    );

    coordinator.clear_summary_in_memory_batch("lease-1");
    assert_eq!(
        coordinator.summary_in_memory_completed("repo-1", "init-session-1"),
        2
    );

    coordinator.clear_summary_in_memory_batch("lease-2");
    assert_eq!(
        coordinator.summary_in_memory_completed("repo-1", "init-session-1"),
        0
    );
}

#[test]
fn summary_lane_classification_only_includes_summary_refresh_mailbox() {
    assert!(is_summary_mailbox(SEMANTIC_CLONES_SUMMARY_REFRESH_MAILBOX));
    assert!(!is_summary_mailbox(
        SEMANTIC_CLONES_SUMMARY_EMBEDDING_MAILBOX
    ));
    assert!(!is_summary_mailbox(SEMANTIC_CLONES_CODE_EMBEDDING_MAILBOX));
    assert!(!is_summary_mailbox(SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX));
}

#[test]
fn summary_effective_work_item_count_ignores_fresh_artefact_jobs() {
    let freshness = SummaryFreshnessState {
        eligible_artefact_ids: ["artefact-1".to_string()].into_iter().collect(),
        fresh_model_backed_artefact_ids: ["artefact-1".to_string()].into_iter().collect(),
    };

    let count =
        summary_effective_work_item_count(&json!({ "artefact_id": "artefact-1" }), &freshness);

    assert_eq!(count, 0);
}

#[test]
fn summary_effective_work_item_count_uses_outstanding_repo_backfill_work() {
    let freshness = SummaryFreshnessState {
        eligible_artefact_ids: [
            "artefact-1".to_string(),
            "artefact-2".to_string(),
            "artefact-3".to_string(),
        ]
        .into_iter()
        .collect(),
        fresh_model_backed_artefact_ids: ["artefact-1".to_string()].into_iter().collect(),
    };

    let count = summary_effective_work_item_count(
        &json!({
            "kind": "repo_backfill",
            "work_item_count": 3,
            "artefact_ids": ["artefact-1", "artefact-2", "artefact-3"]
        }),
        &freshness,
    );

    assert_eq!(count, 2);
}

#[test]
fn semantic_inbox_rows_contribute_to_init_session_mailbox_counts() {
    let conn = Connection::open_in_memory().expect("open in-memory sqlite");
    conn.execute_batch(
        "CREATE TABLE semantic_summary_mailbox_items (
             repo_id TEXT NOT NULL,
             init_session_id TEXT,
             status TEXT NOT NULL,
             item_kind TEXT NOT NULL,
             artefact_id TEXT,
             payload_json TEXT
         );
         CREATE TABLE semantic_embedding_mailbox_items (
             repo_id TEXT NOT NULL,
             init_session_id TEXT,
             representation_kind TEXT NOT NULL,
             status TEXT NOT NULL,
             item_kind TEXT NOT NULL,
             payload_json TEXT
         );",
    )
    .expect("create semantic inbox tables");
    conn.execute(
        "INSERT INTO semantic_summary_mailbox_items (
             repo_id, init_session_id, status, item_kind, artefact_id, payload_json
         ) VALUES (?1, ?2, 'pending', 'artefact', 'artefact-1', NULL)",
        ("repo-1", "init-session-1"),
    )
    .expect("insert summary inbox row");
    conn.execute(
        "INSERT INTO semantic_summary_mailbox_items (
             repo_id, init_session_id, status, item_kind, artefact_id, payload_json
         ) VALUES (?1, ?2, 'leased', 'artefact', 'artefact-2', NULL)",
        ("repo-1", "other-session"),
    )
    .expect("insert other session summary inbox row");
    conn.execute(
        "INSERT INTO semantic_embedding_mailbox_items (
             repo_id, init_session_id, representation_kind, status, item_kind, payload_json
         ) VALUES (?1, ?2, 'code', 'pending', 'artefact', NULL)",
        ("repo-1", "init-session-1"),
    )
    .expect("insert code embedding inbox row");
    conn.execute(
        "INSERT INTO semantic_embedding_mailbox_items (
             repo_id, init_session_id, representation_kind, status, item_kind, payload_json
         ) VALUES (?1, ?2, 'summary', 'leased', 'artefact', NULL)",
        ("repo-1", "init-session-1"),
    )
    .expect("insert summary embedding inbox row");

    let freshness = SummaryFreshnessState {
        eligible_artefact_ids: ["artefact-1".to_string()].into_iter().collect(),
        fresh_model_backed_artefact_ids: Default::default(),
    };
    let mut stats = SessionWorkplaneStats::default();

    load_semantic_summary_session_mailbox_counts(
        &conn,
        &mut stats,
        "repo-1",
        "init-session-1",
        &freshness,
    )
    .expect("load semantic summary mailbox counts");
    load_semantic_embedding_session_mailbox_counts(&conn, &mut stats, "repo-1", "init-session-1")
        .expect("load semantic embedding mailbox counts");
    stats.refresh_lane_counts();

    assert_eq!(stats.summary_refresh_jobs.counts.pending, 1);
    assert_eq!(stats.summary_refresh_jobs.counts.running, 0);
    assert_eq!(stats.code_embedding_jobs.counts.pending, 1);
    assert_eq!(stats.summary_embedding_jobs.counts.running, 1);
    assert_eq!(stats.summary_jobs.pending, 1);
    assert_eq!(stats.embedding_jobs.pending, 1);
    assert_eq!(stats.embedding_jobs.running, 1);
}

#[test]
fn embeddings_completed_count_uses_queue_backlog_until_current_projection_catches_up() {
    let completed = derive_embeddings_completed_count(
        278,
        2,
        StatusCounts {
            pending: 0,
            running: 0,
            failed: 0,
            completed: 278,
        },
        278,
        6,
        278,
        StatusCounts {
            pending: 226,
            running: 1,
            failed: 1,
            completed: 50,
        },
    );

    assert_eq!(completed, 328);
}

#[test]
fn embeddings_completed_count_never_exceeds_available_summaries() {
    let completed = derive_embeddings_completed_count(
        278,
        278,
        StatusCounts {
            pending: 0,
            running: 0,
            failed: 0,
            completed: 278,
        },
        278,
        10,
        40,
        StatusCounts {
            pending: 5,
            running: 0,
            failed: 0,
            completed: 35,
        },
    );

    assert_eq!(completed, 313);
}

#[test]
fn refresh_lane_counts_excludes_clone_rebuild_from_embeddings_lane() {
    let mut stats = SessionWorkplaneStats {
        code_embedding_jobs: SessionMailboxStats {
            counts: StatusCounts {
                pending: 2,
                running: 1,
                failed: 1,
                completed: 3,
            },
            latest_error: None,
        },
        summary_embedding_jobs: SessionMailboxStats {
            counts: StatusCounts {
                pending: 4,
                running: 0,
                failed: 0,
                completed: 5,
            },
            latest_error: None,
        },
        clone_rebuild_jobs: SessionMailboxStats {
            counts: StatusCounts {
                pending: 8,
                running: 9,
                failed: 10,
                completed: 11,
            },
            latest_error: Some("clone rebuild failed".to_string()),
        },
        summary_refresh_jobs: SessionMailboxStats {
            counts: StatusCounts {
                pending: 6,
                running: 7,
                failed: 2,
                completed: 12,
            },
            latest_error: None,
        },
        ..SessionWorkplaneStats::default()
    };

    stats.refresh_lane_counts();

    assert_eq!(
        stats.embedding_jobs,
        StatusCounts {
            pending: 6,
            running: 1,
            failed: 1,
            completed: 8,
        }
    );
    assert_eq!(
        stats.summary_jobs,
        StatusCounts {
            pending: 6,
            running: 7,
            failed: 2,
            completed: 12,
        }
    );
    assert_eq!(stats.warning_failed_jobs_total(), 3);
}

#[test]
fn embeddings_lane_ignores_clone_rebuild_activity_and_warnings() {
    let session = embeddings_only_session();
    let mut stats = SessionWorkplaneStats {
        code_embedding_jobs: SessionMailboxStats {
            counts: StatusCounts {
                pending: 1,
                running: 0,
                failed: 0,
                completed: 2,
            },
            latest_error: None,
        },
        clone_rebuild_jobs: SessionMailboxStats {
            counts: StatusCounts {
                pending: 30,
                running: 40,
                failed: 5,
                completed: 10,
            },
            latest_error: Some("clone rebuild failed".to_string()),
        },
        ..SessionWorkplaneStats::default()
    };
    stats.refresh_lane_counts();

    let lane = derive_embeddings_lane(&session, None, None, None, &stats, None);

    assert_eq!(lane.status, "queued");
    assert_eq!(lane.queue.queued, 1);
    assert_eq!(lane.queue.running, 0);
    assert_eq!(lane.queue.failed, 0);
    assert_eq!(lane.activity_label.as_deref(), Some("Indexing source code"));
    assert!(lane.warnings.is_empty());
}

#[test]
fn summaries_lane_reports_summary_mailbox_blockage_without_waiting_for_embeddings() {
    let session = InitSessionRecord {
        init_session_id: "init-session-1".to_string(),
        repo_id: "repo-1".to_string(),
        repo_root: PathBuf::from("/tmp/repo-1"),
        daemon_config_root: PathBuf::from("/tmp/config-1"),
        selections: StartInitSessionSelections {
            run_sync: true,
            run_ingest: false,
            ingest_backfill: None,
            embeddings_bootstrap: Some(InitEmbeddingsBootstrapRequest {
                config_path: PathBuf::from("/tmp/config-1/config.toml"),
                profile_name: "local_code".to_string(),
                mode: crate::daemon::EmbeddingsBootstrapMode::Local,
                gateway_url_override: None,
                api_key_env: None,
            }),
            summaries_bootstrap: Some(SummaryBootstrapRequest {
                action: SummaryBootstrapAction::ConfigureCloud,
                message: None,
                model_name: None,
                gateway_url_override: None,
            }),
        },
        initial_sync_task_id: None,
        ingest_task_id: None,
        embeddings_bootstrap_task_id: Some("bootstrap-task-1".to_string()),
        summary_bootstrap_task_id: Some("summary-task-1".to_string()),
        follow_up_sync_required: false,
        follow_up_sync_task_id: None,
        next_completion_seq: 0,
        initial_sync_completion_seq: None,
        embeddings_bootstrap_completion_seq: None,
        summary_bootstrap_completion_seq: None,
        follow_up_sync_completion_seq: None,
        submitted_at_unix: 1,
        updated_at_unix: 1,
        terminal_status: None,
        terminal_error: None,
    };
    let summary_run = SummaryBootstrapRunRecord {
        run_id: "summary-task-1".to_string(),
        repo_id: "repo-1".to_string(),
        repo_root: PathBuf::from("/tmp/repo-1"),
        init_session_id: "init-session-1".to_string(),
        request: SummaryBootstrapRequest {
            action: SummaryBootstrapAction::ConfigureCloud,
            message: None,
            model_name: None,
            gateway_url_override: None,
        },
        status: SummaryBootstrapStatus::Completed,
        progress: SummaryBootstrapProgress::default(),
        result: None,
        error: None,
        submitted_at_unix: 1,
        started_at_unix: Some(1),
        updated_at_unix: 1,
        completed_at_unix: Some(1),
    };
    let stats = SessionWorkplaneStats {
        summary_jobs: StatusCounts {
            pending: 1,
            running: 0,
            failed: 0,
            completed: 0,
        },
        blocked_summary_reason: Some(
            "embedding slot `summary_embeddings` is not configured yet".to_string(),
        ),
        ..SessionWorkplaneStats::default()
    };

    let lane = derive_summaries_lane(&session, None, None, Some(&summary_run), &stats, None);

    assert_eq!(lane.status, "waiting");
    assert_eq!(lane.waiting_reason.as_deref(), Some("blocked_mailbox"));
    assert_eq!(
        lane.detail.as_deref(),
        Some("embedding slot `summary_embeddings` is not configured yet")
    );
    assert_eq!(lane.pending_count, 1);
}

#[test]
fn semantic_bootstrap_waiting_reason_distinguishes_embeddings_only() {
    let session = InitSessionRecord {
        init_session_id: "init-session-1".to_string(),
        repo_id: "repo-1".to_string(),
        repo_root: PathBuf::from("/tmp/repo-1"),
        daemon_config_root: PathBuf::from("/tmp/config-1"),
        selections: StartInitSessionSelections {
            run_sync: true,
            run_ingest: false,
            ingest_backfill: None,
            embeddings_bootstrap: Some(InitEmbeddingsBootstrapRequest {
                config_path: PathBuf::from("/tmp/config-1/config.toml"),
                profile_name: "local_code".to_string(),
                mode: crate::daemon::EmbeddingsBootstrapMode::Local,
                gateway_url_override: None,
                api_key_env: None,
            }),
            summaries_bootstrap: Some(SummaryBootstrapRequest {
                action: SummaryBootstrapAction::ConfigureCloud,
                message: None,
                model_name: None,
                gateway_url_override: None,
            }),
        },
        initial_sync_task_id: None,
        ingest_task_id: None,
        embeddings_bootstrap_task_id: Some("bootstrap-task-1".to_string()),
        summary_bootstrap_task_id: Some("summary-task-1".to_string()),
        follow_up_sync_required: true,
        follow_up_sync_task_id: None,
        next_completion_seq: 1,
        initial_sync_completion_seq: None,
        embeddings_bootstrap_completion_seq: None,
        summary_bootstrap_completion_seq: Some(1),
        follow_up_sync_completion_seq: None,
        submitted_at_unix: 1,
        updated_at_unix: 1,
        terminal_status: None,
        terminal_error: None,
    };
    let embeddings_task = DevqlTaskRecord {
        task_id: "bootstrap-task-1".to_string(),
        repo_id: "repo-1".to_string(),
        repo_name: "repo".to_string(),
        repo_provider: "local".to_string(),
        repo_organisation: "local".to_string(),
        repo_identity: "repo".to_string(),
        daemon_config_root: PathBuf::from("/tmp/config-1"),
        repo_root: PathBuf::from("/tmp/repo-1"),
        init_session_id: Some("init-session-1".to_string()),
        kind: DevqlTaskKind::EmbeddingsBootstrap,
        source: DevqlTaskSource::Init,
        spec: crate::daemon::DevqlTaskSpec::EmbeddingsBootstrap(EmbeddingsBootstrapTaskSpec {
            config_path: PathBuf::from("/tmp/config-1/config.toml"),
            profile_name: "local_code".to_string(),
            mode: crate::daemon::EmbeddingsBootstrapMode::Local,
            gateway_url_override: None,
            api_key_env: None,
        }),
        status: DevqlTaskStatus::Running,
        submitted_at_unix: 1,
        started_at_unix: Some(1),
        updated_at_unix: 1,
        completed_at_unix: None,
        queue_position: None,
        tasks_ahead: None,
        error: None,
        progress: crate::daemon::DevqlTaskProgress::EmbeddingsBootstrap(
            crate::daemon::EmbeddingsBootstrapProgress::default(),
        ),
        result: None,
    };
    let summary_run = SummaryBootstrapRunRecord {
        run_id: "summary-task-1".to_string(),
        repo_id: "repo-1".to_string(),
        repo_root: PathBuf::from("/tmp/repo-1"),
        init_session_id: "init-session-1".to_string(),
        request: SummaryBootstrapRequest {
            action: SummaryBootstrapAction::ConfigureCloud,
            message: None,
            model_name: None,
            gateway_url_override: None,
        },
        status: SummaryBootstrapStatus::Completed,
        progress: SummaryBootstrapProgress::default(),
        result: None,
        error: None,
        submitted_at_unix: 1,
        started_at_unix: Some(1),
        updated_at_unix: 1,
        completed_at_unix: Some(1),
    };

    assert_eq!(
        semantic_bootstrap_waiting_reason(&session, Some(&embeddings_task), Some(&summary_run)),
        Some("waiting_for_embeddings_bootstrap")
    );
}

#[test]
fn summaries_lane_waits_for_follow_up_sync_after_summary_bootstrap_finishes_late() {
    let session = InitSessionRecord {
        init_session_id: "init-session-1".to_string(),
        repo_id: "repo-1".to_string(),
        repo_root: PathBuf::from("/tmp/repo-1"),
        daemon_config_root: PathBuf::from("/tmp/config-1"),
        selections: StartInitSessionSelections {
            run_sync: true,
            run_ingest: false,
            ingest_backfill: None,
            embeddings_bootstrap: Some(InitEmbeddingsBootstrapRequest {
                config_path: PathBuf::from("/tmp/config-1/config.toml"),
                profile_name: "local_code".to_string(),
                mode: crate::daemon::EmbeddingsBootstrapMode::Local,
                gateway_url_override: None,
                api_key_env: None,
            }),
            summaries_bootstrap: Some(SummaryBootstrapRequest {
                action: SummaryBootstrapAction::ConfigureCloud,
                message: None,
                model_name: None,
                gateway_url_override: None,
            }),
        },
        initial_sync_task_id: Some("sync-task-1".to_string()),
        ingest_task_id: None,
        embeddings_bootstrap_task_id: Some("bootstrap-task-1".to_string()),
        summary_bootstrap_task_id: Some("summary-task-1".to_string()),
        follow_up_sync_required: true,
        follow_up_sync_task_id: None,
        next_completion_seq: 2,
        initial_sync_completion_seq: Some(1),
        embeddings_bootstrap_completion_seq: None,
        summary_bootstrap_completion_seq: Some(2),
        follow_up_sync_completion_seq: None,
        submitted_at_unix: 1,
        updated_at_unix: 1,
        terminal_status: None,
        terminal_error: None,
    };
    let initial_sync = completed_sync_task("sync-task-1", 10);
    let summary_run = SummaryBootstrapRunRecord {
        run_id: "summary-task-1".to_string(),
        repo_id: "repo-1".to_string(),
        repo_root: PathBuf::from("/tmp/repo-1"),
        init_session_id: "init-session-1".to_string(),
        request: SummaryBootstrapRequest {
            action: SummaryBootstrapAction::ConfigureCloud,
            message: None,
            model_name: None,
            gateway_url_override: None,
        },
        status: SummaryBootstrapStatus::Completed,
        progress: SummaryBootstrapProgress::default(),
        result: None,
        error: None,
        submitted_at_unix: 1,
        started_at_unix: Some(1),
        updated_at_unix: 12,
        completed_at_unix: Some(12),
    };

    let lane = derive_summaries_lane(
        &session,
        Some(&initial_sync),
        None,
        Some(&summary_run),
        &SessionWorkplaneStats::default(),
        None,
    );

    assert_eq!(lane.status, "waiting");
    assert_eq!(
        lane.waiting_reason.as_deref(),
        Some("waiting_for_follow_up_sync")
    );
    assert_eq!(
        lane.activity_label.as_deref(),
        Some("Running a follow-up sync")
    );
}

#[test]
fn summaries_lane_becomes_warning_after_failed_jobs_drain() {
    let session = InitSessionRecord {
        init_session_id: "init-session-1".to_string(),
        repo_id: "repo-1".to_string(),
        repo_root: PathBuf::from("/tmp/repo-1"),
        daemon_config_root: PathBuf::from("/tmp/config-1"),
        selections: StartInitSessionSelections {
            run_sync: true,
            run_ingest: false,
            ingest_backfill: None,
            embeddings_bootstrap: Some(InitEmbeddingsBootstrapRequest {
                config_path: PathBuf::from("/tmp/config-1/config.toml"),
                profile_name: "local_code".to_string(),
                mode: crate::daemon::EmbeddingsBootstrapMode::Local,
                gateway_url_override: None,
                api_key_env: None,
            }),
            summaries_bootstrap: Some(SummaryBootstrapRequest {
                action: SummaryBootstrapAction::ConfigureCloud,
                message: None,
                model_name: None,
                gateway_url_override: None,
            }),
        },
        initial_sync_task_id: Some("sync-task-1".to_string()),
        ingest_task_id: None,
        embeddings_bootstrap_task_id: Some("bootstrap-task-1".to_string()),
        summary_bootstrap_task_id: Some("summary-task-1".to_string()),
        follow_up_sync_required: false,
        follow_up_sync_task_id: None,
        next_completion_seq: 2,
        initial_sync_completion_seq: Some(1),
        embeddings_bootstrap_completion_seq: None,
        summary_bootstrap_completion_seq: Some(2),
        follow_up_sync_completion_seq: None,
        submitted_at_unix: 1,
        updated_at_unix: 1,
        terminal_status: None,
        terminal_error: None,
    };
    let initial_sync = completed_sync_task("sync-task-1", 10);
    let summary_run = SummaryBootstrapRunRecord {
        run_id: "summary-task-1".to_string(),
        repo_id: "repo-1".to_string(),
        repo_root: PathBuf::from("/tmp/repo-1"),
        init_session_id: "init-session-1".to_string(),
        request: SummaryBootstrapRequest {
            action: SummaryBootstrapAction::ConfigureCloud,
            message: None,
            model_name: None,
            gateway_url_override: None,
        },
        status: SummaryBootstrapStatus::Completed,
        progress: SummaryBootstrapProgress::default(),
        result: None,
        error: None,
        submitted_at_unix: 1,
        started_at_unix: Some(1),
        updated_at_unix: 10,
        completed_at_unix: Some(10),
    };
    let stats = SessionWorkplaneStats {
        summary_jobs: StatusCounts {
            pending: 0,
            running: 0,
            failed: 1,
            completed: 9,
        },
        summary_refresh_jobs: SessionMailboxStats {
            counts: StatusCounts {
                pending: 0,
                running: 0,
                failed: 1,
                completed: 9,
            },
            latest_error: Some("summary provider timed out".to_string()),
        },
        ..SessionWorkplaneStats::default()
    };

    let lane = derive_summaries_lane(
        &session,
        Some(&initial_sync),
        None,
        Some(&summary_run),
        &stats,
        Some(InitRuntimeLaneProgressView {
            completed: 277,
            in_memory_completed: 0,
            total: 278,
            remaining: 1,
        }),
    );

    assert_eq!(lane.status, "warning");
    assert_eq!(lane.warnings.len(), 1);
}

#[test]
fn summaries_lane_warns_when_progress_remains_after_summary_jobs_drain() {
    let session = InitSessionRecord {
        init_session_id: "init-session-1".to_string(),
        repo_id: "repo-1".to_string(),
        repo_root: PathBuf::from("/tmp/repo-1"),
        daemon_config_root: PathBuf::from("/tmp/config-1"),
        selections: StartInitSessionSelections {
            run_sync: true,
            run_ingest: false,
            ingest_backfill: None,
            embeddings_bootstrap: Some(InitEmbeddingsBootstrapRequest {
                config_path: PathBuf::from("/tmp/config-1/config.toml"),
                profile_name: "local_code".to_string(),
                mode: crate::daemon::EmbeddingsBootstrapMode::Local,
                gateway_url_override: None,
                api_key_env: None,
            }),
            summaries_bootstrap: Some(SummaryBootstrapRequest {
                action: SummaryBootstrapAction::ConfigureCloud,
                message: None,
                model_name: None,
                gateway_url_override: None,
            }),
        },
        initial_sync_task_id: Some("sync-task-1".to_string()),
        ingest_task_id: None,
        embeddings_bootstrap_task_id: Some("bootstrap-task-1".to_string()),
        summary_bootstrap_task_id: Some("summary-task-1".to_string()),
        follow_up_sync_required: false,
        follow_up_sync_task_id: None,
        next_completion_seq: 2,
        initial_sync_completion_seq: Some(1),
        embeddings_bootstrap_completion_seq: None,
        summary_bootstrap_completion_seq: Some(2),
        follow_up_sync_completion_seq: None,
        submitted_at_unix: 1,
        updated_at_unix: 1,
        terminal_status: None,
        terminal_error: None,
    };
    let initial_sync = completed_sync_task("sync-task-1", 10);
    let summary_run = SummaryBootstrapRunRecord {
        run_id: "summary-task-1".to_string(),
        repo_id: "repo-1".to_string(),
        repo_root: PathBuf::from("/tmp/repo-1"),
        init_session_id: "init-session-1".to_string(),
        request: SummaryBootstrapRequest {
            action: SummaryBootstrapAction::ConfigureCloud,
            message: None,
            model_name: None,
            gateway_url_override: None,
        },
        status: SummaryBootstrapStatus::Completed,
        progress: SummaryBootstrapProgress::default(),
        result: None,
        error: None,
        submitted_at_unix: 1,
        started_at_unix: Some(1),
        updated_at_unix: 10,
        completed_at_unix: Some(10),
    };

    let lane = derive_summaries_lane(
        &session,
        Some(&initial_sync),
        None,
        Some(&summary_run),
        &SessionWorkplaneStats::default(),
        Some(InitRuntimeLaneProgressView {
            completed: 272,
            in_memory_completed: 0,
            total: 285,
            remaining: 13,
        }),
    );

    assert_eq!(lane.status, "warning");
    assert_eq!(lane.waiting_reason, None);
    assert_eq!(
        lane.detail.as_deref(),
        Some(
            "Summary generation finished without producing current summaries for every eligible artefact"
        )
    );
}

#[test]
fn summary_follow_up_can_start_before_embeddings_bootstrap_finishes() {
    let session = InitSessionRecord {
        init_session_id: "init-session-1".to_string(),
        repo_id: "repo-1".to_string(),
        repo_root: PathBuf::from("/tmp/repo-1"),
        daemon_config_root: PathBuf::from("/tmp/config-1"),
        selections: StartInitSessionSelections {
            run_sync: true,
            run_ingest: false,
            ingest_backfill: None,
            embeddings_bootstrap: Some(InitEmbeddingsBootstrapRequest {
                config_path: PathBuf::from("/tmp/config-1/config.toml"),
                profile_name: "local_code".to_string(),
                mode: crate::daemon::EmbeddingsBootstrapMode::Local,
                gateway_url_override: None,
                api_key_env: None,
            }),
            summaries_bootstrap: Some(SummaryBootstrapRequest {
                action: SummaryBootstrapAction::ConfigureCloud,
                message: None,
                model_name: None,
                gateway_url_override: None,
            }),
        },
        initial_sync_task_id: Some("sync-task-1".to_string()),
        ingest_task_id: None,
        embeddings_bootstrap_task_id: Some("bootstrap-task-1".to_string()),
        summary_bootstrap_task_id: Some("summary-task-1".to_string()),
        follow_up_sync_required: true,
        follow_up_sync_task_id: None,
        next_completion_seq: 2,
        initial_sync_completion_seq: Some(1),
        embeddings_bootstrap_completion_seq: None,
        summary_bootstrap_completion_seq: Some(2),
        follow_up_sync_completion_seq: None,
        submitted_at_unix: 1,
        updated_at_unix: 1,
        terminal_status: None,
        terminal_error: None,
    };
    let initial_sync = completed_sync_task("sync-task-1", 10);
    let embeddings_task = DevqlTaskRecord {
        task_id: "bootstrap-task-1".to_string(),
        repo_id: "repo-1".to_string(),
        repo_name: "repo".to_string(),
        repo_provider: "local".to_string(),
        repo_organisation: "local".to_string(),
        repo_identity: "repo".to_string(),
        daemon_config_root: PathBuf::from("/tmp/config-1"),
        repo_root: PathBuf::from("/tmp/repo-1"),
        init_session_id: Some("init-session-1".to_string()),
        kind: DevqlTaskKind::EmbeddingsBootstrap,
        source: DevqlTaskSource::Init,
        spec: crate::daemon::DevqlTaskSpec::EmbeddingsBootstrap(EmbeddingsBootstrapTaskSpec {
            config_path: PathBuf::from("/tmp/config-1/config.toml"),
            profile_name: "local_code".to_string(),
            mode: crate::daemon::EmbeddingsBootstrapMode::Local,
            gateway_url_override: None,
            api_key_env: None,
        }),
        status: DevqlTaskStatus::Running,
        submitted_at_unix: 1,
        started_at_unix: Some(1),
        updated_at_unix: 11,
        completed_at_unix: None,
        queue_position: None,
        tasks_ahead: None,
        error: None,
        progress: crate::daemon::DevqlTaskProgress::EmbeddingsBootstrap(
            crate::daemon::EmbeddingsBootstrapProgress::default(),
        ),
        result: None,
    };
    let summary_run = SummaryBootstrapRunRecord {
        run_id: "summary-task-1".to_string(),
        repo_id: "repo-1".to_string(),
        repo_root: PathBuf::from("/tmp/repo-1"),
        init_session_id: "init-session-1".to_string(),
        request: SummaryBootstrapRequest {
            action: SummaryBootstrapAction::ConfigureCloud,
            message: None,
            model_name: None,
            gateway_url_override: None,
        },
        status: SummaryBootstrapStatus::Completed,
        progress: SummaryBootstrapProgress::default(),
        result: None,
        error: None,
        submitted_at_unix: 1,
        started_at_unix: Some(1),
        updated_at_unix: 12,
        completed_at_unix: Some(12),
    };

    assert!(semantic_follow_up_ready_for_sync(
        &session,
        Some(&initial_sync),
        None,
        Some(&embeddings_task),
        Some(&summary_run),
    ));
}

#[test]
fn embeddings_can_trigger_a_second_follow_up_after_summary_follow_up_completes() {
    let session = InitSessionRecord {
        init_session_id: "init-session-1".to_string(),
        repo_id: "repo-1".to_string(),
        repo_root: PathBuf::from("/tmp/repo-1"),
        daemon_config_root: PathBuf::from("/tmp/config-1"),
        selections: StartInitSessionSelections {
            run_sync: true,
            run_ingest: false,
            ingest_backfill: None,
            embeddings_bootstrap: Some(InitEmbeddingsBootstrapRequest {
                config_path: PathBuf::from("/tmp/config-1/config.toml"),
                profile_name: "local_code".to_string(),
                mode: crate::daemon::EmbeddingsBootstrapMode::Local,
                gateway_url_override: None,
                api_key_env: None,
            }),
            summaries_bootstrap: Some(SummaryBootstrapRequest {
                action: SummaryBootstrapAction::ConfigureCloud,
                message: None,
                model_name: None,
                gateway_url_override: None,
            }),
        },
        initial_sync_task_id: Some("sync-task-1".to_string()),
        ingest_task_id: None,
        embeddings_bootstrap_task_id: Some("bootstrap-task-1".to_string()),
        summary_bootstrap_task_id: Some("summary-task-1".to_string()),
        follow_up_sync_required: true,
        follow_up_sync_task_id: Some("follow-up-sync-1".to_string()),
        next_completion_seq: 4,
        initial_sync_completion_seq: Some(1),
        embeddings_bootstrap_completion_seq: Some(4),
        summary_bootstrap_completion_seq: Some(2),
        follow_up_sync_completion_seq: Some(3),
        submitted_at_unix: 1,
        updated_at_unix: 1,
        terminal_status: None,
        terminal_error: None,
    };
    let initial_sync = completed_sync_task("sync-task-1", 10);
    let follow_up_sync = completed_sync_task("follow-up-sync-1", 14);
    let embeddings_task = DevqlTaskRecord {
        task_id: "bootstrap-task-1".to_string(),
        repo_id: "repo-1".to_string(),
        repo_name: "repo".to_string(),
        repo_provider: "local".to_string(),
        repo_organisation: "local".to_string(),
        repo_identity: "repo".to_string(),
        daemon_config_root: PathBuf::from("/tmp/config-1"),
        repo_root: PathBuf::from("/tmp/repo-1"),
        init_session_id: Some("init-session-1".to_string()),
        kind: DevqlTaskKind::EmbeddingsBootstrap,
        source: DevqlTaskSource::Init,
        spec: crate::daemon::DevqlTaskSpec::EmbeddingsBootstrap(EmbeddingsBootstrapTaskSpec {
            config_path: PathBuf::from("/tmp/config-1/config.toml"),
            profile_name: "local_code".to_string(),
            mode: crate::daemon::EmbeddingsBootstrapMode::Local,
            gateway_url_override: None,
            api_key_env: None,
        }),
        status: DevqlTaskStatus::Completed,
        submitted_at_unix: 1,
        started_at_unix: Some(1),
        updated_at_unix: 20,
        completed_at_unix: Some(20),
        queue_position: None,
        tasks_ahead: None,
        error: None,
        progress: crate::daemon::DevqlTaskProgress::EmbeddingsBootstrap(
            crate::daemon::EmbeddingsBootstrapProgress::default(),
        ),
        result: None,
    };
    let summary_run = SummaryBootstrapRunRecord {
        run_id: "summary-task-1".to_string(),
        repo_id: "repo-1".to_string(),
        repo_root: PathBuf::from("/tmp/repo-1"),
        init_session_id: "init-session-1".to_string(),
        request: SummaryBootstrapRequest {
            action: SummaryBootstrapAction::ConfigureCloud,
            message: None,
            model_name: None,
            gateway_url_override: None,
        },
        status: SummaryBootstrapStatus::Completed,
        progress: SummaryBootstrapProgress::default(),
        result: None,
        error: None,
        submitted_at_unix: 1,
        started_at_unix: Some(1),
        updated_at_unix: 12,
        completed_at_unix: Some(12),
    };

    assert!(semantic_follow_up_ready_for_sync(
        &session,
        Some(&initial_sync),
        Some(&follow_up_sync),
        Some(&embeddings_task),
        Some(&summary_run),
    ));
}

#[test]
fn top_pipeline_lane_reports_failed_sync_task() {
    let session = InitSessionRecord {
        init_session_id: "init-session-1".to_string(),
        repo_id: "repo-1".to_string(),
        repo_root: PathBuf::from("/tmp/repo-1"),
        daemon_config_root: PathBuf::from("/tmp/config-1"),
        selections: StartInitSessionSelections {
            run_sync: true,
            run_ingest: false,
            ingest_backfill: None,
            embeddings_bootstrap: None,
            summaries_bootstrap: None,
        },
        initial_sync_task_id: Some("sync-task-1".to_string()),
        ingest_task_id: None,
        embeddings_bootstrap_task_id: None,
        summary_bootstrap_task_id: None,
        follow_up_sync_required: false,
        follow_up_sync_task_id: None,
        next_completion_seq: 0,
        initial_sync_completion_seq: None,
        embeddings_bootstrap_completion_seq: None,
        summary_bootstrap_completion_seq: None,
        follow_up_sync_completion_seq: None,
        submitted_at_unix: 1,
        updated_at_unix: 1,
        terminal_status: None,
        terminal_error: None,
    };
    let sync_task = DevqlTaskRecord {
        task_id: "sync-task-1".to_string(),
        repo_id: "repo-1".to_string(),
        repo_name: "repo".to_string(),
        repo_provider: "local".to_string(),
        repo_organisation: "local".to_string(),
        repo_identity: "repo".to_string(),
        daemon_config_root: PathBuf::from("/tmp/config-1"),
        repo_root: PathBuf::from("/tmp/repo-1"),
        init_session_id: Some("init-session-1".to_string()),
        kind: DevqlTaskKind::Sync,
        source: DevqlTaskSource::Init,
        spec: crate::daemon::DevqlTaskSpec::Sync(crate::daemon::SyncTaskSpec {
            mode: crate::daemon::SyncTaskMode::Full,
            post_commit_snapshot: None,
        }),
        status: DevqlTaskStatus::Failed,
        submitted_at_unix: 1,
        started_at_unix: Some(1),
        updated_at_unix: 2,
        completed_at_unix: Some(2),
        queue_position: None,
        tasks_ahead: None,
        error: Some("sync failed".to_string()),
        progress: crate::daemon::DevqlTaskProgress::Sync(Default::default()),
        result: None,
    };

    let lane = derive_top_pipeline_lane(
        &session,
        Some(&sync_task),
        None,
        None,
        StatusCounts::default(),
    );

    assert_eq!(lane.status, "failed");
    assert_eq!(lane.waiting_reason.as_deref(), Some("failed"));
    assert_eq!(lane.detail.as_deref(), Some("Syncing repository failed"));
    assert_eq!(lane.task_id.as_deref(), Some("sync-task-1"));
}

#[test]
fn session_status_only_becomes_failed_after_claimed_work_drains() {
    assert_eq!(
        derive_session_status(true, true, false, None, false),
        "failing"
    );
    assert_eq!(
        derive_session_status(true, false, false, None, false),
        "failed"
    );
    assert_eq!(
        derive_session_status(
            false,
            false,
            false,
            Some("waiting_for_current_state_consumer"),
            false,
        ),
        "waiting"
    );
    assert_eq!(
        derive_session_status(false, false, false, None, false),
        "running"
    );
    assert_eq!(
        derive_session_status(false, false, true, None, false),
        "completed"
    );
    assert_eq!(
        derive_session_status(false, false, true, None, true),
        "completed_with_warnings"
    );
}

use std::collections::HashSet;
use std::path::PathBuf;

use crate::host::devql::{DevqlConfig, RepoIdentity};

use super::super::super::types::{
    DevqlTaskKind, DevqlTaskProgress, DevqlTaskRecord, DevqlTaskSource, DevqlTaskSpec,
    DevqlTaskStatus, PostCommitSnapshotSpec, SyncTaskMode, SyncTaskSpec,
};
use super::super::state::PersistedDevqlTaskQueueState;
use super::{
    merge_existing_task, next_runnable_task_indexes, next_runnable_task_indexes_blocking_repo_ids,
    post_commit_derivation_claim_guards, prune_terminal_tasks,
};

fn test_cfg() -> DevqlConfig {
    DevqlConfig {
        daemon_config_root: PathBuf::from("/tmp/config-1"),
        repo_root: PathBuf::from("/tmp/repo-1"),
        repo: RepoIdentity {
            provider: "local".to_string(),
            organization: "local".to_string(),
            name: "repo".to_string(),
            identity: "repo".to_string(),
            repo_id: "repo-1".to_string(),
        },
        pg_dsn: None,
        clickhouse_url: "http://localhost:8123".to_string(),
        clickhouse_user: None,
        clickhouse_password: None,
        clickhouse_database: "default".to_string(),
    }
}

fn sync_spec(mode: SyncTaskMode) -> DevqlTaskSpec {
    DevqlTaskSpec::Sync(SyncTaskSpec {
        mode,
        post_commit_snapshot: None,
    })
}

fn sync_spec_with_snapshot(
    mode: SyncTaskMode,
    commit_sha: &str,
    changed_paths: &[&str],
) -> DevqlTaskSpec {
    DevqlTaskSpec::Sync(SyncTaskSpec {
        mode,
        post_commit_snapshot: Some(PostCommitSnapshotSpec {
            commit_sha: commit_sha.to_string(),
            changed_paths: changed_paths.iter().map(|path| path.to_string()).collect(),
        }),
    })
}

fn sync_task(
    task_id: &str,
    source: DevqlTaskSource,
    mode: SyncTaskMode,
    status: DevqlTaskStatus,
) -> DevqlTaskRecord {
    sync_task_with_spec(task_id, source, sync_spec(mode), status)
}

fn sync_task_with_spec(
    task_id: &str,
    source: DevqlTaskSource,
    spec: DevqlTaskSpec,
    status: DevqlTaskStatus,
) -> DevqlTaskRecord {
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
        source,
        spec,
        status,
        submitted_at_unix: 1,
        started_at_unix: (status == DevqlTaskStatus::Running).then_some(2),
        updated_at_unix: 1,
        completed_at_unix: None,
        queue_position: None,
        tasks_ahead: None,
        progress: DevqlTaskProgress::Sync(Default::default()),
        error: None,
        result: None,
    }
}

#[test]
fn queued_watcher_paths_are_upgraded_by_incoming_post_checkout_full() {
    let cfg = test_cfg();
    let mut state = PersistedDevqlTaskQueueState {
        tasks: vec![sync_task(
            "sync-task-1",
            DevqlTaskSource::Watcher,
            SyncTaskMode::Paths {
                paths: vec!["src/lib.rs".to_string()],
            },
            DevqlTaskStatus::Queued,
        )],
        ..Default::default()
    };

    let merged = merge_existing_task(
        &mut state,
        &cfg,
        DevqlTaskSource::PostCheckout,
        DevqlTaskKind::Sync,
        &sync_spec(SyncTaskMode::Full),
        Some("init-session-1"),
    )
    .expect("incoming full sync should merge into queued path sync");

    assert_eq!(state.tasks.len(), 1);
    assert_eq!(merged.task_id, "sync-task-1");
    assert_eq!(state.tasks[0].source, DevqlTaskSource::PostCheckout);
    assert_eq!(
        state.tasks[0].sync_spec().expect("sync spec").mode,
        SyncTaskMode::Full
    );
}

#[test]
fn queued_repo_policy_paths_keep_source_when_upgraded_by_non_policy_full() {
    let cfg = test_cfg();
    let mut state = PersistedDevqlTaskQueueState {
        tasks: vec![sync_task(
            "sync-task-1",
            DevqlTaskSource::RepoPolicyChange,
            SyncTaskMode::Paths {
                paths: vec!["src/lib.rs".to_string()],
            },
            DevqlTaskStatus::Queued,
        )],
        ..Default::default()
    };

    let merged = merge_existing_task(
        &mut state,
        &cfg,
        DevqlTaskSource::PostCheckout,
        DevqlTaskKind::Sync,
        &sync_spec(SyncTaskMode::Full),
        Some("init-session-1"),
    )
    .expect("incoming full sync should upgrade queued policy path sync");

    assert_eq!(state.tasks.len(), 1);
    assert_eq!(merged.task_id, "sync-task-1");
    assert_eq!(state.tasks[0].source, DevqlTaskSource::RepoPolicyChange);
    assert_eq!(
        state.tasks[0].sync_spec().expect("sync spec").mode,
        SyncTaskMode::Full
    );
}

#[test]
fn queued_post_checkout_full_absorbs_incoming_watcher_paths() {
    let cfg = test_cfg();
    let mut state = PersistedDevqlTaskQueueState {
        tasks: vec![sync_task(
            "sync-task-1",
            DevqlTaskSource::PostCheckout,
            SyncTaskMode::Full,
            DevqlTaskStatus::Queued,
        )],
        ..Default::default()
    };

    let merged = merge_existing_task(
        &mut state,
        &cfg,
        DevqlTaskSource::Watcher,
        DevqlTaskKind::Sync,
        &sync_spec(SyncTaskMode::Paths {
            paths: vec!["src/lib.rs".to_string()],
        }),
        Some("init-session-1"),
    )
    .expect("incoming paths should merge into queued full sync");

    assert_eq!(state.tasks.len(), 1);
    assert_eq!(merged.task_id, "sync-task-1");
    assert_eq!(state.tasks[0].source, DevqlTaskSource::PostCheckout);
    assert_eq!(
        state.tasks[0].sync_spec().expect("sync spec").mode,
        SyncTaskMode::Full
    );
}

#[test]
fn queued_post_checkout_full_does_not_absorb_incoming_post_merge_paths() {
    let cfg = test_cfg();
    let mut state = PersistedDevqlTaskQueueState {
        tasks: vec![sync_task(
            "sync-task-1",
            DevqlTaskSource::PostCheckout,
            SyncTaskMode::Full,
            DevqlTaskStatus::Queued,
        )],
        ..Default::default()
    };

    let merged = merge_existing_task(
        &mut state,
        &cfg,
        DevqlTaskSource::PostMerge,
        DevqlTaskKind::Sync,
        &sync_spec(SyncTaskMode::Paths {
            paths: vec!["src/utils.rs".to_string()],
        }),
        Some("init-session-1"),
    );

    assert!(
        merged.is_none(),
        "post-merge producer work must keep its own task source instead of being hidden by queued post-checkout sync"
    );
    assert_eq!(state.tasks.len(), 1);
    assert_eq!(state.tasks[0].source, DevqlTaskSource::PostCheckout);
    assert_eq!(
        state.tasks[0].sync_spec().expect("sync spec").mode,
        SyncTaskMode::Full
    );
}

#[test]
fn queued_watcher_paths_are_upgraded_by_incoming_manual_full() {
    let cfg = test_cfg();
    let mut state = PersistedDevqlTaskQueueState {
        tasks: vec![sync_task(
            "sync-task-1",
            DevqlTaskSource::Watcher,
            SyncTaskMode::Paths {
                paths: vec!["src/lib.rs".to_string()],
            },
            DevqlTaskStatus::Queued,
        )],
        ..Default::default()
    };

    let merged = merge_existing_task(
        &mut state,
        &cfg,
        DevqlTaskSource::ManualCli,
        DevqlTaskKind::Sync,
        &sync_spec(SyncTaskMode::Full),
        Some("init-session-1"),
    )
    .expect("incoming manual full sync should upgrade queued watcher path sync");

    assert_eq!(state.tasks.len(), 1);
    assert_eq!(merged.task_id, "sync-task-1");
    assert_eq!(state.tasks[0].source, DevqlTaskSource::ManualCli);
    assert_eq!(
        state.tasks[0].sync_spec().expect("sync spec").mode,
        SyncTaskMode::Full
    );
}

#[test]
fn queued_post_checkout_full_runs_before_queued_watcher_paths() {
    let state = PersistedDevqlTaskQueueState {
        tasks: vec![
            sync_task(
                "sync-task-1",
                DevqlTaskSource::Watcher,
                SyncTaskMode::Paths {
                    paths: vec!["src/lib.rs".to_string()],
                },
                DevqlTaskStatus::Queued,
            ),
            sync_task(
                "sync-task-2",
                DevqlTaskSource::PostCheckout,
                SyncTaskMode::Full,
                DevqlTaskStatus::Queued,
            ),
        ],
        ..Default::default()
    };

    let runnable = next_runnable_task_indexes(&state);

    assert_eq!(
        runnable,
        vec![1],
        "post-checkout full sync should own checkout materialization before watcher paths"
    );
}

#[test]
fn queued_tasks_wait_while_producer_spool_job_runs_for_repo() {
    let mut other_repo_task = sync_task(
        "sync-task-2",
        DevqlTaskSource::Watcher,
        SyncTaskMode::Paths {
            paths: vec!["src/other.rs".to_string()],
        },
        DevqlTaskStatus::Queued,
    );
    other_repo_task.repo_id = "repo-2".to_string();

    let state = PersistedDevqlTaskQueueState {
        tasks: vec![
            sync_task(
                "sync-task-1",
                DevqlTaskSource::Watcher,
                SyncTaskMode::Paths {
                    paths: vec!["src/lib.rs".to_string()],
                },
                DevqlTaskStatus::Queued,
            ),
            other_repo_task,
        ],
        ..Default::default()
    };

    let blocked_repo_ids = HashSet::from(["repo-1".to_string()]);
    let runnable = next_runnable_task_indexes_blocking_repo_ids(&state, &blocked_repo_ids);

    assert_eq!(
        runnable,
        vec![1],
        "tasks for a repo with running producer spool work should wait"
    );
}

#[test]
fn queued_post_commit_sync_blocks_matching_post_commit_derivation() {
    let state = PersistedDevqlTaskQueueState {
        tasks: vec![
            sync_task_with_spec(
                "sync-task-1",
                DevqlTaskSource::PostCommit,
                sync_spec_with_snapshot(
                    SyncTaskMode::Paths {
                        paths: vec!["src/lib.rs".to_string()],
                    },
                    "commit-head",
                    &["src/lib.rs"],
                ),
                DevqlTaskStatus::Queued,
            ),
            sync_task_with_spec(
                "sync-task-2",
                DevqlTaskSource::PostCommit,
                sync_spec_with_snapshot(
                    SyncTaskMode::Paths {
                        paths: vec!["src/main.rs".to_string()],
                    },
                    "completed-commit",
                    &["src/main.rs"],
                ),
                DevqlTaskStatus::Completed,
            ),
        ],
        ..Default::default()
    };

    let guards = post_commit_derivation_claim_guards(&state);

    assert_eq!(
        guards.blocked,
        HashSet::from([("repo-1".to_string(), "commit-head".to_string())]),
        "only incomplete post-commit sync snapshots should block derivation"
    );
    assert!(
        guards.abandoned.is_empty(),
        "completed post-commit sync snapshots should not abandon derivation"
    );
}

#[test]
fn failed_post_commit_sync_abandons_matching_post_commit_derivation() {
    let state = PersistedDevqlTaskQueueState {
        tasks: vec![sync_task_with_spec(
            "sync-task-1",
            DevqlTaskSource::PostCommit,
            sync_spec_with_snapshot(
                SyncTaskMode::Paths {
                    paths: vec!["src/lib.rs".to_string()],
                },
                "commit-head",
                &["src/lib.rs"],
            ),
            DevqlTaskStatus::Failed,
        )],
        ..Default::default()
    };

    let guards = post_commit_derivation_claim_guards(&state);

    assert!(guards.blocked.is_empty());
    assert_eq!(
        guards.abandoned,
        HashSet::from([("repo-1".to_string(), "commit-head".to_string())]),
        "failed post-commit refresh sync should abandon dependent derivation"
    );
}

#[test]
fn queued_paths_are_not_upgraded_across_init_sessions() {
    let cfg = test_cfg();
    let mut state = PersistedDevqlTaskQueueState {
        tasks: vec![sync_task(
            "sync-task-1",
            DevqlTaskSource::Watcher,
            SyncTaskMode::Paths {
                paths: vec!["src/lib.rs".to_string()],
            },
            DevqlTaskStatus::Queued,
        )],
        ..Default::default()
    };

    let merged = merge_existing_task(
        &mut state,
        &cfg,
        DevqlTaskSource::PostCheckout,
        DevqlTaskKind::Sync,
        &sync_spec(SyncTaskMode::Full),
        Some("different-init-session"),
    );

    assert!(merged.is_none(), "different init sessions must not merge");
    assert_eq!(
        state.tasks[0].sync_spec().expect("sync spec").mode,
        SyncTaskMode::Paths {
            paths: vec!["src/lib.rs".to_string()],
        }
    );
}

#[test]
fn queued_paths_upgrade_with_matching_snapshot_and_merge_changed_paths() {
    let cfg = test_cfg();
    let mut state = PersistedDevqlTaskQueueState {
        tasks: vec![sync_task_with_spec(
            "sync-task-1",
            DevqlTaskSource::Watcher,
            sync_spec_with_snapshot(
                SyncTaskMode::Paths {
                    paths: vec!["src/lib.rs".to_string()],
                },
                "abc123",
                &["src/lib.rs"],
            ),
            DevqlTaskStatus::Queued,
        )],
        ..Default::default()
    };

    let merged = merge_existing_task(
        &mut state,
        &cfg,
        DevqlTaskSource::PostCheckout,
        DevqlTaskKind::Sync,
        &sync_spec_with_snapshot(SyncTaskMode::Full, "abc123", &["src/main.rs"]),
        Some("init-session-1"),
    )
    .expect("matching snapshots should allow upgrade");

    assert_eq!(merged.task_id, "sync-task-1");
    let sync_spec = state.tasks[0].sync_spec().expect("sync spec");
    assert_eq!(sync_spec.mode, SyncTaskMode::Full);
    let snapshot = sync_spec
        .post_commit_snapshot
        .as_ref()
        .expect("merged snapshot");
    assert_eq!(snapshot.commit_sha, "abc123");
    assert_eq!(
        snapshot.changed_paths,
        vec!["src/lib.rs".to_string(), "src/main.rs".to_string()]
    );
}

#[test]
fn queued_paths_do_not_upgrade_with_mismatched_snapshot() {
    let cfg = test_cfg();
    let mut state = PersistedDevqlTaskQueueState {
        tasks: vec![sync_task_with_spec(
            "sync-task-1",
            DevqlTaskSource::Watcher,
            sync_spec_with_snapshot(
                SyncTaskMode::Paths {
                    paths: vec!["src/lib.rs".to_string()],
                },
                "abc123",
                &["src/lib.rs"],
            ),
            DevqlTaskStatus::Queued,
        )],
        ..Default::default()
    };

    let merged = merge_existing_task(
        &mut state,
        &cfg,
        DevqlTaskSource::PostCheckout,
        DevqlTaskKind::Sync,
        &sync_spec_with_snapshot(SyncTaskMode::Full, "def456", &["src/main.rs"]),
        Some("init-session-1"),
    );

    assert!(merged.is_none(), "mismatched snapshots must not merge");
    assert_eq!(
        state.tasks[0].sync_spec().expect("sync spec").mode,
        SyncTaskMode::Paths {
            paths: vec!["src/lib.rs".to_string()],
        }
    );
}

#[test]
fn queued_paths_do_not_merge_with_incoming_validate() {
    let cfg = test_cfg();
    let mut state = PersistedDevqlTaskQueueState {
        tasks: vec![sync_task(
            "sync-task-1",
            DevqlTaskSource::Watcher,
            SyncTaskMode::Paths {
                paths: vec!["src/lib.rs".to_string()],
            },
            DevqlTaskStatus::Queued,
        )],
        ..Default::default()
    };

    let merged = merge_existing_task(
        &mut state,
        &cfg,
        DevqlTaskSource::ManualCli,
        DevqlTaskKind::Sync,
        &sync_spec(SyncTaskMode::Validate),
        Some("init-session-1"),
    );

    assert!(merged.is_none(), "validate tasks must not merge");
    assert_eq!(
        state.tasks[0].sync_spec().expect("sync spec").mode,
        SyncTaskMode::Paths {
            paths: vec!["src/lib.rs".to_string()],
        }
    );
}

#[test]
fn running_watcher_paths_are_not_upgraded_by_incoming_full() {
    let cfg = test_cfg();
    let mut state = PersistedDevqlTaskQueueState {
        tasks: vec![sync_task(
            "sync-task-1",
            DevqlTaskSource::Watcher,
            SyncTaskMode::Paths {
                paths: vec!["src/lib.rs".to_string()],
            },
            DevqlTaskStatus::Running,
        )],
        ..Default::default()
    };

    let merged = merge_existing_task(
        &mut state,
        &cfg,
        DevqlTaskSource::PostCheckout,
        DevqlTaskKind::Sync,
        &sync_spec(SyncTaskMode::Full),
        Some("init-session-1"),
    );

    assert!(
        merged.is_none(),
        "running path sync should not absorb an incoming full sync"
    );
    assert_eq!(state.tasks.len(), 1);
    assert_eq!(state.tasks[0].source, DevqlTaskSource::Watcher);
    assert_eq!(
        state.tasks[0].sync_spec().expect("sync spec").mode,
        SyncTaskMode::Paths {
            paths: vec!["src/lib.rs".to_string()],
        }
    );
}

fn completed_sync_task(task_id: &str, updated_at_unix: u64) -> DevqlTaskRecord {
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
        spec: DevqlTaskSpec::Sync(SyncTaskSpec {
            mode: SyncTaskMode::Full,
            post_commit_snapshot: None,
        }),
        status: DevqlTaskStatus::Completed,
        submitted_at_unix: updated_at_unix,
        started_at_unix: Some(updated_at_unix),
        updated_at_unix,
        completed_at_unix: Some(updated_at_unix),
        queue_position: None,
        tasks_ahead: None,
        progress: DevqlTaskProgress::Sync(Default::default()),
        error: None,
        result: None,
    }
}

#[test]
fn prune_terminal_tasks_keeps_ids_referenced_by_active_init_sessions() {
    let mut tasks = (0..66)
        .map(|index| completed_sync_task(&format!("sync-task-{index}"), index as u64 + 1))
        .collect::<Vec<_>>();
    let protected = HashSet::from(["sync-task-0".to_string()]);

    prune_terminal_tasks(&mut tasks, &protected);

    assert_eq!(tasks.len(), 65);
    assert!(tasks.iter().any(|task| task.task_id == "sync-task-0"));
    assert!(!tasks.iter().any(|task| task.task_id == "sync-task-1"));
}

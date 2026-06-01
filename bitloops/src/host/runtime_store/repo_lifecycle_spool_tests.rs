use super::*;

fn sqlite_at(dir: &tempfile::TempDir) -> SqliteConnectionPool {
    SqliteConnectionPool::connect(dir.path().join("runtime.sqlite")).expect("open sqlite")
}

fn sample_insert(repo_root: &Path, hook_name: &str) -> LifecycleJobInsert {
    LifecycleJobInsert {
        repo_id: "repo-1".to_string(),
        repo_root: repo_root.to_path_buf(),
        config_root: repo_root.join(".bitloops-test-state/daemon"),
        agent_name: crate::adapters::agents::AGENT_NAME_CODEX.to_string(),
        hook_name: hook_name.to_string(),
        raw_stdin: r#"{"session_id":"session-1","transcript_path":"/tmp/session.jsonl"}"#
            .to_string(),
        workspace_snapshot: Some(LifecycleWorkspaceSnapshot::default()),
        boundary_snapshot: None,
        cwd: repo_root.to_path_buf(),
        received_at_unix: 1_778_800_000,
    }
}

#[test]
fn lifecycle_spool_claims_jobs_by_insertion_sequence_when_same_second() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let sqlite = sqlite_at(&dir);
    initialise_lifecycle_spool_schema(&sqlite)?;

    enqueue_lifecycle_job_sqlite(&sqlite, sample_insert(dir.path(), "first-hook"))?;
    enqueue_lifecycle_job_sqlite(&sqlite, sample_insert(dir.path(), "second-hook"))?;

    let first = claim_next_lifecycle_job(&sqlite)?.expect("first claimed job");
    assert_eq!(first.hook_name, "first-hook");
    delete_lifecycle_job(&sqlite, &first.job_id)?;

    let second = claim_next_lifecycle_job(&sqlite)?.expect("second claimed job");
    assert_eq!(second.hook_name, "second-hook");
    Ok(())
}

#[test]
fn lifecycle_spool_blocks_behind_unavailable_head_job() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let sqlite = sqlite_at(&dir);
    initialise_lifecycle_spool_schema(&sqlite)?;

    enqueue_lifecycle_job_sqlite(&sqlite, sample_insert(dir.path(), "retry-head"))?;
    let first = claim_next_lifecycle_job(&sqlite)?.expect("first claimed job");
    requeue_lifecycle_job(&sqlite, &first.job_id, "transient failure")?;
    enqueue_lifecycle_job_sqlite(&sqlite, sample_insert(dir.path(), "blocked-behind-head"))?;

    assert!(claim_next_lifecycle_job(&sqlite)?.is_none());
    Ok(())
}

#[test]
fn lifecycle_spool_running_head_blocks_later_jobs() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let sqlite = sqlite_at(&dir);
    initialise_lifecycle_spool_schema(&sqlite)?;

    enqueue_lifecycle_job_sqlite(&sqlite, sample_insert(dir.path(), "running-head"))?;
    enqueue_lifecycle_job_sqlite(&sqlite, sample_insert(dir.path(), "blocked-behind-running"))?;

    let first = claim_next_lifecycle_job(&sqlite)?.expect("first claimed job");
    assert_eq!(first.hook_name, "running-head");
    assert!(claim_next_lifecycle_job(&sqlite)?.is_none());
    Ok(())
}

#[test]
fn lifecycle_spool_marks_job_failed_after_max_attempts() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let sqlite = sqlite_at(&dir);
    initialise_lifecycle_spool_schema(&sqlite)?;
    enqueue_lifecycle_job_sqlite(&sqlite, sample_insert(dir.path(), "stop"))?;

    let mut job_id = None;
    for expected_attempt in 1..=MAX_LIFECYCLE_JOB_ATTEMPTS {
        if let Some(job_id) = job_id.as_deref() {
            force_pending_job_available_for_tests(&sqlite, job_id)?;
        }
        let job = claim_next_lifecycle_job(&sqlite)?.expect("claimed retry job");
        assert_eq!(job.attempts, expected_attempt);
        job_id = Some(job.job_id.clone());
        fail_or_requeue_lifecycle_job(&sqlite, &job, "still failing")?;
    }

    let rows = list_lifecycle_jobs_for_tests(&sqlite)?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, LifecycleJobStatus::Failed);
    assert_eq!(rows[0].last_error.as_deref(), Some("still failing"));
    Ok(())
}

#[test]
fn lifecycle_spool_initialise_copies_legacy_stop_jobs() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let sqlite = sqlite_at(&dir);
    sqlite.execute_batch(LIFECYCLE_STOP_SPOOL_SCHEMA_SQLITE)?;
    sqlite.with_write_connection(|conn| {
        conn.execute(
            "INSERT INTO agent_lifecycle_stop_spool_jobs (
                    job_id, repo_id, repo_root, config_root, agent_name, hook_name,
                    raw_stdin, workspace_snapshot, cwd, status, attempts, available_at_unix,
                    received_at_unix, updated_at_unix, last_error
                 ) VALUES (
                    'legacy-job-1', 'repo-legacy', ?1, ?2, ?3, 'Stop',
                    '{}', ?4, ?5, 'pending', 2, 10, 9, 11, 'old error'
                 )",
            params![
                dir.path().to_string_lossy().to_string(),
                dir.path()
                    .join(".bitloops-test-state/daemon")
                    .to_string_lossy()
                    .to_string(),
                crate::adapters::agents::AGENT_NAME_CODEX,
                serde_json::to_string(&LifecycleWorkspaceSnapshot::default())?,
                dir.path().to_string_lossy().to_string(),
            ],
        )?;
        Ok(())
    })?;

    initialise_lifecycle_spool_schema(&sqlite)?;

    let rows = list_lifecycle_jobs_for_tests(&sqlite)?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].job_id, "legacy-job-1");
    assert_eq!(rows[0].repo_id, "repo-legacy");
    assert_eq!(rows[0].status, LifecycleJobStatus::Pending);
    assert_eq!(rows[0].attempts, 2);
    assert_eq!(rows[0].last_error.as_deref(), Some("old error"));
    Ok(())
}

#[test]
fn lifecycle_spool_inserts_and_claims_pending_jobs() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let sqlite = sqlite_at(&dir);
    initialise_lifecycle_spool_schema(&sqlite)?;

    let inserted = enqueue_lifecycle_job_sqlite(
        &sqlite,
        sample_insert(
            dir.path(),
            crate::host::checkpoints::lifecycle::adapters::CODEX_HOOK_STOP,
        ),
    )?;
    assert_eq!(inserted.inserted_jobs, 1);

    let claimed = claim_next_lifecycle_job(&sqlite)?.expect("claimed lifecycle job");
    assert_eq!(claimed.repo_id, "repo-1");
    assert_eq!(
        claimed.agent_name,
        crate::adapters::agents::AGENT_NAME_CODEX
    );
    assert_eq!(claimed.status, LifecycleJobStatus::Running);

    assert!(claim_next_lifecycle_job(&sqlite)?.is_none());
    Ok(())
}

#[test]
fn lifecycle_spool_claims_at_most_one_job() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let sqlite = sqlite_at(&dir);
    initialise_lifecycle_spool_schema(&sqlite)?;

    enqueue_lifecycle_job_sqlite(&sqlite, sample_insert(dir.path(), "first-hook"))?;
    enqueue_lifecycle_job_sqlite(&sqlite, sample_insert(dir.path(), "second-hook"))?;

    let claimed = claim_next_lifecycle_job(&sqlite)?;

    assert!(claimed.is_some());
    Ok(())
}

#[test]
fn lifecycle_spool_recovers_running_jobs() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let sqlite = sqlite_at(&dir);
    initialise_lifecycle_spool_schema(&sqlite)?;
    enqueue_lifecycle_job_sqlite(&sqlite, sample_insert(dir.path(), "stop"))?;

    assert!(claim_next_lifecycle_job(&sqlite)?.is_some());

    let recovered = recover_running_lifecycle_jobs(&sqlite)?;
    assert_eq!(recovered, 1);

    let second_claim = claim_next_lifecycle_job(&sqlite)?.expect("claimed recovered job");
    assert_eq!(second_claim.attempts, 2);
    Ok(())
}

#[test]
fn lifecycle_spool_requeues_failed_jobs_with_error() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let sqlite = sqlite_at(&dir);
    initialise_lifecycle_spool_schema(&sqlite)?;
    enqueue_lifecycle_job_sqlite(&sqlite, sample_insert(dir.path(), "stop"))?;

    let claimed = claim_next_lifecycle_job(&sqlite)?.expect("claimed lifecycle job");
    requeue_lifecycle_job(&sqlite, &claimed.job_id, "parse failed")?;

    let rows = list_lifecycle_jobs_for_tests(&sqlite)?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, LifecycleJobStatus::Pending);
    assert_eq!(rows[0].last_error.as_deref(), Some("parse failed"));
    Ok(())
}

#[test]
fn lifecycle_spool_deletes_completed_jobs() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let sqlite = sqlite_at(&dir);
    initialise_lifecycle_spool_schema(&sqlite)?;
    enqueue_lifecycle_job_sqlite(&sqlite, sample_insert(dir.path(), "stop"))?;

    let claimed = claim_next_lifecycle_job(&sqlite)?.expect("claimed lifecycle job");
    delete_lifecycle_job(&sqlite, &claimed.job_id)?;

    assert!(list_lifecycle_jobs_for_tests(&sqlite)?.is_empty());
    Ok(())
}

#[test]
fn lifecycle_hook_enqueue_writes_sqlite_row_with_bounded_connection() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let db_path = dir.path().join("stores/runtime/runtime.sqlite");

    let insert = sample_insert(dir.path(), "stop");
    let raw_stdin = insert.raw_stdin.clone();
    let result = enqueue_lifecycle_job_hook_safe_at(&db_path, insert)?;

    assert_eq!(result.inserted_jobs, 1);
    let sqlite = SqliteConnectionPool::connect(db_path)?;
    let rows = list_lifecycle_jobs_for_tests(&sqlite)?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].raw_stdin, raw_stdin);
    Ok(())
}

#[test]
fn lifecycle_spool_round_trips_workspace_snapshot() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let sqlite = sqlite_at(&dir);
    initialise_lifecycle_spool_schema(&sqlite)?;
    let mut insert = sample_insert(dir.path(), "stop");
    insert.workspace_snapshot = Some(LifecycleWorkspaceSnapshot {
        modified_files: vec!["src/main.rs".to_string()],
        new_files: vec!["src/new.rs".to_string()],
        deleted_files: vec!["old.rs".to_string()],
    });

    enqueue_lifecycle_job_sqlite(&sqlite, insert)?;

    let rows = list_lifecycle_jobs_for_tests(&sqlite)?;
    assert_eq!(rows.len(), 1);
    let workspace_snapshot = rows[0]
        .workspace_snapshot
        .as_ref()
        .expect("workspace snapshot");
    assert_eq!(workspace_snapshot.modified_files, vec!["src/main.rs"]);
    assert_eq!(workspace_snapshot.new_files, vec!["src/new.rs"]);
    assert_eq!(workspace_snapshot.deleted_files, vec!["old.rs"]);
    Ok(())
}

#[test]
fn lifecycle_spool_round_trips_boundary_snapshot() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let sqlite = sqlite_at(&dir);
    initialise_lifecycle_spool_schema(&sqlite)?;

    let mut insert = sample_insert(dir.path(), "user-prompt-submit");
    insert.boundary_snapshot = Some(LifecycleBoundarySnapshot {
        workspace: Some(LifecycleWorkspaceSnapshot {
            modified_files: vec!["src/main.rs".to_string()],
            new_files: vec!["src/new.rs".to_string()],
            deleted_files: vec!["old.rs".to_string()],
        }),
        pre_untracked_files: vec!["scratch.txt".to_string()],
        transcript_offset: Some(42),
        branch_name: Some("feature/async-hooks".to_string()),
        is_default_branch: Some(false),
    });

    enqueue_lifecycle_job_sqlite(&sqlite, insert)?;
    let rows = list_lifecycle_jobs_for_tests(&sqlite)?;
    let snapshot = rows[0]
        .boundary_snapshot
        .clone()
        .expect("boundary snapshot should round-trip");

    assert_eq!(
        snapshot.workspace.as_ref().unwrap().modified_files,
        vec!["src/main.rs"]
    );
    assert_eq!(snapshot.pre_untracked_files, vec!["scratch.txt"]);
    assert_eq!(snapshot.transcript_offset, Some(42));
    assert_eq!(snapshot.branch_name.as_deref(), Some("feature/async-hooks"));
    assert_eq!(snapshot.is_default_branch, Some(false));
    Ok(())
}

#[test]
fn lifecycle_spool_migrates_legacy_workspace_snapshot_into_boundary_snapshot() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let sqlite = sqlite_at(&dir);
    sqlite.with_write_connection(|conn| {
        conn.execute_batch(LIFECYCLE_SPOOL_SCHEMA_SQLITE)?;
        conn.execute_batch(
            r#"
                INSERT INTO agent_lifecycle_spool_jobs (
                    job_id, repo_id, repo_root, config_root, agent_name, hook_name,
                    raw_stdin, workspace_snapshot, cwd, status, attempts,
                    available_at_unix, received_at_unix, updated_at_unix, last_error
                ) VALUES (
                    'legacy-job', 'repo-1', '/tmp/repo', '/tmp/config', 'codex', 'stop',
                    '{}', '{"modified_files":["legacy.rs"],"new_files":[],"deleted_files":[]}',
                    '/tmp/repo', 'pending', 0, 1, 1, 1, NULL
                );
                "#,
        )?;
        Ok(())
    })?;

    initialise_lifecycle_spool_schema(&sqlite)?;
    let rows = list_lifecycle_jobs_for_tests(&sqlite)?;
    let snapshot = rows[0]
        .boundary_snapshot
        .clone()
        .expect("legacy workspace snapshot should be exposed as boundary snapshot");

    assert_eq!(
        snapshot.workspace.as_ref().unwrap().modified_files,
        vec!["legacy.rs"]
    );
    Ok(())
}

#[test]
fn lifecycle_hook_enqueue_fails_quickly_when_runtime_sqlite_write_lock_is_held() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let db_path = dir.path().join("runtime.sqlite");
    let locker = rusqlite::Connection::open(&db_path)?;
    locker.execute_batch(LIFECYCLE_STOP_SPOOL_SCHEMA_SQLITE)?;
    locker.busy_timeout(Duration::from_secs(30))?;
    locker.execute_batch("BEGIN IMMEDIATE TRANSACTION;")?;

    let started = std::time::Instant::now();
    let result = enqueue_lifecycle_job_hook_safe_at(&db_path, sample_insert(dir.path(), "stop"));

    locker.execute_batch("ROLLBACK")?;
    assert!(result.is_err());
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "hook-safe enqueue waited too long: {:?}",
        started.elapsed()
    );
    Ok(())
}

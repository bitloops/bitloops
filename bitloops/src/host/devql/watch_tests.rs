use crate::host::runtime_store::RepoWatcherRegistration;
use crate::test_support::git_fixtures::init_test_repo;
use crate::test_support::log_capture::capture_logs_async;
use crate::test_support::process_state::with_env_var;

use std::process::Command;
use tempfile::TempDir;

use super::*;

fn seed_runtime_store() -> (TempDir, PathBuf, RepoSqliteRuntimeStore) {
    let dir = TempDir::new().expect("temp dir");
    let repo_root = dir.path().join("repo");
    fs::create_dir_all(&repo_root).expect("create repo root");
    init_test_repo(&repo_root, "main", "Bitloops Test", "bitloops@example.com");
    fs::write(
        repo_root.join(".bitloops.local.toml"),
        "[capture]\nenabled = true\nstrategy = \"manual-commit\"\n",
    )
    .expect("write repo-local watcher policy");
    let store = RepoSqliteRuntimeStore::open_for_roots(dir.path(), &repo_root)
        .expect("open repo runtime store");
    (dir, repo_root, store)
}

#[cfg(unix)]
fn spawn_detached_long_lived_process() -> u32 {
    let output = Command::new("sh")
        .args(["-c", "sleep 60 >/dev/null 2>&1 & echo $!"])
        .output()
        .expect("spawn detached long-lived process");
    assert!(
        output.status.success(),
        "failed to spawn detached long-lived process: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("detached pid stdout should be utf8")
        .trim()
        .parse()
        .expect("detached pid should parse")
}

#[cfg(windows)]
fn spawn_detached_long_lived_process() -> u32 {
    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "$p = Start-Process -FilePath ping -ArgumentList '-n 60 127.0.0.1' -WindowStyle Hidden -PassThru; $p.Id",
        ])
        .output()
        .expect("spawn detached long-lived process");
    assert!(
        output.status.success(),
        "failed to spawn detached long-lived process: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("detached pid stdout should be utf8")
        .trim()
        .parse()
        .expect("detached pid should parse")
}

fn wait_for_pid_exit(pid: u32) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if !process_is_running(pid) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "expected process {pid} to exit during watcher teardown"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn watcher_registration_round_trips_through_repo_runtime_store() {
    let (_dir, repo_root, store) = seed_runtime_store();

    store
        .save_watcher_registration(
            12345,
            "token-123",
            &repo_root,
            crate::host::runtime_store::RepoWatcherRegistrationState::Ready,
        )
        .expect("save watcher registration");
    let entry = store
        .load_watcher_registration()
        .expect("load watcher registration")
        .expect("watcher registration should exist");

    assert_eq!(entry.pid, 12345);
    assert_eq!(entry.restart_token, "token-123");
    assert_eq!(entry.repo_root, repo_root);
    assert_eq!(
        entry.state,
        crate::host::runtime_store::RepoWatcherRegistrationState::Ready
    );
}

#[test]
fn delete_watcher_registration_if_matches_preserves_mismatched_rows() {
    let (_dir, repo_root, store) = seed_runtime_store();

    store
        .save_watcher_registration(
            7,
            "token-a",
            &repo_root,
            crate::host::runtime_store::RepoWatcherRegistrationState::Ready,
        )
        .expect("seed watcher registration");
    store
        .delete_watcher_registration_if_matches(8, "token-b")
        .expect("conditional delete");

    assert!(
        store
            .load_watcher_registration()
            .expect("load watcher registration")
            .is_some(),
        "mismatched conditional delete should preserve the row"
    );
}

#[test]
fn registration_guard_writes_and_removes_owned_row() {
    let (_dir, repo_root, store) = seed_runtime_store();

    {
        let _guard = WatcherRegistrationGuard::acquire(store.clone(), &repo_root)
            .expect("acquire watcher registration guard");
        let entry = store
            .load_watcher_registration()
            .expect("load watcher registration")
            .expect("watcher registration should exist");
        assert_eq!(entry.pid, std::process::id());
        assert!(!entry.restart_token.is_empty());
        assert_eq!(
            entry.state,
            crate::host::runtime_store::RepoWatcherRegistrationState::Ready
        );
    }

    assert!(
        store
            .load_watcher_registration()
            .expect("load watcher registration after drop")
            .is_none(),
        "owned watcher registration should be removed on drop"
    );
}

#[test]
fn ensure_watcher_running_returns_early_when_autostart_is_disabled() {
    let (dir, repo_root, store) = seed_runtime_store();
    with_env_var(DISABLE_WATCHER_AUTOSTART_ENV, Some("1"), || {
        ensure_watcher_running(&repo_root, dir.path()).expect("autostart disabled");
    });

    assert!(
        store
            .load_watcher_registration()
            .expect("load watcher registration")
            .is_none(),
        "disabled autostart must not register a watcher"
    );
}

#[test]
fn dirty_worktree_paths_include_untracked_source_files() {
    let (_dir, repo_root, _store) = seed_runtime_store();
    let source_path = repo_root.join("src").join("math.rs");
    fs::create_dir_all(source_path.parent().expect("source parent")).expect("create src dir");
    fs::write(
        &source_path,
        "pub fn add(a: i32, b: i32) -> i32 { a + b }\n",
    )
    .expect("write source file");

    let paths = dirty_worktree_paths(&repo_root).expect("collect dirty worktree paths");

    assert!(
        paths.iter().any(|path| path.ends_with("src/math.rs")),
        "dirty worktree rescan should include untracked source files, got {paths:?}"
    );
}

#[test]
fn dirty_worktree_rescan_adds_paths_without_internal_store_paths() {
    let (_dir, repo_root, _store) = seed_runtime_store();
    let source_path = repo_root.join("src").join("math.rs");
    fs::create_dir_all(source_path.parent().expect("source parent")).expect("create src dir");
    fs::write(
        &source_path,
        "pub fn add(a: i32, b: i32) -> i32 { a + b }\n",
    )
    .expect("write source file");

    let internal_path = repo_root
        .join(".bitloops")
        .join("stores")
        .join("internal.rs");
    fs::create_dir_all(internal_path.parent().expect("internal parent"))
        .expect("create internal store dir");
    fs::write(&internal_path, "pub fn ignored() {}\n").expect("write internal store file");

    let mut batch = BTreeSet::new();
    assert!(
        add_dirty_worktree_paths_to_batch(&repo_root, &mut batch)
            .expect("add dirty worktree paths"),
        "source file should be added to the watcher batch"
    );

    assert!(
        batch.iter().any(|path| path.ends_with("src/math.rs")),
        "watcher fallback batch should contain source file, got {batch:?}"
    );
    assert!(
        batch
            .iter()
            .all(|path| !path.ends_with(".bitloops/stores/internal.rs")),
        "watcher fallback batch should omit Bitloops internal store files, got {batch:?}"
    );
}

#[test]
fn wait_for_watcher_registration_ready_ignores_stale_rows_until_expected_entry_exists() {
    let (_dir, repo_root, store) = seed_runtime_store();
    store
        .save_watcher_registration(
            7,
            "stale-token",
            &repo_root,
            crate::host::runtime_store::RepoWatcherRegistrationState::Ready,
        )
        .expect("seed stale watcher registration");

    let writer_store = store.clone();
    let writer_repo_root = repo_root.clone();
    let writer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(40));
        writer_store
            .save_watcher_registration(
                42,
                "ready-token",
                &writer_repo_root,
                crate::host::runtime_store::RepoWatcherRegistrationState::Ready,
            )
            .expect("publish ready watcher registration");
    });

    wait_for_watcher_registration_ready(
        42,
        "ready-token",
        Duration::from_millis(500),
        Duration::from_millis(10),
        || store.load_watcher_registration(),
        || Ok(true),
    )
    .expect("wait for expected watcher registration");

    writer.join().expect("join registration writer");
}

#[test]
fn wait_for_watcher_registration_ready_ignores_matching_pending_rows_until_ready() {
    let expected = RepoWatcherRegistration {
        repo_id: "repo-id".to_string(),
        repo_root: PathBuf::from("/tmp/repo"),
        pid: 42,
        restart_token: "ready-token".to_string(),
        state: crate::host::runtime_store::RepoWatcherRegistrationState::Pending,
    };
    let mut load_attempts = 0;

    wait_for_watcher_registration_ready(
        42,
        "ready-token",
        Duration::from_millis(100),
        Duration::from_millis(0),
        || {
            load_attempts += 1;
            if load_attempts < 3 {
                return Ok(Some(expected.clone()));
            }

            Ok(Some(RepoWatcherRegistration {
                state: crate::host::runtime_store::RepoWatcherRegistrationState::Ready,
                ..expected.clone()
            }))
        },
        || Ok(true),
    )
    .expect("wait for ready registration");

    assert!(
        load_attempts >= 3,
        "pending rows should not satisfy readiness"
    );
}

#[test]
fn matching_pending_registration_is_treated_as_inflight_start() {
    let entry = RepoWatcherRegistration {
        repo_id: "repo-id".to_string(),
        repo_root: PathBuf::from("/tmp/repo"),
        pid: 42,
        restart_token: "ready-token".to_string(),
        state: crate::host::runtime_store::RepoWatcherRegistrationState::Pending,
    };

    assert_eq!(
        classify_existing_watcher_registration(&entry, "ready-token", true),
        ExistingWatcherRegistrationDisposition::WaitForReady
    );
}

#[test]
fn wait_for_watcher_registration_ready_fails_when_child_exits_before_ready() {
    let (_dir, _repo_root, store) = seed_runtime_store();

    let err = wait_for_watcher_registration_ready(
        42,
        "ready-token",
        Duration::from_millis(100),
        Duration::from_millis(10),
        || store.load_watcher_registration(),
        || Ok(false),
    )
    .expect_err("readiness wait should fail when child exits");

    assert!(
        err.to_string().contains("exited before becoming ready"),
        "unexpected error: {err:#}"
    );
}

#[test]
fn timed_out_pending_registration_is_released_for_retry() {
    let (_dir, repo_root, store) = seed_runtime_store();
    let pid = std::process::id();
    store
        .save_watcher_registration(
            pid,
            "ready-token",
            &repo_root,
            crate::host::runtime_store::RepoWatcherRegistrationState::Pending,
        )
        .expect("seed pending watcher registration");
    let entry = store
        .load_watcher_registration()
        .expect("load watcher registration")
        .expect("watcher registration should exist");

    let outcome = handle_existing_watcher_registration(
        &store,
        entry,
        "ready-token",
        Duration::from_millis(0),
        Duration::from_millis(0),
    )
    .expect("timed out pending registration should be released");

    assert_eq!(outcome, ExistingWatcherRegistrationHandle::RetrySpawn);
    assert!(
        store
            .load_watcher_registration()
            .expect("load watcher registration after timeout recovery")
            .is_none(),
        "timeout recovery should clear stale pending ownership"
    );
}

#[test]
fn timed_out_pending_cleanup_allows_replacement_pending_claim() {
    let (_dir, repo_root, store) = seed_runtime_store();
    let stale_pid = std::process::id();
    let replacement_pid = stale_pid + 1;
    store
        .save_watcher_registration(
            stale_pid,
            "ready-token",
            &repo_root,
            crate::host::runtime_store::RepoWatcherRegistrationState::Pending,
        )
        .expect("seed pending watcher registration");

    let recovery = recover_timed_out_pending_registration(&store, stale_pid, "ready-token")
        .expect("recover timed out pending registration");
    assert_eq!(recovery, Some(TimedOutPendingRecovery::PendingReleased));

    let displaced = store
        .claim_pending_watcher_registration(replacement_pid, "ready-token", &repo_root)
        .expect("claim replacement pending watcher registration");
    assert!(
        displaced.is_none(),
        "replacement claim should succeed after stale pending ownership is cleared"
    );

    let entry = store
        .load_watcher_registration()
        .expect("load replacement watcher registration")
        .expect("replacement watcher registration should exist");
    assert_eq!(entry.pid, replacement_pid);
    assert_eq!(
        entry.state,
        crate::host::runtime_store::RepoWatcherRegistrationState::Pending
    );
}

#[test]
fn current_watcher_restart_token_hashes_the_current_binary() {
    let token = current_watcher_restart_token().expect("restart token");
    assert_eq!(token.len(), 64);
    assert!(token.chars().all(|ch| ch.is_ascii_hexdigit()));
}

#[test]
fn stop_watcher_terminates_registered_process_and_clears_registration() {
    let (_dir, repo_root, store) = seed_runtime_store();
    let watcher_pid = spawn_detached_long_lived_process();

    store
        .save_watcher_registration(
            watcher_pid,
            "stop-token",
            &repo_root,
            crate::host::runtime_store::RepoWatcherRegistrationState::Ready,
        )
        .expect("seed watcher registration");

    stop_watcher(&repo_root, _dir.path()).expect("stop watcher");

    wait_for_pid_exit(watcher_pid);
    assert!(
        store
            .load_watcher_registration()
            .expect("load watcher registration after stop")
            .is_none(),
        "watcher stop should clear the owned registration"
    );
}

#[test]
fn watcher_lifecycle_exits_when_registration_is_cleared() {
    let (_dir, repo_root, store) = seed_runtime_store();
    let cfg = crate::host::devql::DevqlConfig::from_roots(
        _dir.path().to_path_buf(),
        repo_root.clone(),
        crate::host::devql::resolve_repo_identity(&repo_root).expect("resolve repo identity"),
    )
    .expect("build watcher config");

    assert_eq!(
        evaluate_watcher_exit_reason(
            &cfg,
            &store,
            42,
            "missing-token",
            Instant::now(),
            Duration::from_secs(60),
            false,
        )
        .expect("evaluate watcher lifecycle"),
        Some(WatcherExitReason::RegistrationLost)
    );
}

#[test]
fn watcher_lifecycle_exits_after_idle_timeout_without_pending_batch() {
    let (_dir, repo_root, store) = seed_runtime_store();
    let pid = 42;
    let token = "idle-token";
    store
        .save_watcher_registration(
            pid,
            token,
            &repo_root,
            crate::host::runtime_store::RepoWatcherRegistrationState::Ready,
        )
        .expect("seed watcher registration");
    let cfg = crate::host::devql::DevqlConfig::from_roots(
        _dir.path().to_path_buf(),
        repo_root.clone(),
        crate::host::devql::resolve_repo_identity(&repo_root).expect("resolve repo identity"),
    )
    .expect("build watcher config");

    assert_eq!(
        evaluate_watcher_exit_reason(
            &cfg,
            &store,
            pid,
            token,
            Instant::now() - Duration::from_secs(5),
            Duration::from_secs(1),
            false,
        )
        .expect("evaluate watcher lifecycle"),
        Some(WatcherExitReason::Idle)
    );
    assert_eq!(
        evaluate_watcher_exit_reason(
            &cfg,
            &store,
            pid,
            token,
            Instant::now() - Duration::from_secs(5),
            Duration::from_secs(1),
            true,
        )
        .expect("evaluate watcher lifecycle with pending batch"),
        None
    );
}

#[test]
fn watcher_idle_timeout_uses_env_override_when_present() {
    assert_eq!(
        watcher_idle_timeout_from_env(Some("7")),
        Duration::from_secs(7)
    );
    assert_eq!(
        watcher_idle_timeout_from_env(Some("not-a-number")),
        DEFAULT_WATCHER_IDLE_TIMEOUT
    );
}

#[tokio::test]
async fn run_process_command_logs_terminal_failure() {
    let temp = TempDir::new().expect("temp dir");
    let missing_repo = temp.path().join("missing-repo");
    let daemon_config_root = temp.path().join("config-root");

    let (result, logs) = capture_logs_async(run_process_command(WatcherProcessArgs {
        repo_root: Some(missing_repo),
        daemon_config_root: Some(daemon_config_root),
    }))
    .await;

    assert!(result.is_err(), "missing repo should fail watcher startup");
    assert!(logs.iter().any(|entry| {
        entry.level == log::Level::Error && entry.message.contains("devql watcher failed")
    }));
}

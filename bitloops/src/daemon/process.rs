use super::types::INTERNAL_DAEMON_COMMAND_NAME;
use super::*;

#[path = "process/http.rs"]
mod http;
#[path = "process/liveness.rs"]
mod liveness;
#[path = "process/reaping.rs"]
mod reaping;
#[path = "process/shutdown.rs"]
mod shutdown;
#[path = "process/spawn.rs"]
mod spawn;
#[path = "process/termination.rs"]
mod termination;

pub(super) use self::http::{daemon_http_client, daemon_http_ready, query_health};
#[cfg(test)]
pub(super) use self::liveness::parse_internal_daemon_process_pids;
pub(super) use self::liveness::{
    process_is_running, running_internal_daemon_process_pids_for_config,
};
#[cfg(all(test, unix))]
pub(super) use self::reaping::reap_terminated_child_process;
#[cfg(unix)]
pub(super) use self::reaping::reap_terminated_child_processes;
pub(super) use self::shutdown::{
    cleanup_spawned_daemon_after_startup_failure, terminate_process,
    terminate_process_and_wait_for_shutdown_cleanup, wait_for_shutdown_cleanup,
};
pub(super) use self::spawn::{build_daemon_spawn_command, current_binary_fingerprint};
#[cfg(test)]
pub(super) use self::termination::ChildTerminationOutcome;
pub(super) use self::termination::ChildTerminationRecord;

#[cfg(test)]
use self::http::should_accept_invalid_daemon_certs;
#[cfg(all(test, not(windows)))]
use self::liveness::unix_kill_zero_indicates_running;

pub(super) const DAEMON_HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
pub(super) const DAEMON_HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
pub(super) const DAEMON_READY_REQUEST_TIMEOUT: Duration = Duration::from_millis(500);

#[cfg(test)]
mod tests {
    #[cfg(not(windows))]
    use super::unix_kill_zero_indicates_running;
    use super::{daemon_http_ready, should_accept_invalid_daemon_certs};
    #[cfg(unix)]
    use super::{
        process_is_running, reap_terminated_child_process,
        terminate_process_and_wait_for_shutdown_cleanup, wait_for_shutdown_cleanup,
    };
    use crate::daemon::{DaemonMode, DaemonRuntimeState};
    #[cfg(unix)]
    use crate::test_support::process_state::enter_process_state;
    #[cfg(unix)]
    use std::process::Command;
    use std::time::{Duration, Instant};
    #[cfg(unix)]
    use tempfile::TempDir;
    use tokio::net::TcpListener;
    use tokio::sync::oneshot;

    async fn start_nonresponsive_daemon_socket() -> (String, oneshot::Receiver<()>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind nonresponsive daemon socket");
        let port = listener.local_addr().expect("local addr").port();
        let (accepted_tx, accepted_rx) = oneshot::channel();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept daemon request");
            let _stream = stream;
            let _ = accepted_tx.send(());
            std::future::pending::<()>().await;
        });
        (format!("http://127.0.0.1:{port}"), accepted_rx)
    }

    fn runtime_state_for_url(url: String) -> DaemonRuntimeState {
        DaemonRuntimeState {
            version: 1,
            config_path: "/tmp/bitloops-test-config.toml".into(),
            config_root: "/tmp/bitloops-test-config-root".into(),
            pid: std::process::id(),
            mode: DaemonMode::Detached,
            service_name: None,
            url,
            host: "127.0.0.1".to_string(),
            port: 0,
            bundle_dir: "/tmp/bitloops-test-bundle".into(),
            relational_db_path: "/tmp/bitloops-test-relational.db".into(),
            events_db_path: "/tmp/bitloops-test-events.duckdb".into(),
            blob_store_path: "/tmp/bitloops-test-blob".into(),
            repo_registry_path: "/tmp/bitloops-test-repo-registry.json".into(),
            binary_fingerprint: String::new(),
            updated_at_unix: 0,
        }
    }

    #[test]
    fn daemon_http_client_only_relaxes_loopback_https_urls() {
        assert!(should_accept_invalid_daemon_certs("https://localhost:5667"));
        assert!(should_accept_invalid_daemon_certs("https://127.0.0.1:5667"));
        assert!(should_accept_invalid_daemon_certs("https://[::1]:5667"));
        assert!(!should_accept_invalid_daemon_certs("http://127.0.0.1:5667"));
        assert!(!should_accept_invalid_daemon_certs(
            "https://dev.internal:5667"
        ));
        assert!(!should_accept_invalid_daemon_certs("not-a-url"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn daemon_http_ready_times_out_when_daemon_accepts_without_reply() {
        let (url, accepted_rx) = start_nonresponsive_daemon_socket().await;
        let ready =
            tokio::spawn(async move { daemon_http_ready(&runtime_state_for_url(url)).await });

        tokio::time::timeout(Duration::from_secs(5), accepted_rx)
            .await
            .expect("daemon readiness request should reach the socket")
            .expect("daemon socket should report accepted request");

        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(2)).await;
        tokio::task::yield_now().await;

        assert!(
            ready.is_finished(),
            "daemon readiness probe should not wait forever when the daemon accepts but never replies"
        );
        assert!(
            !ready.await.expect("readiness task should join"),
            "nonresponsive daemon should not be reported ready"
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn unix_pid_probe_treats_permission_denied_as_running() {
        assert!(unix_kill_zero_indicates_running(-1, Some(libc::EPERM)));
        assert!(!unix_kill_zero_indicates_running(-1, Some(libc::ESRCH)));
        assert!(!unix_kill_zero_indicates_running(-1, Some(libc::EINVAL)));
        assert!(unix_kill_zero_indicates_running(0, None));
    }

    #[cfg(unix)]
    #[test]
    fn process_liveness_treats_zombie_child_as_exited() {
        let child = Command::new("sh")
            .args(["-c", "exit 0"])
            .spawn()
            .expect("spawn short-lived child");
        let pid = child.id();
        drop(child);

        let deadline = Instant::now() + Duration::from_secs(2);
        while process_is_running(pid).expect("inspect child process state before reap") {
            assert!(
                Instant::now() < deadline,
                "expected exited zombie child to be treated as no longer running"
            );
            std::thread::sleep(Duration::from_millis(10));
        }

        assert!(
            reap_terminated_child_process(pid, Duration::from_secs(1))
                .expect("reap child process by pid"),
            "expected exited child to be reaped"
        );
        assert!(
            !process_is_running(pid).expect("inspect child process state after reap"),
            "expected reaped child to disappear from process table"
        );
    }

    #[cfg(unix)]
    #[test]
    fn shutdown_cleanup_waits_for_child_process_exit_even_without_runtime_state() {
        let cwd = TempDir::new().expect("temp cwd");
        let state_root = TempDir::new().expect("temp state root");
        let state_root_str = state_root.path().to_string_lossy().to_string();
        let _guard = enter_process_state(
            Some(cwd.path()),
            &[(
                "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
                Some(state_root_str.as_str()),
            )],
        );
        let child = Command::new("sh")
            .args(["-c", "sleep 0.05"])
            .spawn()
            .expect("spawn short-lived child");
        let pid = child.id();
        drop(child);

        wait_for_shutdown_cleanup(pid, Duration::from_secs(1))
            .expect("wait for child shutdown cleanup");
        assert!(
            !process_is_running(pid).expect("inspect child process state after shutdown wait"),
            "expected shutdown cleanup to wait until the child is gone"
        );
    }

    #[cfg(unix)]
    #[test]
    fn shutdown_cleanup_force_kills_lingering_process_once_runtime_is_cleaned() {
        let cwd = TempDir::new().expect("temp cwd");
        let state_root = TempDir::new().expect("temp state root");
        let state_root_str = state_root.path().to_string_lossy().to_string();
        let _guard = enter_process_state(
            Some(cwd.path()),
            &[(
                "BITLOOPS_TEST_STATE_DIR_OVERRIDE",
                Some(state_root_str.as_str()),
            )],
        );
        let child = Command::new("sh")
            .args(["-c", "trap '' TERM; exec sleep 60"])
            .spawn()
            .expect("spawn TERM-ignoring child");
        let pid = child.id();
        drop(child);

        let started = Instant::now();
        terminate_process_and_wait_for_shutdown_cleanup(
            pid,
            Duration::from_secs(2),
            Duration::from_millis(100),
            Duration::from_secs(1),
        )
        .expect("force-kill lingering child during shutdown cleanup");

        assert!(
            started.elapsed() < Duration::from_millis(1500),
            "expected forced shutdown cleanup to finish early, elapsed={:?}",
            started.elapsed()
        );
        assert!(
            !process_is_running(pid).expect("inspect child process state after forced shutdown"),
            "expected forced shutdown cleanup to terminate lingering child"
        );
    }
}

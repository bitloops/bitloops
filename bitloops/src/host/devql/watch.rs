use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::{Args, Parser};
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use sha2::{Digest, Sha256};

#[path = "capture.rs"]
mod capture;

const WATCHER_PID_FILE_NAME: &str = "devql-watcher.pid";
const WATCHER_COMMAND_NAME: &str = "__devql-watcher";
pub const DISABLE_WATCHER_AUTOSTART_ENV: &str = "BITLOOPS_DISABLE_WATCHER_AUTOSTART";

#[derive(Debug, Clone, Args)]
pub struct WatcherProcessArgs {
    #[arg(long)]
    pub repo_root: Option<PathBuf>,
}

#[derive(Debug, Parser)]
pub struct WatcherProcessCli {
    #[command(flatten)]
    pub args: WatcherProcessArgs,
}

#[derive(Debug, Clone, Copy)]
pub struct DevqlWatchOptions {
    pub debounce_ms: u64,
    pub poll_fallback_ms: u64,
}

impl From<crate::config::WatchRuntimeConfig> for DevqlWatchOptions {
    fn from(value: crate::config::WatchRuntimeConfig) -> Self {
        Self {
            debounce_ms: value.watch_debounce_ms,
            poll_fallback_ms: value.watch_poll_fallback_ms,
        }
    }
}

pub fn watcher_pid_file(repo_root: &Path) -> PathBuf {
    repo_root
        .join(crate::utils::paths::BITLOOPS_DIR)
        .join(WATCHER_PID_FILE_NAME)
}

fn watcher_autostart_disabled() -> bool {
    env::var(DISABLE_WATCHER_AUTOSTART_ENV)
        .ok()
        .is_some_and(|value| !value.trim().is_empty() && value.trim() != "0")
}

pub fn ensure_watcher_running(repo_root: &Path) -> Result<()> {
    if watcher_autostart_disabled() {
        return Ok(());
    }

    let pid_file = watcher_pid_file(repo_root);
    let restart_token = current_watcher_restart_token()?;
    if let Some(entry) = read_pid_file(&pid_file)?
        && process_is_running(entry.pid)
    {
        if entry.restart_token.as_deref() == Some(restart_token.as_str()) {
            return Ok(());
        }
        // Restart token mismatch means a different binary is now serving watcher work.
        // Kill the stale watcher so the new process can re-run startup schema init.
        kill_process(entry.pid);
    }

    if pid_file.exists() {
        let _ = fs::remove_file(&pid_file);
    }

    let repo_root = repo_root
        .canonicalize()
        .unwrap_or_else(|_| repo_root.to_path_buf());
    let mut command = build_watcher_spawn_command(&repo_root)?;
    command
        // Avoid pinning the repository directory as the watcher cwd. Temp test
        // repos can be deleted while the detached watcher is still alive.
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let child = command
        .spawn()
        .with_context(|| format!("spawning DevQL watcher for {}", repo_root.display()))?;

    ensure_watcher_pid_parent_dir(&pid_file)?;
    fs::write(&pid_file, format!("{}\n{}", child.id(), restart_token))
        .with_context(|| format!("writing watcher pid file {}", pid_file.display()))?;

    Ok(())
}

pub async fn run_process_command(args: WatcherProcessArgs) -> Result<()> {
    let repo_root = resolve_repo_root(args.repo_root)?;
    let repo = crate::host::devql::resolve_repo_identity(&repo_root)?;
    let cfg = crate::host::devql::DevqlConfig::from_env(repo_root.clone(), repo)?;
    let watch_cfg = crate::config::resolve_watch_runtime_config_for_repo(&repo_root);
    let opts = DevqlWatchOptions::from(watch_cfg);
    let pid_file = watcher_pid_file(&repo_root);

    initialise_local_watch_schema(&repo_root)?;
    let _pid_guard = WatcherPidGuard::acquire(pid_file)?;

    let shutdown = Arc::new(AtomicBool::new(false));
    let worker_cfg = cfg.clone();
    let worker_shutdown = shutdown.clone();
    let runtime_handle = tokio::runtime::Handle::current();
    let mut worker = tokio::task::spawn_blocking(move || {
        run_notify_loop(&worker_cfg, opts, worker_shutdown, runtime_handle)
    });
    let shutdown_signal = wait_for_shutdown_signal();
    tokio::pin!(shutdown_signal);

    tokio::select! {
        worker_result = &mut worker => worker_result.context("joining watcher loop task")?,
        _ = &mut shutdown_signal => {
            shutdown.store(true, Ordering::SeqCst);
            worker.await.context("joining watcher loop task after shutdown")?
        }
    }
}

pub fn run_process_from_cli() -> Result<()> {
    let cli = WatcherProcessCli::parse();
    let runtime = tokio::runtime::Runtime::new().context("creating watcher runtime")?;
    runtime.block_on(run_process_command(cli.args))
}

fn resolve_repo_root(explicit_repo_root: Option<PathBuf>) -> Result<PathBuf> {
    match explicit_repo_root {
        Some(repo_root) => Ok(repo_root),
        None => crate::utils::paths::repo_root(),
    }
}

fn initialise_local_watch_schema(repo_root: &Path) -> Result<()> {
    let backend_cfg = crate::config::resolve_store_backend_config_for_repo(repo_root)
        .context("resolving store config for watcher start")?;
    let sqlite_path = crate::config::resolve_sqlite_db_path_for_repo(
        repo_root,
        backend_cfg.relational.sqlite_path.as_deref(),
    )
    .context("resolving SQLite path for watcher start")?;
    let sqlite = crate::storage::SqliteConnectionPool::connect(sqlite_path)?;
    sqlite.initialise_devql_schema()?;
    Ok(())
}

fn run_notify_loop(
    cfg: &crate::host::devql::DevqlConfig,
    opts: DevqlWatchOptions,
    shutdown: Arc<AtomicBool>,
    runtime_handle: tokio::runtime::Handle,
) -> Result<()> {
    let (tx, rx) = channel();
    let mut watcher = RecommendedWatcher::new(
        move |event| {
            let _ = tx.send(event);
        },
        Config::default().with_poll_interval(Duration::from_millis(opts.poll_fallback_ms.max(250))),
    )
    .context("creating file watcher")?;

    watcher
        .watch(&cfg.repo_root, RecursiveMode::Recursive)
        .with_context(|| format!("watching repo {}", cfg.repo_root.display()))?;

    let debounce = Duration::from_millis(opts.debounce_ms.max(50));
    let mut batch: BTreeSet<PathBuf> = BTreeSet::new();
    let mut window_start: Option<Instant> = None;

    while !shutdown.load(Ordering::SeqCst) {
        if watcher_repo_root_missing(&cfg.repo_root) {
            return Ok(());
        }

        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Ok(event)) => {
                for path in event.paths {
                    if should_ignore_path(&cfg.repo_root, &path)
                        || is_gitignored(&cfg.repo_root, &path)
                    {
                        continue;
                    }
                    batch.insert(path);
                }
                if !batch.is_empty() && window_start.is_none() {
                    window_start = Some(Instant::now());
                }
            }
            Ok(Err(err)) => {
                if watcher_repo_root_missing(&cfg.repo_root) {
                    return Ok(());
                }
                log::warn!("devql watcher event error: {err:#}");
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                anyhow::bail!("devql watcher channel disconnected")
            }
        }

        if let Some(start) = window_start
            && !batch.is_empty()
            && start.elapsed() >= debounce
        {
            let paths = batch.iter().cloned().collect::<Vec<_>>();
            if let Err(err) = capture::capture_temporary_checkpoint_batch_with_handle(
                cfg,
                &paths,
                &runtime_handle,
            ) {
                log::warn!("devql watcher capture failed: {err:#}");
            }
            batch.clear();
            window_start = None;
        }
    }

    Ok(())
}

fn watcher_repo_root_missing(repo_root: &Path) -> bool {
    repo_root.try_exists().map(|exists| !exists).unwrap_or(true)
}

fn should_ignore_path(repo_root: &Path, path: &Path) -> bool {
    let rel = path.strip_prefix(repo_root).unwrap_or(path);
    let rel_str = rel.to_string_lossy();

    if rel_str.starts_with(".git/") || rel_str.starts_with(".bitloops/") {
        return true;
    }
    if rel_str.contains("/node_modules/") || rel_str.contains("/target/") {
        return true;
    }
    if rel_str.ends_with('~')
        || rel_str.ends_with(".swp")
        || rel_str.ends_with(".tmp")
        || rel_str.ends_with(".temp")
    {
        return true;
    }
    false
}

fn is_gitignored(repo_root: &Path, path: &Path) -> bool {
    let rel = path.strip_prefix(repo_root).unwrap_or(path);
    let rel_str = rel.to_string_lossy();
    if rel_str.is_empty() {
        return false;
    }

    crate::host::checkpoints::strategy::manual_commit::run_git(
        repo_root,
        &["check-ignore", "-q", &rel_str],
    )
    .is_ok()
}

fn build_watcher_spawn_command(repo_root: &Path) -> Result<Command> {
    #[cfg(unix)]
    use std::os::unix::process::CommandExt;

    let current_exe =
        std::env::current_exe().context("resolving current executable for watcher")?;
    let mut command = Command::new(current_exe);
    command.arg(WATCHER_COMMAND_NAME);
    command.arg("--repo-root").arg(repo_root);
    #[cfg(unix)]
    {
        command.process_group(0);
    }
    Ok(command)
}

fn ensure_watcher_pid_parent_dir(pid_file: &Path) -> Result<()> {
    let parent = pid_file
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .context("resolving watcher pid parent directory")?;
    fs::create_dir_all(parent)
        .with_context(|| format!("creating watcher pid directory {}", parent.display()))
}

pub(crate) struct PidFileEntry {
    pub(crate) pid: u32,
    /// `None` when the pid file was written by an older build that did not include a restart token.
    /// A missing token is treated as a mismatch, triggering a watcher restart.
    pub(crate) restart_token: Option<String>,
}

fn read_pid_file(pid_file: &Path) -> Result<Option<PidFileEntry>> {
    let data = match fs::read_to_string(pid_file) {
        Ok(data) => data,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(err)
                .with_context(|| format!("reading watcher pid file {}", pid_file.display()));
        }
    };

    let mut lines = data.lines();
    let pid = match lines.next().and_then(|l| l.trim().parse::<u32>().ok()) {
        Some(pid) => pid,
        None => return Ok(None),
    };
    let restart_token = lines
        .next()
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_string);
    Ok(Some(PidFileEntry { pid, restart_token }))
}

fn kill_process(pid: u32) {
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn process_is_running(pid: u32) -> bool {
    #[cfg(windows)]
    {
        Command::new("cmd")
            .args([
                "/C",
                &format!("tasklist /FI \"PID eq {pid}\" | findstr {pid}"),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    #[cfg(not(windows))]
    {
        Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }
}

struct WatcherPidGuard {
    pid_file: PathBuf,
    pid: u32,
    restart_token: String,
}

impl WatcherPidGuard {
    fn acquire(pid_file: PathBuf) -> Result<Self> {
        ensure_watcher_pid_parent_dir(&pid_file)?;
        let pid = std::process::id();
        let restart_token = current_watcher_restart_token()?;
        fs::write(&pid_file, format!("{pid}\n{restart_token}"))
            .with_context(|| format!("writing watcher pid file {}", pid_file.display()))?;
        Ok(Self {
            pid_file,
            pid,
            restart_token,
        })
    }
}

impl Drop for WatcherPidGuard {
    fn drop(&mut self) {
        let entry = read_pid_file(&self.pid_file).ok().flatten();
        if entry.as_ref().map(|entry| entry.pid) == Some(self.pid)
            && entry.and_then(|entry| entry.restart_token) == Some(self.restart_token.clone())
        {
            let _ = fs::remove_file(&self.pid_file);
        }
    }
}

fn current_watcher_restart_token() -> Result<String> {
    let current_exe =
        std::env::current_exe().context("resolving current executable for watcher")?;
    let bytes = fs::read(&current_exe)
        .with_context(|| format!("reading watcher executable {}", current_exe.display()))?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        let mut sigterm =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).ok();
        let ctrl_c = async {
            if tokio::signal::ctrl_c().await.is_err() {
                std::future::pending::<()>().await;
            }
        };
        tokio::select! {
            _ = ctrl_c => {}
            _ = async {
                if let Some(sigterm) = sigterm.as_mut() {
                    sigterm.recv().await;
                } else {
                    std::future::pending::<()>().await;
                }
            } => {}
        }
    }

    #[cfg(not(unix))]
    {
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::test_support::process_state::with_env_var;
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    // ── helpers ──────────────────────────────────────────────────────────────

    fn write_pid_file(dir: &TempDir, content: &str) -> PathBuf {
        let pid_file = dir.path().join("devql-watcher.pid");
        fs::write(&pid_file, content).expect("write pid file");
        pid_file
    }

    // ── read_pid_file ─────────────────────────────────────────────────────────

    #[test]
    fn read_pid_file_returns_none_when_missing() {
        let dir = TempDir::new().expect("temp dir");
        let pid_file = dir.path().join("missing.pid");
        let result = read_pid_file(&pid_file).expect("read should not error");
        assert!(result.is_none());
    }

    #[test]
    fn read_pid_file_parses_legacy_single_line_format() {
        let dir = TempDir::new().expect("temp dir");
        let pid_file = write_pid_file(&dir, "12345\n");
        let entry = read_pid_file(&pid_file)
            .expect("read ok")
            .expect("entry present");
        assert_eq!(entry.pid, 12345);
        assert!(
            entry.restart_token.is_none(),
            "single-line file should yield no restart_token"
        );
    }

    #[test]
    fn read_pid_file_parses_two_line_format_with_restart_token() {
        let dir = TempDir::new().expect("temp dir");
        let pid_file = write_pid_file(&dir, "99\ntoken-123\n");
        let entry = read_pid_file(&pid_file)
            .expect("read ok")
            .expect("entry present");
        assert_eq!(entry.pid, 99);
        assert_eq!(entry.restart_token.as_deref(), Some("token-123"));
    }

    #[test]
    fn read_pid_file_returns_none_for_non_numeric_pid() {
        let dir = TempDir::new().expect("temp dir");
        let pid_file = write_pid_file(&dir, "not-a-pid\n1\n");
        let result = read_pid_file(&pid_file).expect("read should not error");
        assert!(
            result.is_none(),
            "non-numeric first line should return None"
        );
    }

    #[test]
    fn read_pid_file_accepts_missing_restart_token_line() {
        // File with pid but no trailing newline or restart token line
        let dir = TempDir::new().expect("temp dir");
        let pid_file = write_pid_file(&dir, "42");
        let entry = read_pid_file(&pid_file)
            .expect("read ok")
            .expect("entry present");
        assert_eq!(entry.pid, 42);
        assert!(entry.restart_token.is_none());
    }

    #[test]
    fn read_pid_file_keeps_non_numeric_restart_token() {
        let dir = TempDir::new().expect("temp dir");
        let pid_file = write_pid_file(&dir, "77\nbad-version\n");
        let entry = read_pid_file(&pid_file)
            .expect("read ok")
            .expect("entry present");
        assert_eq!(entry.pid, 77);
        assert_eq!(entry.restart_token.as_deref(), Some("bad-version"));
    }

    // ── WatcherPidGuard ───────────────────────────────────────────────────────

    #[test]
    fn pid_guard_writes_pid_file_with_restart_token() {
        let dir = TempDir::new().expect("temp dir");
        let pid_file = dir.path().join("devql-watcher.pid");
        {
            let _guard = WatcherPidGuard::acquire(pid_file.clone()).expect("acquire guard");
            let content = fs::read_to_string(&pid_file).expect("read pid file");
            let mut lines = content.lines();
            let pid_str = lines.next().expect("pid line");
            let restart_token = lines.next().expect("restart token line");
            let pid: u32 = pid_str.parse().expect("pid is numeric");
            assert_eq!(pid, std::process::id());
            assert!(
                !restart_token.is_empty(),
                "restart token should not be empty"
            );
        }
        // Guard dropped — file should be cleaned up
        assert!(
            !pid_file.exists(),
            "pid file should be removed when guard is dropped"
        );
    }

    #[test]
    fn pid_guard_does_not_remove_file_if_pid_was_overwritten() {
        let dir = TempDir::new().expect("temp dir");
        let pid_file = dir.path().join("devql-watcher.pid");
        let guard = WatcherPidGuard::acquire(pid_file.clone()).expect("acquire guard");
        // Overwrite with a different pid so the guard's ownership check fails
        fs::write(&pid_file, "99999\ndifferent-token\n").expect("overwrite pid file");
        drop(guard);
        // File should still exist because the guard saw a different pid
        assert!(
            pid_file.exists(),
            "pid file should not be removed when pid has been overwritten"
        );
    }

    // ── restart token written by ensure_watcher_running ──────────────────────

    #[test]
    fn ensure_watcher_running_pid_file_contains_current_restart_token() {
        // We can't easily spawn a real watcher in a unit test, but we CAN verify that
        // `WatcherPidGuard::acquire` encodes the right restart token, which is the same path
        // used by the spawned watcher process.
        let dir = TempDir::new().expect("temp dir");
        let pid_file = dir.path().join("devql-watcher.pid");
        let _guard = WatcherPidGuard::acquire(pid_file.clone()).expect("acquire");
        let entry = read_pid_file(&pid_file)
            .expect("read ok")
            .expect("entry present");
        assert_eq!(
            entry.restart_token,
            Some(current_watcher_restart_token().expect("restart token")),
            "pid file written by WatcherPidGuard must carry the current restart token"
        );
    }

    #[test]
    fn ensure_watcher_running_returns_early_when_autostart_disabled_env_is_set() {
        let dir = TempDir::new().expect("temp dir");
        let pid_file = watcher_pid_file(dir.path());

        with_env_var(DISABLE_WATCHER_AUTOSTART_ENV, Some("1"), || {
            ensure_watcher_running(dir.path()).expect("autostart-disabled no-op should succeed");
        });

        assert!(
            !pid_file.exists(),
            "watcher pid file should not be created when autostart is disabled"
        );
    }

    #[test]
    fn watcher_repo_root_missing_returns_true_after_repo_is_deleted() {
        let dir = TempDir::new().expect("temp dir");
        let repo_root = dir.path().to_path_buf();

        assert!(!watcher_repo_root_missing(&repo_root));

        drop(dir);

        assert!(watcher_repo_root_missing(&repo_root));
    }

    // ── restart token mismatch detection ─────────────────────────────────────

    #[test]
    fn legacy_pid_file_restart_token_is_none_triggering_restart_logic() {
        // Simulate an old pid file with no restart token line.
        let dir = TempDir::new().expect("temp dir");
        let pid_file = write_pid_file(&dir, "12345\n");
        let entry = read_pid_file(&pid_file)
            .expect("read ok")
            .expect("entry present");
        assert_ne!(
            entry.restart_token,
            Some(current_watcher_restart_token().expect("restart token")),
            "legacy pid file (no token) must not match current restart token"
        );
    }

    #[test]
    fn stale_restart_token_in_pid_file_does_not_match_current() {
        let dir = TempDir::new().expect("temp dir");
        let pid_file = write_pid_file(&dir, "12345\nstale-token\n");
        let entry = read_pid_file(&pid_file)
            .expect("read ok")
            .expect("entry present");
        assert_ne!(
            entry.restart_token,
            Some(current_watcher_restart_token().expect("restart token")),
            "stale restart token must not match current restart token"
        );
    }

    #[test]
    fn current_restart_token_matches_runtime_value() {
        let dir = TempDir::new().expect("temp dir");
        let token = current_watcher_restart_token().expect("restart token");
        let pid_file = write_pid_file(&dir, &format!("1\n{token}\n"));
        let entry = read_pid_file(&pid_file)
            .expect("read ok")
            .expect("entry present");
        assert_eq!(
            entry.restart_token,
            Some(token),
            "correctly tokened pid file must match current restart token"
        );
    }
}

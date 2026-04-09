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

use crate::host::relational_store::DefaultRelationalStore;
use crate::host::runtime_store::RepoSqliteRuntimeStore;

#[path = "capture.rs"]
mod capture;

const WATCHER_COMMAND_NAME: &str = "__devql-watcher";
pub const DISABLE_WATCHER_AUTOSTART_ENV: &str = "BITLOOPS_DISABLE_WATCHER_AUTOSTART";

#[derive(Debug, Clone, Args)]
pub struct WatcherProcessArgs {
    #[arg(long)]
    pub repo_root: Option<PathBuf>,

    #[arg(long = "daemon-config-root", alias = "config-root", hide = true)]
    pub daemon_config_root: Option<PathBuf>,
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

fn watcher_autostart_disabled() -> bool {
    env::var(DISABLE_WATCHER_AUTOSTART_ENV)
        .ok()
        .is_some_and(|value| !value.trim().is_empty() && value.trim() != "0")
}

pub fn ensure_watcher_running(repo_root: &Path, daemon_config_root: &Path) -> Result<()> {
    if watcher_autostart_disabled() {
        return Ok(());
    }

    let restart_token = current_watcher_restart_token()?;
    let repo_root = repo_root
        .canonicalize()
        .unwrap_or_else(|_| repo_root.to_path_buf());
    let daemon_config_root = daemon_config_root
        .canonicalize()
        .unwrap_or_else(|_| daemon_config_root.to_path_buf());
    let runtime_store = RepoSqliteRuntimeStore::open_for_roots(&daemon_config_root, &repo_root)?;

    if let Some(entry) = runtime_store.load_watcher_registration()? {
        if process_is_running(entry.pid) {
            if entry.restart_token == restart_token {
                return Ok(());
            }
            // Restart token mismatch means a different binary is now serving watcher work.
            // Kill the stale watcher so the new process can re-run startup schema init.
            kill_process(entry.pid);
        }
        runtime_store.clear_watcher_registration()?;
    }

    let mut command = build_watcher_spawn_command(&repo_root, &daemon_config_root)?;
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
    runtime_store.save_watcher_registration(child.id(), &restart_token, &repo_root)?;

    Ok(())
}

pub fn restart_watcher(repo_root: &Path, daemon_config_root: &Path) -> Result<()> {
    let runtime_store = RepoSqliteRuntimeStore::open_for_roots(daemon_config_root, repo_root)?;
    if let Some(entry) = runtime_store.load_watcher_registration()?
        && process_is_running(entry.pid)
    {
        kill_process(entry.pid);
    }
    runtime_store.clear_watcher_registration()?;
    if crate::config::settings::is_enabled_for_hooks(repo_root) {
        ensure_watcher_running(repo_root, daemon_config_root)?;
    }
    Ok(())
}

pub async fn run_process_command(args: WatcherProcessArgs) -> Result<()> {
    let repo_root = resolve_repo_root(args.repo_root)?;
    let daemon_config_root = resolve_daemon_config_root(args.daemon_config_root, &repo_root)?;
    let repo = crate::host::devql::resolve_repo_identity(&repo_root)?;
    let cfg = crate::host::devql::DevqlConfig::from_roots(
        daemon_config_root.clone(),
        repo_root.clone(),
        repo,
    )?;
    let _ = crate::host::devql::load_repo_exclusion_matcher(&repo_root)
        .context("loading repo policy exclusions for DevQL watcher start")?;
    let watch_cfg = crate::config::resolve_watch_runtime_config_for_repo(&repo_root);
    let opts = DevqlWatchOptions::from(watch_cfg);

    initialise_local_watch_schema(&repo_root, &daemon_config_root)?;
    let runtime_store = RepoSqliteRuntimeStore::open_for_roots(&daemon_config_root, &repo_root)?;
    let _registration_guard = WatcherRegistrationGuard::acquire(runtime_store, &repo_root)?;

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

fn resolve_daemon_config_root(
    explicit_daemon_config_root: Option<PathBuf>,
    repo_root: &Path,
) -> Result<PathBuf> {
    match explicit_daemon_config_root {
        Some(daemon_config_root) => Ok(daemon_config_root),
        None => crate::config::resolve_daemon_config_root_for_repo(repo_root),
    }
}

fn initialise_local_watch_schema(repo_root: &Path, daemon_config_root: &Path) -> Result<()> {
    let relational = DefaultRelationalStore::open_local_for_roots(daemon_config_root, repo_root)
        .context("opening local relational store for watcher start")?;
    relational.initialise_local_devql_schema()
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
                    if is_repo_policy_excluded(&cfg.repo_root, &path)? {
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

fn is_repo_policy_excluded(repo_root: &Path, path: &Path) -> Result<bool> {
    let matcher = crate::host::devql::load_repo_exclusion_matcher(repo_root)
        .context("loading repo policy exclusions for watcher evaluation")?;
    let rel = path.strip_prefix(repo_root).unwrap_or(path);
    Ok(matcher.excludes_path(rel))
}

fn build_watcher_spawn_command(repo_root: &Path, daemon_config_root: &Path) -> Result<Command> {
    #[cfg(unix)]
    use std::os::unix::process::CommandExt;

    let current_exe =
        std::env::current_exe().context("resolving current executable for watcher")?;
    let mut command = Command::new(current_exe);
    command.arg(WATCHER_COMMAND_NAME);
    command.arg("--repo-root").arg(repo_root);
    command.arg("--daemon-config-root").arg(daemon_config_root);
    #[cfg(unix)]
    {
        command.process_group(0);
    }
    Ok(command)
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

struct WatcherRegistrationGuard {
    runtime_store: RepoSqliteRuntimeStore,
    pid: u32,
    restart_token: String,
}

impl WatcherRegistrationGuard {
    fn acquire(runtime_store: RepoSqliteRuntimeStore, repo_root: &Path) -> Result<Self> {
        let pid = std::process::id();
        let restart_token = current_watcher_restart_token()?;
        runtime_store.save_watcher_registration(pid, &restart_token, repo_root)?;
        Ok(Self {
            runtime_store,
            pid,
            restart_token,
        })
    }
}

impl Drop for WatcherRegistrationGuard {
    fn drop(&mut self) {
        let _ = self
            .runtime_store
            .delete_watcher_registration_if_matches(self.pid, &self.restart_token);
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
    use crate::test_support::git_fixtures::init_test_repo;
    use crate::test_support::process_state::with_env_var;

    use tempfile::TempDir;

    use super::*;

    fn seed_runtime_store() -> (TempDir, PathBuf, RepoSqliteRuntimeStore) {
        let dir = TempDir::new().expect("temp dir");
        let repo_root = dir.path().join("repo");
        fs::create_dir_all(&repo_root).expect("create repo root");
        init_test_repo(&repo_root, "main", "Bitloops Test", "bitloops@example.com");
        let store = RepoSqliteRuntimeStore::open_for_roots(dir.path(), &repo_root)
            .expect("open repo runtime store");
        (dir, repo_root, store)
    }

    #[test]
    fn watcher_registration_round_trips_through_repo_runtime_store() {
        let (_dir, repo_root, store) = seed_runtime_store();

        store
            .save_watcher_registration(12345, "token-123", &repo_root)
            .expect("save watcher registration");
        let entry = store
            .load_watcher_registration()
            .expect("load watcher registration")
            .expect("watcher registration should exist");

        assert_eq!(entry.pid, 12345);
        assert_eq!(entry.restart_token, "token-123");
        assert_eq!(entry.repo_root, repo_root);
    }

    #[test]
    fn delete_watcher_registration_if_matches_preserves_mismatched_rows() {
        let (_dir, repo_root, store) = seed_runtime_store();

        store
            .save_watcher_registration(7, "token-a", &repo_root)
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
    fn current_watcher_restart_token_hashes_the_current_binary() {
        let token = current_watcher_restart_token().expect("restart token");
        assert_eq!(token.len(), 64);
        assert!(token.chars().all(|ch| ch.is_ascii_hexdigit()));
    }
}

use std::collections::BTreeSet;
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

mod capture;

const WATCHER_PID_FILE_NAME: &str = "devql-watcher.pid";
const WATCHER_COMMAND_NAME: &str = "__devql-watcher";

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

impl From<crate::store_config::WatchRuntimeConfig> for DevqlWatchOptions {
    fn from(value: crate::store_config::WatchRuntimeConfig) -> Self {
        Self {
            debounce_ms: value.watch_debounce_ms,
            poll_fallback_ms: value.watch_poll_fallback_ms,
        }
    }
}

pub fn watcher_pid_file(repo_root: &Path) -> PathBuf {
    repo_root
        .join(crate::engine::paths::BITLOOPS_DIR)
        .join(WATCHER_PID_FILE_NAME)
}

pub fn ensure_watcher_running(repo_root: &Path) -> Result<()> {
    let pid_file = watcher_pid_file(repo_root);
    if let Some(pid) = read_pid_file(&pid_file)?
        && process_is_running(pid)
    {
        return Ok(());
    }

    if pid_file.exists() {
        let _ = fs::remove_file(&pid_file);
    }

    let repo_root = repo_root
        .canonicalize()
        .unwrap_or_else(|_| repo_root.to_path_buf());
    let mut command = build_watcher_spawn_command(&repo_root)?;
    command
        .current_dir(&repo_root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let child = command
        .spawn()
        .with_context(|| format!("spawning DevQL watcher for {}", repo_root.display()))?;

    ensure_watcher_pid_parent_dir(&pid_file)?;
    fs::write(&pid_file, child.id().to_string())
        .with_context(|| format!("writing watcher pid file {}", pid_file.display()))?;

    Ok(())
}

pub async fn run_process_command(args: WatcherProcessArgs) -> Result<()> {
    let repo_root = resolve_repo_root(args.repo_root)?;
    let repo = crate::engine::devql::resolve_repo_identity(&repo_root)?;
    let cfg = crate::engine::devql::DevqlConfig::from_env(repo_root.clone(), repo)?;
    let watch_cfg = crate::store_config::resolve_watch_runtime_config_for_repo(&repo_root);
    let opts = DevqlWatchOptions::from(watch_cfg);
    let pid_file = watcher_pid_file(&repo_root);

    initialise_local_watch_schema(&repo_root)?;
    let _pid_guard = WatcherPidGuard::acquire(pid_file)?;

    let shutdown = Arc::new(AtomicBool::new(false));
    let worker_cfg = cfg.clone();
    let worker_shutdown = shutdown.clone();
    let mut worker =
        tokio::task::spawn_blocking(move || run_notify_loop(&worker_cfg, opts, worker_shutdown));
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
        None => crate::engine::paths::repo_root(),
    }
}

fn initialise_local_watch_schema(repo_root: &Path) -> Result<()> {
    let backend_cfg = crate::store_config::resolve_store_backend_config_for_repo(repo_root)
        .context("resolving store config for watcher start")?;
    let sqlite_path = crate::store_config::resolve_sqlite_db_path_for_repo(
        repo_root,
        backend_cfg.relational.sqlite_path.as_deref(),
    )
    .context("resolving SQLite path for watcher start")?;
    let sqlite = crate::engine::db::SqliteConnectionPool::connect(sqlite_path)?;
    sqlite.initialise_checkpoint_schema()?;
    sqlite.initialise_devql_schema()?;
    Ok(())
}

fn run_notify_loop(
    cfg: &crate::engine::devql::DevqlConfig,
    opts: DevqlWatchOptions,
    shutdown: Arc<AtomicBool>,
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
            if let Err(err) = capture::capture_temporary_checkpoint_batch(cfg, &paths) {
                log::warn!("devql watcher capture failed: {err:#}");
            }
            batch.clear();
            window_start = None;
        }
    }

    Ok(())
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

    crate::engine::strategy::manual_commit::run_git(repo_root, &["check-ignore", "-q", &rel_str])
        .is_ok()
}

fn build_watcher_spawn_command(repo_root: &Path) -> Result<Command> {
    let current_exe =
        std::env::current_exe().context("resolving current executable for watcher")?;
    let mut command = Command::new(current_exe);
    command.arg(WATCHER_COMMAND_NAME);
    command.arg("--repo-root").arg(repo_root);
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

fn read_pid_file(pid_file: &Path) -> Result<Option<u32>> {
    let data = match fs::read_to_string(pid_file) {
        Ok(data) => data,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(err)
                .with_context(|| format!("reading watcher pid file {}", pid_file.display()));
        }
    };

    Ok(data.trim().parse::<u32>().ok())
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
}

impl WatcherPidGuard {
    fn acquire(pid_file: PathBuf) -> Result<Self> {
        ensure_watcher_pid_parent_dir(&pid_file)?;
        let pid = std::process::id();
        fs::write(&pid_file, pid.to_string())
            .with_context(|| format!("writing watcher pid file {}", pid_file.display()))?;
        Ok(Self { pid_file, pid })
    }
}

impl Drop for WatcherPidGuard {
    fn drop(&mut self) {
        let current_pid = fs::read_to_string(&self.pid_file)
            .ok()
            .and_then(|value| value.trim().parse::<u32>().ok());
        if current_pid == Some(self.pid) {
            let _ = fs::remove_file(&self.pid_file);
        }
    }
}

async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        let mut sigterm =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).ok();
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
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
        let _ = tokio::signal::ctrl_c().await;
    }
}

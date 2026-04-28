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
const WATCHER_READY_TIMEOUT: Duration = Duration::from_secs(5);
const WATCHER_READY_POLL_INTERVAL: Duration = Duration::from_millis(25);
const WATCHER_RESCAN_MIN_INTERVAL: Duration = Duration::from_secs(2);
const WATCHER_STOP_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_WATCHER_IDLE_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const WATCHER_IDLE_TIMEOUT_ENV: &str = "BITLOOPS_WATCHER_IDLE_TIMEOUT_SECS";

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

    loop {
        if let Some(entry) = runtime_store.load_watcher_registration()? {
            match handle_existing_watcher_registration(
                &runtime_store,
                entry,
                &restart_token,
                WATCHER_READY_TIMEOUT,
                WATCHER_READY_POLL_INTERVAL,
            )? {
                ExistingWatcherRegistrationHandle::Handled => return Ok(()),
                ExistingWatcherRegistrationHandle::RetrySpawn => continue,
            }
        }

        let mut command = build_watcher_spawn_command(&repo_root, &daemon_config_root)?;
        command
            // Avoid pinning the repository directory as the watcher cwd. Temp test
            // repos can be deleted while the detached watcher is still alive.
            .current_dir(std::env::temp_dir())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        let mut child = command
            .spawn()
            .with_context(|| format!("spawning DevQL watcher for {}", repo_root.display()))?;

        if let Some(entry) = runtime_store.claim_pending_watcher_registration(
            child.id(),
            &restart_token,
            &repo_root,
        )? {
            kill_process(child.id());
            match handle_existing_watcher_registration(
                &runtime_store,
                entry,
                &restart_token,
                WATCHER_READY_TIMEOUT,
                WATCHER_READY_POLL_INTERVAL,
            )? {
                ExistingWatcherRegistrationHandle::Handled => return Ok(()),
                ExistingWatcherRegistrationHandle::RetrySpawn => continue,
            }
        }

        let wait_result = wait_for_watcher_registration_ready(
            child.id(),
            &restart_token,
            WATCHER_READY_TIMEOUT,
            WATCHER_READY_POLL_INTERVAL,
            || runtime_store.load_watcher_registration(),
            || Ok(child.try_wait()?.is_none()),
        );
        let wait_error = match wait_result {
            Ok(()) => return Ok(()),
            Err(wait_error) => wait_error,
        };
        match wait_error.downcast_ref::<WatcherRegistrationReadyError>() {
            Some(WatcherRegistrationReadyError::TimedOut { .. }) => {
                match recover_timed_out_pending_registration(
                    &runtime_store,
                    child.id(),
                    &restart_token,
                )? {
                    Some(TimedOutPendingRecovery::ReadyRegistrationPresent) => return Ok(()),
                    Some(TimedOutPendingRecovery::PendingReleased) => {
                        kill_process(child.id());
                        let _ = runtime_store
                            .delete_watcher_registration_if_matches(child.id(), &restart_token);
                    }
                    None => {}
                }
            }
            Some(WatcherRegistrationReadyError::ExitedBeforeReady { .. }) => {
                let _ = runtime_store
                    .delete_pending_watcher_registration_if_matches(child.id(), &restart_token);
            }
            None => {}
        }
        Err::<(), _>(wait_error).with_context(|| {
            format!(
                "waiting for DevQL watcher readiness for {}",
                repo_root.display()
            )
        })?;
    }
}

pub fn restart_watcher(repo_root: &Path, daemon_config_root: &Path) -> Result<()> {
    stop_watcher(repo_root, daemon_config_root)?;
    if crate::config::settings::is_enabled_for_hooks(repo_root) {
        ensure_watcher_running(repo_root, daemon_config_root)?;
    }
    Ok(())
}

pub fn stop_watcher(repo_root: &Path, daemon_config_root: &Path) -> Result<()> {
    let runtime_store = RepoSqliteRuntimeStore::open_for_roots(daemon_config_root, repo_root)?;
    let registration = runtime_store.load_watcher_registration()?;
    runtime_store.clear_watcher_registration()?;

    if let Some(entry) = registration
        && process_is_running(entry.pid)
    {
        kill_process(entry.pid);
        let _ = wait_for_process_exit(entry.pid, WATCHER_STOP_TIMEOUT);
    }

    Ok(())
}

pub async fn run_process_command(args: WatcherProcessArgs) -> Result<()> {
    let result = async {
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
            worker_result = &mut worker => {
                worker_result.context("joining watcher loop task")??;
            }
            _ = &mut shutdown_signal => {
                shutdown.store(true, Ordering::SeqCst);
                worker
                    .await
                    .context("joining watcher loop task after shutdown")??;
            }
        }
        Ok(())
    }
    .await;

    if let Err(err) = &result {
        log::error!("devql watcher failed: {err:#}");
    }

    result
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
    let runtime_store =
        RepoSqliteRuntimeStore::open_for_roots(&cfg.daemon_config_root, &cfg.repo_root)?;
    let registration_guard = WatcherRegistrationGuard::acquire(runtime_store, &cfg.repo_root)?;
    log::info!(
        "devql watcher started: repo_root={} daemon_config_root={}",
        cfg.repo_root.display(),
        cfg.daemon_config_root.display()
    );

    let debounce = Duration::from_millis(opts.debounce_ms.max(50));
    let rescan_interval =
        Duration::from_millis(opts.poll_fallback_ms).max(WATCHER_RESCAN_MIN_INTERVAL);
    let mut last_rescan = Instant::now();
    let mut batch: BTreeSet<PathBuf> = BTreeSet::new();
    let mut window_start: Option<Instant> = None;
    let idle_timeout = watcher_idle_timeout();
    let mut last_external_activity = Instant::now();

    while !shutdown.load(Ordering::SeqCst) {
        match evaluate_watcher_exit_reason(
            cfg,
            &registration_guard.runtime_store,
            registration_guard.pid,
            &registration_guard.restart_token,
            last_external_activity,
            idle_timeout,
            !batch.is_empty() || window_start.is_some(),
        ) {
            Ok(Some(reason)) => {
                log::info!(
                    "devql watcher exiting: repo_root={} reason={}",
                    cfg.repo_root.display(),
                    reason.as_str()
                );
                return Ok(());
            }
            Ok(None) => {}
            Err(err) => {
                log::warn!("devql watcher lifecycle check failed: {err:#}");
            }
        }

        if watcher_repo_root_missing(&cfg.repo_root) {
            return Ok(());
        }

        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Ok(event)) => {
                let mut saw_relevant_path = false;
                for path in event.paths {
                    if should_ignore_path(&cfg.repo_root, &path)
                        || is_gitignored(&cfg.repo_root, &path)
                    {
                        continue;
                    }
                    batch.insert(path);
                    saw_relevant_path = true;
                }
                if saw_relevant_path {
                    last_external_activity = Instant::now();
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

        if last_rescan.elapsed() >= rescan_interval {
            match add_dirty_worktree_paths_to_batch(&cfg.repo_root, &mut batch) {
                Ok(added) => {
                    if added && window_start.is_none() {
                        window_start = Some(Instant::now());
                    }
                }
                Err(err) => {
                    log::warn!("devql watcher worktree rescan failed: {err:#}");
                }
            }
            last_rescan = Instant::now();
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
                // Keep the current batch so transient failures (for example SQLite locks)
                // can retry on the next debounce window instead of dropping changes.
                window_start = Some(Instant::now());
                continue;
            }
            batch.clear();
            window_start = None;
        }
    }

    Ok(())
}

fn add_dirty_worktree_paths_to_batch(
    repo_root: &Path,
    batch: &mut BTreeSet<PathBuf>,
) -> Result<bool> {
    let before = batch.len();
    for path in dirty_worktree_paths(repo_root)? {
        if should_ignore_path(repo_root, &path) || is_gitignored(repo_root, &path) {
            continue;
        }
        batch.insert(path);
    }
    Ok(batch.len() != before)
}

fn dirty_worktree_paths(repo_root: &Path) -> Result<Vec<PathBuf>> {
    let (modified, new_files, deleted) =
        crate::host::checkpoints::strategy::manual_commit::working_tree_changes(repo_root)
            .context("listing dirty worktree paths for DevQL watcher fallback rescan")?;
    let mut paths = modified
        .into_iter()
        .chain(new_files)
        .chain(deleted)
        .filter(|path| !path.trim().is_empty())
        .map(|path| repo_root.join(path))
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    Ok(paths)
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

fn wait_for_process_exit(pid: u32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if !process_is_running(pid) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn watcher_idle_timeout() -> Duration {
    watcher_idle_timeout_from_env(env::var(WATCHER_IDLE_TIMEOUT_ENV).ok().as_deref())
}

fn watcher_idle_timeout_from_env(raw: Option<&str>) -> Duration {
    raw.and_then(|value| value.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_WATCHER_IDLE_TIMEOUT)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WatcherExitReason {
    RepoMissing,
    CaptureDisabled,
    RegistrationLost,
    Idle,
}

impl WatcherExitReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::RepoMissing => "repo_missing",
            Self::CaptureDisabled => "capture_disabled",
            Self::RegistrationLost => "registration_lost",
            Self::Idle => "idle_timeout",
        }
    }
}

fn evaluate_watcher_exit_reason(
    cfg: &crate::host::devql::DevqlConfig,
    runtime_store: &RepoSqliteRuntimeStore,
    pid: u32,
    restart_token: &str,
    last_external_activity: Instant,
    idle_timeout: Duration,
    has_pending_batch: bool,
) -> Result<Option<WatcherExitReason>> {
    if watcher_repo_root_missing(&cfg.repo_root) {
        return Ok(Some(WatcherExitReason::RepoMissing));
    }
    if !crate::config::settings::is_enabled_for_hooks(&cfg.repo_root) {
        return Ok(Some(WatcherExitReason::CaptureDisabled));
    }
    if !watcher_registration_matches(runtime_store, pid, restart_token)? {
        return Ok(Some(WatcherExitReason::RegistrationLost));
    }
    if !has_pending_batch && last_external_activity.elapsed() >= idle_timeout {
        return Ok(Some(WatcherExitReason::Idle));
    }
    Ok(None)
}

fn watcher_registration_matches(
    runtime_store: &RepoSqliteRuntimeStore,
    pid: u32,
    restart_token: &str,
) -> Result<bool> {
    Ok(runtime_store
        .load_watcher_registration()?
        .is_some_and(|entry| entry.pid == pid && entry.restart_token == restart_token))
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
        runtime_store.promote_watcher_registration_to_ready(pid, &restart_token, repo_root)?;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExistingWatcherRegistrationDisposition {
    Ready,
    WaitForReady,
    Replace { kill_running_process: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExistingWatcherRegistrationHandle {
    Handled,
    RetrySpawn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WatcherRegistrationReadyError {
    ExitedBeforeReady { pid: u32 },
    TimedOut { pid: u32 },
}

impl std::fmt::Display for WatcherRegistrationReadyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ExitedBeforeReady { pid } => {
                write!(
                    f,
                    "spawned DevQL watcher process {pid} exited before becoming ready"
                )
            }
            Self::TimedOut { pid } => {
                write!(
                    f,
                    "timed out waiting for DevQL watcher process {pid} to become ready"
                )
            }
        }
    }
}

impl std::error::Error for WatcherRegistrationReadyError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TimedOutPendingRecovery {
    PendingReleased,
    ReadyRegistrationPresent,
}

fn classify_existing_watcher_registration(
    entry: &crate::host::runtime_store::RepoWatcherRegistration,
    expected_restart_token: &str,
    watcher_running: bool,
) -> ExistingWatcherRegistrationDisposition {
    if watcher_running && entry.restart_token == expected_restart_token {
        return match entry.state {
            crate::host::runtime_store::RepoWatcherRegistrationState::Ready => {
                ExistingWatcherRegistrationDisposition::Ready
            }
            crate::host::runtime_store::RepoWatcherRegistrationState::Pending => {
                ExistingWatcherRegistrationDisposition::WaitForReady
            }
        };
    }

    ExistingWatcherRegistrationDisposition::Replace {
        kill_running_process: watcher_running && entry.restart_token != expected_restart_token,
    }
}

fn handle_existing_watcher_registration(
    runtime_store: &RepoSqliteRuntimeStore,
    entry: crate::host::runtime_store::RepoWatcherRegistration,
    expected_restart_token: &str,
    timeout: Duration,
    poll_interval: Duration,
) -> Result<ExistingWatcherRegistrationHandle> {
    match classify_existing_watcher_registration(
        &entry,
        expected_restart_token,
        process_is_running(entry.pid),
    ) {
        ExistingWatcherRegistrationDisposition::Ready => {
            Ok(ExistingWatcherRegistrationHandle::Handled)
        }
        ExistingWatcherRegistrationDisposition::WaitForReady => {
            match wait_for_watcher_registration_ready(
                entry.pid,
                expected_restart_token,
                timeout,
                poll_interval,
                || runtime_store.load_watcher_registration(),
                || Ok(process_is_running(entry.pid)),
            ) {
                Ok(()) => Ok(ExistingWatcherRegistrationHandle::Handled),
                Err(wait_error)
                    if matches!(
                        wait_error.downcast_ref::<WatcherRegistrationReadyError>(),
                        Some(WatcherRegistrationReadyError::ExitedBeforeReady { .. })
                    ) =>
                {
                    runtime_store.delete_pending_watcher_registration_if_matches(
                        entry.pid,
                        &entry.restart_token,
                    )?;
                    Ok(ExistingWatcherRegistrationHandle::RetrySpawn)
                }
                Err(wait_error)
                    if matches!(
                        wait_error.downcast_ref::<WatcherRegistrationReadyError>(),
                        Some(WatcherRegistrationReadyError::TimedOut { .. })
                    ) =>
                {
                    match recover_timed_out_pending_registration(
                        runtime_store,
                        entry.pid,
                        &entry.restart_token,
                    )? {
                        Some(TimedOutPendingRecovery::ReadyRegistrationPresent) => {
                            Ok(ExistingWatcherRegistrationHandle::Handled)
                        }
                        Some(TimedOutPendingRecovery::PendingReleased) => {
                            Ok(ExistingWatcherRegistrationHandle::RetrySpawn)
                        }
                        None => Err(wait_error),
                    }
                }
                Err(wait_error) => Err(wait_error),
            }
        }
        ExistingWatcherRegistrationDisposition::Replace {
            kill_running_process,
        } => {
            if kill_running_process {
                // Restart token mismatch means a different binary is now serving watcher work.
                // Kill the stale watcher so the new process can re-run startup schema init.
                kill_process(entry.pid);
            }
            runtime_store
                .delete_watcher_registration_if_matches(entry.pid, &entry.restart_token)?;
            Ok(ExistingWatcherRegistrationHandle::RetrySpawn)
        }
    }
}

fn recover_timed_out_pending_registration(
    runtime_store: &RepoSqliteRuntimeStore,
    pid: u32,
    expected_restart_token: &str,
) -> Result<Option<TimedOutPendingRecovery>> {
    if runtime_store.delete_pending_watcher_registration_if_matches(pid, expected_restart_token)? {
        return Ok(Some(TimedOutPendingRecovery::PendingReleased));
    }

    match runtime_store.load_watcher_registration()? {
        Some(entry)
            if entry.restart_token == expected_restart_token
                && entry.state
                    == crate::host::runtime_store::RepoWatcherRegistrationState::Ready
                && process_is_running(entry.pid) =>
        {
            Ok(Some(TimedOutPendingRecovery::ReadyRegistrationPresent))
        }
        None => Ok(Some(TimedOutPendingRecovery::PendingReleased)),
        Some(_) => Ok(None),
    }
}

fn wait_for_watcher_registration_ready<FLoad, FWatcherRunning>(
    expected_pid: u32,
    expected_restart_token: &str,
    timeout: Duration,
    poll_interval: Duration,
    mut load_registration: FLoad,
    mut watcher_running: FWatcherRunning,
) -> Result<()>
where
    FLoad: FnMut() -> Result<Option<crate::host::runtime_store::RepoWatcherRegistration>>,
    FWatcherRunning: FnMut() -> Result<bool>,
{
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(entry) = load_registration()?
            && entry.pid == expected_pid
            && entry.restart_token == expected_restart_token
            && entry.state == crate::host::runtime_store::RepoWatcherRegistrationState::Ready
        {
            return Ok(());
        }

        if !watcher_running()? {
            return Err(anyhow::Error::new(
                WatcherRegistrationReadyError::ExitedBeforeReady { pid: expected_pid },
            ));
        }

        if Instant::now() >= deadline {
            return Err(anyhow::Error::new(
                WatcherRegistrationReadyError::TimedOut { pid: expected_pid },
            ));
        }

        std::thread::sleep(poll_interval);
    }
}

async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        let mut sigterm =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(signal) => Some(signal),
                Err(err) => {
                    log::warn!("failed to install SIGTERM handler for devql watcher: {err:#}");
                    None
                }
            };
        let ctrl_c = async {
            if let Err(err) = tokio::signal::ctrl_c().await {
                log::warn!("failed to install Ctrl-C handler for devql watcher: {err:#}");
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
        if let Err(err) = tokio::signal::ctrl_c().await {
            log::warn!("failed to install Ctrl-C handler for devql watcher: {err:#}");
            std::future::pending::<()>().await;
        }
    }
}

#[cfg(test)]
#[path = "watch_tests.rs"]
mod tests;

use std::fs::File;
use std::future::Future;
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use tokio::{sync::mpsc, task};

use crate::api::DashboardServerConfig;
use crate::cli::telemetry_consent;
use crate::config::{bootstrap_default_daemon_environment, default_daemon_config_exists};
use crate::daemon::{self, DaemonMode};
pub const MISSING_SUBCOMMAND_MESSAGE: &str = "missing subcommand. Use one of: `bitloops daemon start`, `bitloops daemon stop`, `bitloops daemon status`, `bitloops daemon restart`, `bitloops daemon logs`";
const DEFAULT_LOG_TAIL_LINES: usize = 200;
const LOG_FOLLOW_POLL_INTERVAL: Duration = Duration::from_millis(250);
const TAIL_SCAN_BLOCK_SIZE: usize = 8 * 1024;

#[derive(Args, Debug, Clone, Default)]
pub struct DaemonArgs {
    #[command(subcommand)]
    pub command: Option<DaemonCommand>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum DaemonCommand {
    /// Start the global Bitloops daemon.
    Start(DaemonStartArgs),
    /// Stop the global Bitloops daemon.
    Stop(DaemonStopArgs),
    /// Show global Bitloops daemon status.
    Status(DaemonStatusArgs),
    /// Restart the global Bitloops daemon.
    Restart(DaemonRestartArgs),
    /// Show daemon log output.
    Logs(DaemonLogsArgs),
}

#[derive(Args, Debug, Clone)]
pub struct DaemonStartArgs {
    /// Path to the Bitloops daemon config file.
    #[arg(long, value_name = "PATH")]
    pub config: Option<std::path::PathBuf>,

    /// Create the default global daemon config and local default stores if missing.
    #[arg(long, default_value_t = false, conflicts_with = "config")]
    pub create_default_config: bool,

    /// Start detached instead of holding the current terminal open.
    #[arg(
        short = 'd',
        long,
        default_value_t = false,
        conflicts_with = "until_stopped"
    )]
    pub detached: bool,

    /// Install or refresh an always-on user-scoped service, then start it.
    #[arg(long, default_value_t = false, conflicts_with = "detached")]
    pub until_stopped: bool,

    /// Hostname to bind the daemon server to.
    #[arg(long)]
    pub host: Option<String>,

    /// Port to bind the daemon server to.
    #[arg(long, default_value_t = crate::api::DEFAULT_DASHBOARD_PORT)]
    pub port: u16,

    /// Force fast local HTTP mode. Requires `--host 127.0.0.1`.
    #[arg(long, default_value_t = false)]
    pub http: bool,

    /// Force a full local dashboard network recheck and refresh discovery hints.
    #[arg(long = "recheck-local-dashboard-net", default_value_t = false)]
    pub recheck_local_dashboard_net: bool,

    /// Path to the dashboard bundle directory (contains index.html).
    #[arg(long = "bundle-dir", alias = "bundle", value_name = "PATH")]
    pub bundle_dir: Option<std::path::PathBuf>,

    /// Enable anonymous telemetry for this CLI version.
    #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "true")]
    pub telemetry: Option<bool>,

    /// Disable anonymous telemetry for this CLI version.
    #[arg(
        long = "no-telemetry",
        conflicts_with = "telemetry",
        default_value_t = false
    )]
    pub no_telemetry: bool,
}

#[derive(Args, Debug, Clone, Default)]
pub struct DaemonStopArgs {
    /// Path to the Bitloops daemon config file.
    #[arg(long, value_name = "PATH")]
    pub config: Option<std::path::PathBuf>,
}

#[derive(Args, Debug, Clone, Default)]
pub struct DaemonStatusArgs {
    /// Path to the Bitloops daemon config file.
    #[arg(long, value_name = "PATH")]
    pub config: Option<std::path::PathBuf>,
}

#[derive(Args, Debug, Clone, Default)]
pub struct DaemonRestartArgs {
    /// Path to the Bitloops daemon config file.
    #[arg(long, value_name = "PATH")]
    pub config: Option<std::path::PathBuf>,
}

#[derive(Args, Debug, Clone, Default)]
pub struct DaemonLogsArgs {
    /// Print the last N lines from the daemon log.
    #[arg(long, value_name = "N", value_parser = parse_log_lines)]
    pub tail: Option<usize>,

    /// Keep streaming appended daemon log lines.
    #[arg(long, default_value_t = false, conflicts_with = "path")]
    pub follow: bool,

    /// Print the daemon log file path and exit.
    #[arg(long, default_value_t = false, conflicts_with_all = ["follow", "tail"])]
    pub path: bool,
}

pub async fn run(args: DaemonArgs) -> Result<()> {
    let Some(command) = args.command else {
        bail!(MISSING_SUBCOMMAND_MESSAGE);
    };

    match command {
        DaemonCommand::Start(args) => run_start(args).await,
        DaemonCommand::Stop(args) => run_stop(args).await,
        DaemonCommand::Status(args) => run_status(args).await,
        DaemonCommand::Restart(args) => run_restart(args).await,
        DaemonCommand::Logs(args) => run_logs(args).await,
    }
}

pub async fn run_start(args: DaemonStartArgs) -> Result<()> {
    let mut out = io::stdout().lock();
    let stdin = io::stdin();
    let mut input = stdin.lock();
    run_start_with_io(args, &mut out, &mut input).await
}

async fn run_start_with_io(
    args: DaemonStartArgs,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<()> {
    log::info!(
        "cli daemon start: detached={} until_stopped={} config={:?} host={:?} port={} http={} recheck_local_dashboard_net={} bundle_dir={:?}",
        args.detached,
        args.until_stopped,
        args.config,
        args.host,
        args.port,
        args.http,
        args.recheck_local_dashboard_net,
        args.bundle_dir
    );
    print_legacy_repo_data_warnings();
    let preflight = resolve_start_preflight(&args, out, input)?;

    if preflight.create_default_config {
        let _ = bootstrap_default_daemon_environment()?;
    }
    let daemon_config = daemon::resolve_daemon_config(args.config.as_deref())?;
    let config = build_server_config(&args);

    if args.until_stopped {
        let state =
            daemon::start_service(&daemon_config, config, preflight.startup_telemetry).await?;
        println!(
            "Bitloops daemon started as an always-on service at {}",
            state.url
        );
        return Ok(());
    }

    let report = daemon::status().await?;
    if report.service.is_some() {
        let state =
            daemon::start_service(&daemon_config, config, preflight.startup_telemetry).await?;
        println!(
            "Bitloops daemon started under the always-on service at {}",
            state.url
        );
        return Ok(());
    }

    if args.detached {
        let state =
            daemon::start_detached(&daemon_config, config, preflight.startup_telemetry).await?;
        println!("Bitloops daemon started in detached mode at {}", state.url);
        return Ok(());
    }

    daemon::start_foreground(
        &daemon_config,
        config,
        false,
        "Bitloops daemon",
        preflight.startup_telemetry,
    )
    .await
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StartPreflightDecision {
    create_default_config: bool,
    startup_telemetry: Option<bool>,
}

fn resolve_start_preflight(
    args: &DaemonStartArgs,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<StartPreflightDecision> {
    let default_config_missing = args.config.is_none() && !default_daemon_config_exists()?;
    let mut create_default_config = args.create_default_config;

    if default_config_missing && !create_default_config {
        if !telemetry_consent::can_prompt_interactively() {
            bail!(missing_default_daemon_bootstrap_message());
        }
        create_default_config = telemetry_consent::prompt_default_config_setup(out, input)?;
        if !create_default_config {
            bail!(missing_default_daemon_bootstrap_message());
        }
    }

    let startup_telemetry = collect_startup_telemetry_choice(
        default_config_missing && create_default_config,
        args,
        out,
        input,
    )?;

    Ok(StartPreflightDecision {
        create_default_config,
        startup_telemetry,
    })
}

fn collect_startup_telemetry_choice(
    created_default_config: bool,
    args: &DaemonStartArgs,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<Option<bool>> {
    let telemetry_choice =
        telemetry_consent::telemetry_flag_choice(args.telemetry, args.no_telemetry);
    if !created_default_config {
        return Ok(telemetry_choice);
    }

    if let Some(choice) = telemetry_choice {
        return Ok(Some(choice));
    }

    if !telemetry_consent::can_prompt_interactively() {
        bail!(telemetry_consent::NON_INTERACTIVE_TELEMETRY_ERROR);
    }

    Ok(Some(telemetry_consent::prompt_telemetry_consent(
        out, input,
    )?))
}

pub async fn run_stop(args: DaemonStopArgs) -> Result<()> {
    log::info!("cli daemon stop: config={:?}", args.config);
    if let Some(config_path) = args.config.as_deref() {
        let _ = daemon::resolve_daemon_config(Some(config_path))?;
    }
    daemon::stop().await?;
    println!("Bitloops daemon stopped.");
    Ok(())
}

pub async fn run_status(args: DaemonStatusArgs) -> Result<()> {
    if let Some(config_path) = args.config.as_deref() {
        let _ = daemon::resolve_daemon_config(Some(config_path))?;
    }
    let report = daemon::status().await?;
    for line in status_lines(&report) {
        println!("{line}");
    }
    for line in legacy_repo_data_warnings() {
        println!("{line}");
    }
    Ok(())
}

pub async fn run_logs(args: DaemonLogsArgs) -> Result<()> {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    run_logs_with_io(args, &mut out).await
}

async fn run_logs_with_io(args: DaemonLogsArgs, out: &mut dyn Write) -> Result<()> {
    run_logs_with_io_and_shutdown(args, out, async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await
}

async fn run_logs_with_io_and_shutdown<S>(
    args: DaemonLogsArgs,
    out: &mut dyn Write,
    shutdown: S,
) -> Result<()>
where
    S: Future<Output = ()>,
{
    run_logs_with_io_and_shutdown_and_poll_interval(args, out, shutdown, LOG_FOLLOW_POLL_INTERVAL)
        .await
}

async fn run_logs_with_io_and_shutdown_and_poll_interval<S>(
    args: DaemonLogsArgs,
    out: &mut dyn Write,
    shutdown: S,
    poll_interval: Duration,
) -> Result<()>
where
    S: Future<Output = ()>,
{
    let log_path = daemon::daemon_log_file_path();
    if args.path {
        writeln!(out, "{}", log_path.display()).context("writing daemon log path")?;
        return Ok(());
    }

    ensure_log_file_exists(&log_path)?;
    let tail_lines = args.tail.unwrap_or(DEFAULT_LOG_TAIL_LINES);
    for line in read_tail_lines(log_path.clone(), tail_lines).await? {
        writeln!(out, "{line}").context("writing daemon log output")?;
    }
    out.flush().context("flushing daemon log output")?;
    if args.follow {
        follow_log_file_until_shutdown(&log_path, out, shutdown, poll_interval).await?;
    }
    Ok(())
}

pub async fn run_restart(args: DaemonRestartArgs) -> Result<()> {
    log::info!("cli daemon restart: config={:?}", args.config);
    let requested_config: Option<daemon::ResolvedDaemonConfig> = args
        .config
        .as_deref()
        .map(|config_path| daemon::resolve_daemon_config(Some(config_path)))
        .transpose()?;
    let report = daemon::status().await?;

    if report.service.is_none()
        && let Some(runtime) = report.runtime
        && matches!(runtime.mode, DaemonMode::Foreground)
    {
        let config = DashboardServerConfig {
            host: Some(runtime.host),
            port: runtime.port,
            no_open: true,
            force_http: runtime.url.starts_with("http://"),
            recheck_local_dashboard_net: false,
            bundle_dir: Some(runtime.bundle_dir),
        };
        daemon::stop().await?;
        let daemon_config = match requested_config {
            Some(ref daemon_config) => daemon_config.clone(),
            None => daemon::resolve_daemon_config(Some(runtime.config_path.as_path()))?,
        };
        return daemon::start_foreground(&daemon_config, config, false, "Bitloops daemon", None)
            .await;
    }

    let state = daemon::restart(requested_config.as_ref()).await?;
    println!("Bitloops daemon restarted at {}", state.url);
    Ok(())
}

pub async fn launch_dashboard() -> Result<()> {
    print_legacy_repo_data_warnings();
    if let Some(url) = daemon::daemon_url()? {
        crate::api::open_in_default_browser(&url)?;
        println!("Opened Bitloops dashboard at {url}");
        return Ok(());
    }

    if !default_daemon_config_exists()? {
        bail!(missing_default_daemon_bootstrap_message());
    }

    let daemon_config = daemon::resolve_daemon_config(None)?;
    let report = daemon::status().await?;
    let config = DashboardServerConfig {
        host: None,
        port: crate::api::DEFAULT_DASHBOARD_PORT,
        no_open: false,
        force_http: false,
        recheck_local_dashboard_net: false,
        bundle_dir: None,
    };

    if report.service.is_some() {
        let state = daemon::start_service(&daemon_config, config, None).await?;
        crate::api::open_in_default_browser(&state.url)?;
        println!("Opened Bitloops dashboard at {}", state.url);
        return Ok(());
    }

    let Some(choice) = daemon::choose_dashboard_launch_mode()? else {
        bail!(
            "Bitloops daemon is not running. Start it with `bitloops daemon start`, `bitloops daemon start -d`, or `bitloops daemon start --until-stopped`."
        );
    };

    match choice {
        DaemonMode::Foreground => {
            daemon::start_foreground(&daemon_config, config, true, "Dashboard", None).await
        }
        DaemonMode::Detached => {
            let state = daemon::start_detached(&daemon_config, config, None).await?;
            crate::api::open_in_default_browser(&state.url)?;
            println!("Opened Bitloops dashboard at {}", state.url);
            Ok(())
        }
        DaemonMode::Service => {
            let state = daemon::start_service(&daemon_config, config, None).await?;
            crate::api::open_in_default_browser(&state.url)?;
            println!("Opened Bitloops dashboard at {}", state.url);
            Ok(())
        }
    }
}

fn build_server_config(args: &DaemonStartArgs) -> DashboardServerConfig {
    DashboardServerConfig {
        host: args.host.clone(),
        port: args.port,
        no_open: true,
        force_http: args.http,
        recheck_local_dashboard_net: args.recheck_local_dashboard_net,
        bundle_dir: args.bundle_dir.clone(),
    }
}

fn missing_default_daemon_bootstrap_message() -> &'static str {
    "Bitloops daemon has not been bootstrapped yet. Run `bitloops start --create-default-config` or `bitloops init --install-default-daemon`."
}

fn status_lines(report: &daemon::DaemonStatusReport) -> Vec<String> {
    let mut lines = Vec::new();
    let log_path = daemon::daemon_log_file_path();

    if let Some(runtime) = report.runtime.as_ref() {
        lines.push("Bitloops daemon: running".to_string());
        lines.push(format!("Mode: {}", runtime.mode));
        lines.push(format!("URL: {}", runtime.url));
        lines.push(format!("Config: {}", runtime.config_path.display()));
        lines.push(format!("Log file: {}", log_path.display()));
        lines.push(format!("PID: {}", runtime.pid));
        append_supervisor_lines(&mut lines, report);
        if let Some(health) = report.health.as_ref() {
            append_health_lines(&mut lines, health);
        }
        return lines;
    }

    if let Some(service) = report.service.as_ref() {
        lines.push("Bitloops daemon: stopped".to_string());
        lines.push("Mode: always-on service".to_string());
        lines.push(format!("Config: {}", service.config_path.display()));
        lines.push(format!("Log file: {}", log_path.display()));
        lines.push(format!(
            "Supervisor service: {} ({}, installed)",
            service.service_name, service.manager
        ));
        lines.push(format!(
            "Supervisor state: {}",
            if report.service_running {
                "running"
            } else {
                "stopped"
            }
        ));
        if let Some(url) = service.last_url.as_ref() {
            lines.push(format!("Last URL: {url}"));
        }
        return lines;
    }

    lines.push("Bitloops daemon: stopped".to_string());
    lines.push("Mode: not running".to_string());
    lines.push(format!("Log file: {}", log_path.display()));
    lines
}

fn ensure_log_file_exists(path: &Path) -> Result<()> {
    if path.is_file() {
        return Ok(());
    }
    bail!(
        "Bitloops daemon log file does not exist yet at {}. Start the daemon and try again.",
        path.display()
    );
}

fn parse_log_lines(value: &str) -> std::result::Result<usize, String> {
    let parsed = value
        .parse::<usize>()
        .map_err(|_| format!("invalid value `{value}` for --tail"))?;
    if parsed == 0 {
        return Err("--tail must be greater than 0".to_string());
    }
    Ok(parsed)
}

async fn read_tail_lines(path: std::path::PathBuf, lines: usize) -> Result<Vec<String>> {
    task::spawn_blocking(move || tail_log_file(&path, lines))
        .await
        .context("joining daemon log tail task")?
}

fn tail_log_file(path: &Path, lines: usize) -> Result<Vec<String>> {
    if lines == 0 {
        return Ok(Vec::new());
    }

    let mut file =
        File::open(path).with_context(|| format!("opening daemon log {}", path.display()))?;
    let start = find_tail_start_offset(&mut file, lines, path)?;
    file.seek(SeekFrom::Start(start))
        .with_context(|| format!("seeking daemon log {}", path.display()))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .with_context(|| format!("reading daemon log {}", path.display()))?;
    let content = String::from_utf8(bytes)
        .with_context(|| format!("decoding daemon log {}", path.display()))?;

    Ok(content.lines().map(str::to_owned).collect())
}

fn find_tail_start_offset(file: &mut File, lines: usize, path: &Path) -> Result<u64> {
    let file_len = file
        .metadata()
        .with_context(|| format!("reading daemon log metadata {}", path.display()))?
        .len();
    if file_len == 0 {
        return Ok(0);
    }

    let mut remaining = file_len;
    let mut needed = lines;
    let mut skip_trailing_newline = file_ends_with_newline(file, file_len, path)?;
    let mut buffer = vec![0_u8; TAIL_SCAN_BLOCK_SIZE];

    while remaining > 0 {
        let read_size = remaining.min(TAIL_SCAN_BLOCK_SIZE as u64) as usize;
        remaining -= read_size as u64;
        file.seek(SeekFrom::Start(remaining))
            .with_context(|| format!("seeking daemon log {}", path.display()))?;
        file.read_exact(&mut buffer[..read_size])
            .with_context(|| format!("reading daemon log {}", path.display()))?;

        for idx in (0..read_size).rev() {
            if buffer[idx] != b'\n' {
                continue;
            }

            let newline_pos = remaining + idx as u64;
            if skip_trailing_newline && newline_pos == file_len - 1 {
                skip_trailing_newline = false;
                continue;
            }

            needed -= 1;
            if needed == 0 {
                return Ok(newline_pos + 1);
            }
        }
    }

    Ok(0)
}

fn file_ends_with_newline(file: &mut File, file_len: u64, path: &Path) -> Result<bool> {
    if file_len == 0 {
        return Ok(false);
    }

    let mut byte = [0_u8; 1];
    file.seek(SeekFrom::Start(file_len - 1))
        .with_context(|| format!("seeking daemon log {}", path.display()))?;
    file.read_exact(&mut byte)
        .with_context(|| format!("reading daemon log {}", path.display()))?;
    Ok(byte[0] == b'\n')
}

async fn follow_log_file_until_shutdown<S>(
    path: &Path,
    out: &mut dyn Write,
    shutdown: S,
    poll_interval: Duration,
) -> Result<()>
where
    S: Future<Output = ()>,
{
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let stop = Arc::new(AtomicBool::new(false));
    let path = path.to_path_buf();
    let worker_stop = stop.clone();
    let worker = task::spawn_blocking(move || {
        follow_log_file(
            &path,
            |chunk| {
                tx.send(chunk.to_owned())
                    .map_err(|_| anyhow::anyhow!("daemon log follow channel closed"))?;
                Ok(())
            },
            &|| worker_stop.load(Ordering::SeqCst),
            poll_interval,
        )
    });
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            maybe_chunk = rx.recv() => {
                let Some(chunk) = maybe_chunk else {
                    break;
                };
                write!(out, "{chunk}").context("writing daemon follow output")?;
                out.flush().context("flushing daemon follow output")?;
            }
            _ = &mut shutdown => {
                stop.store(true, Ordering::SeqCst);
                break;
            }
        }
    }

    worker.await.context("joining daemon log follow task")??;
    while let Ok(chunk) = rx.try_recv() {
        write!(out, "{chunk}").context("writing daemon follow output")?;
    }
    out.flush().context("flushing daemon follow output")?;
    Ok(())
}

fn follow_log_file<F>(
    path: &Path,
    mut on_chunk: F,
    should_stop: &dyn Fn() -> bool,
    poll_interval: Duration,
) -> Result<()>
where
    F: FnMut(&str) -> Result<()>,
{
    let mut position = std::fs::metadata(path)
        .with_context(|| format!("reading daemon log metadata {}", path.display()))?
        .len();

    loop {
        if should_stop() {
            return Ok(());
        }

        let len = std::fs::metadata(path)
            .with_context(|| format!("reading daemon log metadata {}", path.display()))?
            .len();
        if len < position {
            position = 0;
        }

        if len > position {
            let mut file = File::open(path)
                .with_context(|| format!("opening daemon log {}", path.display()))?;
            file.seek(SeekFrom::Start(position))
                .with_context(|| format!("seeking daemon log {}", path.display()))?;
            let mut reader = BufReader::new(file);
            let mut line = String::new();
            loop {
                let bytes = reader
                    .read_line(&mut line)
                    .with_context(|| format!("reading daemon log {}", path.display()))?;
                if bytes == 0 {
                    break;
                }
                on_chunk(&line)?;
                line.clear();
            }
            position = reader
                .stream_position()
                .with_context(|| format!("tracking daemon log cursor {}", path.display()))?;
        }

        thread::sleep(poll_interval);
    }
}

fn append_supervisor_lines(lines: &mut Vec<String>, report: &daemon::DaemonStatusReport) {
    if let Some(service) = report.service.as_ref() {
        lines.push(format!(
            "Supervisor service: {} ({}, installed)",
            service.service_name, service.manager
        ));
        lines.push(format!(
            "Supervisor state: {}",
            if report.service_running {
                "running"
            } else {
                "stopped"
            }
        ));
    }
}

fn append_health_lines(lines: &mut Vec<String>, health: &daemon::DaemonHealthSummary) {
    if let (Some(backend), Some(connected)) =
        (&health.relational_backend, health.relational_connected)
    {
        lines.push(format!(
            "Relational: {} ({})",
            backend,
            if connected {
                "connected"
            } else {
                "disconnected"
            }
        ));
    }
    if let (Some(backend), Some(connected)) = (&health.events_backend, health.events_connected) {
        lines.push(format!(
            "Events: {} ({})",
            backend,
            if connected {
                "connected"
            } else {
                "disconnected"
            }
        ));
    }
    if let (Some(backend), Some(connected)) = (&health.blob_backend, health.blob_connected) {
        lines.push(format!(
            "Blob: {} ({})",
            backend,
            if connected {
                "available"
            } else {
                "unavailable"
            }
        ));
    }
}

fn print_legacy_repo_data_warnings() {
    for line in legacy_repo_data_warnings() {
        eprintln!("{line}");
    }
}

fn legacy_repo_data_warnings() -> Vec<String> {
    let Some(repo_root) = crate::utils::paths::repo_root().ok() else {
        return Vec::new();
    };

    let legacy_paths = [
        repo_root.join(".bitloops").join("stores"),
        repo_root.join(".bitloops").join("embeddings"),
        repo_root.join(".bitloops").join("tmp"),
        repo_root.join(".bitloops").join("metadata"),
    ];
    let found: Vec<_> = legacy_paths
        .into_iter()
        .filter(|path| path.exists())
        .collect();
    if found.is_empty() {
        return Vec::new();
    }

    let mut lines = Vec::with_capacity(found.len() + 1);
    lines.push(
        "Warning: legacy repo-local Bitloops data was found and is ignored unless you configure those paths explicitly in the daemon config.".to_string(),
    );
    lines.extend(
        found
            .into_iter()
            .map(|path| format!("Legacy path: {}", path.display())),
    );
    lines
}

#[cfg(test)]
mod tests;

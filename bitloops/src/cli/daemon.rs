use std::collections::VecDeque;
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
use serde_json::Value;
use tokio::{sync::mpsc, task};

use crate::api::DashboardServerConfig;
use crate::cli::telemetry_consent;
use crate::config::{bootstrap_default_daemon_environment, default_daemon_config_exists};
use crate::daemon::{self, DaemonMode};
#[path = "daemon/args.rs"]
mod args;
#[path = "daemon/display.rs"]
mod display;
#[cfg(test)]
#[path = "daemon/tests.rs"]
mod tests;

const DEFAULT_LOG_TAIL_LINES: usize = 200;
const LOG_FOLLOW_POLL_INTERVAL: Duration = Duration::from_millis(250);
const TAIL_SCAN_BLOCK_SIZE: usize = 8 * 1024;

pub use args::{
    DaemonArgs, DaemonCommand, DaemonLogLevel, DaemonLogsArgs, DaemonRestartArgs, DaemonStartArgs,
    DaemonStatusArgs, DaemonStopArgs, EnrichmentArgs, EnrichmentCommand, EnrichmentPauseArgs,
    EnrichmentResumeArgs, EnrichmentRetryFailedArgs, EnrichmentStatusArgs,
    MISSING_SUBCOMMAND_MESSAGE,
};

use display::{enrichment_status_lines, status_lines};

pub async fn run(args: DaemonArgs) -> Result<()> {
    let Some(command) = args.command else {
        bail!(MISSING_SUBCOMMAND_MESSAGE);
    };

    match command {
        DaemonCommand::Start(args) => run_start(args).await,
        DaemonCommand::Stop(args) => run_stop(args).await,
        DaemonCommand::Status(args) => run_status(args).await,
        DaemonCommand::Restart(args) => run_restart(args).await,
        DaemonCommand::Enable(args) => crate::cli::enable::run(args).await,
        DaemonCommand::Enrichments(args) => run_enrichments(args).await,
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
    let preflight = resolve_start_preflight(&args, out, input)?;

    if preflight.create_default_config {
        let _ = bootstrap_default_daemon_environment()?;
    } else if args.bootstrap_local_stores {
        let _ = crate::config::ensure_daemon_store_artifacts(args.config.as_deref())?;
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

    if daemon::service_metadata()?.is_some() {
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
    resolve_start_preflight_with_state(
        args,
        out,
        input,
        default_config_missing,
        telemetry_consent::can_prompt_interactively(),
    )
}

#[cfg(test)]
fn resolve_start_preflight_for_tests(
    args: &DaemonStartArgs,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
    default_config_missing: bool,
    can_prompt_interactively: bool,
) -> Result<StartPreflightDecision> {
    resolve_start_preflight_with_state(
        args,
        out,
        input,
        default_config_missing,
        can_prompt_interactively,
    )
}

fn resolve_start_preflight_with_state(
    args: &DaemonStartArgs,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
    default_config_missing: bool,
    can_prompt_interactively: bool,
) -> Result<StartPreflightDecision> {
    let mut create_default_config = args.create_default_config;

    if default_config_missing && !create_default_config {
        if !can_prompt_interactively {
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
        can_prompt_interactively,
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
    can_prompt_interactively: bool,
) -> Result<Option<bool>> {
    let telemetry_choice =
        telemetry_consent::telemetry_flag_choice(args.telemetry, args.no_telemetry);
    if !created_default_config {
        return Ok(telemetry_choice);
    }

    if let Some(choice) = telemetry_choice {
        return Ok(Some(choice));
    }

    if !can_prompt_interactively {
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

    // Send $session_end for all active sessions before stopping
    send_session_end_for_all_sessions();

    daemon::stop().await?;
    println!("Bitloops daemon stopped.");
    Ok(())
}

fn send_session_end_for_all_sessions() {
    let Ok(state_dir) = crate::utils::platform_dirs::bitloops_state_dir() else {
        return;
    };
    send_session_end_for_all_sessions_in(state_dir.as_path());
}

fn send_session_end_for_all_sessions_in(state_dir: &Path) {
    let session_path = state_dir.join("telemetry_sessions.json");

    // Load and end all sessions
    let Ok(content) = std::fs::read_to_string(&session_path) else {
        return;
    };
    let Ok(store) = serde_json::from_str::<crate::telemetry::sessions::SessionStore>(&content)
    else {
        return;
    };

    // End all sessions and send $session_end events
    for (repo_root_str, session) in store.sessions() {
        if crate::telemetry::analytics::load_dispatch_context_for_repo(Path::new(repo_root_str))
            .is_none()
        {
            continue;
        }

        let ended = crate::telemetry::sessions::EndedSession {
            session_id: session.session_id.clone(),
            repo_root: repo_root_str.clone(),
            started_at: session.started_at,
            ended_at: crate::telemetry::sessions::now_secs(),
            duration_secs: session.session_duration_secs(),
        };
        crate::telemetry::analytics::track_session_end_detached(&ended, "daemon-stop");
    }

    // Clear the session file
    let _ = crate::telemetry::sessions::SessionStore::default().save(state_dir);
}

pub async fn run_status(args: DaemonStatusArgs) -> Result<()> {
    if let Some(config_path) = args.config.as_deref() {
        let _ = daemon::resolve_daemon_config(Some(config_path))?;
    }
    let report = daemon::status().await?;
    let mut out = io::stdout().lock();
    write_status_output(&report, args.json, &mut out)
}

fn write_status_output(
    report: &daemon::DaemonStatusReport,
    json: bool,
    out: &mut dyn Write,
) -> Result<()> {
    if json {
        serde_json::to_writer_pretty(&mut *out, report)
            .context("serializing daemon status as JSON")?;
        writeln!(out)?;
        return Ok(());
    }

    for line in status_lines(report) {
        writeln!(out, "{line}")?;
    }
    Ok(())
}

pub async fn run_logs(args: DaemonLogsArgs) -> Result<()> {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    run_logs_with_io(args, &mut out).await
}

async fn run_logs_with_io(args: DaemonLogsArgs, out: &mut dyn Write) -> Result<()> {
    run_logs_with_io_and_shutdown_at_path(
        args,
        out,
        async {
            let _ = tokio::signal::ctrl_c().await;
        },
        LOG_FOLLOW_POLL_INTERVAL,
        daemon::daemon_log_file_path(),
    )
    .await
}

#[cfg(test)]
async fn run_logs_with_io_at_path(
    args: DaemonLogsArgs,
    out: &mut dyn Write,
    log_path: std::path::PathBuf,
) -> Result<()> {
    run_logs_with_io_and_shutdown_at_path(args, out, async {}, LOG_FOLLOW_POLL_INTERVAL, log_path)
        .await
}

async fn run_logs_with_io_and_shutdown_at_path<S>(
    args: DaemonLogsArgs,
    out: &mut dyn Write,
    shutdown: S,
    poll_interval: Duration,
    log_path: std::path::PathBuf,
) -> Result<()>
where
    S: Future<Output = ()>,
{
    if args.path {
        writeln!(out, "{}", log_path.display()).context("writing daemon log path")?;
        return Ok(());
    }

    ensure_log_file_exists(&log_path)?;
    let tail_lines = args.tail.unwrap_or(DEFAULT_LOG_TAIL_LINES);
    let initial_lines =
        read_log_view_lines(log_path.clone(), tail_lines, args.levels.clone()).await?;
    write_log_lines(out, initial_lines)?;
    out.flush().context("flushing daemon log output")?;
    if args.follow {
        follow_log_file_until_shutdown(&log_path, out, shutdown, poll_interval, args.levels)
            .await?;
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

pub async fn run_enrichments(args: EnrichmentArgs) -> Result<()> {
    let Some(command) = args.command else {
        bail!(
            "missing subcommand. Use one of: `bitloops daemon enrichments status`, `bitloops daemon enrichments pause`, `bitloops daemon enrichments resume`, `bitloops daemon enrichments retry-failed`"
        );
    };

    match command {
        EnrichmentCommand::Status(_) => {
            let status = daemon::enrichment_status()?;
            for line in enrichment_status_lines(&status) {
                println!("{line}");
            }
            Ok(())
        }
        EnrichmentCommand::Pause(args) => {
            let result = daemon::pause_enrichments(args.reason)?;
            println!("{}", result.message);
            Ok(())
        }
        EnrichmentCommand::Resume(_) => {
            let result = daemon::resume_enrichments()?;
            println!("{}", result.message);
            Ok(())
        }
        EnrichmentCommand::RetryFailed(_) => {
            let result = daemon::retry_failed_enrichments()?;
            println!("{}", result.message);
            Ok(())
        }
    }
}

pub async fn launch_dashboard() -> Result<()> {
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

fn ensure_log_file_exists(path: &Path) -> Result<()> {
    if path.is_file() {
        return Ok(());
    }
    bail!(
        "Bitloops daemon log file does not exist yet at {}. Start the daemon and try again.",
        path.display()
    );
}

async fn read_tail_lines(path: std::path::PathBuf, lines: usize) -> Result<Vec<String>> {
    task::spawn_blocking(move || tail_log_file(&path, lines))
        .await
        .context("joining daemon log tail task")?
}

async fn read_log_view_lines(
    path: std::path::PathBuf,
    lines: usize,
    levels: Vec<DaemonLogLevel>,
) -> Result<Vec<String>> {
    if levels.is_empty() {
        return read_tail_lines(path, lines).await;
    }

    task::spawn_blocking(move || tail_log_file_after_filter(&path, lines, &levels))
        .await
        .context("joining filtered daemon log tail task")?
}

fn write_log_lines(out: &mut dyn Write, lines: Vec<String>) -> Result<()> {
    for line in lines {
        writeln!(out, "{line}").context("writing daemon log output")?;
    }
    Ok(())
}

fn should_emit_log_line(line: &str, levels: &[DaemonLogLevel]) -> bool {
    if levels.is_empty() {
        return true;
    }

    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return false;
    };
    let Some(level) = value.get("level").and_then(Value::as_str) else {
        return false;
    };

    let Some(level) = parse_log_level_value(level) else {
        return false;
    };
    levels.contains(&level)
}

fn parse_log_level_value(value: &str) -> Option<DaemonLogLevel> {
    match value.trim().to_ascii_uppercase().as_str() {
        "DEBUG" => Some(DaemonLogLevel::Debug),
        "INFO" => Some(DaemonLogLevel::Info),
        "WARN" | "WARNING" => Some(DaemonLogLevel::Warn),
        "ERROR" => Some(DaemonLogLevel::Error),
        _ => None,
    }
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

fn tail_log_file_after_filter(
    path: &Path,
    lines: usize,
    levels: &[DaemonLogLevel],
) -> Result<Vec<String>> {
    if lines == 0 {
        return Ok(Vec::new());
    }

    let file =
        File::open(path).with_context(|| format!("opening daemon log {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut tail = VecDeque::new();

    for line in reader.lines() {
        let line = line.with_context(|| format!("reading daemon log {}", path.display()))?;
        if should_emit_log_line(&line, levels) {
            push_tail_line(&mut tail, line, lines);
        }
    }

    Ok(tail.into_iter().collect())
}

fn push_tail_line(tail: &mut VecDeque<String>, line: String, lines: usize) {
    if tail.len() == lines {
        tail.pop_front();
    }
    tail.push_back(line);
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
    levels: Vec<DaemonLogLevel>,
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
                if !should_emit_log_line(chunk, &levels) {
                    return Ok(());
                }
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

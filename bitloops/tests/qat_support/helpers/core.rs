pub fn sanitize_name(input: &str) -> String {
    let mut slug = String::with_capacity(input.len());
    let mut last_was_dash = false;

    for ch in input.chars() {
        let normalized = match ch {
            'a'..='z' | '0'..='9' => Some(ch),
            'A'..='Z' => Some(ch.to_ascii_lowercase()),
            _ if ch.is_ascii_whitespace() || matches!(ch, '-' | '_' | '/' | ':') => Some('-'),
            _ => None,
        };

        if let Some(value) = normalized {
            if value == '-' {
                if slug.is_empty() || last_was_dash {
                    continue;
                }
                last_was_dash = true;
            } else {
                last_was_dash = false;
            }
            slug.push(value);
        }
    }

    slug.trim_matches('-').to_string()
}

pub fn ensure_bitloops_repo_name(repo_name: &str) -> Result<()> {
    ensure!(
        repo_name == BITLOOPS_REPO_NAME,
        "unsupported repository `{repo_name}`; only `bitloops` is supported by qat"
    );
    Ok(())
}

pub fn enable_watcher_autostart_for_scenario(world: &mut QatWorld) -> Result<()> {
    world.watcher_autostart_enabled = true;
    Ok(())
}

const QAT_EVENTUAL_TIMEOUT_ENV: &str = "BITLOOPS_QAT_EVENTUAL_TIMEOUT_SECS";
// Watcher-driven sync materialisation is asynchronous end-to-end: the CLI
// restarts the watcher, the watcher debounces filesystem events, and the daemon
// then consumes the spooled sync work. CI can legitimately take longer here.
const DEFAULT_QAT_EVENTUAL_TIMEOUT_SECS: u64 = 120;
const QAT_EVENTUAL_POLL_INTERVAL_MILLIS: u64 = 250;

fn qat_eventual_timeout() -> StdDuration {
    parse_timeout_seconds(
        std::env::var(QAT_EVENTUAL_TIMEOUT_ENV).ok().as_deref(),
        DEFAULT_QAT_EVENTUAL_TIMEOUT_SECS,
    )
}

fn qat_eventual_poll_interval() -> StdDuration {
    StdDuration::from_millis(QAT_EVENTUAL_POLL_INTERVAL_MILLIS)
}

fn wait_for_qat_condition<T, Observe, Ready, Describe>(
    timeout: StdDuration,
    poll_interval: StdDuration,
    expected: &str,
    mut observe: Observe,
    is_ready: Ready,
    describe: Describe,
) -> Result<T>
where
    Observe: FnMut() -> Result<T>,
    Ready: Fn(&T) -> bool,
    Describe: Fn(&T) -> String,
{
    let started = Instant::now();
    let mut attempts = 0_usize;
    let mut last_observation: String;

    loop {
        attempts += 1;
        match observe() {
            Ok(value) => {
                let summary = describe(&value);
                if is_ready(&value) {
                    return Ok(value);
                }
                last_observation = format!("value: {summary}");
            }
            Err(err) => {
                last_observation = format!("error: {err:#}");
            }
        }

        if started.elapsed() >= timeout {
            bail!(
                "timed out after {}s waiting for {expected}; attempts={attempts}; last observation={}",
                timeout.as_secs(),
                last_observation
            );
        }

        std::thread::sleep(poll_interval);
    }
}

pub fn ensure_daemon_for_scenario(world: &mut QatWorld) -> Result<()> {
    stop_daemon_for_scenario(world).ok();

    let stderr_log_path = daemon_stderr_log_path(world.run_dir());
    let daemon_started = Instant::now();
    let mut attempt_errors = Vec::new();
    for port in daemon_candidate_ports(world.run_dir()) {
        let attempt_started = Instant::now();
        append_world_log(
            world,
            &format!("Starting foreground daemon for scenario using port candidate {port}.\n"),
        )?;

        let mut child = spawn_daemon_process(world, &port, &stderr_log_path)?;
        match wait_for_daemon_ready(world.run_dir(), &mut child, &stderr_log_path) {
            Ok((runtime_state_path, runtime_state)) => {
                append_timing_log(
                    world,
                    "daemon startup",
                    daemon_started.elapsed(),
                    format!(
                        "port={port} attempts={} pid={}",
                        attempt_errors.len() + 1,
                        runtime_state.pid
                    ),
                )?;
                append_world_log(
                    world,
                    &format!(
                        "Daemon ready for scenario on {} (pid {}, requested port {}).\nRuntime state: {}\nStderr log: {}\n",
                        runtime_state.url,
                        runtime_state.pid,
                        port,
                        runtime_state_path.display(),
                        stderr_log_path.display()
                    ),
                )?;
                world.daemon_url = Some(runtime_state.url.clone());
                world.daemon_runtime_state_path = Some(runtime_state_path.clone());
                world.daemon_stderr_log_path = Some(stderr_log_path.clone());
                world.daemon_process = Some(crate::qat_support::world::ScenarioDaemonProcess {
                    child,
                    requested_port: port,
                    stderr_log_path,
                    runtime_state_path,
                });
                return Ok(());
            }
            Err(err) => {
                append_timing_log(
                    world,
                    "daemon startup attempt",
                    attempt_started.elapsed(),
                    format!("port={port} status=failed"),
                )?;
                append_world_log(
                    world,
                    &format!("Daemon startup attempt failed for port candidate {port}: {err:#}\n"),
                )?;
                let _ = child.kill();
                let _ = child.wait();
                attempt_errors.push(format!("port {port}: {err:#}"));
            }
        }
    }

    bail!(
        "failed to bootstrap and start daemon for QAT scenario\n{}",
        attempt_errors.join("\n\n")
    );
}

fn error_chain_contains_not_found(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|io_err| io_err.kind() == std::io::ErrorKind::NotFound)
    })
}

fn text_has_database_locked_error(text: &str) -> bool {
    text.to_ascii_lowercase().contains("database is locked")
}

const WATCHER_TEARDOWN_TIMEOUT_SECS: u64 = 5;
const WATCHER_TEARDOWN_POLL_INTERVAL_MILLIS: u64 = 50;

fn scenario_daemon_config_root(world: &QatWorld) -> Result<PathBuf> {
    with_scenario_app_env(world, || {
        bitloops::config::default_daemon_config_path()
            .context("resolving scenario daemon config path")?
            .parent()
            .map(Path::to_path_buf)
            .context("resolving scenario daemon config directory")
    })
}

fn open_scenario_runtime_sqlite(
    world: &QatWorld,
) -> Result<bitloops::storage::SqliteConnectionPool> {
    let config_root = scenario_daemon_config_root(world)?;
    let db_path = bitloops::config::resolve_repo_runtime_db_path_for_config_root(&config_root);
    let sqlite = bitloops::storage::SqliteConnectionPool::connect(db_path.clone())
        .with_context(|| format!("opening scenario runtime sqlite {}", db_path.display()))?;
    sqlite
        .initialise_runtime_checkpoint_schema()
        .context("initialising scenario runtime checkpoint schema")?;
    Ok(sqlite)
}

fn open_scenario_daemon_runtime_store(world: &QatWorld) -> Result<QatDaemonSqliteRuntimeStore> {
    with_scenario_app_env(world, QatDaemonSqliteRuntimeStore::open)
        .context("opening scenario daemon runtime store")
}

fn latest_completed_sync_task_source(world: &QatWorld) -> Result<Option<DevqlTaskSource>> {
    let repo_id = bitloops::host::devql::resolve_repo_id(world.repo_dir())
        .context("resolving repo id for latest completed sync task source")?;
    let store = open_scenario_daemon_runtime_store(world)?;
    let Some(state) = store
        .load_devql_task_queue_state()
        .context("loading DevQL task queue state")?
    else {
        return Ok(None);
    };

    Ok(state
        .tasks
        .iter()
        .enumerate()
        .filter(|task| {
            task.1.repo_id == repo_id
                && task.1.kind == DevqlTaskKind::Sync
                && task.1.status == DevqlTaskStatus::Completed
        })
        .max_by_key(|(index, task)| {
            (
                task.completed_at_unix.unwrap_or(0),
                task.updated_at_unix,
                *index,
            )
        })
        .map(|(_, task)| task.source))
}

fn completed_sync_task_source_count(
    world: &QatWorld,
    expected_source: DevqlTaskSource,
) -> Result<usize> {
    let repo_id = bitloops::host::devql::resolve_repo_id(world.repo_dir())
        .context("resolving repo id for completed sync task source count")?;
    let store = open_scenario_daemon_runtime_store(world)?;
    let Some(state) = store
        .load_devql_task_queue_state()
        .context("loading DevQL task queue state")?
    else {
        return Ok(0);
    };

    Ok(state
        .tasks
        .iter()
        .filter(|task| {
            task.repo_id == repo_id
                && task.kind == DevqlTaskKind::Sync
                && task.status == DevqlTaskStatus::Completed
                && task.source == expected_source
        })
        .count())
}

fn completed_sync_task_with_source_exists(
    world: &QatWorld,
    expected_source: DevqlTaskSource,
) -> Result<bool> {
    Ok(completed_sync_task_source_count(world, expected_source)? > 0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SyncTaskSummaryField {
    Work,
    Added,
    Changed,
    Removed,
    Unchanged,
    CacheHits,
    CacheMisses,
    ParseErrors,
}

impl SyncTaskSummaryField {
    fn label(self) -> &'static str {
        match self {
            Self::Work => "work",
            Self::Added => "added",
            Self::Changed => "changed",
            Self::Removed => "removed",
            Self::Unchanged => "unchanged",
            Self::CacheHits => "cache hits",
            Self::CacheMisses => "cache misses",
            Self::ParseErrors => "parse errors",
        }
    }
}

fn parse_sync_task_summary_field(raw: &str) -> Result<SyncTaskSummaryField> {
    let normalised = raw
        .trim()
        .to_ascii_lowercase()
        .replace(['_', '-'], " ");
    match normalised.as_str() {
        "work" => Ok(SyncTaskSummaryField::Work),
        "added" => Ok(SyncTaskSummaryField::Added),
        "changed" => Ok(SyncTaskSummaryField::Changed),
        "removed" => Ok(SyncTaskSummaryField::Removed),
        "unchanged" => Ok(SyncTaskSummaryField::Unchanged),
        "cache hits" => Ok(SyncTaskSummaryField::CacheHits),
        "cache misses" => Ok(SyncTaskSummaryField::CacheMisses),
        "parse errors" => Ok(SyncTaskSummaryField::ParseErrors),
        other => bail!("unsupported DevQL sync task summary field `{other}`"),
    }
}

fn sync_task_summary_field_value(
    summary: &bitloops::host::devql::SyncSummary,
    field: SyncTaskSummaryField,
) -> usize {
    match field {
        SyncTaskSummaryField::Work => {
            summary.paths_added
                + summary.paths_changed
                + summary.paths_removed
                + summary.paths_unchanged
        }
        SyncTaskSummaryField::Added => summary.paths_added,
        SyncTaskSummaryField::Changed => summary.paths_changed,
        SyncTaskSummaryField::Removed => summary.paths_removed,
        SyncTaskSummaryField::Unchanged => summary.paths_unchanged,
        SyncTaskSummaryField::CacheHits => summary.cache_hits,
        SyncTaskSummaryField::CacheMisses => summary.cache_misses,
        SyncTaskSummaryField::ParseErrors => summary.parse_errors,
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct CompletedSyncTaskSummaryBrief {
    task_id: String,
    source: String,
    mode: String,
    paths_added: usize,
    paths_changed: usize,
    paths_removed: usize,
    paths_unchanged: usize,
    cache_hits: usize,
    cache_misses: usize,
    parse_errors: usize,
}

fn completed_sync_task_summary_brief(
    task: &bitloops::daemon::DevqlTaskRecord,
) -> CompletedSyncTaskSummaryBrief {
    let Some(summary) = task.sync_result() else {
        return CompletedSyncTaskSummaryBrief {
            task_id: task.task_id.clone(),
            source: task.source.to_string(),
            mode: "<missing sync result>".to_string(),
            ..Default::default()
        };
    };

    CompletedSyncTaskSummaryBrief {
        task_id: task.task_id.clone(),
        source: task.source.to_string(),
        mode: summary.mode.clone(),
        paths_added: summary.paths_added,
        paths_changed: summary.paths_changed,
        paths_removed: summary.paths_removed,
        paths_unchanged: summary.paths_unchanged,
        cache_hits: summary.cache_hits,
        cache_misses: summary.cache_misses,
        parse_errors: summary.parse_errors,
    }
}

fn completed_sync_task_summary_brief_field_value(
    brief: &CompletedSyncTaskSummaryBrief,
    field: SyncTaskSummaryField,
) -> usize {
    match field {
        SyncTaskSummaryField::Work => {
            brief.paths_added + brief.paths_changed + brief.paths_removed + brief.paths_unchanged
        }
        SyncTaskSummaryField::Added => brief.paths_added,
        SyncTaskSummaryField::Changed => brief.paths_changed,
        SyncTaskSummaryField::Removed => brief.paths_removed,
        SyncTaskSummaryField::Unchanged => brief.paths_unchanged,
        SyncTaskSummaryField::CacheHits => brief.cache_hits,
        SyncTaskSummaryField::CacheMisses => brief.cache_misses,
        SyncTaskSummaryField::ParseErrors => brief.parse_errors,
    }
}

fn completed_sync_task_summary_field_exceeds_min(
    tasks: &[CompletedSyncTaskSummaryBrief],
    field: SyncTaskSummaryField,
    min: usize,
) -> bool {
    tasks
        .iter()
        .any(|task| completed_sync_task_summary_brief_field_value(task, field) > min)
}

fn format_completed_sync_task_summary_diagnostics(
    tasks: &[CompletedSyncTaskSummaryBrief],
) -> String {
    if tasks.is_empty() {
        return "no post-snapshot completed sync tasks".to_string();
    }

    tasks
        .iter()
        .map(|task| {
            format!(
                "task_id={} source={} mode={} added={} changed={} removed={} unchanged={} cache_hits={} cache_misses={} parse_errors={}",
                task.task_id,
                task.source,
                task.mode,
                task.paths_added,
                task.paths_changed,
                task.paths_removed,
                task.paths_unchanged,
                task.cache_hits,
                task.cache_misses,
                task.parse_errors
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn completed_sync_task_summary_briefs_after_source_snapshot(
    world: &QatWorld,
    expected_source: DevqlTaskSource,
    snapshot_count: usize,
) -> Result<Vec<CompletedSyncTaskSummaryBrief>> {
    let repo_id = bitloops::host::devql::resolve_repo_id(world.repo_dir())
        .context("resolving repo id for completed sync task summary assertion")?;
    let store = open_scenario_daemon_runtime_store(world)?;
    let Some(state) = store
        .load_devql_task_queue_state()
        .context("loading DevQL task queue state")?
    else {
        return Ok(Vec::new());
    };

    Ok(state
        .tasks
        .iter()
        .filter(|task| {
            task.repo_id == repo_id
                && task.kind == DevqlTaskKind::Sync
                && task.status == DevqlTaskStatus::Completed
                && task.source == expected_source
        })
        .skip(snapshot_count)
        .map(completed_sync_task_summary_brief)
        .collect())
}

fn parse_devql_task_source(raw: &str) -> Result<DevqlTaskSource> {
    match raw.trim() {
        "init" => Ok(DevqlTaskSource::Init),
        "manual_cli" | "manual-cli" | "manual" => Ok(DevqlTaskSource::ManualCli),
        "watcher" => Ok(DevqlTaskSource::Watcher),
        "post_commit" | "post-commit" => Ok(DevqlTaskSource::PostCommit),
        "post_merge" | "post-merge" => Ok(DevqlTaskSource::PostMerge),
        "post_checkout" | "post-checkout" => Ok(DevqlTaskSource::PostCheckout),
        "repo_policy_change" | "repo-policy-change" => Ok(DevqlTaskSource::RepoPolicyChange),
        other => bail!("unsupported expected DevQL task source `{other}`"),
    }
}

fn watcher_process_is_running(pid: u32) -> Result<bool> {
    #[cfg(windows)]
    {
        Ok(Command::new("cmd")
            .args([
                "/C",
                &format!("tasklist /FI \"PID eq {pid}\" | findstr {pid}"),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("running `tasklist` for DevQL watcher")?
            .success())
    }

    #[cfg(not(windows))]
    {
        Ok(Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("running `kill -0` for DevQL watcher")?
            .success())
    }
}

fn terminate_watcher_process(pid: u32) -> Result<()> {
    #[cfg(windows)]
    {
        let status = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("running `taskkill` for DevQL watcher")?;
        ensure!(
            status.success(),
            "failed to stop DevQL watcher process {pid}"
        );
    }

    #[cfg(not(windows))]
    {
        let status = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("running `kill -TERM` for DevQL watcher")?;
        ensure!(
            status.success(),
            "failed to stop DevQL watcher process {pid}"
        );
    }

    Ok(())
}

fn stop_registered_watcher_for_scenario(world: &QatWorld) -> Result<()> {
    if !world.watcher_autostart_enabled {
        return Ok(());
    }

    let repo_root = world.repo_dir().to_string_lossy().to_string();
    let sqlite = open_scenario_runtime_sqlite(world)?;
    let Some((pid, restart_token, state)) = sqlite
        .with_connection(|conn| {
            use rusqlite::OptionalExtension as _;

            conn.query_row(
                "SELECT pid, restart_token, state
                 FROM repo_watcher_registrations
                 WHERE repo_root = ?1
                 LIMIT 1",
                rusqlite::params![repo_root.as_str()],
                |row| {
                    Ok((
                        row.get::<_, u32>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(anyhow::Error::from)
        })
        .context("loading scenario watcher registration")?
    else {
        return Ok(());
    };

    append_world_log(
        world,
        &format!(
            "Found DevQL watcher registration for scenario repo: pid={} state={}.\n",
            pid, state
        ),
    )?;

    if watcher_process_is_running(pid)? {
        append_world_log(
            world,
            &format!(
                "Registered DevQL watcher still running during teardown; terminating pid {}.\n",
                pid
            ),
        )?;
        terminate_watcher_process(pid)?;
        wait_for_qat_condition(
            StdDuration::from_secs(WATCHER_TEARDOWN_TIMEOUT_SECS),
            StdDuration::from_millis(WATCHER_TEARDOWN_POLL_INTERVAL_MILLIS),
            &format!("DevQL watcher process {} to exit", pid),
            || watcher_process_is_running(pid),
            |running| !*running,
            |running| format!("running={running}"),
        )
        .with_context(|| {
            format!(
                "waiting for DevQL watcher process {} to exit during scenario teardown",
                pid
            )
        })?;
    } else {
        append_world_log(
            world,
            &format!(
                "Registered DevQL watcher pid {} was already stopped.\n",
                pid
            ),
        )?;
    }

    sqlite
        .with_connection(|conn| {
            conn.execute(
                "DELETE FROM repo_watcher_registrations
                 WHERE repo_root = ?1 AND pid = ?2 AND restart_token = ?3",
                rusqlite::params![repo_root.as_str(), pid, restart_token.as_str()],
            )
            .map(|_| ())
            .map_err(anyhow::Error::from)
        })
        .context("clearing scenario watcher registration")?;
    append_world_log(world, "Cleared DevQL watcher registration for scenario.\n")?;

    Ok(())
}

pub fn assert_devql_watcher_registered_and_running_for_repo(
    world: &QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let repo_root = world.repo_dir().to_string_lossy().to_string();
    let sqlite = open_scenario_runtime_sqlite(world)?;
    let Some((pid, state)) = sqlite
        .with_connection(|conn| {
            use rusqlite::OptionalExtension as _;

            conn.query_row(
                "SELECT pid, state
                 FROM repo_watcher_registrations
                 WHERE repo_root = ?1
                 LIMIT 1",
                rusqlite::params![repo_root.as_str()],
                |row| Ok((row.get::<_, u32>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(anyhow::Error::from)
        })
        .context("loading DevQL watcher registration")?
    else {
        bail!("expected DevQL watcher registration for repo `{repo_name}`");
    };

    ensure!(
        state == "ready",
        "expected DevQL watcher registration state `ready`, got `{state}`"
    );
    ensure!(
        watcher_process_is_running(pid)?,
        "expected DevQL watcher pid {pid} to be running"
    );
    Ok(())
}

pub fn stop_daemon_for_scenario(world: &mut QatWorld) -> Result<()> {
    if world.run_dir.is_none() || world.repo_dir.is_none() || world.terminal_log_path.is_none() {
        return Ok(());
    }

    let had_daemon = world.daemon_process.is_some() || world.daemon_url.is_some();
    let mut stop_error = None;

    if had_daemon {
        let mut attempts = 0_u8;
        loop {
            match run_command_capture(
                world,
                "bitloops daemon stop",
                build_bitloops_command(world, &["daemon", "stop"])?,
            ) {
                Ok(output) if output.status.success() => {
                    append_world_log(world, "Daemon stopped for scenario via CLI.\n")?;
                    break;
                }
                Ok(output) => {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    if text_has_database_locked_error(&stdout)
                        || text_has_database_locked_error(&stderr)
                    {
                        attempts += 1;
                        if attempts <= 3 {
                            append_world_log(
                                world,
                                &format!(
                                    "Daemon stop hit a transient SQLite lock (attempt {attempts}/3); retrying.\n",
                                ),
                            )?;
                            std::thread::sleep(std::time::Duration::from_millis(
                                200 * u64::from(attempts),
                            ));
                            continue;
                        }

                        append_world_log(
                            world,
                            "Daemon stop remained locked after retries; falling back to forced process teardown.\n",
                        )?;
                        break;
                    }

                    append_world_log(
                        world,
                        &format!(
                            "Daemon stop returned non-zero.\nstdout:\n{stdout}\nstderr:\n{stderr}\n",
                        ),
                    )?;
                    stop_error = Some(anyhow!(
                        "bitloops daemon stop returned non-zero\nstdout:\n{stdout}\nstderr:\n{stderr}"
                    ));
                    break;
                }
                Err(err) if error_chain_contains_not_found(&err) => {
                    append_world_log(
                        world,
                        "Daemon stop skipped because the bitloops binary is no longer present.\n",
                    )?;
                    break;
                }
                Err(err) => {
                    let locked = text_has_database_locked_error(&err.to_string());
                    if locked {
                        attempts += 1;
                        if attempts <= 3 {
                            append_world_log(
                                world,
                                &format!(
                                    "Daemon stop command hit a transient SQLite lock (attempt {attempts}/3); retrying.\n",
                                ),
                            )?;
                            std::thread::sleep(std::time::Duration::from_millis(
                                200 * u64::from(attempts),
                            ));
                            continue;
                        }
                        append_world_log(
                            world,
                            "Daemon stop command remained locked after retries; falling back to forced process teardown.\n",
                        )?;
                        break;
                    }
                    append_world_log(world, &format!("Daemon stop failed: {err:#}\n"))?;
                    stop_error = Some(err.context("running bitloops daemon stop"));
                    break;
                }
            }
        }
    }

    if let Some(mut process) = world.daemon_process.take() {
        match process.child.try_wait() {
            Ok(Some(status)) => {
                append_world_log(
                    world,
                    &format!("Foreground daemon child already exited with status {status}.\n"),
                )?;
            }
            Ok(None) => {
                append_world_log(
                    world,
                    &format!(
                        "Foreground daemon child still running after stop; terminating pid {}.\n",
                        process.child.id()
                    ),
                )?;
                if let Err(err) = process.child.kill() {
                    append_world_log(
                        world,
                        &format!("Failed to terminate foreground daemon child: {err}\n"),
                    )?;
                }
                if let Err(err) = process.child.wait() {
                    append_world_log(
                        world,
                        &format!("Failed waiting for foreground daemon child: {err}\n"),
                    )?;
                }
            }
            Err(err) => {
                append_world_log(
                    world,
                    &format!("Failed to inspect foreground daemon child: {err}\n"),
                )?;
            }
        }
    }

    if let Err(err) = stop_registered_watcher_for_scenario(world) {
        append_world_log(world, &format!("DevQL watcher teardown failed: {err:#}\n"))?;
        stop_error = Some(match stop_error.take() {
            Some(existing) => anyhow!("{existing:#}\n\n{err:#}"),
            None => err,
        });
    }

    world.daemon_url = None;
    world.daemon_runtime_state_path = None;
    world.daemon_stderr_log_path = None;

    if let Some(err) = stop_error {
        return Err(err);
    }

    Ok(())
}

pub fn run_clean_start(world: &mut QatWorld, flow_name: &str) -> Result<()> {
    let config = world.run_config().clone();
    let flow_slug = sanitize_name(flow_name);
    ensure!(
        !flow_slug.is_empty(),
        "flow name must produce a non-empty slug"
    );

    let scenario_slug = world
        .scenario_slug
        .clone()
        .unwrap_or_else(|| "scenario".to_string());
    let run_dir = config
        .suite_root
        .join(format!("{scenario_slug}-{flow_slug}-{}", short_run_id()));
    let repo_dir = run_dir.join(BITLOOPS_REPO_NAME);
    let terminal_log_path = run_dir.join("terminal.log");
    let metadata_path = run_dir.join("run.json");

    fs::create_dir_all(&repo_dir).context("creating qat repo directory")?;

    world.flow_name = Some(flow_name.to_string());
    world.run_dir = Some(run_dir);
    world.repo_dir = Some(repo_dir);
    world.terminal_log_path = Some(terminal_log_path);
    world.metadata_path = Some(metadata_path);

    let init_output = run_command_capture(
        world,
        "git init",
        build_git_command(world, &["init", "-q"], &[]),
    )?;
    ensure_success(&init_output, "git init")?;
    configure_git_identity(world)?;
    write_run_metadata(world)?;
    append_world_log(
        world,
        &format!(
            "Initialized clean run directory at {}\n",
            world.run_dir().display()
        ),
    )?;
    Ok(())
}

pub fn run_init_commit_for_repo(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    if repo_has_head(world)? {
        append_world_log(world, "InitCommit skipped because HEAD already exists.\n")?;
        return Ok(());
    }

    let readme_path = world.repo_dir().join("README.md");
    fs::write(
        &readme_path,
        format!("# {repo_name}\n\nInitial repo for Bitloops foundation tests.\n"),
    )
    .with_context(|| format!("writing {}", readme_path.display()))?;
    run_git_success(world, &["add", "-A"], &[], "git add -A")?;
    run_git_success(
        world,
        &["commit", "-m", "chore: initial commit"],
        &[],
        "git commit initial",
    )?;
    capture_head_sha(world)?;
    Ok(())
}

pub fn run_init_commit_without_post_commit_refresh_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    if repo_has_head(world)? {
        append_world_log(
            world,
            "InitCommit without post-commit refresh skipped because HEAD already exists.\n",
        )?;
        return Ok(());
    }

    let readme_path = world.repo_dir().join("README.md");
    fs::write(
        &readme_path,
        format!("# {repo_name}\n\nInitial repo for Bitloops foundation tests.\n"),
    )
    .with_context(|| format!("writing {}", readme_path.display()))?;
    run_git_success(world, &["add", "-A"], &[], "git add -A")?;
    let output = run_command_capture(
        world,
        "git commit initial (no post-commit refresh)",
        build_init_commit_without_post_commit_refresh_command(world),
    )?;
    ensure_success(&output, "git commit initial (no post-commit refresh)")?;
    capture_head_sha(world)?;
    Ok(())
}

pub fn run_init_commit_with_relative_day_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
    days_ago: i64,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    if repo_has_head(world)? {
        append_world_log(
            world,
            "InitCommit with relative day skipped because HEAD already exists.\n",
        )?;
        return Ok(());
    }

    let readme_path = world.repo_dir().join("README.md");
    fs::write(
        &readme_path,
        format!("# {repo_name}\n\nInitial repo for Bitloops foundation tests.\n"),
    )
    .with_context(|| format!("writing {}", readme_path.display()))?;
    let git_date = git_date_for_relative_day(days_ago)?;
    let env = [
        ("GIT_AUTHOR_DATE", OsString::from(git_date.clone())),
        ("GIT_COMMITTER_DATE", OsString::from(git_date)),
    ];
    run_git_success(world, &["add", "-A"], &env, "git add -A")?;
    run_git_success(
        world,
        &["commit", "-m", "chore: initial commit"],
        &env,
        "git commit initial",
    )?;
    capture_head_sha(world)?;
    Ok(())
}

pub fn run_create_vite_app_project_for_repo(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    create_offline_vite_react_ts_scaffold(world.repo_dir())
}

pub fn run_init_bitloops_for_repo(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    run_init_bitloops_with_agent(world, repo_name, "claude-code", false, None)
}

fn normalise_onboarding_agent_name(agent_name: &str) -> &str {
    if agent_name.eq_ignore_ascii_case("claude") {
        AGENT_NAME_CLAUDE_CODE
    } else if agent_name.eq_ignore_ascii_case("open-code") {
        AGENT_NAME_OPEN_CODE
    } else {
        agent_name
    }
}

fn normalise_smoke_agent_name(agent_name: &str) -> &str {
    let normalised = normalise_onboarding_agent_name(agent_name);
    if normalised.eq_ignore_ascii_case(AGENT_NAME_CLAUDE_CODE) {
        AGENT_NAME_CLAUDE_CODE
    } else if normalised.eq_ignore_ascii_case(AGENT_NAME_CURSOR) {
        AGENT_NAME_CURSOR
    } else if normalised.eq_ignore_ascii_case(AGENT_NAME_GEMINI) {
        AGENT_NAME_GEMINI
    } else if normalised.eq_ignore_ascii_case(AGENT_NAME_COPILOT) {
        AGENT_NAME_COPILOT
    } else if normalised.eq_ignore_ascii_case(AGENT_NAME_CODEX) {
        AGENT_NAME_CODEX
    } else if normalised.eq_ignore_ascii_case(AGENT_NAME_OPEN_CODE) {
        AGENT_NAME_OPEN_CODE
    } else {
        normalised
    }
}

pub fn run_init_bitloops_with_agent(
    world: &mut QatWorld,
    repo_name: &str,
    agent_name: &str,
    force: bool,
    sync: Option<bool>,
) -> Result<()> {
    run_init_bitloops_with_agents(world, repo_name, &[agent_name], force, sync)
}

pub fn run_init_bitloops_with_agents(
    world: &mut QatWorld,
    repo_name: &str,
    agent_names: &[&str],
    force: bool,
    sync: Option<bool>,
) -> Result<()> {
    run_init_bitloops_with_agent_config(world, repo_name, agent_names, force, sync, None, None)
}

pub fn run_init_bitloops_with_agent_sync_ingest_backfill(
    world: &mut QatWorld,
    repo_name: &str,
    agent_name: &str,
    sync: bool,
    ingest: bool,
    backfill: usize,
) -> Result<()> {
    run_init_bitloops_with_agent_config(
        world,
        repo_name,
        &[agent_name],
        false,
        Some(sync),
        Some(ingest),
        Some(backfill),
    )
}

fn run_init_bitloops_with_agent_config(
    world: &mut QatWorld,
    repo_name: &str,
    agent_names: &[&str],
    force: bool,
    sync: Option<bool>,
    ingest: Option<bool>,
    backfill: Option<usize>,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    ensure!(
        !agent_names.is_empty(),
        "at least one agent must be provided for init"
    );

    let normalised_agent_names = agent_names
        .iter()
        .map(|agent_name| normalise_onboarding_agent_name(agent_name))
        .collect::<Vec<_>>();
    world.agent_name = normalised_agent_names
        .first()
        .map(|agent_name| (*agent_name).to_string());

    let args_owned = build_init_bitloops_args_with_options(
        &normalised_agent_names,
        force,
        sync,
        ingest,
        backfill,
    );
    let label = format!("bitloops {}", args_owned.join(" "));
    let mut attempts = 0_u8;

    loop {
        let args: Vec<&str> = args_owned.iter().map(String::as_str).collect();
        let output = run_command_capture(world, &label, build_bitloops_command(world, &args)?)
            .with_context(|| format!("running {label}"))?;
        if output.status.success() {
            return Ok(());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let locked = stdout.contains("database is locked") || stderr.contains("database is locked");
        if locked && attempts < 2 {
            attempts += 1;
            std::thread::sleep(std::time::Duration::from_millis(250 * u64::from(attempts)));
            continue;
        }

        return ensure_success(&output, &label);
    }
}

fn build_init_bitloops_args(agent_name: &str, force: bool, sync: Option<bool>) -> Vec<String> {
    build_init_bitloops_args_with_options(&[agent_name], force, sync, None, None)
}

fn build_init_bitloops_args_with_options(
    agent_names: &[&str],
    force: bool,
    sync: Option<bool>,
    ingest: Option<bool>,
    backfill: Option<usize>,
) -> Vec<String> {
    debug_assert!(!agent_names.is_empty(), "init requires at least one agent");

    let mut args = vec!["init".to_string()];
    for agent_name in agent_names {
        args.push("--agent".to_string());
        args.push((*agent_name).to_string());
    }

    let sync_choice = sync.unwrap_or(false);
    let ingest_choice = if backfill.is_some() {
        true
    } else {
        ingest.unwrap_or(false)
    };
    args.push(format!("--sync={sync_choice}"));
    args.push(format!("--ingest={ingest_choice}"));
    if let Some(backfill_window) = backfill {
        args.push(format!("--backfill={backfill_window}"));
    }

    if force {
        args.push("--force".to_string());
    }

    args
}

fn build_init_bitloops_args_with_producer_contract_options(
    agent_name: &str,
    sync: bool,
) -> Vec<String> {
    vec![
        "init".to_string(),
        "--install-default-daemon".to_string(),
        "--agent".to_string(),
        agent_name.to_string(),
        "--no-embeddings".to_string(),
        "--no-summaries".to_string(),
        format!("--sync={sync}"),
        "--ingest=false".to_string(),
    ]
}

pub fn run_init_bitloops_producer_contract_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
    agent_name: &str,
    sync: bool,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    enable_watcher_autostart_for_scenario(world)?;

    let normalised_agent_name = normalise_onboarding_agent_name(agent_name);
    world.agent_name = Some(normalised_agent_name.to_string());
    let args_owned =
        build_init_bitloops_args_with_producer_contract_options(normalised_agent_name, sync);
    let label = format!("bitloops {}", args_owned.join(" "));
    let args = args_owned.iter().map(String::as_str).collect::<Vec<_>>();
    let output = run_command_capture(world, &label, build_bitloops_command(world, &args)?)
        .with_context(|| format!("running {label}"))?;
    ensure_success(&output, &label)
}

pub fn run_bitloops_enable_with_flags(
    world: &mut QatWorld,
    repo_name: &str,
    flags: &[&str],
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let mut args = vec!["enable"];
    args.extend_from_slice(flags);
    let label = format!("bitloops {}", args.join(" "));
    run_enable(world, &args, &label)
}

fn run_enable(world: &mut QatWorld, args: &[&str], label: &str) -> Result<()> {
    let output = run_command_capture(world, label, build_bitloops_command(world, args)?)
        .with_context(|| format!("running {label}"))?;
    ensure_success(&output, label)
}

pub fn run_bitloops_disable_with_flags(
    world: &mut QatWorld,
    repo_name: &str,
    flags: &[&str],
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let mut args = vec!["disable"];
    args.extend_from_slice(flags);
    let label = format!("bitloops {}", args.join(" "));
    run_bitloops_success(world, &args, &label)
}

pub fn run_bitloops_uninstall_full(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    run_bitloops_success(
        world,
        &["uninstall", "--full", "-f"],
        "bitloops uninstall --full",
    )
}

pub fn assert_bitloops_binary_removed(world: &mut QatWorld) -> Result<()> {
    let mut cmd = build_bitloops_command(world, &["--version"])?;
    match cmd.output() {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Binary not found — this is the expected outcome after full uninstall
            Ok(())
        }
        Err(e) => bail!("unexpected error running bitloops --version: {e}"),
        Ok(output) => {
            ensure!(
                !output.status.success(),
                "expected bitloops --version to fail after full uninstall, but it exited with code 0"
            );
            Ok(())
        }
    }
}

pub fn run_bitloops_uninstall_hooks(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    run_bitloops_success(
        world,
        &[
            "uninstall",
            "--agent-hooks",
            "--git-hooks",
            "--only-current-project",
            "-f",
        ],
        "bitloops uninstall --agent-hooks --git-hooks",
    )
}

pub fn ensure_claude_auth_for_repo(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    ensure_claude_authenticated(world)
}

pub fn run_devql_init_for_repo(world: &QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    run_bitloops_success(world, &["devql", "init"], "bitloops devql init")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DevqlTaskEnqueueKind {
    Sync,
    Ingest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevqlTaskBriefRecord {
    pub task_id: String,
    pub kind: String,
    pub status: String,
    pub repo: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevqlTaskQueueStatusSnapshot {
    pub state: String,
    pub queued: usize,
    pub running: usize,
    pub failed: usize,
    pub completed_recent: usize,
    pub pause_reason: Option<String>,
    pub last_action: Option<String>,
    pub current_repo_tasks: Vec<DevqlTaskBriefRecord>,
}

fn build_devql_task_enqueue_args(kind: DevqlTaskEnqueueKind, flags: &[&str]) -> Vec<String> {
    let kind_arg = match kind {
        DevqlTaskEnqueueKind::Sync => "sync",
        DevqlTaskEnqueueKind::Ingest => "ingest",
    };
    let mut args = vec![
        "devql".to_string(),
        "tasks".to_string(),
        "enqueue".to_string(),
        "--kind".to_string(),
        kind_arg.to_string(),
    ];
    args.extend(flags.iter().map(|flag| (*flag).to_string()));
    args
}

fn build_devql_tasks_args(command_args: &[&str]) -> Vec<String> {
    let mut args = vec!["devql".to_string(), "tasks".to_string()];
    args.extend(command_args.iter().map(|value| (*value).to_string()));
    args
}

fn run_devql_task_enqueue_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
    kind: DevqlTaskEnqueueKind,
    flags: &[&str],
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    if matches!(kind, DevqlTaskEnqueueKind::Sync) {
        world.last_test_harness_target_generation = None;
    }
    let args = build_devql_task_enqueue_args(kind, flags);
    let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let label = format!("bitloops {}", args.join(" "));
    let output = run_command_capture(world, &label, build_bitloops_command(world, &arg_refs)?)?;
    world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    update_last_task_id_from_output(world, &stdout, TaskIdCaptureMode::CaptureSubmission);
    world.last_command_stdout = Some(stdout);
    ensure_success(&output, &label)?;

    if matches!(kind, DevqlTaskEnqueueKind::Sync) && flags.contains(&"--status") {
        let repo_id = bitloops::host::devql::resolve_repo_id(world.repo_dir())
            .context("resolving repo_id for TestHarness sync target generation")?;
        world.last_test_harness_target_generation =
            load_latest_test_harness_generation_state(world, &repo_id)?
                .map(|(_, state)| state.latest_generation_seq);
    }

    Ok(())
}

fn run_devql_tasks_command_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
    command_args: &[&str],
    expect_success: bool,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let args = build_devql_tasks_args(command_args);
    let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let label = format!("bitloops {}", args.join(" "));
    let output = run_command_capture(world, &label, build_bitloops_command(world, &arg_refs)?)?;
    world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    update_last_task_id_from_output(world, &stdout, TaskIdCaptureMode::PreserveExisting);
    world.last_command_stdout = Some(stdout);
    if expect_success {
        ensure_success(&output, &label)?;
    }
    Ok(())
}

pub fn enqueue_devql_ingest_task_with_status_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    run_devql_task_enqueue_for_repo(
        world,
        repo_name,
        DevqlTaskEnqueueKind::Ingest,
        &["--status"],
    )
}

pub fn enqueue_devql_ingest_task_without_status_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    run_devql_task_enqueue_for_repo(world, repo_name, DevqlTaskEnqueueKind::Ingest, &[])
}

pub fn enqueue_devql_ingest_task_with_backfill_and_status_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
    backfill: usize,
) -> Result<()> {
    let backfill = backfill.to_string();
    run_devql_task_enqueue_for_repo(
        world,
        repo_name,
        DevqlTaskEnqueueKind::Ingest,
        &["--backfill", backfill.as_str(), "--status"],
    )
}

pub fn assert_version_output(world: &mut QatWorld) -> Result<()> {
    let output = run_command_capture(
        world,
        "bitloops --version",
        build_bitloops_command(world, &["--version"])?,
    )?;
    ensure_success(&output, "bitloops --version")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let re =
        regex::Regex::new(r"Bitloops CLI v\d+\.\d+\.\d+").context("compiling version regex")?;
    ensure!(
        re.is_match(&stdout),
        "expected semver in version output, got:\n{}",
        stdout
    );
    Ok(())
}

pub fn assert_daemon_config_exists(world: &QatWorld) -> Result<()> {
    let home = world.run_dir().join("home");
    let macos_config = home
        .join("Library")
        .join("Application Support")
        .join("bitloops")
        .join("config.toml");
    let xdg_config = home.join("xdg").join("bitloops").join("config.toml");
    ensure!(
        macos_config.exists() || xdg_config.exists(),
        "expected daemon config at {} or {}",
        macos_config.display(),
        xdg_config.display()
    );
    Ok(())
}

pub fn assert_config_has_relational_store(world: &QatWorld) -> Result<()> {
    let home = world.run_dir().join("home");
    let macos_config = home
        .join("Library")
        .join("Application Support")
        .join("bitloops")
        .join("config.toml");
    let xdg_config = home.join("xdg").join("bitloops").join("config.toml");
    let config_path = if macos_config.exists() {
        macos_config
    } else {
        xdg_config
    };
    let content = fs::read_to_string(&config_path)
        .with_context(|| format!("reading {}", config_path.display()))?;
    ensure!(
        content.contains("[stores.relational]"),
        "daemon config missing [stores.relational] section:\n{}",
        content
    );
    ensure!(
        content.contains("sqlite_path") || content.contains("postgres_dsn"),
        "daemon config missing relational store path:\n{}",
        content
    );
    Ok(())
}

pub fn assert_config_has_event_store(world: &QatWorld) -> Result<()> {
    let home = world.run_dir().join("home");
    let macos_config = home
        .join("Library")
        .join("Application Support")
        .join("bitloops")
        .join("config.toml");
    let xdg_config = home.join("xdg").join("bitloops").join("config.toml");
    let config_path = if macos_config.exists() {
        macos_config
    } else {
        xdg_config
    };
    let content = fs::read_to_string(&config_path)
        .with_context(|| format!("reading {}", config_path.display()))?;
    ensure!(
        content.contains("[stores.events]"),
        "daemon config missing [stores.events] section:\n{}",
        content
    );
    ensure!(
        content.contains("duckdb_path"),
        "daemon config missing event store path:\n{}",
        content
    );
    Ok(())
}

pub fn assert_config_has_blob_store(world: &QatWorld) -> Result<()> {
    let home = world.run_dir().join("home");
    let macos_config = home
        .join("Library")
        .join("Application Support")
        .join("bitloops")
        .join("config.toml");
    let xdg_config = home.join("xdg").join("bitloops").join("config.toml");
    let config_path = if macos_config.exists() {
        macos_config
    } else {
        xdg_config
    };
    let content = fs::read_to_string(&config_path)
        .with_context(|| format!("reading {}", config_path.display()))?;
    ensure!(
        content.contains("[stores.blob]"),
        "daemon config missing [stores.blob] section:\n{}",
        content
    );
    ensure!(
        content.contains("local_path"),
        "daemon config missing blob store path:\n{}",
        content
    );
    Ok(())
}

pub fn assert_store_paths_exist(world: &QatWorld) -> Result<()> {
    let home = world.run_dir().join("home");
    let macos_config = home
        .join("Library")
        .join("Application Support")
        .join("bitloops")
        .join("config.toml");
    let xdg_config = home.join("xdg").join("bitloops").join("config.toml");
    let config_path = if macos_config.exists() {
        macos_config
    } else {
        xdg_config
    };
    let resolved = resolve_daemon_config(Some(&config_path))
        .with_context(|| format!("resolving daemon config from {}", config_path.display()))?;

    ensure!(
        resolved.relational_db_path.is_file(),
        "SQLite store file does not exist at {}",
        resolved.relational_db_path.display()
    );
    ensure!(
        resolved.events_db_path.is_file(),
        "DuckDB store file does not exist at {}",
        resolved.events_db_path.display()
    );
    ensure!(
        resolved.blob_store_path.is_dir(),
        "Blob store directory does not exist at {}",
        resolved.blob_store_path.display()
    );
    Ok(())
}

pub fn assert_global_runtime_artefacts_removed(world: &QatWorld) -> Result<()> {
    let (config_dir, data_dir, cache_dir, state_dir) = with_scenario_app_env(world, || {
        Ok::<_, anyhow::Error>((
            bitloops::utils::platform_dirs::bitloops_config_dir()?,
            bitloops::utils::platform_dirs::bitloops_data_dir()?,
            bitloops::utils::platform_dirs::bitloops_cache_dir()?,
            bitloops::utils::platform_dirs::bitloops_state_dir()?,
        ))
    })?;
    ensure!(
        !config_dir.exists(),
        "expected Bitloops config dir to be removed: {}",
        config_dir.display()
    );
    ensure!(
        !data_dir.exists(),
        "expected Bitloops data dir to be removed: {}",
        data_dir.display()
    );
    ensure!(
        !cache_dir.exists(),
        "expected Bitloops cache dir to be removed: {}",
        cache_dir.display()
    );
    ensure!(
        !state_dir.exists(),
        "expected Bitloops state dir to be removed: {}",
        state_dir.display()
    );
    Ok(())
}

pub fn assert_file_exists_in_repo(
    world: &QatWorld,
    repo_name: &str,
    relative_path: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let full_path = world.repo_dir().join(relative_path);
    ensure!(
        full_path.exists(),
        "expected path to exist: {}",
        full_path.display()
    );
    Ok(())
}

pub fn assert_file_missing_in_repo(
    world: &QatWorld,
    repo_name: &str,
    relative_path: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let full_path = world.repo_dir().join(relative_path);
    ensure!(
        !full_path.exists(),
        "expected path to not exist: {}",
        full_path.display()
    );
    Ok(())
}

fn agent_hooks_path(repo_root: &Path, agent_name: &str) -> Result<std::path::PathBuf> {
    Ok(match agent_name {
        AGENT_NAME_CLAUDE_CODE => repo_root.join(".claude").join("settings.json"),
        AGENT_NAME_CODEX => repo_root.join(".codex").join("hooks.json"),
        AGENT_NAME_CURSOR => repo_root.join(".cursor").join("hooks.json"),
        AGENT_NAME_GEMINI => repo_root.join(".gemini").join("settings.json"),
        AGENT_NAME_COPILOT => repo_root
            .join(".github")
            .join("hooks")
            .join("bitloops.json"),
        AGENT_NAME_OPEN_CODE => repo_root
            .join(".opencode")
            .join("plugins")
            .join("bitloops.ts"),
        other => bail!("unsupported agent for hook assertion: {other}"),
    })
}

fn managed_agent_hook_marker(agent_name: &str) -> Result<&'static str> {
    Ok(match agent_name {
        AGENT_NAME_CLAUDE_CODE => "bitloops hooks claude-code ",
        AGENT_NAME_CODEX => "bitloops hooks codex ",
        AGENT_NAME_CURSOR => "bitloops hooks cursor ",
        AGENT_NAME_GEMINI => "bitloops hooks gemini ",
        AGENT_NAME_COPILOT => "bitloops hooks copilot ",
        AGENT_NAME_OPEN_CODE => "Auto-generated by `bitloops init --agent opencode`",
        other => bail!("unsupported agent for hook assertion: {other}"),
    })
}

fn repo_has_bitloops_git_post_commit_hook(repo_root: &Path) -> Result<bool> {
    let post_commit_path = repo_root.join(".git").join("hooks").join("post-commit");
    if !post_commit_path.exists() {
        return Ok(false);
    }
    let post_commit_content = fs::read_to_string(&post_commit_path)
        .with_context(|| format!("reading {}", post_commit_path.display()))?;
    Ok(post_commit_content.contains("bitloops hooks git post-commit"))
}

fn assert_git_post_commit_hook_installed_at(repo_root: &Path) -> Result<()> {
    let post_commit_path = repo_root.join(".git").join("hooks").join("post-commit");
    ensure!(
        post_commit_path.exists(),
        "expected git post-commit hook at {}",
        post_commit_path.display()
    );
    ensure!(
        repo_has_bitloops_git_post_commit_hook(repo_root)?,
        "missing post-commit bitloops hook in {}",
        post_commit_path.display()
    );
    Ok(())
}

pub fn assert_git_post_commit_hook_installed(world: &QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    assert_git_post_commit_hook_installed_at(world.repo_dir())
}

pub fn assert_agent_hooks_installed(
    world: &QatWorld,
    repo_name: &str,
    agent_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let normalised_agent_name = normalise_smoke_agent_name(agent_name);
    let registration = bitloops::adapters::agents::AgentAdapterRegistry::builtin()
        .resolve(normalised_agent_name)
        .with_context(|| format!("resolving agent registration for `{normalised_agent_name}`"))?;
    let hooks_path = agent_hooks_path(world.repo_dir(), normalised_agent_name)?;
    ensure!(
        hooks_path.exists(),
        "expected managed agent hooks file at {}",
        hooks_path.display()
    );
    ensure!(
        registration.are_hooks_installed(world.repo_dir()),
        "expected `{normalised_agent_name}` hooks to be installed according to the adapter registration"
    );
    assert_git_post_commit_hook_installed_at(world.repo_dir())
}

pub fn assert_agent_hooks_removed(
    world: &QatWorld,
    repo_name: &str,
    agent_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let normalised_agent_name = normalise_smoke_agent_name(agent_name);
    let registration = bitloops::adapters::agents::AgentAdapterRegistry::builtin()
        .resolve(normalised_agent_name)
        .with_context(|| format!("resolving agent registration for `{normalised_agent_name}`"))?;
    ensure!(
        !registration.are_hooks_installed(world.repo_dir()),
        "expected `{normalised_agent_name}` hooks to be removed according to the adapter registration"
    );
    let hooks_path = agent_hooks_path(world.repo_dir(), normalised_agent_name)?;
    if hooks_path.exists() {
        let content = fs::read_to_string(&hooks_path)
            .with_context(|| format!("reading {}", hooks_path.display()))?;
        let managed_marker = managed_agent_hook_marker(normalised_agent_name)?;
        ensure!(
            !content.contains(managed_marker),
            "agent hooks file still contains managed marker `{managed_marker}` after uninstall: {}",
            hooks_path.display()
        );
    }
    Ok(())
}

pub fn assert_git_hooks_removed(world: &QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let post_commit_path = world
        .repo_dir()
        .join(".git")
        .join("hooks")
        .join("post-commit");
    if post_commit_path.exists() {
        let content = fs::read_to_string(&post_commit_path)
            .with_context(|| format!("reading {}", post_commit_path.display()))?;
        ensure!(
            !content.contains("bitloops hooks git post-commit"),
            "git post-commit hook still contains bitloops after uninstall: {}",
            post_commit_path.display()
        );
    }
    Ok(())
}

pub fn assert_status_shows_disabled(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let output = run_command_capture(
        world,
        "bitloops checkpoints status",
        build_bitloops_command(world, &["checkpoints", "status"])?,
    )?;
    ensure_success(&output, "bitloops checkpoints status")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    ensure!(
        stdout.contains("Disabled"),
        "expected disabled status output, got:\n{}",
        stdout
    );
    let settings =
        load_settings(world.repo_dir()).context("loading repo settings after bitloops disable")?;
    ensure!(
        !settings.enabled,
        "expected capture.enabled=false after disable, but settings report enabled=true"
    );
    Ok(())
}

pub fn run_devql_query(world: &mut QatWorld, query: &str) -> Result<serde_json::Value> {
    let output = run_command_capture(
        world,
        "bitloops devql query",
        build_bitloops_command(world, &["devql", "query", query, "--compact"])?,
    )?;
    world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
    ensure_success(&output, "bitloops devql query")?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    world.last_command_stdout = Some(stdout.clone());
    serde_json::from_str(stdout.trim()).context("parsing devql query json output")
}

pub fn run_devql_graphql_query(world: &mut QatWorld, query: &str) -> Result<serde_json::Value> {
    let output = run_command_capture(
        world,
        "bitloops devql query --graphql",
        build_bitloops_command(world, &["devql", "query", "--graphql", query, "--compact"])?,
    )?;
    world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
    ensure_success(&output, "bitloops devql query --graphql")?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    world.last_command_stdout = Some(stdout.clone());
    serde_json::from_str(stdout.trim()).context("parsing raw DevQL GraphQL json output")
}

pub fn resolve_head_sha(world: &QatWorld) -> Result<String> {
    let output = run_command_capture(
        world,
        "git rev-parse HEAD",
        build_git_command(world, &["rev-parse", "HEAD"], &[]),
    )?;
    ensure_success(&output, "git rev-parse HEAD")?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn capture_head_sha(world: &mut QatWorld) -> Result<String> {
    let sha = resolve_head_sha(world)?;
    world.captured_commit_shas.push(sha.clone());
    Ok(sha)
}

pub fn count_json_array_rows(value: &serde_json::Value) -> usize {
    match value {
        serde_json::Value::Array(arr) => arr.len(),
        serde_json::Value::Object(obj) => obj
            .get("rows")
            .or_else(|| obj.get("data"))
            .and_then(serde_json::Value::as_array)
            .map(std::vec::Vec::len)
            .unwrap_or(0),
        _ => 0,
    }
}

fn count_artefacts_across_source_files(world: &mut QatWorld) -> Result<usize> {
    if let Ok(value) = run_devql_query(world, r#"repo("bitloops")->artefacts()->limit(500)"#) {
        let count = count_json_array_rows(&value);
        if count > 0 {
            return Ok(count);
        }
    }

    let mut pending = vec![world.repo_dir().to_path_buf()];
    let mut file_paths = Vec::new();
    while let Some(dir) = pending.pop() {
        if !dir.exists() {
            continue;
        }
        for entry in fs::read_dir(&dir)
            .with_context(|| format!("reading source directory {}", dir.display()))?
        {
            let entry = entry.with_context(|| format!("reading entry in {}", dir.display()))?;
            let path = entry.path();
            if path.is_dir() {
                let dir_name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("");
                if matches!(
                    dir_name,
                    ".git" | ".bitloops" | "node_modules" | "target" | "dist"
                ) {
                    continue;
                }
                pending.push(path);
                continue;
            }
            let Some(extension) = path.extension().and_then(|ext| ext.to_str()) else {
                continue;
            };
            if !matches!(extension, "ts" | "tsx" | "js" | "jsx" | "rs" | "py") {
                continue;
            }
            let relative = path
                .strip_prefix(world.repo_dir())
                .with_context(|| format!("making path relative for {}", path.display()))?
                .to_string_lossy()
                .replace('\\', "/");
            file_paths.push(relative);
        }
    }

    let mut total = 0;
    for file_path in file_paths {
        let query = format!(
            r#"repo("bitloops")->file("{}")->artefacts()->limit(500)"#,
            escape_devql_string(&file_path)
        );
        let value = match run_devql_query(world, &query) {
            Ok(value) => value,
            Err(err) => {
                let message = err.to_string();
                if message.contains("missing string field `canonical_kind`")
                    || message.contains("unknown path")
                {
                    append_world_log(
                        world,
                        &format!(
                            "Skipping artefacts count for `{file_path}` due unsupported path or backend mismatch.\n"
                        ),
                    )?;
                    continue;
                }
                return Err(err);
            }
        };
        total += count_json_array_rows(&value);
    }
    Ok(total)
}

pub fn run_first_change_using_claude_code_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    run_first_change_using_agent_for_repo(world, repo_name, AGENT_NAME_CLAUDE_CODE)
}

pub fn run_claude_code_prompt_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
    prompt: &str,
) -> Result<()> {
    run_agent_prompt_for_repo(world, repo_name, AGENT_NAME_CLAUDE_CODE, prompt)
}

pub fn run_agent_prompt_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
    agent_name: &str,
    prompt: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let normalised_agent_name = normalise_smoke_agent_name(agent_name);
    world.agent_name = Some(normalised_agent_name.to_string());

    match normalised_agent_name {
        AGENT_NAME_CLAUDE_CODE => run_deterministic_claude_smoke_prompt(world, prompt),
        AGENT_NAME_CURSOR => run_cursor_prompt(world, prompt),
        AGENT_NAME_GEMINI => run_gemini_prompt(world, prompt),
        AGENT_NAME_COPILOT => run_copilot_prompt(world, prompt),
        AGENT_NAME_CODEX => run_codex_prompt(world, prompt),
        AGENT_NAME_OPEN_CODE => run_opencode_prompt(world, prompt),
        other => bail!("unsupported smoke agent: {other}"),
    }
}

pub fn run_second_change_using_claude_code_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    run_second_change_using_agent_for_repo(world, repo_name, AGENT_NAME_CLAUDE_CODE)
}

pub fn run_first_change_using_agent_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
    agent_name: &str,
) -> Result<()> {
    run_change_using_agent_for_repo(world, repo_name, agent_name, FIRST_CLAUDE_PROMPT)
}

pub fn run_second_change_using_agent_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
    agent_name: &str,
) -> Result<()> {
    run_change_using_agent_for_repo(world, repo_name, agent_name, SECOND_CLAUDE_PROMPT)
}

fn run_change_using_agent_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
    agent_name: &str,
    prompt: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let normalised_agent_name = normalise_smoke_agent_name(agent_name);
    world.agent_name = Some(normalised_agent_name.to_string());

    match normalised_agent_name {
        AGENT_NAME_CLAUDE_CODE => run_deterministic_claude_smoke_prompt(world, prompt),
        AGENT_NAME_CURSOR => run_cursor_prompt(world, prompt),
        AGENT_NAME_GEMINI => run_gemini_prompt(world, prompt),
        AGENT_NAME_COPILOT => run_copilot_prompt(world, prompt),
        AGENT_NAME_CODEX => run_codex_prompt(world, prompt),
        AGENT_NAME_OPEN_CODE => run_opencode_prompt(world, prompt),
        other => bail!("unsupported smoke agent: {other}"),
    }
}

// ── DevQL sync helpers ───────────────────────────────────────

const DAEMON_CAPABILITY_EVENT_STATUS_TIMEOUT_ENV: &str =
    "BITLOOPS_QAT_DAEMON_CAPABILITY_EVENT_STATUS_TIMEOUT_SECS";
const DEFAULT_DAEMON_CAPABILITY_EVENT_STATUS_TIMEOUT_SECS: u64 = 60;
const DAEMON_CAPABILITY_EVENT_STATUS_POLL_INTERVAL_MILLIS: u64 = 250;

pub fn enqueue_devql_sync_task_with_status_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    run_devql_task_enqueue_for_repo(world, repo_name, DevqlTaskEnqueueKind::Sync, &["--status"])
}

pub fn enqueue_devql_sync_task_without_status_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    run_devql_task_enqueue_for_repo(world, repo_name, DevqlTaskEnqueueKind::Sync, &[])
}

pub fn enqueue_devql_sync_task_with_paths_and_status_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
    paths: &[String],
) -> Result<()> {
    ensure!(
        !paths.is_empty(),
        "expected at least one path for scoped sync"
    );
    let joined = paths.join(",");
    run_devql_task_enqueue_for_repo(
        world,
        repo_name,
        DevqlTaskEnqueueKind::Sync,
        &["--paths", joined.as_str(), "--status"],
    )
}

pub fn enqueue_devql_full_sync_task_with_status_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    run_devql_task_enqueue_for_repo(
        world,
        repo_name,
        DevqlTaskEnqueueKind::Sync,
        &["--full", "--status"],
    )
}

pub fn enqueue_devql_sync_validate_task_with_status_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    run_devql_task_enqueue_for_repo(
        world,
        repo_name,
        DevqlTaskEnqueueKind::Sync,
        &["--validate", "--status"],
    )
}

pub fn enqueue_devql_sync_repair_task_with_status_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    run_devql_task_enqueue_for_repo(
        world,
        repo_name,
        DevqlTaskEnqueueKind::Sync,
        &["--repair", "--status"],
    )
}

pub fn attempt_to_enqueue_devql_sync_task_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let args = build_devql_task_enqueue_args(DevqlTaskEnqueueKind::Sync, &[]);
    let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let label = "bitloops devql tasks enqueue --kind sync (expect failure)";
    let output = run_command_capture(world, label, build_bitloops_command(world, &arg_refs)?)?;
    world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    world.last_command_stdout = Some(format!("{stdout}\n{stderr}"));
    Ok(())
}

pub fn attempt_to_enqueue_devql_sync_task_require_daemon_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let args = build_devql_task_enqueue_args(DevqlTaskEnqueueKind::Sync, &["--require-daemon"]);
    let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let label = "bitloops devql tasks enqueue --kind sync --require-daemon (expect failure)";
    let output = run_command_capture(world, label, build_bitloops_command(world, &arg_refs)?)?;
    world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    world.last_command_stdout = Some(format!("{stdout}\n{stderr}"));
    Ok(())
}

pub fn run_devql_tasks_status_for_repo(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    run_devql_tasks_command_for_repo(world, repo_name, &["status"], true)
}

fn devql_task_queue_status_is_idle(snapshot: &DevqlTaskQueueStatusSnapshot) -> bool {
    snapshot.queued == 0 && snapshot.running == 0 && snapshot.failed == 0
}

pub fn wait_for_devql_task_queue_idle_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let timeout = qat_eventual_timeout();
    let started = Instant::now();
    let mut attempts = 0_usize;

    let status = loop {
        attempts += 1;
        run_devql_tasks_status_for_repo(world, repo_name)?;
        let status =
            parse_task_queue_status(world.last_command_stdout.as_deref().unwrap_or(""))?;
        let observation = format!(
            "queued={}, running={}, failed={}, current_repo_tasks={}",
            status.queued,
            status.running,
            status.failed,
            status.current_repo_tasks.len()
        );
        ensure!(
            status.failed == 0,
            "DevQL task queue has failed tasks while waiting for idle; attempts={attempts}; observation={observation}"
        );
        if devql_task_queue_status_is_idle(&status) {
            break status;
        }
        if started.elapsed() >= timeout {
            bail!(
                "timed out after {}s waiting for DevQL task queue to become idle; attempts={attempts}; last observation: {observation}",
                timeout.as_secs()
            );
        }
        std::thread::sleep(qat_eventual_poll_interval());
    };
    world.last_command_exit_code = Some(0);
    world.last_command_stdout = Some(format!(
        "DevQL task queue reached idle state: queued={}, running={}",
        status.queued, status.running
    ));
    append_timing_log(
        world,
        "wait DevQL task queue idle",
        started.elapsed(),
        format!("repo={repo_name} attempts={attempts}"),
    )?;
    Ok(())
}

pub fn wait_for_completed_sync_task_source_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
    expected_source: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let started = Instant::now();
    let expected_source = parse_devql_task_source(expected_source)?;
    let source_key = expected_source.to_string();
    if let Some(snapshot_count) = world
        .completed_sync_task_source_snapshots
        .get(&source_key)
        .copied()
    {
        wait_for_qat_condition(
            qat_eventual_timeout(),
            qat_eventual_poll_interval(),
            &format!(
                "completed DevQL sync task with source `{expected_source}` after snapshot count {snapshot_count}"
            ),
            || completed_sync_task_source_count(world, expected_source),
            |count| *count > snapshot_count,
            |count| format!("count={count}, snapshot={snapshot_count}"),
        )?;
    } else {
        wait_for_qat_condition(
            qat_eventual_timeout(),
            qat_eventual_poll_interval(),
            &format!("completed DevQL sync task with source `{expected_source}`"),
            || completed_sync_task_with_source_exists(world, expected_source),
            |exists| *exists,
            |exists| format!("exists={exists}"),
        )?;
    }
    append_timing_log(
        world,
        "wait completed sync task source",
        started.elapsed(),
        format!("repo={repo_name} source={expected_source}"),
    )?;
    Ok(())
}

pub fn wait_for_completed_sync_task_source_summary_field_greater_than_since_snapshot_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
    expected_source: &str,
    field: &str,
    min: usize,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let started = Instant::now();
    let expected_source = parse_devql_task_source(expected_source)?;
    let source_key = expected_source.to_string();
    let snapshot_count = world
        .completed_sync_task_source_snapshots
        .get(&source_key)
        .copied()
        .ok_or_else(|| {
            anyhow!(
                "no completed DevQL sync task source snapshot captured for `{expected_source}`"
            )
        })?;
    let field = parse_sync_task_summary_field(field)?;

    wait_for_qat_condition(
        qat_eventual_timeout(),
        qat_eventual_poll_interval(),
        &format!(
            "completed DevQL sync task with source `{expected_source}` to show {} > {min} after snapshot count {snapshot_count}",
            field.label()
        ),
        || {
            completed_sync_task_summary_briefs_after_source_snapshot(
                world,
                expected_source,
                snapshot_count,
            )
        },
        |tasks| completed_sync_task_summary_field_exceeds_min(tasks, field, min),
        |tasks| format_completed_sync_task_summary_diagnostics(tasks),
    )?;
    append_timing_log(
        world,
        "wait completed sync task summary",
        started.elapsed(),
        format!("repo={repo_name} source={expected_source} field={}", field.label()),
    )?;
    Ok(())
}

pub fn snapshot_completed_sync_task_source_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
    expected_source: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let expected_source = parse_devql_task_source(expected_source)?;
    let count = completed_sync_task_source_count(world, expected_source)?;
    world
        .completed_sync_task_source_snapshots
        .insert(expected_source.to_string(), count);
    Ok(())
}

pub fn assert_latest_completed_sync_task_source_for_repo(
    world: &QatWorld,
    repo_name: &str,
    expected_source: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let expected_source = parse_devql_task_source(expected_source)?;
    let actual = latest_completed_sync_task_source(world)?;
    ensure!(
        actual == Some(expected_source),
        "expected latest completed sync task source `{expected_source}`, got `{actual:?}`"
    );
    Ok(())
}

pub fn run_devql_tasks_list_for_repo(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    run_devql_tasks_command_for_repo(world, repo_name, &["list"], true)
}

pub fn run_devql_tasks_list_for_status_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
    status: &str,
) -> Result<()> {
    run_devql_tasks_command_for_repo(world, repo_name, &["list", "--status", status], true)
}

pub fn watch_last_devql_task_for_repo(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    let task_id = world
        .last_task_id
        .clone()
        .ok_or_else(|| anyhow!("no DevQL task id captured for watch"))?;
    run_devql_tasks_command_for_repo(world, repo_name, &["watch", task_id.as_str()], true)
}

pub fn pause_devql_tasks_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
    reason: Option<&str>,
) -> Result<()> {
    if let Some(reason) = reason {
        run_devql_tasks_command_for_repo(world, repo_name, &["pause", "--reason", reason], true)
    } else {
        run_devql_tasks_command_for_repo(world, repo_name, &["pause"], true)
    }
}

pub fn resume_devql_tasks_for_repo(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    run_devql_tasks_command_for_repo(world, repo_name, &["resume"], true)
}

pub fn cancel_last_devql_task_for_repo(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    let task_id = world
        .last_task_id
        .clone()
        .ok_or_else(|| anyhow!("no DevQL task id captured for cancel"))?;
    run_devql_tasks_command_for_repo(world, repo_name, &["cancel", task_id.as_str()], true)
}

pub fn add_new_rust_source_file(world: &QatWorld) -> Result<()> {
    let file_path = world.repo_dir().join("src").join("lib.rs");
    fs::write(
        &file_path,
        "pub fn greet(name: &str) -> String {\n    format!(\"Hello, {}!\", name)\n}\n",
    )
    .with_context(|| format!("writing {}", file_path.display()))?;
    Ok(())
}

pub fn add_source_file_at_path_for_repo(
    world: &QatWorld,
    repo_name: &str,
    relative_path: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    write_deterministic_source_file(world.repo_dir(), relative_path, false)
}

fn nudge_source_file_at_path_for_repo(
    world: &QatWorld,
    repo_name: &str,
    relative_path: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let path = world.repo_dir().join(relative_path);
    ensure!(
        path.exists(),
        "expected source file `{relative_path}` to exist before watcher nudge"
    );
    let mut file = OpenOptions::new()
        .append(true)
        .open(&path)
        .with_context(|| format!("opening {} for watcher nudge append", path.display()))?;
    file.write_all(b"\n")
        .with_context(|| format!("appending watcher nudge newline to {}", path.display()))?;
    Ok(())
}

pub fn modify_rust_main(world: &QatWorld) -> Result<()> {
    let file_path = world.repo_dir().join("src").join("main.rs");
    fs::write(
        &file_path,
        "mod lib;\n\nfn main() {\n    println!(\"{}\", lib::greet(\"Bitloops\"));\n}\n",
    )
    .with_context(|| format!("writing {}", file_path.display()))?;
    Ok(())
}

pub fn modify_source_file_at_path_for_repo(
    world: &QatWorld,
    repo_name: &str,
    relative_path: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    write_deterministic_source_file(world.repo_dir(), relative_path, true)
}

pub fn delete_rust_source_file(world: &QatWorld) -> Result<()> {
    let candidates = [
        world.repo_dir().join("src").join("lib.rs"),
        world.repo_dir().join("src").join("main.rs"),
    ];
    for path in &candidates {
        if path.exists() {
            fs::remove_file(path).with_context(|| format!("deleting {}", path.display()))?;
            return Ok(());
        }
    }
    bail!("no known Rust source file found to delete")
}

pub fn commit_without_hooks(world: &mut QatWorld) -> Result<()> {
    run_git_success(world, &["add", "-A"], &[], "git add -A")?;
    let diff_output = run_command_capture(
        world,
        "git diff --cached --quiet",
        build_git_command(world, &["diff", "--cached", "--quiet"], &[]),
    )?;
    let diff_code = diff_output.status.code().unwrap_or_default();
    let output = run_command_capture(
        world,
        "git commit (no hooks)",
        build_commit_without_hooks_command(world, diff_code == 0),
    )?;
    ensure_success(&output, "git commit (no hooks)")?;
    capture_head_sha(world)?;
    Ok(())
}

pub fn commit_with_hooks(world: &mut QatWorld) -> Result<()> {
    run_git_success(world, &["add", "-A"], &[], "git add -A")?;
    let diff_output = run_command_capture(
        world,
        "git diff --cached --quiet",
        build_git_command(world, &["diff", "--cached", "--quiet"], &[]),
    )?;
    let diff_code = diff_output.status.code().unwrap_or_default();
    let mut args = vec!["commit", "-m", "QAT change (hooks enabled)"];
    if diff_code == 0 {
        args.insert(1, "--allow-empty");
    }
    let output = run_command_capture(
        world,
        "git commit (hooks enabled)",
        build_git_command(world, &args, &[]),
    )?;
    ensure_success(&output, "git commit (hooks enabled)")?;
    capture_head_sha(world)?;
    Ok(())
}

pub fn create_branch_with_source_file_and_return_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
    branch_name: &str,
    relative_path: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let base_branch = current_branch_name(world)?;
    run_git_success(
        world,
        &["checkout", "-b", branch_name],
        &[],
        &format!("git checkout -b {branch_name}"),
    )?;
    write_deterministic_source_file(world.repo_dir(), relative_path, false)?;
    commit_without_hooks(world)?;
    run_git_success(
        world,
        &["checkout", base_branch.as_str()],
        &[],
        &format!("git checkout {base_branch}"),
    )?;
    capture_head_sha(world)?;
    Ok(())
}

pub fn checkout_branch_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
    branch_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    run_git_success(
        world,
        &["checkout", branch_name],
        &[],
        &format!("git checkout {branch_name}"),
    )?;
    capture_head_sha(world)?;
    Ok(())
}

pub fn checkout_previous_branch_for_repo(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    run_git_success(
        world,
        &["checkout", "-"],
        &[],
        "git checkout previous branch",
    )?;
    capture_head_sha(world)?;
    Ok(())
}

pub fn git_reset_hard_head_for_repo(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    run_git_success(
        world,
        &["reset", "--hard", "HEAD"],
        &[],
        "git reset --hard HEAD",
    )?;
    capture_head_sha(world)?;
    Ok(())
}

pub fn git_clean_fd_for_repo(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    run_git_success(world, &["clean", "-fd"], &[], "git clean -fd")
}

pub fn stage_changes_without_committing(world: &QatWorld) -> Result<()> {
    let output = run_command_capture(
        world,
        "git add -A (stage only)",
        build_git_command(world, &["add", "-A"], &[]),
    )?;
    ensure_success(&output, "git add -A (stage only)")
}

pub fn simulate_git_pull_with_changes(world: &mut QatWorld) -> Result<()> {
    run_git_success(
        world,
        &["checkout", "-b", "qat-remote-changes"],
        &[],
        "git checkout -b qat-remote-changes",
    )?;
    let file_path = world.repo_dir().join("src").join("utils.rs");
    fs::write(
        &file_path,
        "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n",
    )
    .with_context(|| format!("writing {}", file_path.display()))?;
    commit_without_hooks(world)?;
    run_git_success(
        world,
        &["checkout", "-"],
        &[],
        "git checkout previous branch",
    )?;
    run_git_success(
        world,
        &[
            "merge",
            "qat-remote-changes",
            "--no-ff",
            "-m",
            "Merge remote changes",
        ],
        &[],
        "git merge remote changes",
    )?;
    capture_head_sha(world)?;
    Ok(())
}

pub fn create_branch_with_additional_files(world: &mut QatWorld) -> Result<()> {
    run_git_success(
        world,
        &["checkout", "-b", "qat-feature-branch"],
        &[],
        "git checkout -b qat-feature-branch",
    )?;
    let file_path = world.repo_dir().join("src").join("config.rs");
    fs::write(
        &file_path,
        "pub const APP_NAME: &str = \"qat-sample\";\npub const VERSION: &str = \"0.1.0\";\n",
    )
    .with_context(|| format!("writing {}", file_path.display()))?;
    Ok(())
}

pub fn wait_for_test_harness_capability_event_completion_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let repo_id = bitloops::host::devql::resolve_repo_id(world.repo_dir())
        .context("resolving repo_id while waiting for TestHarness capability-event completion")?;
    let target_generation = world.last_test_harness_target_generation.ok_or_else(|| {
        anyhow!(
            "missing TestHarness target generation in {}; run DevQL sync with status before waiting for capability-event completion",
            repo_name
        )
    })?;
    let timeout = parse_timeout_seconds(
        std::env::var(DAEMON_CAPABILITY_EVENT_STATUS_TIMEOUT_ENV)
            .ok()
            .as_deref(),
        DEFAULT_DAEMON_CAPABILITY_EVENT_STATUS_TIMEOUT_SECS,
    );
    let started = Instant::now();
    let mut attempts = 0_usize;
    let mut last_payload = serde_json::json!({});

    loop {
        attempts += 1;
        let output = run_command_capture(
            world,
            "bitloops daemon status --json",
            build_bitloops_command(world, &["daemon", "status", "--json"])?,
        )?;
        world.last_command_exit_code = Some(output.status.code().unwrap_or(-1));
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        world.last_command_stdout = Some(stdout.clone());
        ensure_success(&output, "bitloops daemon status --json")?;

        let payload: serde_json::Value = serde_json::from_str(stdout.trim())
            .context("parsing bitloops daemon status --json output")?;
        if let Some(current_repo_run) = payload
            .get("capability_events")
            .and_then(|value| value.get("current_repo_run"))
            .filter(|current_repo_run| {
                current_repo_run
                    .get("capability_id")
                    .and_then(serde_json::Value::as_str)
                    == Some("test_harness")
                    && current_repo_run
                        .get("repo_id")
                        .and_then(serde_json::Value::as_str)
                        == Some(repo_id.as_str())
            })
        {
            if current_repo_run
                .get("status")
                .and_then(serde_json::Value::as_str)
                == Some("failed")
            {
                let run_id = current_repo_run
                    .get("run_id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("<unknown>");
                let handler_id = current_repo_run
                    .get("handler_id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("<unknown>");
                let event_kind = current_repo_run
                    .get("event_kind")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("<unknown>");
                let error = current_repo_run
                    .get("error")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("<no error>");
                bail!(
                    "test_harness capability event run failed while waiting for completion: run_id={run_id}; handler_id={handler_id}; event_kind={event_kind}; error={error}"
                );
            }
        }

        if let Some((_, state)) = load_latest_test_harness_generation_state(world, &repo_id)? {
            if test_harness_generation_state_reached_target(&state, target_generation) {
                return Ok(());
            }

            if let Some(last_error) = state.last_error.as_deref() {
                if let Some((_, persisted_run)) =
                    load_latest_test_harness_capability_event_run(world, &repo_id)?
                {
                    if persisted_run.status == bitloops::daemon::CapabilityEventRunStatus::Failed
                        && persisted_run.to_generation_seq >= target_generation
                    {
                        let run_id = persisted_run.run_id;
                        let handler_id = persisted_run.handler_id;
                        let event_kind = persisted_run.event_kind;
                        let error = persisted_run
                            .error
                            .unwrap_or_else(|| last_error.to_string());
                        bail!(
                            "test_harness capability event run failed while waiting for generation {target_generation}: run_id={run_id}; handler_id={handler_id}; event_kind={event_kind}; error={error}"
                        );
                    }
                }
                bail!(
                    "test_harness current-state cursor failed while waiting for generation {target_generation}: error={last_error}"
                );
            }
        }

        last_payload = payload;
        if started.elapsed() >= timeout {
            let last_payload = serde_json::to_string(&last_payload)
                .unwrap_or_else(|_| "<failed to serialize payload>".to_string());
            bail!(
                "timed out after {}s waiting for test_harness capability-event completion in {}; attempts={attempts}; last payload={last_payload}",
                timeout.as_secs(),
                repo_name
            );
        }
        std::thread::sleep(StdDuration::from_millis(
            DAEMON_CAPABILITY_EVENT_STATUS_POLL_INTERVAL_MILLIS,
        ));
    }
}

fn load_latest_test_harness_capability_event_run(
    world: &QatWorld,
    repo_id: &str,
) -> Result<
    Option<(
        std::path::PathBuf,
        bitloops::daemon::CapabilityEventRunRecord,
    )>,
> {
    let candidates = daemon_runtime_store_candidate_paths(world.run_dir());

    let mut latest: Option<(
        std::path::PathBuf,
        bitloops::daemon::CapabilityEventRunRecord,
    )> = None;
    for path in &candidates {
        if !path.exists() {
            continue;
        }

        let store = bitloops::host::runtime_store::DaemonSqliteRuntimeStore::open_at(path.clone())
            .with_context(|| format!("opening daemon runtime store {}", path.display()))?;
        let Some(run) = latest_capability_event_run(
            load_latest_test_harness_current_state_run(&store, repo_id)?,
            load_latest_test_harness_pack_reconcile_run(&store, repo_id)?,
        ) else {
            continue;
        };

        let replace = latest.as_ref().is_none_or(|(_, current)| {
            capability_event_run_sort_key(&run) > capability_event_run_sort_key(current)
        });
        if replace {
            latest = Some((path.clone(), run));
        }
    }

    Ok(latest)
}

fn latest_capability_event_run(
    current_state_run: Option<bitloops::daemon::CapabilityEventRunRecord>,
    legacy_run: Option<bitloops::daemon::CapabilityEventRunRecord>,
) -> Option<bitloops::daemon::CapabilityEventRunRecord> {
    match (current_state_run, legacy_run) {
        (Some(current_state), Some(legacy)) => {
            if capability_event_run_sort_key(&legacy)
                > capability_event_run_sort_key(&current_state)
            {
                Some(legacy)
            } else {
                Some(current_state)
            }
        }
        (Some(current_state), None) => Some(current_state),
        (None, Some(legacy)) => Some(legacy),
        (None, None) => None,
    }
}

fn capability_event_run_sort_key(
    run: &bitloops::daemon::CapabilityEventRunRecord,
) -> (u64, u64, u64, u64, u64) {
    (
        run.to_generation_seq,
        run.from_generation_seq,
        run.updated_at_unix,
        run.completed_at_unix.unwrap_or_default(),
        run.submitted_at_unix,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestHarnessGenerationState {
    latest_generation_seq: u64,
    last_applied_generation_seq: Option<u64>,
    last_error: Option<String>,
}

fn load_latest_test_harness_generation_state(
    world: &QatWorld,
    repo_id: &str,
) -> Result<Option<(std::path::PathBuf, TestHarnessGenerationState)>> {
    let candidates = daemon_runtime_store_candidate_paths(world.run_dir());

    let mut latest: Option<(std::path::PathBuf, TestHarnessGenerationState)> = None;
    for path in &candidates {
        if !path.exists() {
            continue;
        }

        let store = bitloops::host::runtime_store::DaemonSqliteRuntimeStore::open_at(path.clone())
            .with_context(|| format!("opening daemon runtime store {}", path.display()))?;
        let Some(state) = load_test_harness_generation_state(&store, repo_id)? else {
            continue;
        };

        let replace = latest.as_ref().is_none_or(|(_, current)| {
            test_harness_generation_state_sort_key(&state)
                > test_harness_generation_state_sort_key(current)
        });
        if replace {
            latest = Some((path.clone(), state));
        }
    }

    Ok(latest)
}

fn test_harness_generation_state_sort_key(state: &TestHarnessGenerationState) -> (u64, u64) {
    (
        state.latest_generation_seq,
        state.last_applied_generation_seq.unwrap_or_default(),
    )
}

fn test_harness_generation_state_reached_target(
    state: &TestHarnessGenerationState,
    target_generation: u64,
) -> bool {
    state.last_applied_generation_seq.unwrap_or_default() >= target_generation
}

fn load_test_harness_generation_state(
    store: &bitloops::host::runtime_store::DaemonSqliteRuntimeStore,
    repo_id: &str,
) -> Result<Option<TestHarnessGenerationState>> {
    use rusqlite::OptionalExtension;

    store.with_connection(|conn| {
        let Some(latest_generation_seq) = conn
            .query_row(
                "SELECT MAX(generation_seq) FROM capability_workplane_cursor_generations WHERE repo_id = ?1",
                rusqlite::params![repo_id],
                |row| row.get::<_, Option<i64>>(0),
            )
            .map(|value| value.and_then(|value| u64::try_from(value).ok()))?
        else {
            return Ok(None);
        };

        let mailbox = conn
            .query_row(
                "SELECT last_applied_generation_seq, last_error \
                 FROM capability_workplane_cursor_mailboxes \
                 WHERE repo_id = ?1 AND capability_id = ?2 AND mailbox_name = ?3",
                rusqlite::params![repo_id, "test_harness", "test_harness.current_state"],
                |row| {
                    Ok((
                        row.get::<_, Option<i64>>(0)?
                            .and_then(|value| u64::try_from(value).ok()),
                        row.get::<_, Option<String>>(1)?,
                    ))
                },
            )
            .optional()?;

        let (last_applied_generation_seq, last_error) =
            mailbox.unwrap_or((None, None));
        Ok(Some(TestHarnessGenerationState {
            latest_generation_seq,
            last_applied_generation_seq,
            last_error,
        }))
    })
}

fn load_latest_test_harness_current_state_run(
    store: &bitloops::host::runtime_store::DaemonSqliteRuntimeStore,
    repo_id: &str,
) -> Result<Option<bitloops::daemon::CapabilityEventRunRecord>> {
    use rusqlite::OptionalExtension;

    store.with_connection(|conn| {
        conn.query_row(
            "SELECT run_id, repo_id, repo_root, mailbox_name, capability_id, init_session_id, from_generation_seq, to_generation_seq, reconcile_mode, status, attempts, submitted_at_unix, started_at_unix, updated_at_unix, completed_at_unix, error \
             FROM capability_workplane_cursor_runs \
             WHERE capability_id = ?1 AND mailbox_name = ?2 AND repo_id = ?3 \
             ORDER BY to_generation_seq DESC, from_generation_seq DESC, updated_at_unix DESC, completed_at_unix DESC, submitted_at_unix DESC, rowid DESC \
             LIMIT 1",
            rusqlite::params!["test_harness", "test_harness.current_state", repo_id],
            |row| {
                let as_u64 = |index| -> rusqlite::Result<u64> {
                    row.get::<_, i64>(index)
                        .map(|value| u64::try_from(value).unwrap_or_default())
                };
                let opt_u64 = |index| -> rusqlite::Result<Option<u64>> {
                    row.get::<_, Option<i64>>(index).map(|value| {
                        value.and_then(|value| u64::try_from(value).ok())
                    })
                };
                let repo_id = row.get::<_, String>(1)?;
                let consumer_id = row.get::<_, String>(3)?;

                Ok(bitloops::daemon::CapabilityEventRunRecord {
                    run_id: row.get(0)?,
                    repo_id: repo_id.clone(),
                    capability_id: row.get(4)?,
                    init_session_id: row.get(5)?,
                    consumer_id: consumer_id.clone(),
                    handler_id: consumer_id.clone(),
                    from_generation_seq: as_u64(6)?,
                    to_generation_seq: as_u64(7)?,
                    reconcile_mode: row.get(8)?,
                    event_kind: "current_state_consumer".to_string(),
                    lane_key: format!("{repo_id}:{consumer_id}"),
                    event_payload_json: String::new(),
                    status: parse_capability_event_run_status(&row.get::<_, String>(9)?)?,
                    attempts: row.get(10)?,
                    submitted_at_unix: as_u64(11)?,
                    started_at_unix: opt_u64(12)?,
                    updated_at_unix: as_u64(13)?,
                    completed_at_unix: opt_u64(14)?,
                    error: row.get(15)?,
                })
            },
        )
        .optional()
        .map_err(anyhow::Error::from)
    })
}

fn load_latest_test_harness_pack_reconcile_run(
    store: &bitloops::host::runtime_store::DaemonSqliteRuntimeStore,
    repo_id: &str,
) -> Result<Option<bitloops::daemon::CapabilityEventRunRecord>> {
    use rusqlite::OptionalExtension;

    store.with_connection(|conn| {
        conn.query_row(
            "SELECT run_id, repo_id, capability_id, consumer_id, from_generation_seq, to_generation_seq, reconcile_mode, status, attempts, submitted_at_unix, started_at_unix, updated_at_unix, completed_at_unix, error \
             FROM pack_reconcile_runs \
             WHERE capability_id = ?1 AND consumer_id = ?2 AND repo_id = ?3 \
             ORDER BY to_generation_seq DESC, from_generation_seq DESC, updated_at_unix DESC, completed_at_unix DESC, submitted_at_unix DESC, rowid DESC \
             LIMIT 1",
            rusqlite::params!["test_harness", "test_harness.current_state", repo_id],
            |row| {
                let as_u64 = |index| -> rusqlite::Result<u64> {
                    row.get::<_, i64>(index)
                        .map(|value| u64::try_from(value).unwrap_or_default())
                };
                let opt_u64 = |index| -> rusqlite::Result<Option<u64>> {
                    row.get::<_, Option<i64>>(index).map(|value| {
                        value.and_then(|value| u64::try_from(value).ok())
                    })
                };
                let repo_id = row.get::<_, String>(1)?;
                let consumer_id = row.get::<_, String>(3)?;

                Ok(bitloops::daemon::CapabilityEventRunRecord {
                    run_id: row.get(0)?,
                    repo_id: repo_id.clone(),
                    capability_id: row.get(2)?,
                    init_session_id: None,
                    consumer_id: consumer_id.clone(),
                    handler_id: consumer_id.clone(),
                    from_generation_seq: as_u64(4)?,
                    to_generation_seq: as_u64(5)?,
                    reconcile_mode: row.get(6)?,
                    event_kind: String::new(),
                    lane_key: format!("{repo_id}:{consumer_id}"),
                    event_payload_json: String::new(),
                    status: parse_capability_event_run_status(&row.get::<_, String>(7)?)?,
                    attempts: row.get(8)?,
                    submitted_at_unix: as_u64(9)?,
                    started_at_unix: opt_u64(10)?,
                    updated_at_unix: as_u64(11)?,
                    completed_at_unix: opt_u64(12)?,
                    error: row.get(13)?,
                })
            },
        )
        .optional()
        .map_err(anyhow::Error::from)
    })
}

fn parse_capability_event_run_status(
    value: &str,
) -> rusqlite::Result<bitloops::daemon::CapabilityEventRunStatus> {
    match value {
        "queued" => Ok(bitloops::daemon::CapabilityEventRunStatus::Queued),
        "running" => Ok(bitloops::daemon::CapabilityEventRunStatus::Running),
        "completed" => Ok(bitloops::daemon::CapabilityEventRunStatus::Completed),
        "failed" => Ok(bitloops::daemon::CapabilityEventRunStatus::Failed),
        "cancelled" => Ok(bitloops::daemon::CapabilityEventRunStatus::Cancelled),
        other => Err(rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::other(format!(
                "unknown capability-event run status `{other}`"
            ))),
        )),
    }
}

fn resolve_qat_sync_state_path(
    world: &QatWorld,
) -> Result<(
    std::path::PathBuf,
    bitloops::host::runtime_store::PersistedDevqlTaskQueueState,
)> {
    let candidates = daemon_runtime_store_candidate_paths(world.run_dir());

    for path in &candidates {
        if !path.exists() {
            continue;
        }
        let store = bitloops::host::runtime_store::DaemonSqliteRuntimeStore::open_at(path.clone())
            .with_context(|| format!("opening daemon runtime store {}", path.display()))?;
        if let Some(state) = store
            .load_devql_task_queue_state()
            .with_context(|| format!("loading DevQL task queue state from {}", path.display()))?
        {
            return Ok((path.clone(), state));
        }
    }

    bail!(
        "could not find daemon DevQL task queue state in runtime store; looked in: {}",
        candidates
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    )
}

type CompletedHeadTasks = (
    String,
    std::path::PathBuf,
    Vec<(String, bitloops::host::devql::SyncSummary)>,
);

fn completed_tasks_for_current_head(
    world: &QatWorld,
    repo_name: &str,
) -> Result<CompletedHeadTasks> {
    ensure_bitloops_repo_name(repo_name)?;
    let head_output = run_command_capture(
        world,
        "git rev-parse HEAD (sync history assertion)",
        build_git_command(world, &["rev-parse", "HEAD"], &[]),
    )?;
    ensure_success(&head_output, "git rev-parse HEAD (sync history assertion)")?;

    let head_sha = String::from_utf8_lossy(&head_output.stdout)
        .trim()
        .to_string();
    ensure!(
        !head_sha.is_empty(),
        "expected non-empty HEAD SHA for sync history assertion"
    );

    let (sync_state_path, snapshot) = resolve_qat_sync_state_path(world)?;

    let head_tasks: Vec<(String, bitloops::host::devql::SyncSummary)> = snapshot
        .tasks
        .into_iter()
        .filter(|task| task.kind == bitloops::daemon::DevqlTaskKind::Sync)
        .filter(|task| task.status == bitloops::daemon::DevqlTaskStatus::Completed)
        .filter_map(|task| {
            let source = task.source.to_string();
            task.sync_result().cloned().map(|summary| (source, summary))
        })
        .filter(|(_, summary)| summary.head_commit_sha.as_deref() == Some(head_sha.as_str()))
        .collect();

    ensure!(
        !head_tasks.is_empty(),
        "expected at least one completed sync task for HEAD `{head_sha}` in {}; found none",
        sync_state_path.display()
    );

    Ok((head_sha, sync_state_path, head_tasks))
}

fn format_task_diagnostics(head_tasks: &[(String, bitloops::host::devql::SyncSummary)]) -> String {
    head_tasks
        .iter()
        .map(|(source, summary)| {
            format!(
                "source={} mode={} added={} changed={} removed={} unchanged={}",
                source,
                summary.mode,
                summary.paths_added,
                summary.paths_changed,
                summary.paths_removed,
                summary.paths_unchanged
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

pub fn assert_sync_history_has_added_for_current_head(
    world: &QatWorld,
    repo_name: &str,
) -> Result<()> {
    let (head_sha, _, head_tasks) = completed_tasks_for_current_head(world, repo_name)?;

    if head_tasks
        .iter()
        .any(|(_, summary)| summary.paths_added > 0)
    {
        return Ok(());
    }
    bail!(
        "expected at least one completed sync task with pathsAdded > 0 for HEAD `{head_sha}`; observed: {}",
        format_task_diagnostics(&head_tasks)
    )
}

pub fn assert_sync_history_has_changed_for_current_head(
    world: &QatWorld,
    repo_name: &str,
) -> Result<()> {
    let (head_sha, _, head_tasks) = completed_tasks_for_current_head(world, repo_name)?;

    if head_tasks
        .iter()
        .any(|(_, summary)| summary.paths_changed > 0)
    {
        return Ok(());
    }
    bail!(
        "expected at least one completed sync task with pathsChanged > 0 for HEAD `{head_sha}`; observed: {}",
        format_task_diagnostics(&head_tasks)
    )
}

pub fn assert_sync_history_has_removed_for_current_head(
    world: &QatWorld,
    repo_name: &str,
) -> Result<()> {
    let (head_sha, _, head_tasks) = completed_tasks_for_current_head(world, repo_name)?;

    if head_tasks
        .iter()
        .any(|(_, summary)| summary.paths_removed > 0)
    {
        return Ok(());
    }
    bail!(
        "expected at least one completed sync task with pathsRemoved > 0 for HEAD `{head_sha}`; observed: {}",
        format_task_diagnostics(&head_tasks)
    )
}

pub fn assert_sync_history_has_artefacts_for_current_head(
    world: &QatWorld,
    repo_name: &str,
) -> Result<()> {
    let (head_sha, _, head_tasks) = completed_tasks_for_current_head(world, repo_name)?;

    if head_tasks
        .iter()
        .any(|(_, summary)| summary.paths_added + summary.paths_unchanged > 0)
    {
        return Ok(());
    }
    bail!(
        "expected at least one completed sync task with artefacts indexed (pathsAdded + pathsUnchanged > 0) for HEAD `{head_sha}`; observed: {}",
        format_task_diagnostics(&head_tasks)
    )
}

/// Parse a numeric field from the sync summary output.
/// Format: "sync complete: 5 added, 0 changed, 0 removed, 3 unchanged, 3 cache hits (1 cache misses, 0 parse errors)"
pub fn parse_sync_summary_field(stdout: &str, field: &str) -> Option<usize> {
    for segment in stdout.split([',', '(', ')']) {
        let trimmed = segment.trim();
        if let Some(rest) = trimmed.strip_suffix(field) {
            let number_str = rest
                .trim()
                .rsplit(' ')
                .next()
                .unwrap_or("")
                .trim_end_matches(':');
            if let Ok(value) = number_str.parse::<usize>() {
                return Some(value);
            }
        }
    }
    None
}

pub fn parse_task_id_from_submission(stdout: &str) -> Option<String> {
    stdout.lines().find_map(|line| {
        let line = line.trim();
        let prefix = "task queued: task=";
        let suffix = " repo=";
        let remainder = line.strip_prefix(prefix)?;
        let task_id = remainder.split(suffix).next()?.trim();
        if task_id.is_empty() {
            None
        } else {
            Some(task_id.to_string())
        }
    })
}

pub fn parse_task_briefs(stdout: &str) -> Vec<DevqlTaskBriefRecord> {
    stdout
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let remainder = line.strip_prefix("task ")?;
            let (task_id, remainder) = remainder.split_once(": kind=")?;
            let (kind, remainder) = remainder.split_once(" status=")?;
            let (status, repo) = remainder.split_once(" repo=")?;
            Some(DevqlTaskBriefRecord {
                task_id: task_id.trim().to_string(),
                kind: kind.trim().to_string(),
                status: status.trim().to_string(),
                repo: repo.trim().to_string(),
            })
        })
        .collect()
}

pub fn parse_task_queue_status(stdout: &str) -> Result<DevqlTaskQueueStatusSnapshot> {
    let mut state = None;
    let mut queued = None;
    let mut running = None;
    let mut failed = None;
    let mut completed_recent = None;
    let mut pause_reason = None;
    let mut last_action = None;
    let mut current_repo_tasks = Vec::new();
    let mut in_current_repo_tasks = false;

    for raw_line in stdout.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line == "DevQL task queue" {
            continue;
        }
        if line == "current_repo_tasks:" {
            in_current_repo_tasks = true;
            continue;
        }
        if in_current_repo_tasks {
            let tasks = parse_task_briefs(line);
            if tasks.is_empty() {
                in_current_repo_tasks = false;
            } else {
                current_repo_tasks.extend(tasks);
                continue;
            }
        }
        if let Some(value) = line.strip_prefix("state: ") {
            state = Some(value.trim().to_string());
            continue;
        }
        if let Some(value) = line.strip_prefix("queued: ") {
            queued = Some(value.trim().parse::<usize>()?);
            continue;
        }
        if let Some(value) = line.strip_prefix("running: ") {
            running = Some(value.trim().parse::<usize>()?);
            continue;
        }
        if let Some(value) = line.strip_prefix("failed: ") {
            failed = Some(value.trim().parse::<usize>()?);
            continue;
        }
        if let Some(value) = line.strip_prefix("completed_recent: ") {
            completed_recent = Some(value.trim().parse::<usize>()?);
            continue;
        }
        if let Some(value) = line.strip_prefix("pause_reason: ") {
            pause_reason = Some(value.trim().to_string());
            continue;
        }
        if let Some(value) = line.strip_prefix("last_action: ") {
            last_action = Some(value.trim().to_string());
        }
    }

    Ok(DevqlTaskQueueStatusSnapshot {
        state: state.ok_or_else(|| anyhow!("missing task queue `state` field"))?,
        queued: queued.ok_or_else(|| anyhow!("missing task queue `queued` field"))?,
        running: running.ok_or_else(|| anyhow!("missing task queue `running` field"))?,
        failed: failed.ok_or_else(|| anyhow!("missing task queue `failed` field"))?,
        completed_recent: completed_recent
            .ok_or_else(|| anyhow!("missing task queue `completed_recent` field"))?,
        pause_reason,
        last_action,
        current_repo_tasks,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TaskIdCaptureMode {
    CaptureSubmission,
    PreserveExisting,
}

fn update_last_task_id_from_output(world: &mut QatWorld, stdout: &str, mode: TaskIdCaptureMode) {
    match mode {
        TaskIdCaptureMode::CaptureSubmission => {
            if let Some(task_id) = parse_task_id_from_submission(stdout) {
                world.last_task_id = Some(task_id);
                return;
            }
            if let Some(task) = parse_task_briefs(stdout).into_iter().next() {
                world.last_task_id = Some(task.task_id);
            }
        }
        TaskIdCaptureMode::PreserveExisting => {
            if world.last_task_id.is_none()
                && let Some(task) = parse_task_briefs(stdout).into_iter().next()
            {
                world.last_task_id = Some(task.task_id);
            }
        }
    }
}

fn write_deterministic_source_file(
    repo_dir: &Path,
    relative_path: &str,
    modified: bool,
) -> Result<()> {
    let path = repo_dir.join(relative_path);
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("source file `{relative_path}` is missing a parent directory"))?;
    fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    let body = deterministic_source_contents(relative_path, modified)?;
    fs::write(&path, body).with_context(|| format!("writing {}", path.display()))
}

fn deterministic_source_contents(relative_path: &str, modified: bool) -> Result<String> {
    let normalized = relative_path.replace('\\', "/");
    let stem = normalized
        .rsplit('/')
        .next()
        .unwrap_or(normalized.as_str())
        .trim_end_matches(".rs")
        .trim_end_matches(".ts");

    if normalized.ends_with("main.rs") {
        let label = if modified { "modified" } else { "added" };
        return Ok(format!(
            "fn main() {{\n    println!(\"{stem}-{label}\");\n}}\n"
        ));
    }
    if normalized.ends_with(".rs") {
        let function_name = stem.replace('-', "_");
        let value = if modified { "modified" } else { "added" };
        return Ok(format!(
            "pub fn {function_name}() -> &'static str {{\n    \"{function_name}-{value}\"\n}}\n"
        ));
    }
    if normalized.ends_with(".ts") {
        let function_name = stem.replace('-', "_");
        let value = if modified { "modified" } else { "added" };
        return Ok(format!(
            "export function {function_name}(): string {{\n  return \"{function_name}-{value}\";\n}}\n"
        ));
    }

    bail!("unsupported deterministic source file path `{relative_path}`")
}

pub fn commit_for_relative_day_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
    days_ago: i64,
    label: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let git_date = git_date_for_relative_day(days_ago)?;
    let env = [
        ("GIT_AUTHOR_DATE", OsString::from(git_date.clone())),
        ("GIT_COMMITTER_DATE", OsString::from(git_date)),
    ];

    run_git_success(world, &["add", "-A"], &env, "git add -A")?;

    let diff_output = run_command_capture(
        world,
        "git diff --cached --quiet",
        build_git_command(world, &["diff", "--cached", "--quiet"], &env),
    )?;

    let diff_code = diff_output.status.code().unwrap_or_default();
    ensure!(
        diff_code <= 1,
        "git diff --cached --quiet failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&diff_output.stdout),
        String::from_utf8_lossy(&diff_output.stderr)
    );

    let mut args = vec!["commit", "-m", label];
    if diff_code == 0 {
        args.insert(1, "--allow-empty");
    }
    run_git_success(world, &args, &env, "git commit relative day")?;
    if !repo_has_bitloops_git_post_commit_hook(world.repo_dir())? {
        run_bitloops_success(
            world,
            &["hooks", "git", "post-commit"],
            "bitloops hooks git post-commit",
        )?;
    }
    capture_head_sha(world)?;
    Ok(())
}

pub fn assert_bitloops_stores_exist_for_repo(world: &QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let repo_stores_dir = world.repo_dir().join(".bitloops").join("stores");
    let (stores_dir, relational, events) = if repo_stores_dir.exists() {
        (
            repo_stores_dir.clone(),
            repo_stores_dir.join("relational").join("relational.db"),
            repo_stores_dir.join("event").join("events.duckdb"),
        )
    } else {
        with_scenario_app_env(world, || {
            let cfg = resolve_store_backend_config_for_repo(world.repo_dir())
                .context("resolving store backend config for QAT store assertions")?;
            let relational = resolve_sqlite_db_path_for_repo(
                world.repo_dir(),
                cfg.relational.sqlite_path.as_deref(),
            )
            .context("resolving relational store path for QAT store assertions")?;
            let events = resolve_duckdb_db_path_for_repo(
                world.repo_dir(),
                cfg.events.duckdb_path.as_deref(),
            );
            let stores_dir = relational
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_default();
            Ok::<_, anyhow::Error>((stores_dir, relational, events))
        })?
    };
    ensure!(
        stores_dir.exists(),
        "expected stores directory to exist at {}",
        stores_dir.display()
    );
    ensure!(
        relational.exists(),
        "expected relational store at {}",
        relational.display()
    );
    ensure!(
        events.exists(),
        "expected events store at {}",
        events.display()
    );
    Ok(())
}

pub fn assert_claude_session_exists_for_repo(world: &QatWorld, repo_name: &str) -> Result<()> {
    assert_agent_session_exists_for_repo(world, repo_name, AGENT_NAME_CLAUDE_CODE)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct AgentPreCommitInteractionSnapshot {
    session_ids: Vec<String>,
    uncheckpointed_turn_ids: Vec<String>,
}

fn collect_agent_pre_commit_interactions(
    sessions: &[InteractionSession],
    turns: &[InteractionTurn],
    agent_name: &str,
) -> AgentPreCommitInteractionSnapshot {
    let normalised_agent_name = normalise_smoke_agent_name(agent_name);
    let session_ids: Vec<String> = sessions
        .iter()
        .filter(|session| session.agent_type == normalised_agent_name)
        .map(|session| session.session_id.clone())
        .collect();

    let uncheckpointed_turn_ids = turns
        .iter()
        .filter(|turn| {
            turn.checkpoint_id.is_none()
                && session_ids
                    .iter()
                    .any(|session_id| session_id == &turn.session_id)
        })
        .map(|turn| turn.turn_id.clone())
        .collect();

    AgentPreCommitInteractionSnapshot {
        session_ids,
        uncheckpointed_turn_ids,
    }
}

fn load_agent_pre_commit_interactions(
    world: &QatWorld,
    repo_name: &str,
    agent_name: &str,
) -> Result<AgentPreCommitInteractionSnapshot> {
    ensure_bitloops_repo_name(repo_name)?;
    let normalised_agent_name = normalise_smoke_agent_name(agent_name);
    let (sessions, turns) = with_scenario_app_env(world, || {
        let runtime_store = RepoSqliteRuntimeStore::open(world.repo_dir())?;
        let spool = runtime_store.interaction_spool()?;
        let sessions = spool.list_sessions(Some(normalised_agent_name), 100)?;
        let turns = spool.list_uncheckpointed_turns()?;
        Ok::<_, anyhow::Error>((sessions, turns))
    })
    .context("listing Bitloops interaction spool state before commit")?;

    Ok(collect_agent_pre_commit_interactions(
        &sessions,
        &turns,
        normalised_agent_name,
    ))
}

pub fn assert_agent_interaction_exists_before_commit_for_repo(
    world: &QatWorld,
    repo_name: &str,
    agent_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let normalised_agent_name = normalise_smoke_agent_name(agent_name);
    let snapshot = load_agent_pre_commit_interactions(world, repo_name, normalised_agent_name)?;

    ensure!(
        !snapshot.session_ids.is_empty(),
        "expected persisted {normalised_agent_name} interaction session before commit, found none"
    );
    ensure!(
        snapshot
            .session_ids
            .iter()
            .all(|session_id| !session_id.trim().is_empty()),
        "expected persisted {normalised_agent_name} interaction sessions to have non-empty ids, found {:?}",
        snapshot.session_ids
    );
    ensure!(
        !snapshot.uncheckpointed_turn_ids.is_empty(),
        "expected uncheckpointed {normalised_agent_name} interaction turns before commit for sessions {:?}, found none",
        snapshot.session_ids
    );
    ensure!(
        snapshot
            .uncheckpointed_turn_ids
            .iter()
            .all(|turn_id| !turn_id.trim().is_empty()),
        "expected persisted {normalised_agent_name} interaction turns to have non-empty ids, found {:?}",
        snapshot.uncheckpointed_turn_ids
    );

    let mappings =
        with_scenario_app_env(world, || read_commit_checkpoint_mappings(world.repo_dir()))
            .context("reading Bitloops checkpoint mappings before commit")?;
    ensure!(
        mappings.is_empty(),
        "expected no checkpoint mappings before commit, found {} with interaction sessions {:?} and turns {:?}",
        mappings.len(),
        snapshot.session_ids,
        snapshot.uncheckpointed_turn_ids
    );
    Ok(())
}

pub fn assert_agent_session_exists_for_repo(
    world: &QatWorld,
    repo_name: &str,
    agent_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let normalised_agent_name = normalise_smoke_agent_name(agent_name);
    let sessions = with_scenario_app_env(world, || {
        let backend = create_session_backend_or_local(world.repo_dir());
        backend.list_sessions()
    })
    .context("listing persisted Bitloops sessions")?;

    if let Some(session) = sessions
        .iter()
        .find(|session| session.agent_type == normalised_agent_name)
    {
        ensure!(
            !session.session_id.is_empty(),
            "expected {normalised_agent_name} session to have a session id"
        );
        ensure!(
            !session.transcript_path.is_empty(),
            "expected {normalised_agent_name} session to record a transcript path"
        );
        if !session.first_prompt.trim().is_empty() || session.pending.step_count > 0 {
            return Ok(());
        }
    }

    let expected_session_id = smoke_session_id(world, normalised_agent_name);
    let transcript_path = expected_smoke_transcript_path(world, normalised_agent_name);
    let context_paths = find_persisted_session_context_paths(world, &expected_session_id)
        .with_context(|| format!("locating persisted context for {expected_session_id}"))?;

    ensure!(
        !context_paths.is_empty(),
        "expected persisted {normalised_agent_name} session metadata for {expected_session_id}"
    );
    ensure!(
        transcript_path.exists(),
        "expected {normalised_agent_name} transcript at {}",
        transcript_path.display()
    );
    let transcript = fs::read_to_string(&transcript_path)
        .with_context(|| format!("reading {}", transcript_path.display()))?;
    ensure!(
        !transcript.trim().is_empty(),
        "expected {normalised_agent_name} transcript at {} to be non-empty",
        transcript_path.display()
    );

    let mut found_valid_context = false;
    for context_path in context_paths {
        let context = fs::read_to_string(&context_path)
            .with_context(|| format!("reading {}", context_path.display()))?;
        if context.contains(&format!("Session ID: {expected_session_id}"))
            && !context.trim().is_empty()
        {
            found_valid_context = true;
            break;
        }
    }

    ensure!(
        found_valid_context,
        "expected persisted {normalised_agent_name} context for session {expected_session_id}"
    );
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommitCheckpointRow {
    commit_sha: String,
    checkpoint_id: String,
}

fn load_commit_checkpoint_rows(
    world: &QatWorld,
    repo_name: &str,
) -> Result<Vec<CommitCheckpointRow>> {
    ensure_bitloops_repo_name(repo_name)?;
    let conn = open_relational_connection(world)?;
    let repo_id = resolve_repo_id(&conn)?;
    let mut stmt = conn
        .prepare(
            "SELECT commit_sha, checkpoint_id
             FROM commit_checkpoints
             WHERE repo_id = ?1
             ORDER BY created_at DESC, checkpoint_id DESC",
        )
        .context("preparing commit_checkpoints query for QAT assertions")?;
    let rows = stmt
        .query_map([repo_id.as_str()], |row| {
            Ok(CommitCheckpointRow {
                commit_sha: row.get::<_, String>(0)?.trim().to_string(),
                checkpoint_id: row.get::<_, String>(1)?.trim().to_string(),
            })
        })
        .with_context(|| format!("querying commit_checkpoints rows for repo `{repo_id}`"))?;

    let mut out = Vec::new();
    for row in rows {
        let row = row.context("decoding commit_checkpoints row")?;
        if row.commit_sha.is_empty()
            || !bitloops::host::checkpoints::checkpoint_id::is_valid_checkpoint_id(
                &row.checkpoint_id,
            )
        {
            continue;
        }
        out.push(row);
    }
    Ok(out)
}

fn count_commit_checkpoint_rows(rows: &[CommitCheckpointRow]) -> usize {
    rows.len()
}

fn captured_commit_shas_with_checkpoint_rows(
    captured_shas: &[String],
    rows: &[CommitCheckpointRow],
) -> Vec<String> {
    captured_shas
        .iter()
        .filter(|sha| {
            rows.iter()
                .any(|row| row.commit_sha.as_str() == sha.as_str())
        })
        .cloned()
        .collect()
}

fn checkpoint_ids_for_commit_sha(rows: &[CommitCheckpointRow], commit_sha: &str) -> Vec<String> {
    rows.iter()
        .filter(|row| row.commit_sha == commit_sha)
        .map(|row| row.checkpoint_id.clone())
        .collect()
}

pub fn assert_checkpoint_mapping_exists_for_repo(world: &QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let rows = wait_for_qat_condition(
        qat_eventual_timeout(),
        qat_eventual_poll_interval(),
        "Bitloops commit_checkpoints rows to be persisted",
        || load_commit_checkpoint_rows(world, repo_name),
        |rows| !rows.is_empty(),
        |rows| format!("rows={}", count_commit_checkpoint_rows(rows)),
    )?;
    let Some(checkpoint_id) = rows.first().map(|row| row.checkpoint_id.as_str()) else {
        bail!("expected at least one Bitloops commit_checkpoints row");
    };

    let summary = with_scenario_app_env(world, || read_committed(world.repo_dir(), checkpoint_id))
        .with_context(|| format!("reading committed checkpoint summary for {checkpoint_id}"))?;
    ensure!(
        summary.is_some(),
        "expected committed checkpoint summary for {checkpoint_id}"
    );
    Ok(())
}

pub fn assert_checkpoint_mapping_count_at_least_for_repo(
    world: &QatWorld,
    repo_name: &str,
    min_count: usize,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let rows = wait_for_qat_condition(
        qat_eventual_timeout(),
        qat_eventual_poll_interval(),
        &format!("at least {min_count} Bitloops commit_checkpoints rows"),
        || load_commit_checkpoint_rows(world, repo_name),
        |rows| count_commit_checkpoint_rows(rows) >= min_count,
        |rows| format!("rows={}", count_commit_checkpoint_rows(rows)),
    )?;
    ensure!(
        count_commit_checkpoint_rows(&rows) >= min_count,
        "expected at least {min_count} Bitloops commit_checkpoints rows, got {}",
        count_commit_checkpoint_rows(&rows)
    );
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitTimelineCommit {
    pub sha: String,
    pub subject: String,
    pub author_iso: String,
}

pub fn parse_git_timeline(stdout: &str) -> Vec<GitTimelineCommit> {
    stdout
        .lines()
        .filter_map(|line| {
            let (sha, rest) = line.split_once('|')?;
            let (subject, author_iso) = rest.split_once('|')?;
            Some(GitTimelineCommit {
                sha: sha.to_string(),
                subject: subject.to_string(),
                author_iso: author_iso.to_string(),
            })
        })
        .collect()
}

pub fn captured_commit_history_is_ordered(
    commits: &[GitTimelineCommit],
    captured_shas: &[String],
) -> bool {
    let mut previous_index = None;
    for sha in captured_shas {
        let Some(index) = commits.iter().position(|commit| commit.sha == *sha) else {
            return false;
        };
        if let Some(previous_index) = previous_index
            && index >= previous_index
        {
            return false;
        }
        previous_index = Some(index);
    }
    true
}

pub fn assert_captured_commit_history_is_ordered_for_repo(
    world: &QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    ensure!(
        !world.captured_commit_shas.is_empty(),
        "expected captured commit SHAs before asserting history order"
    );
    let rows = load_commit_checkpoint_rows(world, repo_name)?;
    let checkpointed_captured_shas =
        captured_commit_shas_with_checkpoint_rows(&world.captured_commit_shas, &rows);
    ensure!(
        checkpointed_captured_shas.len() >= 2,
        "expected at least two captured commits with persisted commit_checkpoints rows; captured={:?} checkpointed={:?}",
        world.captured_commit_shas,
        checkpointed_captured_shas
    );

    let output = run_command_capture(
        world,
        "git log ordered history",
        build_git_command(
            world,
            &["log", "--pretty=format:%H|%s|%aI", "-n", "50"],
            &[],
        ),
    )?;
    ensure_success(&output, "git log ordered history")?;
    let commits = parse_git_timeline(String::from_utf8_lossy(&output.stdout).as_ref());
    ensure!(
        captured_commit_history_is_ordered(&commits, &checkpointed_captured_shas),
        "expected checkpointed captured commits {:?} to appear in chronological order within git log {:?}; all captured commits were {:?}",
        checkpointed_captured_shas,
        commits
            .iter()
            .map(|commit| commit.sha.as_str())
            .collect::<Vec<_>>(),
        world.captured_commit_shas
    );
    Ok(())
}

fn relative_day_timeline_commits_for_repo(
    world: &QatWorld,
    repo_name: &str,
) -> Result<Vec<GitTimelineCommit>> {
    ensure_bitloops_repo_name(repo_name)?;
    let output = run_command_capture(
        world,
        "git log timeline",
        build_git_command(
            world,
            &["log", "--pretty=format:%H|%s|%aI", "-n", "30"],
            &[],
        ),
    )?;
    ensure_success(&output, "git log timeline")?;
    let commits = parse_git_timeline(String::from_utf8_lossy(&output.stdout).as_ref());
    ensure!(
        commits.len() >= 3,
        "expected at least 3 commits, got {}",
        commits.len()
    );

    let yesterday = expected_date_for_relative_day(1)?;
    let today = expected_date_for_relative_day(0)?;

    ensure!(
        commits.iter().any(|commit| {
            commit.subject == "chore: initial commit" && commit.author_iso.starts_with(&yesterday)
        }),
        "missing initial commit dated {yesterday}"
    );
    ensure!(
        commits.iter().any(|commit| {
            commit.subject == "test: committed yesterday"
                && commit.author_iso.starts_with(&yesterday)
        }),
        "missing yesterday checkpoint commit dated {yesterday}"
    );
    ensure!(
        commits.iter().any(|commit| {
            commit.subject == "test: committed today" && commit.author_iso.starts_with(&today)
        }),
        "missing today checkpoint commit dated {today}"
    );

    Ok(commits)
}

pub fn assert_relative_day_git_timeline_for_repo(world: &QatWorld, repo_name: &str) -> Result<()> {
    let _ = relative_day_timeline_commits_for_repo(world, repo_name)?;
    Ok(())
}

pub fn assert_init_yesterday_and_final_today_commit_checkpoints_for_repo(
    world: &QatWorld,
    repo_name: &str,
) -> Result<()> {
    let commits = relative_day_timeline_commits_for_repo(world, repo_name)?;
    let rows = load_commit_checkpoint_rows(world, repo_name)?;
    let yesterday = expected_date_for_relative_day(1)?;
    let today = expected_date_for_relative_day(0)?;
    let target_commits = [
        ("test: committed yesterday", yesterday.as_str()),
        ("test: committed today", today.as_str()),
    ];

    for (subject, day_prefix) in target_commits {
        let commit = commits
            .iter()
            .find(|commit| commit.subject == subject && commit.author_iso.starts_with(day_prefix))
            .ok_or_else(|| anyhow!("missing checkpointed commit `{subject}` dated {day_prefix}"))?;
        let checkpoint_ids = checkpoint_ids_for_commit_sha(&rows, &commit.sha);
        ensure!(
            !checkpoint_ids.is_empty(),
            "expected commit `{}` ({}) to have at least one commit_checkpoints row",
            commit.subject,
            commit.sha
        );
        let mut found_summary = false;
        for checkpoint_id in checkpoint_ids {
            let summary =
                with_scenario_app_env(world, || read_committed(world.repo_dir(), &checkpoint_id))
                    .with_context(|| {
                    format!("reading committed checkpoint summary for {checkpoint_id}")
                })?;
            if summary.is_some() {
                found_summary = true;
                break;
            }
        }
        ensure!(
            found_summary,
            "expected commit `{}` ({}) to resolve to a persisted checkpoint summary",
            commit.subject,
            commit.sha
        );
    }

    Ok(())
}

pub fn assert_devql_artefacts_query_returns_results(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let count = wait_for_qat_condition(
        qat_eventual_timeout(),
        qat_eventual_poll_interval(),
        "DevQL artefacts query to return results",
        || count_artefacts_across_source_files(world),
        |count| *count >= 1,
        |count| format!("artefacts={count}"),
    )?;
    world.last_query_result_count = Some(count);
    ensure!(
        count >= 1,
        "expected at least 1 artefact from devql query, got {count}"
    );
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SelectArtefactsSearchObservation {
    count: usize,
    symbols: Vec<String>,
}

fn build_select_artefacts_search_query(search: &str) -> String {
    format!(
        r#"query {{
  selectArtefacts(by: {{ search: "{}" }}) {{
    count
    artefacts {{
      path
      symbolFqn
    }}
  }}
}}"#,
        escape_devql_string(search)
    )
}

fn extract_select_artefacts_search_observation(
    value: &serde_json::Value,
) -> Result<SelectArtefactsSearchObservation> {
    let select_artefacts = value
        .get("selectArtefacts")
        .ok_or_else(|| anyhow!("expected selectArtefacts payload in GraphQL response"))?;
    let artefacts = select_artefacts
        .get("artefacts")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| anyhow!("expected selectArtefacts.artefacts array in GraphQL response"))?;
    let count = select_artefacts
        .get("count")
        .and_then(serde_json::Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(artefacts.len());
    let symbols = artefacts
        .iter()
        .filter_map(|artefact| {
            artefact
                .get("symbolFqn")
                .and_then(serde_json::Value::as_str)
                .map(ToString::to_string)
        })
        .collect::<Vec<_>>();

    Ok(SelectArtefactsSearchObservation { count, symbols })
}

fn observe_select_artefacts_search(
    world: &mut QatWorld,
    search: &str,
) -> Result<SelectArtefactsSearchObservation> {
    let query = build_select_artefacts_search_query(search);
    let value = run_devql_graphql_query(world, &query)?;
    extract_select_artefacts_search_observation(&value)
}

pub fn assert_devql_select_artefacts_search_returns_at_least(
    world: &mut QatWorld,
    repo_name: &str,
    search: &str,
    min_count: usize,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let observation = wait_for_qat_condition(
        qat_eventual_timeout(),
        qat_eventual_poll_interval(),
        &format!(
            "DevQL selectArtefacts search for `{search}` to return at least {min_count} results"
        ),
        || observe_select_artefacts_search(world, search),
        |observation| observation.count >= min_count,
        |observation| {
            format!(
                "count={}; symbols=[{}]",
                observation.count,
                observation.symbols.join(", ")
            )
        },
    )?;
    world.last_query_result_count = Some(observation.count);
    ensure!(
        observation.count >= min_count,
        "expected DevQL selectArtefacts search for `{search}` to return at least {min_count} results, got {} with symbols [{}]",
        observation.count,
        observation.symbols.join(", ")
    );
    Ok(())
}

pub fn assert_devql_select_artefacts_search_returns_symbol(
    world: &mut QatWorld,
    repo_name: &str,
    search: &str,
    expected_symbol: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let observation = wait_for_qat_condition(
        qat_eventual_timeout(),
        qat_eventual_poll_interval(),
        &format!(
            "DevQL selectArtefacts search for `{search}` to return symbol `{expected_symbol}`"
        ),
        || observe_select_artefacts_search(world, search),
        |observation| {
            observation
                .symbols
                .iter()
                .any(|symbol| symbol == expected_symbol)
        },
        |observation| {
            format!(
                "count={}; symbols=[{}]",
                observation.count,
                observation.symbols.join(", ")
            )
        },
    )?;
    world.last_query_result_count = Some(observation.count);
    ensure!(
        observation
            .symbols
            .iter()
            .any(|symbol| symbol == expected_symbol),
        "expected DevQL selectArtefacts search for `{search}` to include `{expected_symbol}`, got count={} with symbols [{}]",
        observation.count,
        observation.symbols.join(", ")
    );
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ArchitectureEntryPointObservation {
    id: String,
    path: Option<String>,
    entry_kind: Option<String>,
    label: String,
    computed: bool,
    asserted: bool,
    suppressed: bool,
    effective: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ArchitectureContainerObservation {
    id: String,
    key: Option<String>,
    kind: Option<String>,
    label: String,
    deployment_kinds: Vec<String>,
    entry_points: Vec<(String, String)>,
    component_keys: Vec<String>,
}

struct ContextGuidanceObservation {
    total_count: usize,
    kinds: Vec<String>,
}

fn build_architecture_entry_points_query(kind: &str) -> String {
    format!(
        r#"query {{
  project(path: ".") {{
    architectureEntryPoints(kind: "{}", first: 50) {{
      id
      path
      entryKind
      label
      computed
      asserted
      suppressed
      effective
    }}
  }}
}}"#,
        escape_devql_string(kind)
    )
}

fn extract_architecture_entry_points(
    value: &serde_json::Value,
) -> Result<Vec<ArchitectureEntryPointObservation>> {
    let nodes = value
        .get("project")
        .and_then(|project| project.get("architectureEntryPoints"))
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| anyhow!("expected project.architectureEntryPoints array"))?;

    nodes
        .iter()
        .map(|node| {
            Ok(ArchitectureEntryPointObservation {
                id: node
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| anyhow!("architecture entry point missing id"))?
                    .to_string(),
                path: node
                    .get("path")
                    .and_then(serde_json::Value::as_str)
                    .map(ToString::to_string),
                entry_kind: node
                    .get("entryKind")
                    .and_then(serde_json::Value::as_str)
                    .map(ToString::to_string),
                label: node
                    .get("label")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                computed: node
                    .get("computed")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
                asserted: node
                    .get("asserted")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
                suppressed: node
                    .get("suppressed")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
                effective: node
                    .get("effective")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
            })
        })
        .collect()
}

fn observe_architecture_entry_points(
    world: &mut QatWorld,
    _repo_name: &str,
    kind: &str,
) -> Result<Vec<ArchitectureEntryPointObservation>> {
    let query = build_architecture_entry_points_query(kind);
    let value = run_devql_graphql_query(world, &query)?;
    extract_architecture_entry_points(&value)
}

fn find_architecture_entry_point(
    world: &mut QatWorld,
    repo_name: &str,
    kind: &str,
    path: &str,
) -> Result<Option<ArchitectureEntryPointObservation>> {
    Ok(observe_architecture_entry_points(world, repo_name, kind)?
        .into_iter()
        .find(|entry_point| {
            entry_point.entry_kind.as_deref() == Some(kind)
                && entry_point.path.as_deref() == Some(path)
                && entry_point.effective
        }))
}

fn build_architecture_containers_query(system_key: Option<&str>) -> String {
    let system_key_arg = system_key
        .map(|system_key| format!(r#"systemKey: "{}","#, escape_devql_string(system_key)))
        .unwrap_or_default();

    format!(
        r#"query {{
  project(path: ".") {{
    architectureContainers({system_key_arg} first: 50) {{
      id
      key
      kind
      label
      deploymentUnits {{
        properties
      }}
      entryPoints {{
        path
        entryKind
      }}
      components {{
        properties
      }}
    }}
  }}
}}"#
    )
}

fn extract_architecture_containers(
    value: &serde_json::Value,
) -> Result<Vec<ArchitectureContainerObservation>> {
    let containers = value
        .get("project")
        .and_then(|project| project.get("architectureContainers"))
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| anyhow!("expected project.architectureContainers array"))?;

    containers
        .iter()
        .map(|container| {
            let deployment_kinds = container
                .get("deploymentUnits")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|deployment| {
                    deployment
                        .get("properties")
                        .and_then(|properties| properties.get("deployment_kind"))
                        .and_then(serde_json::Value::as_str)
                        .map(ToString::to_string)
                })
                .collect::<Vec<_>>();

            let entry_points = container
                .get("entryPoints")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|entry_point| {
                    Some((
                        entry_point
                            .get("entryKind")
                            .and_then(serde_json::Value::as_str)?
                            .to_string(),
                        entry_point
                            .get("path")
                            .and_then(serde_json::Value::as_str)?
                            .to_string(),
                    ))
                })
                .collect::<Vec<_>>();

            let component_keys = container
                .get("components")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|component| {
                    component
                        .get("properties")
                        .and_then(|properties| properties.get("component_key"))
                        .and_then(serde_json::Value::as_str)
                        .map(ToString::to_string)
                })
                .collect::<Vec<_>>();

            Ok(ArchitectureContainerObservation {
                id: container
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| anyhow!("architecture container missing id"))?
                    .to_string(),
                key: container
                    .get("key")
                    .and_then(serde_json::Value::as_str)
                    .map(ToString::to_string),
                kind: container
                    .get("kind")
                    .and_then(serde_json::Value::as_str)
                    .map(ToString::to_string),
                label: container
                    .get("label")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                deployment_kinds,
                entry_points,
                component_keys,
            })
        })
        .collect()
}

fn observe_architecture_containers(
    world: &mut QatWorld,
    system_key: Option<&str>,
) -> Result<Vec<ArchitectureContainerObservation>> {
    let query = build_architecture_containers_query(system_key);
    let value = run_devql_graphql_query(world, &query)?;
    extract_architecture_containers(&value)
}

fn find_architecture_container_for_entry_point(
    world: &mut QatWorld,
    entry_kind: &str,
    path: &str,
) -> Result<Option<ArchitectureContainerObservation>> {
    Ok(observe_architecture_containers(world, None)?
        .into_iter()
        .find(|container| {
            container
                .entry_points
                .iter()
                .any(|(kind, entry_path)| kind == entry_kind && entry_path == path)
        }))
}

fn build_context_guidance_query(path: &str, kind: Option<&str>) -> String {
    let kind_arg = kind
        .map(|kind| format!(r#", kind: "{}""#, escape_devql_string(kind)))
        .unwrap_or_default();

    format!(
        r#"query {{
  selectArtefacts(by: {{ path: "{}" }}) {{
    contextGuidance(category: DECISION{kind_arg}) {{
      overview
      items(first: 10) {{
        category
        kind
        guidance
        evidenceExcerpt
      }}
    }}
  }}
}}"#,
        escape_devql_string(path),
        kind_arg = kind_arg
    )
}

fn observe_context_guidance(
    world: &mut QatWorld,
    path: &str,
    kind: Option<&str>,
) -> Result<ContextGuidanceObservation> {
    let query = build_context_guidance_query(path, kind);
    let value = run_devql_graphql_query(world, &query)?;

    let guidance = value
        .get("selectArtefacts")
        .and_then(|value| value.get("contextGuidance"))
        .ok_or_else(|| {
            anyhow!("expected selectArtefacts.contextGuidance payload in GraphQL response")
        })?;

    let total_count = guidance
        .get("overview")
        .and_then(|overview| overview.get("totalCount"))
        .and_then(serde_json::Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(0);

    let kinds = match guidance.get("items").and_then(serde_json::Value::as_array) {
        Some(items) => items
            .iter()
            .filter_map(|item| item.get("kind").and_then(serde_json::Value::as_str))
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        None => Vec::new(),
    };

    Ok(ContextGuidanceObservation { total_count, kinds })
}

pub fn assert_architecture_entry_point_effective(
    world: &mut QatWorld,
    repo_name: &str,
    kind: &str,
    path: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;

    let observation = wait_for_qat_condition(
        qat_eventual_timeout(),
        qat_eventual_poll_interval(),
        &format!("architecture entry point `{kind}` at `{path}` to be effective"),
        || find_architecture_entry_point(world, repo_name, kind, path),
        Option::is_some,
        |entry_point| {
            entry_point
                .as_ref()
                .map(|entry_point| {
                    format!(
                        "id={}; label={}; computed={}; asserted={}",
                        entry_point.id,
                        entry_point.label,
                        entry_point.computed,
                        entry_point.asserted
                    )
                })
                .unwrap_or_else(|| "not found".to_string())
        },
    )?;

    ensure!(
        observation.is_some(),
        "expected architecture entry point `{kind}` at `{path}` to be effective"
    );

    Ok(())
}

pub fn assert_architecture_container_exposes_entry_point(
    world: &mut QatWorld,
    repo_name: &str,
    container_kind: &str,
    entry_kind: &str,
    path: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;

    let container = wait_for_qat_condition(
        qat_eventual_timeout(),
        qat_eventual_poll_interval(),
        &format!("architecture container `{container_kind}` exposing `{entry_kind}` at `{path}`"),
        || find_architecture_container_for_entry_point(world, entry_kind, path),
        |container| {
            container.as_ref().is_some_and(|container| {
                container.kind.as_deref() == Some(container_kind)
                    && !container.deployment_kinds.is_empty()
                    && !container.component_keys.is_empty()
            })
        },
        |container| {
            container
                .as_ref()
                .map(|container| {
                    format!(
                        "id={}; key={:?}; kind={:?}; label={}; deployment_kinds={:?}; component_keys={:?}",
                        container.id,
                        container.key,
                        container.kind,
                        container.label,
                        container.deployment_kinds,
                        container.component_keys
                    )
                })
                .unwrap_or_else(|| "not found".to_string())
        },
    )?;

    ensure!(
        container
            .as_ref()
            .is_some_and(|container| container.kind.as_deref() == Some(container_kind)),
        "expected architecture container `{container_kind}` exposing `{entry_kind}` at `{path}`"
    );

    Ok(())
}

pub fn assert_architecture_system_membership_for_entry_point(
    world: &mut QatWorld,
    repo_name: &str,
    system_key: &str,
    entry_kind: &str,
    path: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;

    let container = find_architecture_container_for_entry_point(world, entry_kind, path)?
        .ok_or_else(|| anyhow!("container for `{entry_kind}` at `{path}` was not available"))?;

    let mutation = format!(
        r#"mutation {{
  assertArchitectureSystemMembership(input: {{
    systemKey: "{}",
    systemLabel: "QAT Shared System",
    containerId: "{}",
    reason: "QAT shared architecture system membership",
    source: "qat",
    confidence: 0.92
  }}) {{
    success
    systemId
    containerId
    assertionIds
  }}
}}"#,
        escape_devql_string(system_key),
        escape_devql_string(&container.id)
    );

    let value = run_devql_graphql_query(world, &mutation)?;

    ensure!(
        value
            .get("assertArchitectureSystemMembership")
            .and_then(|result| result.get("success"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        "expected assertArchitectureSystemMembership to succeed"
    );

    let query = format!(
        r#"query {{
  project(path: ".") {{
    architectureContainers(systemKey: "{}", first: 50) {{
      id
      entryPoints {{
        path
        entryKind
      }}
    }}
  }}
}}"#,
        escape_devql_string(system_key)
    );

    let observed = wait_for_qat_condition(
        qat_eventual_timeout(),
        qat_eventual_poll_interval(),
        &format!("architecture system `{system_key}` to include container `{}`", container.id),
        || run_devql_graphql_query(world, &query),
        |value| {
            value
                .get("project")
                .and_then(|project| project.get("architectureContainers"))
                .and_then(serde_json::Value::as_array)
                .is_some_and(|containers| {
                    containers.iter().any(|observed| {
                        observed.get("id").and_then(serde_json::Value::as_str)
                            == Some(container.id.as_str())
                            && observed
                                .get("entryPoints")
                                .and_then(serde_json::Value::as_array)
                                .is_some_and(|entry_points| {
                                    entry_points.iter().any(|entry_point| {
                                        entry_point
                                            .get("entryKind")
                                            .and_then(serde_json::Value::as_str)
                                            == Some(entry_kind)
                                            && entry_point
                                                .get("path")
                                                .and_then(serde_json::Value::as_str)
                                                == Some(path)
                                    })
                                })
                    })
                })
        },
        |value| value.to_string(),
    )?;

    ensure!(
        observed
            .get("project")
            .and_then(|project| project.get("architectureContainers"))
            .is_some(),
        "expected architecture system `{system_key}` membership to be queryable"
    );

    Ok(())
}

pub fn assert_architecture_suppression_revoke_roundtrip(
    world: &mut QatWorld,
    repo_name: &str,
    kind: &str,
    path: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;

    let entry_point = wait_for_qat_condition(
        qat_eventual_timeout(),
        qat_eventual_poll_interval(),
        &format!("architecture entry point `{kind}` at `{path}` before suppression"),
        || find_architecture_entry_point(world, repo_name, kind, path),
        Option::is_some,
        |entry_point| {
            entry_point
                .as_ref()
                .map(|entry_point| entry_point.id.clone())
                .unwrap_or_else(|| "not found".to_string())
        },
    )?
    .ok_or_else(|| anyhow!("entry point `{kind}` at `{path}` was not available"))?;

    let suppress_query = format!(
        r#"mutation {{
  assertArchitectureGraphFact(input: {{
    action: SUPPRESS,
    targetKind: NODE,
    node: {{ id: "{}", kind: ENTRY_POINT }},
    reason: "QAT suppression round-trip",
    source: "qat"
  }}) {{
    success
    assertionId
  }}
}}"#,
        escape_devql_string(&entry_point.id)
    );

    let suppress_value = run_devql_graphql_query(world, &suppress_query)?;

    let assertion_id = suppress_value
        .get("assertArchitectureGraphFact")
        .and_then(|result| result.get("assertionId"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow!("expected architecture suppression assertion id"))?
        .to_string();

    let hidden = wait_for_qat_condition(
        qat_eventual_timeout(),
        qat_eventual_poll_interval(),
        &format!("architecture entry point `{kind}` at `{path}` to be suppressed"),
        || find_architecture_entry_point(world, repo_name, kind, path),
        Option::is_none,
        |entry_point| {
            entry_point
                .as_ref()
                .map(|entry_point| format!("still effective as {}", entry_point.id))
                .unwrap_or_else(|| "hidden".to_string())
        },
    )?;

    ensure!(
        hidden.is_none(),
        "expected architecture entry point `{kind}` at `{path}` to be hidden after suppression"
    );

    let revoke_query = format!(
        r#"mutation {{
  revokeArchitectureGraphAssertion(id: "{}") {{
    success
    revoked
    id
  }}
}}"#,
        escape_devql_string(&assertion_id)
    );

    let revoke_value = run_devql_graphql_query(world, &revoke_query)?;

    let revoked = revoke_value
        .get("revokeArchitectureGraphAssertion")
        .and_then(|result| result.get("revoked"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);

    ensure!(
        revoked,
        "expected revokeArchitectureGraphAssertion to revoke `{assertion_id}`"
    );

    assert_architecture_entry_point_effective(world, repo_name, kind, path)
}

pub fn assert_architecture_manual_entry_point(
    world: &mut QatWorld,
    repo_name: &str,
    kind: &str,
    path: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;

    let assert_query = format!(
        r#"mutation {{
  assertArchitectureGraphFact(input: {{
    action: ASSERT,
    targetKind: NODE,
    node: {{
      kind: ENTRY_POINT,
      label: "QAT manual entry point",
      path: "{}",
      entryKind: "{}"
    }},
    reason: "QAT manual architecture graph assertion",
    source: "qat",
    confidence: 0.91
  }}) {{
    success
    assertionId
  }}
}}"#,
        escape_devql_string(path),
        escape_devql_string(kind)
    );

    let value = run_devql_graphql_query(world, &assert_query)?;

    ensure!(
        value
            .get("assertArchitectureGraphFact")
            .and_then(|result| result.get("success"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        "expected assertArchitectureGraphFact to succeed"
    );

    let entry_point = wait_for_qat_condition(
        qat_eventual_timeout(),
        qat_eventual_poll_interval(),
        &format!("manual architecture entry point `{kind}` at `{path}` to be effective"),
        || find_architecture_entry_point(world, repo_name, kind, path),
        |entry_point| {
            entry_point
                .as_ref()
                .is_some_and(|entry_point| entry_point.asserted && !entry_point.computed)
        },
        |entry_point| {
            entry_point
                .as_ref()
                .map(|entry_point| {
                    format!(
                        "id={}; computed={}; asserted={}; suppressed={}",
                        entry_point.id,
                        entry_point.computed,
                        entry_point.asserted,
                        entry_point.suppressed
                    )
                })
                .unwrap_or_else(|| "not found".to_string())
        },
    )?;

    ensure!(
        entry_point
            .as_ref()
            .is_some_and(|entry_point| entry_point.asserted && !entry_point.computed),
        "expected manual architecture entry point `{kind}` at `{path}` to be asserted-only"
    );

    Ok(())
}

pub fn assert_devql_context_guidance_returns_at_least(
    world: &mut QatWorld,
    repo_name: &str,
    path: &str,
    min_count: usize,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;

    let observation = wait_for_qat_condition(
        qat_eventual_timeout(),
        qat_eventual_poll_interval(),
        &format!("DevQL context guidance for `{path}` to return at least {min_count} items"),
        || observe_context_guidance(world, path, None),
        |observation| observation.total_count >= min_count,
        |observation| {
            format!(
                "total_count={}, kinds=[{}]",
                observation.total_count,
                observation.kinds.join(", ")
            )
        },
    )?;

    ensure!(
        observation.total_count >= min_count,
        "expected DevQL context guidance for `{path}` to return at least {min_count} items, got {} with kinds [{}]",
        observation.total_count,
        observation.kinds.join(", ")
    );

    Ok(())
}

pub fn assert_devql_context_guidance_includes_kind(
    world: &mut QatWorld,
    repo_name: &str,
    path: &str,
    expected_kind: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;

    let observation = wait_for_qat_condition(
        qat_eventual_timeout(),
        qat_eventual_poll_interval(),
        &format!("DevQL context guidance for `{path}` to include kind `{expected_kind}`"),
        || observe_context_guidance(world, path, Some(expected_kind)),
        |observation| observation.kinds.iter().any(|kind| kind == expected_kind),
        |observation| {
            format!(
                "total_count={}, kinds=[{}]",
                observation.total_count,
                observation.kinds.join(", ")
            )
        },
    )?;

    ensure!(
        observation.kinds.iter().any(|kind| kind == expected_kind),
        "expected DevQL context guidance for `{path}` to include kind `{expected_kind}`, got total_count={} with kinds [{}]",
        observation.total_count,
        observation.kinds.join(", ")
    );

    Ok(())
}

fn checkpoint_agent_candidates(agent: &str) -> Vec<String> {
    let mut candidates = vec![agent.to_string()];
    if agent == "claude" {
        candidates.push("claude-code".to_string());
    } else if agent == "claude-code" {
        candidates.push("claude".to_string());
    }
    candidates
}

fn build_chat_history_query(path: &str) -> String {
    format!(
        r#"repo("bitloops")->file("{}")->artefacts()->chatHistory()->limit(10)"#,
        escape_devql_string(path)
    )
}

fn count_chat_history_edges_for_agent(value: &serde_json::Value, agent_name: &str) -> usize {
    let candidates = checkpoint_agent_candidates(agent_name);
    value.as_array().map_or(0, |rows| {
        rows.iter()
            .flat_map(|row| {
                row.get("chatHistory")
                    .and_then(|chat_history| chat_history.get("edges"))
                    .and_then(serde_json::Value::as_array)
                    .into_iter()
                    .flatten()
            })
            .filter(|edge| {
                edge.get("node")
                    .and_then(|node| node.get("agent"))
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|candidate| candidates.iter().any(|value| value == candidate))
            })
            .count()
    })
}

pub fn assert_devql_checkpoints_query_returns_results(
    world: &mut QatWorld,
    repo_name: &str,
    agent: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let mut max_count = 0_usize;
    for candidate in checkpoint_agent_candidates(agent) {
        let query = format!(
            r#"repo("bitloops")->checkpoints(agent:"{}")->limit(5)"#,
            escape_devql_string(&candidate)
        );
        let value = run_devql_query(world, &query)?;
        let count = count_json_array_rows(&value);
        max_count = max_count.max(count);
        if count >= 1 {
            world.last_query_result_count = Some(count);
            return Ok(());
        }
    }

    world.last_query_result_count = Some(max_count);
    ensure!(
        max_count >= 1,
        "expected at least 1 checkpoint for agent {agent}, got {max_count}"
    );
    Ok(())
}

pub fn assert_devql_chat_history_returns_results(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let agent_name = world
        .agent_name
        .as_deref()
        .ok_or_else(|| anyhow!("no agent name captured for chat history assertion"))?;
    let agent_name = agent_name.to_string();
    let candidate_paths = chat_history_candidate_paths(world)?;
    let mut best_path = String::new();
    let mut best_count = 0_usize;

    for target_path in candidate_paths {
        let query = build_chat_history_query(&target_path);
        let value = run_devql_query(world, &query)?;
        let count = count_chat_history_edges_for_agent(&value, &agent_name);
        if count > best_count {
            best_count = count;
            best_path = target_path.clone();
        }
        if count >= 1 {
            world.last_query_result_count = Some(count);
            return Ok(());
        }
    }

    world.last_query_result_count = Some(best_count);
    ensure!(
        best_count >= 1,
        "expected at least 1 chat history result for agent `{agent_name}` across queryable touched paths, best path `{best_path}` produced {best_count}"
    );
    Ok(())
}

// ── DevQL ingest DB-first assertions and git topology helpers ───────────────

/// Parse a numeric field from the ingest summary output.
/// Format: "DevQL ingest complete: commits_processed=1, ...".
pub fn parse_ingest_summary_field(stdout: &str, field: &str) -> Option<usize> {
    let needle = format!("{field}=");
    let suffix = stdout.split(&needle).nth(1)?;
    let raw = suffix
        .split([',', '\n', ' '])
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    raw.parse::<usize>().ok()
}

fn relational_db_path_for_world(world: &QatWorld) -> Result<std::path::PathBuf> {
    with_scenario_app_env(world, || {
        let cfg = resolve_store_backend_config_for_repo(world.repo_dir())
            .context("resolving store backend config for ingest DB assertions")?;
        resolve_sqlite_db_path_for_repo(world.repo_dir(), cfg.relational.sqlite_path.as_deref())
            .context("resolving relational store path for ingest DB assertions")
    })
}

fn open_relational_connection(world: &QatWorld) -> Result<rusqlite::Connection> {
    let relational_db_path = relational_db_path_for_world(world)?;
    rusqlite::Connection::open(&relational_db_path).with_context(|| {
        format!(
            "opening relational store for ingest assertions at {}",
            relational_db_path.display()
        )
    })
}

fn query_repo_id_optional(conn: &rusqlite::Connection, sql: &str) -> Result<Option<String>> {
    use rusqlite::OptionalExtension;
    match conn
        .query_row(sql, [], |row| row.get::<_, String>(0))
        .optional()
    {
        Ok(value) => Ok(value),
        Err(err) => {
            let message = err.to_string();
            if message.contains("no such table") || message.contains("no such column") {
                return Ok(None);
            }
            Err(err).with_context(|| format!("querying repo id with `{sql}`"))
        }
    }
}

fn resolve_repo_id(conn: &rusqlite::Connection) -> Result<String> {
    for sql in [
        "SELECT repo_id FROM commit_ingest_ledger ORDER BY updated_at DESC LIMIT 1",
        "SELECT repo_id FROM commits ORDER BY committed_at DESC LIMIT 1",
        "SELECT repo_id FROM artefacts_historical LIMIT 1",
        "SELECT repo_id FROM symbol_features LIMIT 1",
        "SELECT repo_id FROM symbol_semantics LIMIT 1",
        "SELECT repo_id FROM symbol_embeddings LIMIT 1",
        "SELECT repo_id FROM artefacts_current LIMIT 1",
        "SELECT repo_id FROM current_file_state LIMIT 1",
        "SELECT repo_id FROM repositories WHERE provider = 'local' ORDER BY created_at DESC LIMIT 1",
        "SELECT repo_id FROM repositories ORDER BY created_at DESC LIMIT 1",
    ] {
        if let Some(repo_id) = query_repo_id_optional(conn, sql)? {
            return Ok(repo_id);
        }
    }
    bail!("unable to resolve repo_id from relational store for ingest assertions")
}

fn checkpoint_touched_paths_for_repo(
    conn: &rusqlite::Connection,
    repo_id: &str,
) -> Result<Vec<String>> {
    let sql = "SELECT COALESCE(path_after, path_before) \
               FROM checkpoint_files \
               WHERE repo_id = ?1 \
               ORDER BY event_time DESC, checkpoint_id DESC, relation_id DESC \
               LIMIT 20";
    let mut stmt = match conn.prepare(sql) {
        Ok(stmt) => stmt,
        Err(err) => {
            let message = err.to_string();
            if message.contains("no such table") || message.contains("no such column") {
                return Ok(Vec::new());
            }
            return Err(err).with_context(|| format!("preparing `{sql}`"));
        }
    };
    let rows = stmt
        .query_map([repo_id], |row| row.get::<_, Option<String>>(0))
        .with_context(|| format!("querying checkpoint touched paths for repo `{repo_id}`"))?;
    let mut paths = Vec::new();
    for row in rows {
        let Some(path) = row.context("reading checkpoint touched path")? else {
            continue;
        };
        let path = path.trim();
        if path.is_empty() || paths.iter().any(|existing| existing == path) {
            continue;
        }
        paths.push(path.to_string());
    }
    Ok(paths)
}

fn chat_history_candidate_paths(world: &QatWorld) -> Result<Vec<String>> {
    let mut candidates = vec![smoke_target_relative_path(world)];
    let conn = open_relational_connection(world)?;
    let repo_id = resolve_repo_id(&conn)?;
    for path in checkpoint_touched_paths_for_repo(&conn, &repo_id)? {
        if !candidates.iter().any(|existing| existing == &path) {
            candidates.push(path);
        }
    }
    Ok(candidates)
}

fn git_reachable_shas(world: &QatWorld, max_count: Option<usize>) -> Result<Vec<String>> {
    let mut args_owned = vec!["rev-list".to_string()];
    if let Some(limit) = max_count {
        args_owned.push(format!("--max-count={limit}"));
    }
    args_owned.push("HEAD".to_string());
    let args: Vec<&str> = args_owned.iter().map(String::as_str).collect();
    let output = run_command_capture(
        world,
        "git rev-list HEAD",
        build_git_command(world, &args, &[]),
    )?;
    ensure_success(&output, "git rev-list HEAD")?;
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect())
}

fn completed_ledger_shas(world: &QatWorld) -> Result<Vec<String>> {
    let conn = open_relational_connection(world)?;
    let repo_id = resolve_repo_id(&conn)?;
    let mut stmt = conn
        .prepare(
            "SELECT commit_sha \
             FROM commit_ingest_ledger \
             WHERE repo_id = ?1 AND history_status = 'completed'",
        )
        .context("preparing completed ledger SHA query")?;
    let rows = stmt
        .query_map(rusqlite::params![repo_id], |row| row.get::<_, String>(0))
        .context("querying completed ledger SHAs")?;
    let mut shas = Vec::new();
    for row in rows {
        shas.push(row.context("reading completed ledger SHA row")?);
    }
    Ok(shas)
}

fn completed_ledger_count(world: &QatWorld) -> Result<usize> {
    let conn = open_relational_connection(world)?;
    let repo_id = resolve_repo_id(&conn)?;
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) \
             FROM commit_ingest_ledger \
             WHERE repo_id = ?1 AND history_status = 'completed'",
            rusqlite::params![repo_id],
            |row| row.get(0),
        )
        .context("counting completed commit_ingest_ledger rows")?;
    usize::try_from(count).context("converting completed ledger count to usize")
}

fn artefacts_current_count(world: &QatWorld) -> Result<usize> {
    let conn = open_relational_connection(world)?;
    let repo_id = resolve_repo_id(&conn)?;
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1",
            rusqlite::params![repo_id],
            |row| row.get(0),
        )
        .context("counting artefacts_current rows")?;
    usize::try_from(count).context("converting artefacts_current row count to usize")
}

fn artefacts_current_count_for_path(world: &QatWorld, path: &str) -> Result<usize> {
    let conn = open_relational_connection(world)?;
    let repo_id = resolve_repo_id(&conn)?;
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM artefacts_current WHERE repo_id = ?1 AND path = ?2",
            rusqlite::params![repo_id, path],
            |row| row.get(0),
        )
        .with_context(|| format!("counting artefacts_current rows for path `{path}`"))?;
    usize::try_from(count).context("converting artefacts_current path count to usize")
}

fn current_file_state_effective_content_id(world: &QatWorld, path: &str) -> Result<Option<String>> {
    let conn = open_relational_connection(world)?;
    let repo_id = resolve_repo_id(&conn)?;
    use rusqlite::OptionalExtension;
    conn.query_row(
        "SELECT effective_content_id \
         FROM current_file_state \
         WHERE repo_id = ?1 AND path = ?2",
        rusqlite::params![repo_id, path],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .with_context(|| format!("loading current_file_state effective_content_id for `{path}`"))
}

fn file_state_count_for_commit(world: &QatWorld, commit_sha: &str) -> Result<usize> {
    let conn = open_relational_connection(world)?;
    let repo_id = resolve_repo_id(&conn)?;
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM file_state WHERE repo_id = ?1 AND commit_sha = ?2",
            rusqlite::params![repo_id, commit_sha],
            |row| row.get(0),
        )
        .with_context(|| format!("counting file_state rows for commit `{commit_sha}`"))?;
    usize::try_from(count).context("converting file_state count to usize")
}

fn file_state_count_for_commit_path(
    world: &QatWorld,
    commit_sha: &str,
    path: &str,
) -> Result<usize> {
    let conn = open_relational_connection(world)?;
    let repo_id = resolve_repo_id(&conn)?;
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM file_state WHERE repo_id = ?1 AND commit_sha = ?2 AND path = ?3",
            rusqlite::params![repo_id, commit_sha, path],
            |row| row.get(0),
        )
        .with_context(|| {
            format!("counting file_state rows for commit `{commit_sha}` path `{path}`")
        })?;
    usize::try_from(count).context("converting file_state path count to usize")
}

fn commit_has_changed_files(world: &QatWorld, commit_sha: &str) -> Result<bool> {
    let has_parent = run_command_capture(
        world,
        "git rev-parse <sha>^",
        build_git_command(world, &["rev-parse", &format!("{commit_sha}^")], &[]),
    )?
    .status
    .success();

    let command_label = if has_parent {
        "git diff-tree --no-commit-id --name-only -r <sha>"
    } else {
        "git show --name-only --pretty=format: <sha>"
    };

    let command_args: Vec<String> = if has_parent {
        vec![
            "diff-tree".to_string(),
            "--no-commit-id".to_string(),
            "--name-only".to_string(),
            "-r".to_string(),
            commit_sha.to_string(),
        ]
    } else {
        vec![
            "show".to_string(),
            "--name-only".to_string(),
            "--pretty=format:".to_string(),
            commit_sha.to_string(),
        ]
    };
    let command_args_ref: Vec<&str> = command_args.iter().map(String::as_str).collect();
    let output = run_command_capture(
        world,
        command_label,
        build_git_command(world, &command_args_ref, &[]),
    )?;
    ensure_success(&output, command_label)?;

    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .any(|line| !line.is_empty()))
}

fn current_branch_name(world: &QatWorld) -> Result<String> {
    let output = run_command_capture(
        world,
        "git rev-parse --abbrev-ref HEAD",
        build_git_command(world, &["rev-parse", "--abbrev-ref", "HEAD"], &[]),
    )?;
    ensure_success(&output, "git rev-parse --abbrev-ref HEAD")?;
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    ensure!(!branch.is_empty(), "current git branch name is empty");
    Ok(branch)
}

fn post_commit_devql_refresh_disabled_env() -> [(&'static str, OsString); 1] {
    [(
        "BITLOOPS_DISABLE_POST_COMMIT_DEVQL_REFRESH",
        OsString::from("1"),
    )]
}

fn expected_commit_path_pairs(
    expected_commit_shas: &[String],
    expected_paths: &[String],
) -> Result<Vec<(String, String)>> {
    ensure!(
        !expected_commit_shas.is_empty(),
        "no expected commit SHAs captured for expected path pairing"
    );
    ensure!(
        !expected_paths.is_empty(),
        "no expected paths captured for expected path pairing"
    );
    ensure!(
        expected_paths.len() <= expected_commit_shas.len(),
        "expected path count {} exceeds expected SHA count {}",
        expected_paths.len(),
        expected_commit_shas.len()
    );
    Ok(expected_commit_shas
        .iter()
        .take(expected_paths.len())
        .cloned()
        .zip(expected_paths.iter().cloned())
        .collect())
}

fn build_commit_without_hooks_command(world: &QatWorld, allow_empty: bool) -> Command {
    let disable_refresh_env = post_commit_devql_refresh_disabled_env();
    let mut args = vec!["commit", "-m", "QAT change (no hooks)"];
    if allow_empty {
        args.insert(1, "--allow-empty");
    }
    build_git_command(world, &args, &disable_refresh_env)
}

fn build_init_commit_without_post_commit_refresh_command(world: &QatWorld) -> Command {
    let disable_refresh_env = post_commit_devql_refresh_disabled_env();
    build_git_command(
        world,
        &["commit", "-m", "chore: initial commit"],
        &disable_refresh_env,
    )
}

fn write_and_commit_rust_file(
    world: &mut QatWorld,
    relative_path: &str,
    function_name: &str,
    body_value: usize,
    env: &[(&str, OsString)],
    commit_message: &str,
) -> Result<String> {
    let path = world.repo_dir().join(relative_path);
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("path `{relative_path}` has no parent directory"))?;
    fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    fs::write(
        &path,
        format!("pub fn {function_name}() -> usize {{\n    {body_value}\n}}\n"),
    )
    .with_context(|| format!("writing {}", path.display()))?;
    run_git_success(world, &["add", "-A"], env, "git add -A")?;
    run_git_success(
        world,
        &["commit", "-m", commit_message],
        env,
        "git commit ingest topology",
    )?;
    capture_head_sha(world)
}

fn set_expected_commits_and_paths(world: &mut QatWorld, shas: Vec<String>, paths: Vec<String>) {
    world.expected_commit_shas = shas;
    world.expected_paths = paths;
}

fn refresh_rewrite_delta(world: &mut QatWorld, expected_segment_len: usize) -> Result<()> {
    let post = git_reachable_shas(world, Some(expected_segment_len))?;
    world.post_rewrite_shas = post.clone();
    let pre: std::collections::BTreeSet<String> = world.pre_rewrite_shas.iter().cloned().collect();
    world.rewrite_new_shas = post.into_iter().filter(|sha| !pre.contains(sha)).collect();
    Ok(())
}

pub fn snapshot_ingest_db_state_for_repo(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let shas = completed_ledger_shas(world)?;
    world.completed_ledger_count_snapshot = Some(shas.len());
    world.completed_ledger_shas_snapshot = shas;
    world.artefacts_current_count_snapshot = Some(artefacts_current_count(world)?);
    Ok(())
}

pub fn create_ingest_commits_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
    count: usize,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    ensure!(count > 0, "commit batch size must be greater than zero");
    let disable_refresh_env = post_commit_devql_refresh_disabled_env();
    let mut shas = Vec::with_capacity(count);
    let mut paths = Vec::with_capacity(count);
    for index in 0..count {
        let seq = world.captured_commit_shas.len() + 1;
        let relative_path = format!("src/ingest_batch_{seq}.rs");
        let function_name = format!("ingest_batch_{seq}");
        let commit_message = format!("feat: ingest batch commit {}", index + 1);
        let sha = write_and_commit_rust_file(
            world,
            &relative_path,
            &function_name,
            seq,
            &disable_refresh_env,
            &commit_message,
        )?;
        shas.push(sha);
        paths.push(relative_path);
    }
    set_expected_commits_and_paths(world, shas, paths);
    Ok(())
}

pub fn create_non_ff_merge_with_two_feature_commits_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let disable_refresh_env = post_commit_devql_refresh_disabled_env();
    let base_branch = current_branch_name(world)?;
    let feature_branch = "qat-non-ff-feature";
    run_git_success(
        world,
        &["checkout", "-b", feature_branch],
        &[],
        "git checkout -b non-ff feature branch",
    )?;

    let first_path = "src/non_ff_feature_one.rs".to_string();
    let first_sha = write_and_commit_rust_file(
        world,
        &first_path,
        "non_ff_feature_one",
        1,
        &disable_refresh_env,
        "feat: non-ff feature commit 1",
    )?;
    let second_path = "src/non_ff_feature_two.rs".to_string();
    let second_sha = write_and_commit_rust_file(
        world,
        &second_path,
        "non_ff_feature_two",
        2,
        &disable_refresh_env,
        "feat: non-ff feature commit 2",
    )?;

    run_git_success(
        world,
        &["checkout", base_branch.as_str()],
        &[],
        "git checkout base branch for non-ff merge",
    )?;
    run_git_success(
        world,
        &[
            "merge",
            "--no-ff",
            feature_branch,
            "-m",
            "merge: non-ff feature branch",
        ],
        &disable_refresh_env,
        "git merge --no-ff",
    )?;
    let merge_sha = capture_head_sha(world)?;
    set_expected_commits_and_paths(
        world,
        vec![first_sha, second_sha, merge_sha],
        vec![first_path, second_path],
    );
    Ok(())
}

pub fn create_ff_merge_with_two_feature_commits_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let disable_refresh_env = post_commit_devql_refresh_disabled_env();
    let base_branch = current_branch_name(world)?;
    let feature_branch = "qat-ff-feature";
    run_git_success(
        world,
        &["checkout", "-b", feature_branch],
        &[],
        "git checkout -b ff feature branch",
    )?;

    let first_path = "src/ff_feature_one.rs".to_string();
    let first_sha = write_and_commit_rust_file(
        world,
        &first_path,
        "ff_feature_one",
        11,
        &disable_refresh_env,
        "feat: ff feature commit 1",
    )?;
    let second_path = "src/ff_feature_two.rs".to_string();
    let second_sha = write_and_commit_rust_file(
        world,
        &second_path,
        "ff_feature_two",
        22,
        &disable_refresh_env,
        "feat: ff feature commit 2",
    )?;

    run_git_success(
        world,
        &["checkout", base_branch.as_str()],
        &[],
        "git checkout base branch for ff merge",
    )?;
    run_git_success(
        world,
        &["merge", "--ff-only", feature_branch],
        &disable_refresh_env,
        "git merge --ff-only",
    )?;
    let _ = capture_head_sha(world)?;
    set_expected_commits_and_paths(
        world,
        vec![first_sha, second_sha],
        vec![first_path, second_path],
    );
    Ok(())
}

pub fn cherry_pick_two_commits_for_repo(world: &mut QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let disable_refresh_env = post_commit_devql_refresh_disabled_env();
    let base_branch = current_branch_name(world)?;
    let source_branch = "qat-cherry-source";
    run_git_success(
        world,
        &["checkout", "-b", source_branch],
        &[],
        "git checkout -b cherry-pick source",
    )?;

    let first_path = "src/cherry_source_one.rs".to_string();
    let source_sha_one = write_and_commit_rust_file(
        world,
        &first_path,
        "cherry_source_one",
        101,
        &disable_refresh_env,
        "feat: cherry source commit 1",
    )?;
    let second_path = "src/cherry_source_two.rs".to_string();
    let source_sha_two = write_and_commit_rust_file(
        world,
        &second_path,
        "cherry_source_two",
        202,
        &disable_refresh_env,
        "feat: cherry source commit 2",
    )?;

    run_git_success(
        world,
        &["checkout", base_branch.as_str()],
        &[],
        "git checkout base branch for cherry-pick",
    )?;
    run_git_success(
        world,
        &["cherry-pick", source_sha_one.as_str()],
        &disable_refresh_env,
        "git cherry-pick source commit 1",
    )?;
    let cherry_sha_one = capture_head_sha(world)?;
    run_git_success(
        world,
        &["cherry-pick", source_sha_two.as_str()],
        &disable_refresh_env,
        "git cherry-pick source commit 2",
    )?;
    let cherry_sha_two = capture_head_sha(world)?;
    run_git_success(
        world,
        &["branch", "-D", source_branch],
        &[],
        "git branch -D cherry-pick source",
    )?;

    set_expected_commits_and_paths(
        world,
        vec![cherry_sha_one, cherry_sha_two],
        vec![first_path, second_path],
    );
    Ok(())
}

pub fn capture_top_reachable_shas_before_rewrite_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
    count: usize,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    ensure!(count > 0, "rewrite capture count must be greater than zero");
    let pre = git_reachable_shas(world, Some(count))?;
    ensure!(
        pre.len() == count,
        "expected to capture {count} pre-rewrite SHAs, got {}",
        pre.len()
    );
    world.pre_rewrite_shas = pre;
    world.post_rewrite_shas.clear();
    world.rewrite_new_shas.clear();
    Ok(())
}

pub fn rewrite_last_commits_with_rebase_edit_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
    count: usize,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    ensure!(count > 0, "rebase rewrite count must be greater than zero");
    ensure!(
        world.pre_rewrite_shas.len() == count,
        "pre-rewrite SHAs must be captured for exactly {count} commits before rebase rewrite"
    );
    let disable_refresh_env = post_commit_devql_refresh_disabled_env();
    let script = "echo '// qat rebase edit marker' >> src/main.rs && git add src/main.rs && git commit --amend --no-edit";
    let upstream = format!("HEAD~{count}");
    run_git_success(
        world,
        &["rebase", "-x", script, upstream.as_str()],
        &disable_refresh_env,
        "git rebase -x amend",
    )?;
    let _ = capture_head_sha(world)?;
    refresh_rewrite_delta(world, count)?;
    world.expected_commit_shas = world.rewrite_new_shas.clone();
    Ok(())
}

pub fn reset_and_rewrite_last_commits_for_repo(
    world: &mut QatWorld,
    repo_name: &str,
    count: usize,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    ensure!(count > 0, "reset rewrite count must be greater than zero");
    ensure!(
        world.pre_rewrite_shas.len() == count,
        "pre-rewrite SHAs must be captured for exactly {count} commits before reset rewrite"
    );
    let disable_refresh_env = post_commit_devql_refresh_disabled_env();
    let target = format!("HEAD~{count}");
    run_git_success(
        world,
        &["reset", "--hard", target.as_str()],
        &[],
        "git reset --hard for rewrite",
    )?;

    let mut replacement_shas = Vec::with_capacity(count);
    let mut replacement_paths = Vec::with_capacity(count);
    for index in 0..count {
        let seq = world.captured_commit_shas.len() + 1;
        let relative_path = format!("src/reset_rewrite_{seq}.rs");
        let function_name = format!("reset_rewrite_{seq}");
        let sha = write_and_commit_rust_file(
            world,
            &relative_path,
            &function_name,
            500 + index,
            &disable_refresh_env,
            &format!("feat: reset rewrite replacement {}", index + 1),
        )?;
        replacement_shas.push(sha);
        replacement_paths.push(relative_path);
    }
    set_expected_commits_and_paths(world, replacement_shas, replacement_paths);
    refresh_rewrite_delta(world, count)?;
    Ok(())
}

pub fn assert_all_reachable_shas_completed_in_ledger(
    world: &QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let reachable = git_reachable_shas(world, None)?;
    let completed: std::collections::BTreeSet<String> =
        completed_ledger_shas(world)?.into_iter().collect();
    let missing: Vec<String> = reachable
        .iter()
        .filter(|sha| !completed.contains(*sha))
        .cloned()
        .collect();
    ensure!(
        missing.is_empty(),
        "reachable commits missing completed ledger rows: {}",
        missing.join(", ")
    );
    Ok(())
}

pub fn assert_artefacts_current_has_rows(world: &QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let count = artefacts_current_count(world)?;
    ensure!(
        count > 0,
        "expected artefacts_current to contain rows, got {count}"
    );
    Ok(())
}

pub fn assert_artefacts_current_contains_path(
    world: &QatWorld,
    repo_name: &str,
    path: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let count = artefacts_current_count_for_path(world, path)?;
    ensure!(
        count > 0,
        "expected artefacts_current rows for `{path}`, got {count}"
    );
    Ok(())
}

pub fn assert_artefacts_current_contains_path_eventually(
    world: &mut QatWorld,
    repo_name: &str,
    path: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let started = Instant::now();
    let expected = format!("artefacts_current to eventually contain `{path}`");
    let first_attempt = wait_for_qat_condition(
        qat_eventual_timeout(),
        qat_eventual_poll_interval(),
        &expected,
        || assert_artefacts_current_contains_path(world, repo_name, path),
        |_| true,
        |_| format!("artefacts_current contains `{path}`"),
    );
    if let Err(first_err) = first_attempt {
        append_world_log(
            world,
            &format!(
                "Initial watcher materialisation wait for `{path}` timed out; nudging source file and retrying.\n",
            ),
        )?;
        nudge_source_file_at_path_for_repo(world, repo_name, path)?;
        wait_for_qat_condition(
            qat_eventual_timeout(),
            qat_eventual_poll_interval(),
            &expected,
            || assert_artefacts_current_contains_path(world, repo_name, path),
            |_| true,
            |_| format!("artefacts_current contains `{path}`"),
        )
        .map_err(|retry_err| {
            anyhow!(
                "artefacts_current watcher materialisation did not recover after file nudge\nfirst attempt: {first_err:#}\nretry attempt: {retry_err:#}"
            )
        })?;
    }
    append_timing_log(
        world,
        "wait artefacts_current contains",
        started.elapsed(),
        format!("repo={repo_name} path={path} nudge=allowed"),
    )?;
    Ok(())
}

pub fn assert_artefacts_current_contains_path_eventually_without_nudge(
    world: &QatWorld,
    repo_name: &str,
    path: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let started = Instant::now();
    wait_for_qat_condition(
        qat_eventual_timeout(),
        qat_eventual_poll_interval(),
        &format!("artefacts_current to contain `{path}` without nudge"),
        || assert_artefacts_current_contains_path(world, repo_name, path),
        |_| true,
        |_| format!("artefacts_current contains `{path}`"),
    )?;
    append_timing_log(
        world,
        "wait artefacts_current contains",
        started.elapsed(),
        format!("repo={repo_name} path={path} nudge=disabled"),
    )?;
    Ok(())
}

pub fn assert_artefacts_current_lacks_path(
    world: &QatWorld,
    repo_name: &str,
    path: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let count = artefacts_current_count_for_path(world, path)?;
    ensure!(
        count == 0,
        "expected artefacts_current to omit `{path}`, got {count} rows"
    );
    Ok(())
}

pub fn assert_artefacts_current_lacks_path_eventually(
    world: &QatWorld,
    repo_name: &str,
    path: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let started = Instant::now();
    wait_for_qat_condition(
        qat_eventual_timeout(),
        qat_eventual_poll_interval(),
        &format!("artefacts_current to omit `{path}`"),
        || {
            assert_artefacts_current_lacks_path(world, repo_name, path)?;
            Ok(true)
        },
        |ready| *ready,
        |ready| format!("ready={ready}"),
    )?;
    append_timing_log(
        world,
        "wait artefacts_current lacks",
        started.elapsed(),
        format!("repo={repo_name} path={path}"),
    )?;
    Ok(())
}

pub fn snapshot_current_file_state_content_ids_for_paths(
    world: &mut QatWorld,
    repo_name: &str,
    paths: &[String],
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    ensure!(
        !paths.is_empty(),
        "expected at least one path for current_file_state snapshot"
    );
    for path in paths {
        let current = current_file_state_effective_content_id(world, path)?;
        world
            .current_file_state_content_id_snapshots
            .insert(path.clone(), current);
    }
    Ok(())
}

pub fn assert_current_file_state_content_id_changed_since_snapshot_for_path(
    world: &QatWorld,
    repo_name: &str,
    path: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let before = world
        .current_file_state_content_id_snapshots
        .get(path)
        .cloned()
        .ok_or_else(|| anyhow!("no current_file_state snapshot captured for `{path}`"))?;
    let after = current_file_state_effective_content_id(world, path)?;
    ensure!(
        after != before,
        "expected current_file_state effective_content_id for `{path}` to change, but both snapshots were {after:?}"
    );
    Ok(())
}

pub fn assert_current_file_state_content_id_changed_eventually_for_path(
    world: &QatWorld,
    repo_name: &str,
    path: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let started = Instant::now();
    wait_for_qat_condition(
        qat_eventual_timeout(),
        qat_eventual_poll_interval(),
        &format!("current_file_state content id for `{path}` to change"),
        || {
            assert_current_file_state_content_id_changed_since_snapshot_for_path(
                world, repo_name, path,
            )?;
            Ok(true)
        },
        |ready| *ready,
        |ready| format!("ready={ready}"),
    )?;
    append_timing_log(
        world,
        "wait current_file_state content id changed",
        started.elapsed(),
        format!("repo={repo_name} path={path}"),
    )?;
    Ok(())
}

pub fn assert_current_file_state_content_id_unchanged_since_snapshot_for_path(
    world: &QatWorld,
    repo_name: &str,
    path: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let before = world
        .current_file_state_content_id_snapshots
        .get(path)
        .cloned()
        .ok_or_else(|| anyhow!("no current_file_state snapshot captured for `{path}`"))?;
    let after = current_file_state_effective_content_id(world, path)?;
    ensure!(
        after == before,
        "expected current_file_state effective_content_id for `{path}` to remain {before:?}, got {after:?}"
    );
    Ok(())
}

pub fn assert_last_task_id_captured(world: &QatWorld) -> Result<()> {
    let task_id = world
        .last_task_id
        .as_deref()
        .ok_or_else(|| anyhow!("expected a captured DevQL task id"))?;
    ensure!(
        !task_id.trim().is_empty(),
        "expected non-empty captured DevQL task id"
    );
    Ok(())
}

pub fn assert_last_task_id_matches_kind(world: &QatWorld, expected_kind: &str) -> Result<()> {
    let task_id = world
        .last_task_id
        .as_deref()
        .ok_or_else(|| anyhow!("expected a captured DevQL task id"))?;
    let expected_prefix = format!("{expected_kind}-task-");
    ensure!(
        task_id.starts_with(&expected_prefix),
        "expected tracked DevQL task kind `{expected_kind}`, got `{task_id}`"
    );
    Ok(())
}

pub fn assert_task_queue_state_in_last_output(
    world: &QatWorld,
    expected_state: &str,
) -> Result<()> {
    let stdout = world.last_command_stdout.as_deref().unwrap_or("");
    let status = parse_task_queue_status(stdout)?;
    ensure!(
        status.state == expected_state,
        "expected DevQL task queue state `{expected_state}`, got `{}`\nstdout: {stdout}",
        status.state
    );
    Ok(())
}

pub fn assert_task_queue_pause_reason_in_last_output(
    world: &QatWorld,
    expected_reason: &str,
) -> Result<()> {
    let stdout = world.last_command_stdout.as_deref().unwrap_or("");
    let status = parse_task_queue_status(stdout)?;
    let actual = status
        .pause_reason
        .ok_or_else(|| anyhow!("expected a pause reason in DevQL task queue status output"))?;
    ensure!(
        actual == expected_reason,
        "expected DevQL task queue pause reason `{expected_reason}`, got `{actual}`\nstdout: {stdout}"
    );
    Ok(())
}

pub fn assert_task_list_in_last_output_contains_last_task(world: &QatWorld) -> Result<()> {
    let expected_task_id = world
        .last_task_id
        .as_deref()
        .ok_or_else(|| anyhow!("expected a captured DevQL task id before asserting task list"))?;
    let stdout = world.last_command_stdout.as_deref().unwrap_or("");
    let tasks = parse_task_briefs(stdout);
    ensure!(
        tasks.iter().any(|task| task.task_id == expected_task_id),
        "expected task list to include `{expected_task_id}`, observed {:?}\nstdout: {stdout}",
        tasks
            .iter()
            .map(|task| task.task_id.as_str())
            .collect::<Vec<_>>()
    );
    Ok(())
}

pub fn assert_last_task_status_in_last_output(
    world: &QatWorld,
    expected_status: &str,
) -> Result<()> {
    let expected_task_id = world
        .last_task_id
        .as_deref()
        .ok_or_else(|| anyhow!("expected a captured DevQL task id before asserting task status"))?;
    let stdout = world.last_command_stdout.as_deref().unwrap_or("");
    let tasks = parse_task_briefs(stdout);
    let task = tasks
        .iter()
        .find(|task| task.task_id == expected_task_id)
        .ok_or_else(|| anyhow!("expected task `{expected_task_id}` in parsed task output"))?;
    ensure!(
        task.status == expected_status,
        "expected task `{expected_task_id}` to have status `{expected_status}`, got `{}`\nstdout: {stdout}",
        task.status
    );
    Ok(())
}

pub fn assert_expected_shas_completed_in_ledger(world: &QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    ensure!(
        !world.expected_commit_shas.is_empty(),
        "no expected commit SHAs captured for this scenario"
    );
    let completed: std::collections::BTreeSet<String> =
        completed_ledger_shas(world)?.into_iter().collect();
    let missing: Vec<String> = world
        .expected_commit_shas
        .iter()
        .filter(|sha| !completed.contains(*sha))
        .cloned()
        .collect();
    ensure!(
        missing.is_empty(),
        "expected commit SHAs missing from completed ledger: {}",
        missing.join(", ")
    );
    Ok(())
}

pub fn assert_expected_shas_have_file_state_rows(world: &QatWorld, repo_name: &str) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    ensure!(
        !world.expected_commit_shas.is_empty(),
        "no expected commit SHAs captured for file_state assertion"
    );
    let mut missing = Vec::new();
    for sha in &world.expected_commit_shas {
        if file_state_count_for_commit(world, sha)? == 0 && commit_has_changed_files(world, sha)? {
            missing.push(sha.clone());
        }
    }
    ensure!(
        missing.is_empty(),
        "expected file_state rows for commit SHAs, but none found for: {}",
        missing.join(", ")
    );
    Ok(())
}

pub fn assert_expected_paths_have_file_state_rows_for_expected_shas(
    world: &QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let pairs = expected_commit_path_pairs(&world.expected_commit_shas, &world.expected_paths)?;
    let mut missing = Vec::new();
    for (sha, path) in pairs {
        if file_state_count_for_commit_path(world, &sha, &path)? == 0 {
            missing.push(format!("{path}@{sha}"));
        }
    }
    ensure!(
        missing.is_empty(),
        "expected file_state rows for expected path/SHA pairs, but none found for: {}",
        missing.join(", ")
    );
    Ok(())
}

pub fn assert_no_new_completed_shas_since_snapshot(
    world: &QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let before: std::collections::BTreeSet<String> = world
        .completed_ledger_shas_snapshot
        .iter()
        .cloned()
        .collect();
    let after: std::collections::BTreeSet<String> =
        completed_ledger_shas(world)?.into_iter().collect();
    let new_shas: Vec<String> = after.difference(&before).cloned().collect();
    ensure!(
        new_shas.is_empty(),
        "expected no new completed ledger SHAs, got: {}",
        new_shas.join(", ")
    );
    Ok(())
}

pub fn assert_exact_expected_shas_newly_completed_since_snapshot(
    world: &QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    ensure!(
        !world.expected_commit_shas.is_empty(),
        "no expected commit SHAs captured for new-ledger assertion"
    );
    let before: std::collections::BTreeSet<String> = world
        .completed_ledger_shas_snapshot
        .iter()
        .cloned()
        .collect();
    let after: std::collections::BTreeSet<String> =
        completed_ledger_shas(world)?.into_iter().collect();
    let actual_new: std::collections::BTreeSet<String> =
        after.difference(&before).cloned().collect();
    let expected_new: std::collections::BTreeSet<String> =
        world.expected_commit_shas.iter().cloned().collect();
    ensure!(
        actual_new == expected_new,
        "expected newly completed SHAs {:?}, got {:?}",
        expected_new,
        actual_new
    );
    Ok(())
}

pub fn assert_ledger_completed_count_unchanged_since_snapshot(
    world: &QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let before = world
        .completed_ledger_count_snapshot
        .ok_or_else(|| anyhow!("completed ledger count snapshot is missing"))?;
    let after = completed_ledger_count(world)?;
    ensure!(
        before == after,
        "expected completed ledger count to stay {before}, got {after}"
    );
    Ok(())
}

pub fn assert_artefacts_current_count_unchanged_since_snapshot(
    world: &QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let before = world
        .artefacts_current_count_snapshot
        .ok_or_else(|| anyhow!("artefacts_current count snapshot is missing"))?;
    let after = artefacts_current_count(world)?;
    ensure!(
        before == after,
        "expected artefacts_current count to stay {before}, got {after}"
    );
    Ok(())
}

pub fn assert_artefacts_current_count_increased_since_snapshot(
    world: &QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    let before = world
        .artefacts_current_count_snapshot
        .ok_or_else(|| anyhow!("artefacts_current count snapshot is missing"))?;
    let after = artefacts_current_count(world)?;
    ensure!(
        after > before,
        "expected artefacts_current count to increase from {before}, got {after}"
    );
    Ok(())
}

pub fn assert_only_latest_reachable_shas_completed_in_ledger(
    world: &QatWorld,
    repo_name: &str,
    latest_count: usize,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    wait_for_qat_condition(
        qat_eventual_timeout(),
        qat_eventual_poll_interval(),
        "bounded backfill ingest ledger to match latest reachable commits",
        || {
            let reachable = git_reachable_shas(world, None)?;
            let completed = completed_ledger_shas(world)?;
            latest_reachable_ledger_snapshot(&reachable, &completed, latest_count)
        },
        |snapshot| snapshot.completed_reachable == snapshot.expected_latest,
        |snapshot| {
            format!(
                "expected_latest={:?}; completed_reachable={:?}; reachable_total={}; completed_total={}",
                snapshot.expected_latest,
                snapshot.completed_reachable,
                snapshot.reachable_total,
                snapshot.completed_total
            )
        },
    )?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LatestReachableLedgerSnapshot {
    expected_latest: std::collections::BTreeSet<String>,
    completed_reachable: std::collections::BTreeSet<String>,
    reachable_total: usize,
    completed_total: usize,
}

fn latest_reachable_ledger_snapshot(
    reachable: &[String],
    completed: &[String],
    latest_count: usize,
) -> Result<LatestReachableLedgerSnapshot> {
    ensure!(
        latest_count > 0,
        "latest reachable SHA count must be greater than zero"
    );
    ensure!(
        reachable.len() >= latest_count,
        "expected at least {latest_count} reachable commits, found {}",
        reachable.len()
    );
    let expected_latest = reachable.iter().take(latest_count).cloned().collect();
    let reachable_set: std::collections::BTreeSet<String> = reachable.iter().cloned().collect();
    let completed_set: std::collections::BTreeSet<String> = completed.iter().cloned().collect();
    let completed_reachable = completed_set
        .intersection(&reachable_set)
        .cloned()
        .collect();
    Ok(LatestReachableLedgerSnapshot {
        expected_latest,
        completed_reachable,
        reachable_total: reachable.len(),
        completed_total: completed.len(),
    })
}

pub fn assert_rewrite_new_shas_count(
    world: &QatWorld,
    repo_name: &str,
    expected_count: usize,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    ensure!(
        world.rewrite_new_shas.len() == expected_count,
        "expected {expected_count} rewritten SHAs, got {} (new={:?}, pre={:?}, post={:?})",
        world.rewrite_new_shas.len(),
        world.rewrite_new_shas,
        world.pre_rewrite_shas,
        world.post_rewrite_shas
    );
    Ok(())
}

pub fn assert_pre_rewrite_shas_absent_from_post_segment(
    world: &QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    ensure!(
        !world.pre_rewrite_shas.is_empty() && !world.post_rewrite_shas.is_empty(),
        "pre/post rewrite SHA segments are not populated"
    );
    let post: std::collections::BTreeSet<String> =
        world.post_rewrite_shas.iter().cloned().collect();
    let retained_old: Vec<String> = world
        .pre_rewrite_shas
        .iter()
        .filter(|sha| post.contains(*sha))
        .cloned()
        .collect();
    ensure!(
        retained_old.is_empty(),
        "expected rewritten old SHAs to be absent from post-rewrite segment, retained: {}",
        retained_old.join(", ")
    );
    Ok(())
}

pub fn assert_rewrite_new_shas_completed_in_ledger(
    world: &QatWorld,
    repo_name: &str,
) -> Result<()> {
    ensure_bitloops_repo_name(repo_name)?;
    ensure!(
        !world.rewrite_new_shas.is_empty(),
        "rewrite_new_shas is empty; rewrite assertions require rewritten SHAs"
    );
    let completed: std::collections::BTreeSet<String> =
        completed_ledger_shas(world)?.into_iter().collect();
    let missing: Vec<String> = world
        .rewrite_new_shas
        .iter()
        .filter(|sha| !completed.contains(*sha))
        .cloned()
        .collect();
    ensure!(
        missing.is_empty(),
        "rewritten SHAs missing from completed ledger rows: {}",
        missing.join(", ")
    );
    Ok(())
}

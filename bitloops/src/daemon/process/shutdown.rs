use super::liveness::process_is_running;
#[cfg(unix)]
use super::reaping::reap_terminated_child_process;
use super::*;

pub(in crate::daemon) fn terminate_process(pid: u32) -> Result<()> {
    #[cfg(windows)]
    {
        let status = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("running `taskkill` for Bitloops daemon")?;
        if !status.success() {
            bail!("failed to stop Bitloops daemon process {pid}");
        }
    }

    #[cfg(not(windows))]
    {
        let status = Command::new("kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("running `kill -TERM` for Bitloops daemon")?;
        if !status.success() {
            bail!("failed to stop Bitloops daemon process {pid}");
        }
    }

    Ok(())
}

pub(in crate::daemon) fn terminate_process_and_wait_for_shutdown_cleanup(
    pid: u32,
    timeout: Duration,
    runtime_clean_exit_grace: Duration,
    force_kill_timeout: Duration,
) -> Result<()> {
    terminate_process(pid)?;
    wait_for_shutdown_cleanup_with_force_kill(
        pid,
        timeout,
        runtime_clean_exit_grace,
        force_kill_timeout,
    )
}

pub(in crate::daemon) fn cleanup_spawned_daemon_after_startup_failure(
    child: &mut std::process::Child,
    launch_mode: &str,
) -> Result<()> {
    let pid = child.id();
    if let Some(status) = child.try_wait().with_context(|| {
        format!("checking spawned {launch_mode} daemon pid={pid} status before cleanup")
    })? {
        log::debug!(
            "spawned {launch_mode} daemon pid={pid} exited before startup cleanup: {status}"
        );
        if let Err(err) = read_runtime_state(Path::new(".")) {
            log::warn!(
                "failed to inspect daemon runtime state after spawned {launch_mode} daemon pid={pid} exited: {err:#}"
            );
        }
        return Ok(());
    }

    if let Err(err) = terminate_process(pid) {
        if child
            .try_wait()
            .with_context(|| {
                format!(
                    "checking spawned {launch_mode} daemon pid={pid} status after terminate failure"
                )
            })?
            .is_none()
        {
            return Err(err);
        }
        log::debug!(
            "spawned {launch_mode} daemon pid={pid} exited before terminate completed: {err:#}"
        );
    }

    wait_for_spawned_child_shutdown_cleanup(
        child,
        launch_mode,
        STOP_TIMEOUT,
        STOP_RUNTIME_CLEAN_EXIT_GRACE,
        FORCE_KILL_TIMEOUT,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ShutdownCleanupStatus {
    process_exited: bool,
    runtime_cleaned: bool,
}

impl ShutdownCleanupStatus {
    fn complete(self) -> bool {
        self.process_exited && self.runtime_cleaned
    }
}

fn force_kill_process(pid: u32) -> Result<()> {
    if !process_is_running(pid)? {
        return Ok(());
    }

    #[cfg(windows)]
    {
        let status = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("running `taskkill /F` for Bitloops daemon")?;
        if !status.success() {
            bail!("failed to force-stop Bitloops daemon process {pid}");
        }
    }

    #[cfg(not(windows))]
    {
        let status = Command::new("kill")
            .arg("-KILL")
            .arg(pid.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("running `kill -KILL` for Bitloops daemon")?;
        if !status.success() {
            bail!("failed to force-stop Bitloops daemon process {pid}");
        }
    }

    Ok(())
}

fn wait_for_spawned_child_shutdown_cleanup(
    child: &mut std::process::Child,
    launch_mode: &str,
    timeout: Duration,
    runtime_clean_exit_grace: Duration,
    force_kill_timeout: Duration,
) -> Result<()> {
    let pid = child.id();
    let graceful_started = Instant::now();
    let mut runtime_cleaned_since = None::<Instant>;
    let mut force_kill_started = None::<Instant>;

    loop {
        let status = spawned_child_shutdown_cleanup_status(child)?;
        if status.complete() {
            return Ok(());
        }

        if let Some(force_started) = force_kill_started {
            if status.process_exited {
                cleanup_stale_runtime_state_after_forced_shutdown(status);
                return Ok(());
            }
            if force_started.elapsed() > force_kill_timeout {
                bail!(
                    "Bitloops daemon did not shut down after forced termination (process_exited={}, runtime_cleaned={})",
                    status.process_exited,
                    status.runtime_cleaned
                );
            }
        } else {
            if status.runtime_cleaned && !status.process_exited {
                let cleaned_since = runtime_cleaned_since.get_or_insert_with(Instant::now);
                if cleaned_since.elapsed() >= runtime_clean_exit_grace {
                    log::warn!(
                        "spawned {launch_mode} daemon process {pid} remained alive for {:?} after runtime cleanup; forcing termination",
                        runtime_clean_exit_grace
                    );
                    force_kill_spawned_child(child, launch_mode)?;
                    force_kill_started = Some(Instant::now());
                    continue;
                }
            } else {
                runtime_cleaned_since = None;
            }

            if graceful_started.elapsed() > timeout {
                if !status.process_exited {
                    log::warn!(
                        "spawned {launch_mode} daemon process {pid} exceeded startup cleanup timeout of {:?}; forcing termination (runtime_cleaned={})",
                        timeout,
                        status.runtime_cleaned
                    );
                    force_kill_spawned_child(child, launch_mode)?;
                    force_kill_started = Some(Instant::now());
                    continue;
                }

                cleanup_stale_runtime_state_after_forced_shutdown(status);
                return Ok(());
            }
        }

        std::thread::sleep(Duration::from_millis(100));
    }
}

fn spawned_child_shutdown_cleanup_status(
    child: &mut std::process::Child,
) -> Result<ShutdownCleanupStatus> {
    Ok(ShutdownCleanupStatus {
        process_exited: child
            .try_wait()
            .context("checking spawned Bitloops daemon process")?
            .is_some(),
        runtime_cleaned: read_runtime_state(Path::new("."))?.is_none(),
    })
}

fn force_kill_spawned_child(child: &mut std::process::Child, launch_mode: &str) -> Result<()> {
    let pid = child.id();
    if child
        .try_wait()
        .with_context(|| {
            format!("checking spawned {launch_mode} daemon pid={pid} status before force kill")
        })?
        .is_some()
    {
        return Ok(());
    }

    match child.kill() {
        Ok(()) => Ok(()),
        Err(err) => {
            if child
                .try_wait()
                .with_context(|| {
                    format!(
                        "checking spawned {launch_mode} daemon pid={pid} status after force kill failure"
                    )
                })?
                .is_some()
            {
                Ok(())
            } else {
                Err(err).with_context(|| {
                    format!("force-killing spawned {launch_mode} daemon pid={pid}")
                })
            }
        }
    }
}

pub(in crate::daemon) fn wait_for_shutdown_cleanup(pid: u32, timeout: Duration) -> Result<()> {
    wait_for_shutdown_cleanup_with_force_kill(
        pid,
        timeout,
        STOP_RUNTIME_CLEAN_EXIT_GRACE,
        FORCE_KILL_TIMEOUT,
    )
}

fn process_has_exited_or_was_reaped(pid: u32) -> Result<bool> {
    #[cfg(unix)]
    if reap_terminated_child_process(pid, Duration::ZERO)? {
        return Ok(true);
    }

    process_is_running(pid).map(|running| !running)
}

fn wait_for_shutdown_cleanup_with_force_kill(
    pid: u32,
    timeout: Duration,
    runtime_clean_exit_grace: Duration,
    force_kill_timeout: Duration,
) -> Result<()> {
    let graceful_started = Instant::now();
    let mut runtime_cleaned_since = None::<Instant>;
    let mut force_kill_started = None::<Instant>;

    loop {
        let status = shutdown_cleanup_status(pid)?;
        if status.complete() {
            return Ok(());
        }

        if let Some(force_started) = force_kill_started {
            if status.process_exited {
                cleanup_stale_runtime_state_after_forced_shutdown(status);
                return Ok(());
            }
            if force_started.elapsed() > force_kill_timeout {
                bail!(
                    "Bitloops daemon did not shut down after forced termination (process_exited={}, runtime_cleaned={})",
                    status.process_exited,
                    status.runtime_cleaned
                );
            }
        } else {
            if status.runtime_cleaned && !status.process_exited {
                let cleaned_since = runtime_cleaned_since.get_or_insert_with(Instant::now);
                if cleaned_since.elapsed() >= runtime_clean_exit_grace {
                    log::warn!(
                        "daemon process {} remained alive for {:?} after runtime cleanup; forcing termination",
                        pid,
                        runtime_clean_exit_grace
                    );
                    force_kill_process(pid)?;
                    force_kill_started = Some(Instant::now());
                    continue;
                }
            } else {
                runtime_cleaned_since = None;
            }

            if graceful_started.elapsed() > timeout {
                if !status.process_exited {
                    log::warn!(
                        "daemon process {} exceeded graceful shutdown timeout of {:?}; forcing termination (runtime_cleaned={})",
                        pid,
                        timeout,
                        status.runtime_cleaned
                    );
                    force_kill_process(pid)?;
                    force_kill_started = Some(Instant::now());
                    continue;
                }

                cleanup_stale_runtime_state_after_forced_shutdown(status);
                return Ok(());
            }
        }

        std::thread::sleep(Duration::from_millis(100));
    }
}

fn shutdown_cleanup_status(pid: u32) -> Result<ShutdownCleanupStatus> {
    Ok(ShutdownCleanupStatus {
        process_exited: process_has_exited_or_was_reaped(pid)?,
        runtime_cleaned: read_runtime_state(Path::new("."))?.is_none(),
    })
}

fn cleanup_stale_runtime_state_after_forced_shutdown(status: ShutdownCleanupStatus) {
    if !status.runtime_cleaned
        && let Err(err) = delete_runtime_state()
    {
        log::warn!("failed to clear stale daemon runtime state after forced shutdown: {err:#}");
    }
}

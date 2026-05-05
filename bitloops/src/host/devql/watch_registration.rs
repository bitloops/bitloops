use std::time::{Duration, Instant};

use anyhow::Result;

use crate::host::runtime_store::{
    RepoSqliteRuntimeStore, RepoWatcherRegistration, RepoWatcherRegistrationState,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ExistingWatcherRegistrationDisposition {
    Ready,
    WaitForReady,
    Replace { kill_running_process: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ExistingWatcherRegistrationHandle {
    Handled,
    RetrySpawn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WatcherRegistrationReadyError {
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
pub(super) enum TimedOutPendingRecovery {
    PendingReleased,
    ReadyRegistrationPresent,
}

pub(super) fn classify_existing_watcher_registration(
    entry: &RepoWatcherRegistration,
    expected_restart_token: &str,
    watcher_running: bool,
) -> ExistingWatcherRegistrationDisposition {
    if watcher_running && entry.restart_token == expected_restart_token {
        return match entry.state {
            RepoWatcherRegistrationState::Ready => ExistingWatcherRegistrationDisposition::Ready,
            RepoWatcherRegistrationState::Pending => {
                ExistingWatcherRegistrationDisposition::WaitForReady
            }
        };
    }

    ExistingWatcherRegistrationDisposition::Replace {
        kill_running_process: watcher_running && entry.restart_token != expected_restart_token,
    }
}

pub(super) fn handle_existing_watcher_registration(
    runtime_store: &RepoSqliteRuntimeStore,
    entry: RepoWatcherRegistration,
    expected_restart_token: &str,
    timeout: Duration,
    poll_interval: Duration,
) -> Result<ExistingWatcherRegistrationHandle> {
    match classify_existing_watcher_registration(
        &entry,
        expected_restart_token,
        super::process_is_running(entry.pid),
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
                || Ok(super::process_is_running(entry.pid)),
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
                super::kill_process(entry.pid);
            }
            runtime_store
                .delete_watcher_registration_if_matches(entry.pid, &entry.restart_token)?;
            Ok(ExistingWatcherRegistrationHandle::RetrySpawn)
        }
    }
}

pub(super) fn recover_timed_out_pending_registration(
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
                && entry.state == RepoWatcherRegistrationState::Ready
                && super::process_is_running(entry.pid) =>
        {
            Ok(Some(TimedOutPendingRecovery::ReadyRegistrationPresent))
        }
        None => Ok(Some(TimedOutPendingRecovery::PendingReleased)),
        Some(_) => Ok(None),
    }
}

pub(super) fn wait_for_watcher_registration_ready<FLoad, FWatcherRunning>(
    expected_pid: u32,
    expected_restart_token: &str,
    timeout: Duration,
    poll_interval: Duration,
    mut load_registration: FLoad,
    mut watcher_running: FWatcherRunning,
) -> Result<()>
where
    FLoad: FnMut() -> Result<Option<RepoWatcherRegistration>>,
    FWatcherRunning: FnMut() -> Result<bool>,
{
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(entry) = load_registration()?
            && entry.pid == expected_pid
            && entry.restart_token == expected_restart_token
            && entry.state == RepoWatcherRegistrationState::Ready
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

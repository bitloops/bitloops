use super::termination::{ChildTerminationOutcome, ChildTerminationRecord};
use super::*;

pub(in crate::daemon) fn reap_terminated_child_process(
    pid: u32,
    timeout: Duration,
) -> Result<bool> {
    if pid > i32::MAX as u32 {
        return Ok(false);
    }

    let deadline = Instant::now() + timeout;
    loop {
        let mut status = 0;
        let result = unsafe { libc::waitpid(pid as libc::pid_t, &mut status, libc::WNOHANG) };
        if result == pid as libc::pid_t {
            return Ok(true);
        }
        if result == 0 {
            if Instant::now() >= deadline {
                return Ok(false);
            }
            std::thread::sleep(Duration::from_millis(25));
            continue;
        }

        let raw_os_error = std::io::Error::last_os_error().raw_os_error();
        match raw_os_error {
            Some(libc::ECHILD) => return Ok(false),
            Some(libc::EINTR) => continue,
            _ => {
                return Err(std::io::Error::last_os_error())
                    .context("waiting for Bitloops daemon child process");
            }
        }
    }
}

#[cfg(not(unix))]
pub(in crate::daemon) fn reap_terminated_child_process(
    _pid: u32,
    _timeout: Duration,
) -> Result<bool> {
    Ok(false)
}

#[cfg(unix)]
pub(in crate::daemon) fn reap_terminated_child_processes() -> Result<Vec<ChildTerminationRecord>> {
    let mut reaped = Vec::new();
    loop {
        let mut status = 0;
        let result = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) };
        if result > 0 {
            reaped.push(ChildTerminationRecord {
                pid: result as u32,
                outcome: decode_child_termination_status(status),
            });
            continue;
        }
        if result == 0 {
            return Ok(reaped);
        }

        let raw_os_error = std::io::Error::last_os_error().raw_os_error();
        return match raw_os_error {
            Some(libc::ECHILD) => Ok(reaped),
            Some(libc::EINTR) => continue,
            _ => Err(std::io::Error::last_os_error())
                .context("reaping Bitloops daemon child processes"),
        };
    }
}

#[cfg(unix)]
fn decode_child_termination_status(status: i32) -> ChildTerminationOutcome {
    if libc::WIFEXITED(status) {
        return ChildTerminationOutcome::Exited {
            code: libc::WEXITSTATUS(status),
        };
    }
    if libc::WIFSIGNALED(status) {
        return ChildTerminationOutcome::Signaled {
            signal: libc::WTERMSIG(status),
            core_dumped: libc::WCOREDUMP(status),
        };
    }
    if libc::WIFSTOPPED(status) {
        return ChildTerminationOutcome::Stopped {
            signal: libc::WSTOPSIG(status),
        };
    }
    if libc::WIFCONTINUED(status) {
        return ChildTerminationOutcome::Continued;
    }
    ChildTerminationOutcome::Unknown { raw_status: status }
}

#[cfg(not(unix))]
pub(in crate::daemon) fn reap_terminated_child_processes() -> Result<Vec<ChildTerminationRecord>> {
    Ok(Vec::new())
}

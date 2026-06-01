use super::*;

pub(in crate::daemon) fn process_is_running(pid: u32) -> Result<bool> {
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
            .map(|status| status.success())
            .unwrap_or(false))
    }

    #[cfg(not(windows))]
    {
        if pid > i32::MAX as u32 {
            return Ok(false);
        }

        let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
        let visible = unix_kill_zero_indicates_running(
            result,
            std::io::Error::last_os_error().raw_os_error(),
        );
        if !visible {
            return Ok(false);
        }

        if let Some(exited_child) = unix_child_has_exited_without_reap(pid) {
            return Ok(!exited_child);
        }

        Ok(!unix_process_is_zombie(pid))
    }
}

pub(in crate::daemon) fn running_internal_daemon_process_pids_for_config(
    config_path: &Path,
) -> Result<Vec<u32>> {
    #[cfg(unix)]
    {
        let output = Command::new("ps")
            .args(["-axo", "pid=,command="])
            .stdin(Stdio::null())
            .output()
            .context("listing Bitloops daemon processes")?;
        if !output.status.success() {
            return Ok(Vec::new());
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(parse_internal_daemon_process_pids(
            &stdout,
            std::process::id(),
            Some(config_path),
        ))
    }

    #[cfg(not(unix))]
    {
        let _ = config_path;
        Ok(Vec::new())
    }
}

pub(in crate::daemon) fn parse_internal_daemon_process_pids(
    ps_output: &str,
    current_pid: u32,
    config_path: Option<&Path>,
) -> Vec<u32> {
    ps_output
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim_start();
            let (pid_raw, command) = trimmed.split_once(char::is_whitespace)?;
            let pid = pid_raw.parse::<u32>().ok()?;
            if pid == current_pid || !command.contains(INTERNAL_DAEMON_COMMAND_NAME) {
                return None;
            }
            if let Some(config_path) = config_path
                && !command_matches_config_path(command, config_path)
            {
                return None;
            }
            Some(pid)
        })
        .collect()
}

fn command_matches_config_path(command: &str, config_path: &Path) -> bool {
    let expected = config_path.to_string_lossy();
    command_contains_exact_flag_value(command, "--config-path ", &expected)
        || command_contains_exact_flag_value(command, "--config-path=", &expected)
}

fn command_contains_exact_flag_value(command: &str, prefix: &str, expected: &str) -> bool {
    let needle = format!("{prefix}{expected}");
    let mut search_from = 0;
    while let Some(offset) = command[search_from..].find(&needle) {
        let end = search_from + offset + needle.len();
        if command[end..]
            .chars()
            .next()
            .is_none_or(char::is_whitespace)
        {
            return true;
        }
        search_from = end;
    }
    false
}

#[cfg(not(windows))]
pub(super) fn unix_kill_zero_indicates_running(result: i32, raw_os_error: Option<i32>) -> bool {
    if result == 0 {
        return true;
    }

    matches!(raw_os_error, Some(libc::EPERM))
}

#[cfg(not(windows))]
fn unix_child_has_exited_without_reap(pid: u32) -> Option<bool> {
    let mut siginfo: libc::siginfo_t = unsafe { std::mem::zeroed() };
    loop {
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                pid as libc::id_t,
                &mut siginfo,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if result == 0 {
            let child_pid = unsafe { siginfo.si_pid() };
            return if child_pid == 0 {
                Some(false)
            } else {
                Some(true)
            };
        }

        match std::io::Error::last_os_error().raw_os_error() {
            Some(libc::ECHILD) => return None,
            Some(libc::EINTR) => continue,
            _ => return None,
        }
    }
}

#[cfg(not(windows))]
fn unix_process_is_zombie(pid: u32) -> bool {
    let output = Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .stdin(Stdio::null())
        .output();
    let Ok(output) = output else {
        return false;
    };
    if !output.status.success() {
        return false;
    }

    String::from_utf8_lossy(&output.stdout)
        .trim_start()
        .starts_with('Z')
}

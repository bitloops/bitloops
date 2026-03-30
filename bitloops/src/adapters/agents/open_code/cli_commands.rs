use std::time::Duration;

use anyhow::{Result, anyhow};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::Instant;

pub const OPENCODE_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

pub fn run_opencode_session_delete(session_id: &str) -> Result<()> {
    run_opencode_session_delete_with_runner(session_id, run_opencode_command_with_timeout)
}

fn run_opencode_session_delete_with_runner<F>(session_id: &str, runner: F) -> Result<()>
where
    F: FnOnce(&[&str], &str) -> Result<Output>,
{
    let output = runner(&["session", "delete", session_id], "delete")?;
    if output.status.success() {
        return Ok(());
    }

    let combined = combined_output_string(&output);
    if combined.contains("Session not found") {
        return Ok(());
    }

    let status = output
        .status
        .code()
        .map(|code| code.to_string())
        .unwrap_or_else(|| "terminated by signal".to_string());
    Err(anyhow!(
        "opencode session delete failed: exit status {status} (output: {combined})"
    ))
}

pub fn run_opencode_import(export_file_path: &str) -> Result<()> {
    run_opencode_import_with_runner(export_file_path, run_opencode_command_with_timeout)
}

fn run_opencode_import_with_runner<F>(export_file_path: &str, runner: F) -> Result<()>
where
    F: FnOnce(&[&str], &str) -> Result<Output>,
{
    let output = runner(&["import", export_file_path], "import")?;
    if output.status.success() {
        return Ok(());
    }

    let combined = combined_output_string(&output);
    let status = output
        .status
        .code()
        .map(|code| code.to_string())
        .unwrap_or_else(|| "terminated by signal".to_string());
    Err(anyhow!(
        "opencode import failed: exit status {status} (output: {combined})"
    ))
}

fn run_opencode_command_with_timeout(args: &[&str], command_kind: &str) -> Result<Output> {
    run_command_with_timeout("opencode", args, command_kind, OPENCODE_COMMAND_TIMEOUT)
}

fn run_command_with_timeout(
    program: &str,
    args: &[&str],
    command_kind: &str,
    timeout: Duration,
) -> Result<Output> {
    let mut child = Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| anyhow!("failed to spawn {program} command: {err}"))?;

    let start = Instant::now();
    loop {
        if child
            .try_wait()
            .map_err(|err| anyhow!("failed waiting for {program} command: {err}"))?
            .is_some()
        {
            return child
                .wait_with_output()
                .map_err(|err| anyhow!("failed collecting {program} command output: {err}"));
        }

        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            let timeout_secs = timeout.as_secs_f64();
            return Err(anyhow!(
                "{program} {command_kind} timed out after {timeout_secs}s"
            ));
        }

        thread::sleep(Duration::from_millis(25));
    }
}

fn combined_output_string(output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stdout.is_empty() {
        stderr.to_string()
    } else if stderr.is_empty() {
        stdout.to_string()
    } else {
        format!("{stdout}{stderr}")
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::process::ExitStatusExt;
    use std::process::ExitStatus;

    use super::*;

    fn output(code: i32, stdout: &str, stderr: &str) -> Output {
        Output {
            status: ExitStatus::from_raw(code << 8),
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    fn signalled_output(signal: i32, stdout: &str, stderr: &str) -> Output {
        Output {
            status: ExitStatus::from_raw(signal),
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    #[test]
    fn run_opencode_session_delete_accepts_missing_session_output() {
        let result = run_opencode_session_delete_with_runner("missing-session", |args, kind| {
            assert_eq!(args, ["session", "delete", "missing-session"]);
            assert_eq!(kind, "delete");
            Ok(output(1, "", "Session not found"))
        });

        assert!(result.is_ok());
    }

    #[test]
    fn run_opencode_session_delete_surfaces_failure_output() {
        let err = run_opencode_session_delete_with_runner("session-123", |_, _| {
            Ok(output(7, "stdout: ", "delete failed"))
        })
        .expect_err("delete failure should bubble up");

        let message = err.to_string();
        assert!(message.contains("opencode session delete failed"));
        assert!(message.contains("exit status 7"));
        assert!(message.contains("stdout: delete failed"));
    }

    #[test]
    fn run_opencode_import_surfaces_signal_termination() {
        let err = run_opencode_import_with_runner("/tmp/export.json", |args, kind| {
            assert_eq!(args, ["import", "/tmp/export.json"]);
            assert_eq!(kind, "import");
            Ok(signalled_output(9, "", "terminated"))
        })
        .expect_err("signal termination should bubble up");

        let message = err.to_string();
        assert!(message.contains("opencode import failed"));
        assert!(message.contains("terminated by signal"));
        assert!(message.contains("terminated"));
    }

    #[test]
    fn combined_output_string_prefers_available_streams() {
        assert_eq!(combined_output_string(&output(0, "ok", "")), "ok");
        assert_eq!(combined_output_string(&output(0, "", "warn")), "warn");
        assert_eq!(combined_output_string(&output(0, "out", "err")), "outerr");
    }

    #[test]
    fn run_command_with_timeout_collects_output() {
        let output = run_command_with_timeout(
            "/bin/sh",
            &["-c", "printf stdout; printf stderr >&2"],
            "smoke-test",
            Duration::from_secs(1),
        )
        .expect("shell command should succeed");

        assert!(output.status.success());
        assert_eq!(combined_output_string(&output), "stdoutstderr");
    }

    #[test]
    fn run_command_with_timeout_times_out() {
        let err = run_command_with_timeout(
            "/bin/sh",
            &["-c", "sleep 1"],
            "sleep-test",
            Duration::from_millis(50),
        )
        .expect_err("long-running command should time out");

        let message = err.to_string();
        assert!(message.contains("/bin/sh sleep-test timed out"));
        assert!(message.contains("0.05"));
    }

    #[test]
    fn run_command_with_timeout_reports_spawn_failures() {
        let err = run_command_with_timeout(
            "/path/that/does/not/exist/opencode",
            &[],
            "spawn-test",
            Duration::from_secs(1),
        )
        .expect_err("missing program should fail to spawn");

        assert!(err.to_string().contains("failed to spawn"));
    }
}

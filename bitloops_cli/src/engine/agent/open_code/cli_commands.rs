use std::time::Duration;

use anyhow::{Result, anyhow};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::Instant;

pub const OPENCODE_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

pub fn run_opencode_session_delete(session_id: &str) -> Result<()> {
    let output = run_opencode_command_with_timeout(&["session", "delete", session_id], "delete")?;
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
    let output = run_opencode_command_with_timeout(&["import", export_file_path], "import")?;
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
    let mut child = Command::new("opencode")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| anyhow!("failed to spawn opencode command: {err}"))?;

    let start = Instant::now();
    loop {
        if child
            .try_wait()
            .map_err(|err| anyhow!("failed waiting for opencode command: {err}"))?
            .is_some()
        {
            return child
                .wait_with_output()
                .map_err(|err| anyhow!("failed collecting opencode command output: {err}"));
        }

        if start.elapsed() >= OPENCODE_COMMAND_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err(anyhow!(
                "opencode {command_kind} timed out after {}s",
                OPENCODE_COMMAND_TIMEOUT.as_secs()
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

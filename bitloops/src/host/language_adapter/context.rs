use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageCommandOutput {
    pub success: bool,
    pub combined_output: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageAdapterContext {
    pub repo_root: PathBuf,
    pub repo_id: String,
    pub commit_sha: Option<String>,
    pub command_timeout: Duration,
}

impl LanguageAdapterContext {
    pub fn new(repo_root: PathBuf, repo_id: impl Into<String>, commit_sha: Option<String>) -> Self {
        Self {
            repo_root,
            repo_id: repo_id.into(),
            commit_sha,
            command_timeout: Duration::from_secs(30),
        }
    }

    pub fn run_command_capture(
        &self,
        program: &str,
        args: &[&str],
    ) -> Result<LanguageCommandOutput> {
        let mut child = Command::new(program)
            .current_dir(&self.repo_root)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                anyhow!(
                    "failed to execute `{program} {}` in {}: {error}",
                    args.join(" "),
                    self.repo_root.display()
                )
            })?;
        let stdout_reader = spawn_stream_capture(child.stdout.take());
        let stderr_reader = spawn_stream_capture(child.stderr.take());

        let deadline = Instant::now() + self.command_timeout;
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let combined_output =
                        collect_combined_output(program, stdout_reader, stderr_reader)?;
                    return Ok(LanguageCommandOutput {
                        success: status.success(),
                        combined_output,
                    });
                }
                Ok(None) if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let combined_output =
                        collect_combined_output(program, stdout_reader, stderr_reader)
                            .unwrap_or_default();
                    return Err(anyhow!(
                        "timed out after {}s while running `{program} {}`{}",
                        self.command_timeout.as_secs(),
                        args.join(" "),
                        if combined_output.trim().is_empty() {
                            String::new()
                        } else {
                            format!(" ({})", combined_output.replace('\n', " "))
                        }
                    ));
                }
                Ok(None) => thread::sleep(Duration::from_millis(200)),
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = collect_combined_output(program, stdout_reader, stderr_reader);
                    return Err(anyhow!("failed polling `{program}` process: {error}"));
                }
            }
        }
    }
}

fn spawn_stream_capture<R>(stream: Option<R>) -> JoinHandle<std::io::Result<Vec<u8>>>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut buffer = Vec::new();
        if let Some(mut stream) = stream {
            stream.read_to_end(&mut buffer)?;
        }
        Ok(buffer)
    })
}

fn collect_combined_output(
    program: &str,
    stdout_reader: JoinHandle<std::io::Result<Vec<u8>>>,
    stderr_reader: JoinHandle<std::io::Result<Vec<u8>>>,
) -> Result<String> {
    let stdout = join_stream_capture(program, "stdout", stdout_reader)?;
    let stderr = join_stream_capture(program, "stderr", stderr_reader)?;
    Ok(format!(
        "{}\n{}",
        String::from_utf8_lossy(&stdout),
        String::from_utf8_lossy(&stderr)
    ))
}

fn join_stream_capture(
    program: &str,
    stream_name: &str,
    reader: JoinHandle<std::io::Result<Vec<u8>>>,
) -> Result<Vec<u8>> {
    match reader.join() {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(error)) => Err(anyhow!(
            "failed reading `{program}` {stream_name} stream: {error}"
        )),
        Err(_) => Err(anyhow!("`{program}` {stream_name} capture thread panicked")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_context(timeout: Duration) -> LanguageAdapterContext {
        let mut context = LanguageAdapterContext::new(
            std::env::current_dir().expect("current directory"),
            "repo-id",
            None,
        );
        context.command_timeout = timeout;
        context
    }

    fn run_shell_command(
        ctx: &LanguageAdapterContext,
        script: &str,
    ) -> Result<LanguageCommandOutput> {
        let (program, args) = shell_command(script);
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        ctx.run_command_capture(program, &arg_refs)
    }

    #[cfg(unix)]
    fn shell_command(script: &str) -> (&'static str, Vec<String>) {
        ("sh", vec!["-c".to_string(), script.to_string()])
    }

    #[cfg(windows)]
    fn shell_command(script: &str) -> (&'static str, Vec<String>) {
        (
            "powershell",
            vec![
                "-NoProfile".to_string(),
                "-Command".to_string(),
                script.to_string(),
            ],
        )
    }

    #[cfg(unix)]
    const SMALL_OUTPUT_SCRIPT: &str = "printf stdout; printf stderr >&2";
    #[cfg(windows)]
    const SMALL_OUTPUT_SCRIPT: &str =
        "[Console]::Out.Write('stdout'); [Console]::Error.Write('stderr')";

    #[cfg(unix)]
    const LARGE_OUTPUT_SCRIPT: &str = "awk 'BEGIN { for (i = 0; i < 2097152; i++) printf \"x\" }'";
    #[cfg(windows)]
    const LARGE_OUTPUT_SCRIPT: &str = "[Console]::Out.Write(('x' * 2097152))";

    #[cfg(unix)]
    const TIMEOUT_SCRIPT: &str = "printf partial; sleep 1";
    #[cfg(windows)]
    const TIMEOUT_SCRIPT: &str = "[Console]::Out.Write('partial'); Start-Sleep -Seconds 1";

    #[test]
    fn run_command_capture_collects_stdout_and_stderr() {
        let output = run_shell_command(&test_context(Duration::from_secs(2)), SMALL_OUTPUT_SCRIPT)
            .expect("command should succeed");

        assert!(output.success);
        assert!(output.combined_output.contains("stdout"));
        assert!(output.combined_output.contains("stderr"));
    }

    #[test]
    fn run_command_capture_handles_large_output_without_timing_out() {
        let output = run_shell_command(&test_context(Duration::from_secs(5)), LARGE_OUTPUT_SCRIPT)
            .expect("large output command should complete");

        assert!(output.success);
        assert!(output.combined_output.len() >= 2_097_152);
    }

    #[test]
    fn run_command_capture_timeout_includes_captured_output() {
        let error = run_shell_command(&test_context(Duration::from_millis(100)), TIMEOUT_SCRIPT)
            .expect_err("command should time out");
        let message = error.to_string();

        assert!(message.contains("timed out"));
        assert!(message.contains("partial"));
    }
}

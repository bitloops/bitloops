use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
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

        let deadline = Instant::now() + self.command_timeout;
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let output = child.wait_with_output().map_err(|error| {
                        anyhow!("failed waiting for `{program}` process output: {error}")
                    })?;
                    let combined_output = format!(
                        "{}\n{}",
                        String::from_utf8_lossy(&output.stdout),
                        String::from_utf8_lossy(&output.stderr)
                    );
                    return Ok(LanguageCommandOutput {
                        success: status.success(),
                        combined_output,
                    });
                }
                Ok(None) if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let output = child.wait_with_output().ok();
                    let combined_output = output
                        .map(|output| {
                            format!(
                                "{}\n{}",
                                String::from_utf8_lossy(&output.stdout),
                                String::from_utf8_lossy(&output.stderr)
                            )
                        })
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
                    return Err(anyhow!("failed polling `{program}` process: {error}"));
                }
            }
        }
    }
}

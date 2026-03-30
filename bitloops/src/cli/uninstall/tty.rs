#[cfg(not(test))]
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::{env, fs};

use anyhow::{Result, anyhow, bail};

pub(super) fn can_prompt_interactively() -> bool {
    if let Ok(value) = env::var("BITLOOPS_TEST_TTY") {
        return value == "1" && command_exists("stty");
    }

    #[cfg(test)]
    {
        false
    }

    #[cfg(not(test))]
    {
        if io::stdin().is_terminal() && io::stdout().is_terminal() && command_exists("stty") {
            return true;
        }

        fs::OpenOptions::new().read(true).open("/dev/tty").is_ok() && command_exists("stty")
    }
}

pub(super) struct SttyRawMode {
    original_mode: String,
}

impl SttyRawMode {
    pub(super) fn enter() -> Result<Self> {
        let tty = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/tty")
            .map_err(|err| anyhow!("failed to open tty: {err}"))?;

        let output = Command::new("stty")
            .arg("-g")
            .stdin(Stdio::from(
                tty.try_clone()
                    .map_err(|err| anyhow!("failed to clone tty handle: {err}"))?,
            ))
            .output()
            .map_err(|err| anyhow!("failed to read tty mode: {err}"))?;
        if !output.status.success() {
            bail!("failed to read tty mode");
        }

        let original_mode = String::from_utf8(output.stdout)
            .map_err(|err| anyhow!("failed to parse tty mode: {err}"))?
            .trim()
            .to_string();

        let status = Command::new("stty")
            .args(["-icanon", "-echo", "min", "1", "time", "0"])
            .stdin(Stdio::from(tty))
            .status()
            .map_err(|err| anyhow!("failed to set raw tty mode: {err}"))?;
        if !status.success() {
            bail!("failed to set raw tty mode");
        }

        Ok(Self { original_mode })
    }
}

impl Drop for SttyRawMode {
    fn drop(&mut self) {
        if let Ok(tty) = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/tty")
        {
            let _ = Command::new("stty")
                .arg(self.original_mode.clone())
                .stdin(Stdio::from(tty))
                .status();
        }
    }
}

fn command_exists(program: &str) -> bool {
    let Some(path) = env::var_os("PATH") else {
        return false;
    };

    env::split_paths(&path).any(|dir| {
        let candidate = dir.join(program);
        candidate.is_file()
            || executable_with_extensions(&dir, program)
                .iter()
                .any(|candidate| candidate.is_file())
    })
}

fn executable_with_extensions(dir: &Path, program: &str) -> [PathBuf; 3] {
    [
        dir.join(format!("{program}.exe")),
        dir.join(format!("{program}.cmd")),
        dir.join(format!("{program}.bat")),
    ]
}

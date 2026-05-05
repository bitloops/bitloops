use std::io::{self, BufRead, BufReader, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::{env, fs};

use anyhow::{Context, Result, bail};

pub const SHELL_COMPLETION_COMMENT: &str = "# Bitloops CLI shell completion";

pub fn shell_completion_target(home: &Path) -> Result<(String, PathBuf, String)> {
    let shell = env::var("SHELL").unwrap_or_default();
    if shell.contains("zsh") {
        return Ok((
            "Zsh".to_string(),
            home.join(".zshrc"),
            "autoload -Uz compinit && compinit && source <(bitloops completion zsh)".to_string(),
        ));
    }
    if shell.contains("bash") {
        let mut rc = home.join(".bashrc");
        if home.join(".bash_profile").exists() {
            rc = home.join(".bash_profile");
        }
        return Ok((
            "Bash".to_string(),
            rc,
            "source <(bitloops completion bash)".to_string(),
        ));
    }
    if shell.contains("fish") {
        return Ok((
            "Fish".to_string(),
            home.join(".config").join("fish").join("config.fish"),
            "bitloops completion fish | source".to_string(),
        ));
    }
    bail!("unsupported shell")
}

pub fn append_shell_completion(rc_file: &Path, completion_line: &str) -> Result<()> {
    if let Some(parent) = rc_file.parent() {
        fs::create_dir_all(parent).context("creating shell rc directory")?;
    }
    let mut f = fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(rc_file)
        .with_context(|| format!("opening {}", rc_file.display()))?;
    writeln!(f)?;
    writeln!(f, "{SHELL_COMPLETION_COMMENT}")?;
    writeln!(f, "{completion_line}")?;
    Ok(())
}

fn is_completion_configured(rc_file: &Path) -> bool {
    fs::read_to_string(rc_file)
        .map(|content| content.contains("bitloops completion"))
        .unwrap_or(false)
}

fn prompt_enable_shell_completion(
    w: &mut dyn Write,
    input: &mut dyn BufRead,
    shell_name: &str,
) -> Result<bool> {
    write!(
        w,
        "Enable shell completion? (detected: {shell_name}) [y/N]: "
    )?;
    w.flush()?;

    let mut line = String::new();
    let read = input
        .read_line(&mut line)
        .context("reading shell completion prompt response")?;
    if read == 0 {
        return Ok(false);
    }

    let answer = line.trim().to_ascii_lowercase();
    Ok(matches!(answer.as_str(), "y" | "yes"))
}

pub(crate) fn run_post_install_shell_completion_with_io(
    w: &mut dyn Write,
    input: &mut dyn BufRead,
) -> Result<()> {
    let home = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?;

    let (shell_name, rc_file, completion_line) = match shell_completion_target(&home) {
        Ok(target) => target,
        Err(err) if err.to_string().contains("unsupported shell") => {
            writeln!(
                w,
                "Note: Shell completion not available for your shell. Supported: zsh, bash, fish."
            )?;
            return Ok(());
        }
        Err(err) => return Err(err),
    };

    if is_completion_configured(&rc_file) {
        writeln!(
            w,
            "✓ Shell completion already configured in {}",
            rc_file.display()
        )?;
        return Ok(());
    }

    if !prompt_enable_shell_completion(w, input, &shell_name)? {
        return Ok(());
    }

    append_shell_completion(&rc_file, &completion_line)
        .with_context(|| format!("failed to update {}", rc_file.display()))?;
    writeln!(w, "✓ Shell completion added to {}", rc_file.display())?;
    writeln!(w, "  Restart your shell to activate")?;
    Ok(())
}

pub fn run_post_install_shell_completion(w: &mut dyn Write) -> Result<()> {
    let stdin = io::stdin();
    if !stdin.is_terminal() {
        writeln!(
            w,
            "Note: Shell completion setup skipped: non-interactive environment."
        )?;
        return Ok(());
    }

    let mut input = BufReader::new(stdin.lock());
    run_post_install_shell_completion_with_io(w, &mut input)
}

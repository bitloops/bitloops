use std::fs;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};

use crate::cli::enable::SHELL_COMPLETION_COMMENT;
use crate::utils::platform_dirs::bitloops_home_dir;

pub(super) fn uninstall_shell_integration(out: &mut dyn Write) -> Result<()> {
    let home = bitloops_home_dir()?;
    let rc_files = [
        home.join(".zshrc"),
        home.join(".bashrc"),
        home.join(".bash_profile"),
        home.join(".config").join("fish").join("config.fish"),
    ];

    let mut touched = 0usize;
    for rc_file in rc_files {
        if cleanup_shell_file(&rc_file)? {
            writeln!(
                out,
                "  Removed shell integration from {}",
                rc_file.display()
            )?;
            touched += 1;
        }
    }

    if touched == 0 {
        writeln!(out, "  No managed shell integration found.")?;
    }

    Ok(())
}

fn cleanup_shell_file(path: &Path) -> Result<bool> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err).with_context(|| format!("reading {}", path.display())),
    };

    let mut lines = Vec::new();
    let source_lines = content.lines().collect::<Vec<_>>();
    let mut idx = 0usize;
    let mut changed = false;

    while idx < source_lines.len() {
        let line = source_lines[idx];
        if line.trim() == SHELL_COMPLETION_COMMENT {
            changed = true;
            idx += 1;
            if idx < source_lines.len() && source_lines[idx].trim().contains("bitloops completion")
            {
                idx += 1;
            }
            while idx < source_lines.len() && source_lines[idx].trim().is_empty() {
                idx += 1;
            }
            continue;
        }

        if line.trim().contains("bitloops completion") {
            changed = true;
            idx += 1;
            continue;
        }

        lines.push(line);
        idx += 1;
    }

    if !changed {
        return Ok(false);
    }

    let mut rewritten = lines.join("\n");
    if !rewritten.is_empty() {
        rewritten.push('\n');
        fs::write(path, rewritten).with_context(|| format!("writing {}", path.display()))?;
    } else {
        fs::remove_file(path).with_context(|| format!("removing {}", path.display()))?;
    }

    Ok(true)
}

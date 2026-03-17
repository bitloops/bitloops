use std::collections::BTreeSet;
use std::io::{self, IsTerminal, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::{env, fs};

use anyhow::{Result, anyhow, bail};

use crate::commands::enable::initialized_agents;

use super::agent_hooks::{DEFAULT_AGENT, agent_display, available_agents, detect_agents};

pub fn can_prompt_interactively() -> bool {
    if let Ok(v) = env::var("BITLOOPS_TEST_TTY") {
        return v == "1";
    }
    if io::stdin().is_terminal() && io::stdout().is_terminal() {
        return true;
    }
    fs::OpenOptions::new().read(true).open("/dev/tty").is_ok()
}

pub fn detect_or_select_agent(
    repo_root: &Path,
    out: &mut dyn Write,
    select_fn: Option<&super::AgentSelector>,
) -> Result<Vec<String>> {
    let detected = detect_agents(repo_root);
    let installed = initialized_agents(repo_root);

    if detected.len() == 1 {
        writeln!(out, "Detected agent: {}", agent_display(&detected[0]))?;
        writeln!(out)?;
    }

    if detected.len() > 1 {
        let labels = detected
            .iter()
            .map(|a| agent_display(a))
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(out, "Detected multiple agents: {labels}")?;
        writeln!(out)?;
    }

    if !can_prompt_interactively() {
        if !detected.is_empty() {
            return Ok(detected);
        }
        if !installed.is_empty() {
            return Ok(installed);
        }
        writeln!(out, "Agent: {} (default)", agent_display(DEFAULT_AGENT))?;
        writeln!(out)?;
        return Ok(vec![DEFAULT_AGENT.to_string()]);
    }

    if detected.is_empty() {
        writeln!(
            out,
            "No agent configuration detected (e.g., .claude, .codex, .cursor, .gemini, or .opencode)."
        )?;
        writeln!(
            out,
            "This is normal - some agents don't require a config directory."
        )?;
        writeln!(out)?;
    }

    let available = available_agents();
    let defaults = if !installed.is_empty() {
        installed
    } else if !detected.is_empty() {
        detected.clone()
    } else {
        vec![DEFAULT_AGENT.to_string()]
    };

    let mut selected = match select_fn {
        Some(select) => select(&available).map_err(|e| anyhow!(e))?,
        None => prompt_select_agents(&available, &defaults, out)?,
    };

    if selected.is_empty() {
        bail!("no agents selected");
    }

    let available_set: BTreeSet<&str> = available.iter().map(String::as_str).collect();
    for name in &selected {
        if !available_set.contains(name.as_str()) {
            bail!("failed to get selected agent {name}");
        }
    }

    let mut seen = BTreeSet::new();
    selected.retain(|name| seen.insert(name.clone()));

    let labels = selected
        .iter()
        .map(|s| agent_display(s))
        .collect::<Vec<_>>()
        .join(", ");
    writeln!(out, "Selected agents: {labels}")?;
    writeln!(out)?;
    Ok(selected)
}

fn prompt_select_agents(
    available: &[String],
    defaults: &[String],
    out: &mut dyn Write,
) -> Result<Vec<String>> {
    let default_set: BTreeSet<&str> = defaults.iter().map(String::as_str).collect();
    let labels: Vec<String> = available
        .iter()
        .map(|agent| agent_display(agent).to_string())
        .collect();

    let mut selected: Vec<bool> = available
        .iter()
        .map(|agent| default_set.contains(agent.as_str()))
        .collect();

    if selected.iter().all(|is_selected| !is_selected) && !selected.is_empty() {
        selected[0] = true;
    }

    let mut cursor = 0usize;
    let mut tty_in = fs::OpenOptions::new().read(true).open("/dev/tty")?;
    let _raw_mode = SttyRawMode::enter()?;
    let mut rendered_lines = render_agent_picker(out, &labels, &selected, cursor, None)?;

    loop {
        match read_key(&mut tty_in)? {
            Key::Up => {
                cursor = cursor.saturating_sub(1);
            }
            Key::Down => {
                if cursor + 1 < labels.len() {
                    cursor += 1;
                }
            }
            Key::Toggle => {
                if !selected.is_empty() {
                    selected[cursor] = !selected[cursor];
                }
            }
            Key::SelectAll => {
                let all_selected = selected.iter().all(|is_selected| *is_selected);
                selected.fill(!all_selected);
            }
            Key::Cancel => bail!("cancelled by user"),
            Key::Submit => break,
            Key::Unknown => {}
        }
        rendered_lines =
            render_agent_picker(out, &labels, &selected, cursor, Some(rendered_lines))?;
    }

    writeln!(out)?;
    out.flush()?;

    let selected_agents: Vec<String> = selected
        .into_iter()
        .enumerate()
        .filter_map(|(idx, is_selected)| {
            if is_selected {
                Some(available[idx].clone())
            } else {
                None
            }
        })
        .collect();

    if selected_agents.is_empty() {
        bail!("no agents selected");
    }
    Ok(selected_agents)
}

#[derive(Clone, Copy)]
enum Key {
    Up,
    Down,
    Toggle,
    SelectAll,
    Cancel,
    Submit,
    Unknown,
}

struct SttyRawMode {
    original_mode: String,
}

impl SttyRawMode {
    fn enter() -> Result<Self> {
        let tty = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/tty")
            .map_err(|e| anyhow!("failed to open tty: {e}"))?;

        let output = Command::new("stty")
            .arg("-g")
            .stdin(Stdio::from(
                tty.try_clone()
                    .map_err(|e| anyhow!("failed to clone tty handle: {e}"))?,
            ))
            .output()
            .map_err(|e| anyhow!("failed to read tty mode: {e}"))?;
        if !output.status.success() {
            bail!("failed to read tty mode");
        }

        let original_mode = String::from_utf8(output.stdout)
            .map_err(|e| anyhow!("failed to parse tty mode: {e}"))?
            .trim()
            .to_string();

        let status = Command::new("stty")
            .args(["-icanon", "-echo", "min", "1", "time", "0"])
            .stdin(Stdio::from(tty))
            .status()
            .map_err(|e| anyhow!("failed to set raw tty mode: {e}"))?;
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

fn render_agent_picker(
    out: &mut dyn Write,
    labels: &[String],
    selected: &[bool],
    cursor: usize,
    previous_lines: Option<usize>,
) -> Result<usize> {
    let mut lines = Vec::new();
    lines.push("Which agents are you using?".to_string());
    lines.push("Use space to select, enter to confirm.".to_string());
    lines.push(String::new());
    for (idx, label) in labels.iter().enumerate() {
        let pointer = if idx == cursor {
            "\x1b[38;2;116;4;228m>\x1b[0m"
        } else {
            " "
        };

        if selected[idx] {
            lines.push(format!("{pointer} \x1b[38;2;116;4;228m[•] {label}\x1b[0m"));
        } else {
            lines.push(format!("{pointer} [ ] {label}"));
        }
    }
    lines.push(String::new());
    lines.push("x toggle • ↑/↓ move • enter submit • ctrl+a all".to_string());

    if let Some(previous_lines) = previous_lines {
        if previous_lines > 1 {
            write!(out, "\x1b[{}F", previous_lines - 1)?;
        } else {
            write!(out, "\r")?;
        }
    }

    for (idx, line) in lines.iter().enumerate() {
        write!(out, "\r\x1b[2K{line}")?;
        if idx + 1 < lines.len() {
            writeln!(out)?;
        }
    }
    out.flush()?;
    Ok(lines.len())
}

fn read_key(input: &mut dyn Read) -> Result<Key> {
    let mut first = [0u8; 1];
    input.read_exact(&mut first)?;
    match first[0] {
        3 => Ok(Key::Cancel),
        b' ' | b'x' => Ok(Key::Toggle),
        b'\r' | b'\n' => Ok(Key::Submit),
        1 => Ok(Key::SelectAll),
        b'k' => Ok(Key::Up),
        b'j' => Ok(Key::Down),
        27 => {
            let mut seq = [0u8; 2];
            if input.read_exact(&mut seq).is_err() {
                return Ok(Key::Unknown);
            }
            if seq == [b'[', b'A'] {
                Ok(Key::Up)
            } else if seq == [b'[', b'B'] {
                Ok(Key::Down)
            } else {
                Ok(Key::Unknown)
            }
        }
        _ => Ok(Key::Unknown),
    }
}

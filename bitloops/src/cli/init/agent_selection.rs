use std::collections::BTreeSet;
#[cfg(not(test))]
use std::io::{self, IsTerminal};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::{env, fs};

use anyhow::{Result, anyhow, bail};

use crate::utils::branding::{BITLOOPS_PURPLE_HEX, color_hex_if_enabled, should_use_color_output};

use super::agent_hooks::{DEFAULT_AGENT, agent_display, available_agents, detect_agents};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitAgentSelection {
    pub agents: Vec<String>,
    pub enable_devql_guidance: bool,
}

pub fn can_prompt_interactively() -> bool {
    #[cfg(test)]
    if let Some(v) = crate::cli::telemetry_consent::test_tty_override() {
        return v && command_exists("stty");
    }
    if let Ok(v) = env::var("BITLOOPS_TEST_TTY") {
        return v == "1" && command_exists("stty");
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

pub fn detect_or_select_agent(
    repo_root: &Path,
    out: &mut dyn Write,
    default_enable_devql_guidance: bool,
    select_fn: Option<&super::AgentSelector>,
) -> Result<InitAgentSelection> {
    let policy_agents = policy_supported_agents(repo_root)?;
    let detected = matches!(
        policy_agents,
        PolicySupportedAgents::Unconfigured | PolicySupportedAgents::PolicyWithoutSupported
    )
    .then(|| detect_agents(repo_root))
    .unwrap_or_default();

    if !can_prompt_interactively() {
        match policy_agents {
            PolicySupportedAgents::Configured(configured) => {
                if configured.is_empty() {
                    bail!(
                        "no supported agents configured in the discovered Bitloops repo policy; rerun `bitloops init` interactively or pass `--agent`"
                    );
                }

                return Ok(InitAgentSelection {
                    agents: configured,
                    enable_devql_guidance: default_enable_devql_guidance,
                });
            }
            PolicySupportedAgents::PolicyWithoutSupported if !detected.is_empty() => {
                bail!(
                    "no supported agents configured in the discovered Bitloops repo policy; rerun `bitloops init` interactively or pass `--agent`"
                );
            }
            PolicySupportedAgents::Unconfigured | PolicySupportedAgents::PolicyWithoutSupported => {
            }
        }
        if !detected.is_empty() {
            return Ok(InitAgentSelection {
                agents: detected,
                enable_devql_guidance: default_enable_devql_guidance,
            });
        }
        writeln!(out, "Agent: {} (default)", agent_display(DEFAULT_AGENT))?;
        writeln!(out)?;
        return Ok(InitAgentSelection {
            agents: vec![DEFAULT_AGENT.to_string()],
            enable_devql_guidance: default_enable_devql_guidance,
        });
    }

    let available = available_agents();
    let defaults = match policy_agents {
        PolicySupportedAgents::Configured(configured) => configured,
        PolicySupportedAgents::Unconfigured | PolicySupportedAgents::PolicyWithoutSupported
            if !detected.is_empty() =>
        {
            detected.clone()
        }
        PolicySupportedAgents::Unconfigured | PolicySupportedAgents::PolicyWithoutSupported => {
            vec![DEFAULT_AGENT.to_string()]
        }
    };

    let mut selected = match select_fn {
        Some(select) => {
            select(&available, default_enable_devql_guidance).map_err(|e| anyhow!(e))?
        }
        None => prompt_select_agents(&available, &defaults, default_enable_devql_guidance, out)?,
    };

    if selected.agents.is_empty() {
        bail!("no agents selected");
    }

    let available_set: BTreeSet<&str> = available.iter().map(String::as_str).collect();
    for name in &selected.agents {
        if !available_set.contains(name.as_str()) {
            bail!("failed to get selected agent {name}");
        }
    }

    let mut seen = BTreeSet::new();
    selected.agents.retain(|name| seen.insert(name.clone()));
    Ok(selected)
}

enum PolicySupportedAgents {
    Unconfigured,
    PolicyWithoutSupported,
    Configured(Vec<String>),
}

fn prompt_select_agents(
    available: &[String],
    defaults: &[String],
    default_enable_devql_guidance: bool,
    out: &mut dyn Write,
) -> Result<InitAgentSelection> {
    let default_set: BTreeSet<&str> = defaults.iter().map(String::as_str).collect();
    let labels: Vec<String> = available
        .iter()
        .map(|agent| agent_display(agent).to_string())
        .collect();

    let mut selected: Vec<bool> = available
        .iter()
        .map(|agent| default_set.contains(agent.as_str()))
        .collect();

    let mut cursor = 0usize;
    let mut enable_devql_guidance = default_enable_devql_guidance;
    let mut tty_in = fs::OpenOptions::new().read(true).open("/dev/tty")?;
    let _raw_mode = SttyRawMode::enter()?;
    let mut rendered_lines =
        render_agent_picker(out, &labels, &selected, enable_devql_guidance, cursor, None)?;

    loop {
        match read_key(&mut tty_in)? {
            Key::Up => {
                cursor = cursor.saturating_sub(1);
            }
            Key::Down => {
                if cursor < labels.len() {
                    cursor += 1;
                }
            }
            Key::Toggle => {
                if cursor == labels.len() {
                    enable_devql_guidance = !enable_devql_guidance;
                } else if !selected.is_empty() {
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
        rendered_lines = render_agent_picker(
            out,
            &labels,
            &selected,
            enable_devql_guidance,
            cursor,
            Some(rendered_lines),
        )?;
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
    Ok(InitAgentSelection {
        agents: selected_agents,
        enable_devql_guidance,
    })
}

fn policy_supported_agents(repo_root: &Path) -> Result<PolicySupportedAgents> {
    let policy = crate::config::discover_repo_policy_optional(repo_root)?;
    if policy.root.is_none() {
        return Ok(PolicySupportedAgents::Unconfigured);
    }

    if policy.agents.get("supported").is_none() {
        return Ok(PolicySupportedAgents::PolicyWithoutSupported);
    }

    crate::config::settings::supported_agents_from_policy(&policy)
        .map(PolicySupportedAgents::Configured)
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
    enable_devql_guidance: bool,
    cursor: usize,
    previous_lines: Option<usize>,
) -> Result<usize> {
    let mut lines = Vec::new();
    lines.push("Select agents to integrate:".to_string());
    lines.push(style_picker_hint("Use space to select, enter to confirm."));
    lines.push(String::new());
    for (idx, label) in labels.iter().enumerate() {
        let pointer = if idx == cursor {
            color_hex_if_enabled(">", BITLOOPS_PURPLE_HEX)
        } else {
            " ".to_string()
        };
        let checkbox = if selected[idx] {
            selected_picker_checkbox()
        } else {
            "[ ]".to_string()
        };
        let label = if selected[idx] {
            selected_picker_label(label)
        } else {
            label.clone()
        };
        lines.push(format!("{pointer} {checkbox} {label}"));
    }
    lines.push(String::new());
    let pointer = if cursor == labels.len() {
        color_hex_if_enabled(">", BITLOOPS_PURPLE_HEX)
    } else {
        " ".to_string()
    };
    let checkbox = if enable_devql_guidance {
        selected_picker_checkbox()
    } else {
        "[ ]".to_string()
    };
    let label = if enable_devql_guidance {
        selected_picker_label("Enable DevQL Guidance")
    } else {
        "Enable DevQL Guidance".to_string()
    };
    lines.push(format!("{pointer} {checkbox} {label}"));
    lines.push(String::new());
    lines.push(format!(
        "space {} • ↑/↓ {} • enter {}",
        style_picker_hint("toggle"),
        style_picker_hint("move"),
        style_picker_hint("submit")
    ));

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

fn style_picker_hint(detail: &str) -> String {
    if should_use_color_output() {
        format!("\x1b[2;3m{detail}\x1b[0m")
    } else {
        detail.to_string()
    }
}

fn selected_picker_checkbox() -> String {
    const SELECTION_WHITE_HEX: &str = "#ffffff";
    format!(
        "{}{}{}",
        color_hex_if_enabled("[", SELECTION_WHITE_HEX),
        color_hex_if_enabled("•", BITLOOPS_PURPLE_HEX),
        color_hex_if_enabled("]", SELECTION_WHITE_HEX)
    )
}

fn selected_picker_label(label: &str) -> String {
    const SELECTION_WHITE_HEX: &str = "#ffffff";
    color_hex_if_enabled(label, SELECTION_WHITE_HEX)
}

fn read_key(input: &mut dyn Read) -> Result<Key> {
    let mut first = [0u8; 1];
    input.read_exact(&mut first)?;
    match first[0] {
        3 => Ok(Key::Cancel),
        b' ' => Ok(Key::Toggle),
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

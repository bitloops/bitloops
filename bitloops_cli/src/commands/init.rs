use std::collections::BTreeSet;
use std::io::{self, BufRead, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::{env, fs};

use anyhow::{Result, anyhow, bail};
use clap::Args;

use crate::commands::enable::{find_repo_root, initialized_agents};
use crate::engine::agent::HookSupport;
use crate::engine::agent::claude_code::hooks as claude_hooks;
use crate::engine::agent::cursor::agent::CursorAgent;
use crate::engine::agent::gemini_cli::agent::GeminiCliAgent;
use crate::engine::agent::open_code::agent::OpenCodeAgent;
use crate::engine::settings;

const AGENT_CLAUDE_CODE: &str = "claude-code";
const AGENT_CURSOR: &str = "cursor";
const AGENT_GEMINI_CLI: &str = "gemini-cli";
const AGENT_OPEN_CODE: &str = "opencode";
const DEFAULT_AGENT: &str = AGENT_CLAUDE_CODE;
const TELEMETRY_OPTOUT_ENV: &str = "BITLOOPS_TELEMETRY_OPTOUT";

pub type AgentSelector = dyn Fn(&[String]) -> std::result::Result<Vec<String>, String>;

#[derive(Args)]
pub struct InitArgs {
    /// Remove and reinstall existing hooks for selected agents
    #[arg(long, short = 'f')]
    pub force: bool,

    /// Target a specific agent setup (claude-code|cursor|gemini-cli|opencode)
    #[arg(long)]
    pub agent: Option<String>,

    /// Enable anonymous usage analytics
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub telemetry: bool,
}

pub async fn run(args: InitArgs) -> Result<()> {
    let mut out = io::stdout().lock();
    run_with_writer(args, &mut out, None)
}

fn run_with_writer(
    args: InitArgs,
    out: &mut dyn Write,
    select_fn: Option<&AgentSelector>,
) -> Result<()> {
    let cwd = env::current_dir()?;
    let repo_root = find_repo_root(&cwd)?;
    let local_dev = settings::load_settings(&repo_root)
        .unwrap_or_default()
        .local_dev;

    let selected_agents = if let Some(agent) = args.agent.as_deref() {
        vec![normalize_agent_name(agent)?]
    } else {
        detect_or_select_agent(&repo_root, out, select_fn)?
    };

    let mut total_installed = 0usize;
    let mut selected_labels = Vec::new();

    for agent in &selected_agents {
        let (label, count) = install_agent_hooks(&repo_root, agent, local_dev, args.force)?;
        selected_labels.push(label.clone());
        total_installed += count;
        if count > 0 {
            writeln!(out, "Installed {count} {label} hook(s).")?;
        } else {
            writeln!(out, "{label} hooks are already initialized.")?;
        }
    }

    maybe_capture_telemetry_consent(&repo_root, args.telemetry, args.agent.is_none(), out)?;

    writeln!(out)?;
    writeln!(out, "Initialized agents: {}", selected_labels.join(", "))?;
    writeln!(out, "Total hooks installed: {total_installed}")?;
    writeln!(out, "Bitloops agent initialization complete.")?;
    Ok(())
}

fn install_agent_hooks(
    repo_root: &Path,
    agent_name: &str,
    local_dev: bool,
    force: bool,
) -> Result<(String, usize)> {
    match agent_name {
        AGENT_CLAUDE_CODE => Ok((
            "Claude Code".to_string(),
            claude_hooks::install_hooks(repo_root, force)?,
        )),
        AGENT_CURSOR => Ok((
            "Cursor".to_string(),
            HookSupport::install_hooks(&CursorAgent, local_dev, force)?,
        )),
        AGENT_GEMINI_CLI => Ok((
            "Gemini CLI".to_string(),
            HookSupport::install_hooks(&GeminiCliAgent, local_dev, force)?,
        )),
        AGENT_OPEN_CODE => Ok((
            "OpenCode".to_string(),
            HookSupport::install_hooks(&OpenCodeAgent, local_dev, force)?,
        )),
        other => bail!("unknown agent name: {other}"),
    }
}

fn normalize_agent_name(value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("missing agent name");
    }
    match trimmed {
        AGENT_CLAUDE_CODE => Ok(AGENT_CLAUDE_CODE.to_string()),
        AGENT_CURSOR => Ok(AGENT_CURSOR.to_string()),
        AGENT_GEMINI_CLI | "gemini" => Ok(AGENT_GEMINI_CLI.to_string()),
        AGENT_OPEN_CODE | "open-code" => Ok(AGENT_OPEN_CODE.to_string()),
        _ => bail!("unknown agent name: {trimmed}"),
    }
}

fn detect_agents(repo_root: &Path) -> Vec<String> {
    let mut detected = Vec::new();
    if repo_root.join(".claude").is_dir() {
        detected.push(AGENT_CLAUDE_CODE.to_string());
    }
    if repo_root.join(".cursor").is_dir() {
        detected.push(AGENT_CURSOR.to_string());
    }
    if repo_root.join(".gemini").is_dir() {
        detected.push(AGENT_GEMINI_CLI.to_string());
    }
    if repo_root.join(".opencode").is_dir() {
        detected.push(AGENT_OPEN_CODE.to_string());
    }
    detected
}

fn available_agents() -> Vec<String> {
    vec![
        AGENT_CLAUDE_CODE.to_string(),
        AGENT_CURSOR.to_string(),
        AGENT_GEMINI_CLI.to_string(),
        AGENT_OPEN_CODE.to_string(),
    ]
}

fn agent_display(agent: &str) -> &'static str {
    match agent {
        AGENT_CLAUDE_CODE => "Claude Code",
        AGENT_CURSOR => "Cursor",
        AGENT_GEMINI_CLI => "Gemini CLI",
        AGENT_OPEN_CODE => "OpenCode",
        _ => "Unknown",
    }
}

pub fn can_prompt_interactively() -> bool {
    if let Ok(v) = env::var("BITLOOPS_TEST_TTY") {
        return v == "1";
    }
    if io::stdin().is_terminal() && io::stdout().is_terminal() {
        return true;
    }
    fs::OpenOptions::new().read(true).open("/dev/tty").is_ok()
}

fn telemetry_settings_target(repo_root: &Path) -> PathBuf {
    let local = settings::settings_local_path(repo_root);
    if local.exists() {
        local
    } else {
        settings::settings_path(repo_root)
    }
}

fn persist_telemetry_choice(repo_root: &Path, choice: bool) -> Result<()> {
    let mut merged = settings::load_settings(repo_root).unwrap_or_default();
    merged.telemetry = Some(choice);
    settings::save_settings(&merged, &telemetry_settings_target(repo_root))?;
    Ok(())
}

fn prompt_telemetry_consent(out: &mut dyn Write, input: &mut dyn BufRead) -> Result<bool> {
    writeln!(out)?;
    writeln!(out, "Help improve Bitloops CLI?")?;
    writeln!(
        out,
        "Share anonymous usage data. No code or personal info collected."
    )?;

    loop {
        write!(out, "Enable anonymous telemetry? [Y/n]: ")?;
        out.flush()?;

        let mut line = String::new();
        input.read_line(&mut line)?;
        let answer = line.trim().to_ascii_lowercase();
        match answer.as_str() {
            "" | "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => {
                writeln!(out, "Please answer yes or no.")?;
            }
        }
    }
}

fn maybe_capture_telemetry_consent(
    repo_root: &Path,
    telemetry_flag: bool,
    allow_prompt: bool,
    out: &mut dyn Write,
) -> Result<()> {
    if !telemetry_flag
        || env::var(TELEMETRY_OPTOUT_ENV)
            .ok()
            .is_some_and(|v| !v.trim().is_empty())
    {
        return persist_telemetry_choice(repo_root, false);
    }

    let existing = settings::load_settings(repo_root).unwrap_or_default();
    if existing.telemetry.is_some() {
        return Ok(());
    }

    if !allow_prompt || !can_prompt_interactively() {
        return Ok(());
    }

    let stdin = io::stdin();
    let mut input = stdin.lock();
    let consent = prompt_telemetry_consent(out, &mut input)?;
    persist_telemetry_choice(repo_root, consent)
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
        // #7404e4
        let pointer = if idx == cursor {
            "\x1b[38;2;116;4;228m>\x1b[0m"
        } else {
            " "
        };
        if selected[idx] {
            // #7404e4
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

pub fn detect_or_select_agent(
    repo_root: &Path,
    out: &mut dyn Write,
    select_fn: Option<&AgentSelector>,
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
            "No agent configuration detected (e.g., .claude, .cursor, .gemini, or .opencode)."
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{Cli, Commands};
    use crate::engine::settings;
    use crate::test_support::process_state::{with_cwd, with_env_var, with_process_state};
    use clap::Parser;
    use std::io::Cursor;

    fn setup_git_repo(dir: &tempfile::TempDir) {
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir.path())
            .status()
            .expect("git init");
    }

    #[test]
    fn init_args_supports_agent_flag() {
        let parsed =
            Cli::try_parse_from(["bitloops", "init", "--agent", "cursor"]).expect("parse init");
        let Some(Commands::Init(args)) = parsed.command else {
            panic!("expected init command");
        };
        assert_eq!(args.agent.as_deref(), Some("cursor"));
    }

    #[test]
    fn init_cmd_agent_flag_no_value_errors() {
        let err = Cli::try_parse_from(["bitloops", "init", "--agent"])
            .err()
            .expect("expected clap parsing error");
        let rendered = err.to_string();
        assert!(
            rendered.contains("a value is required") || rendered.contains("requires a value"),
            "unexpected clap error: {rendered}"
        );
    }

    #[test]
    fn run_init_with_unknown_agent_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        setup_git_repo(&dir);
        with_cwd(dir.path(), || {
            let mut out = Vec::new();
            let err = run_with_writer(
                InitArgs {
                    force: false,
                    agent: Some("bad-agent".to_string()),
                    telemetry: true,
                },
                &mut out,
                None,
            )
            .unwrap_err();
            assert!(format!("{err:#}").contains("unknown agent name"));
        });
    }

    #[test]
    fn run_init_with_agent_claude_installs_claude_hooks() {
        let dir = tempfile::tempdir().unwrap();
        setup_git_repo(&dir);
        with_cwd(dir.path(), || {
            let mut out = Vec::new();
            run_with_writer(
                InitArgs {
                    force: false,
                    agent: Some(AGENT_CLAUDE_CODE.to_string()),
                    telemetry: true,
                },
                &mut out,
                None,
            )
            .unwrap();
            assert!(dir.path().join(".claude/settings.json").exists());
        });
    }

    #[test]
    fn run_init_with_agent_cursor_installs_cursor_hooks() {
        let dir = tempfile::tempdir().unwrap();
        setup_git_repo(&dir);
        with_cwd(dir.path(), || {
            let mut out = Vec::new();
            run_with_writer(
                InitArgs {
                    force: false,
                    agent: Some(AGENT_CURSOR.to_string()),
                    telemetry: true,
                },
                &mut out,
                None,
            )
            .unwrap();

            let hooks = std::fs::read_to_string(dir.path().join(".cursor/hooks.json")).unwrap();
            assert!(hooks.contains("bitloops hooks cursor session-start"));
        });
    }

    #[test]
    fn run_init_with_agent_gemini_installs_gemini_hooks() {
        let dir = tempfile::tempdir().unwrap();
        setup_git_repo(&dir);
        with_cwd(dir.path(), || {
            let mut out = Vec::new();
            run_with_writer(
                InitArgs {
                    force: false,
                    agent: Some(AGENT_GEMINI_CLI.to_string()),
                    telemetry: true,
                },
                &mut out,
                None,
            )
            .unwrap();
            assert!(dir.path().join(".gemini/settings.json").exists());
        });
    }

    #[test]
    fn run_init_with_force_reinstalls_claude_hooks() {
        let dir = tempfile::tempdir().unwrap();
        setup_git_repo(&dir);
        with_cwd(dir.path(), || {
            let mut first_out = Vec::new();
            run_with_writer(
                InitArgs {
                    force: false,
                    agent: Some(AGENT_CLAUDE_CODE.to_string()),
                    telemetry: true,
                },
                &mut first_out,
                None,
            )
            .unwrap();
            let mut second_out = Vec::new();
            run_with_writer(
                InitArgs {
                    force: true,
                    agent: Some(AGENT_CLAUDE_CODE.to_string()),
                    telemetry: true,
                },
                &mut second_out,
                None,
            )
            .unwrap();
            let second = String::from_utf8(second_out).unwrap();
            assert!(second.contains("Installed"));
        });
    }

    #[test]
    fn detect_or_select_agent_no_detection_no_tty_falls_back_to_default() {
        let dir = tempfile::tempdir().unwrap();
        setup_git_repo(&dir);
        with_process_state(
            Some(dir.path()),
            &[("BITLOOPS_TEST_TTY", Some("0"))],
            || {
                let mut out = Vec::new();
                let selected = detect_or_select_agent(dir.path(), &mut out, None).unwrap();
                assert_eq!(selected, vec![DEFAULT_AGENT.to_string()]);
            },
        );
    }

    #[test]
    fn detect_or_select_agent_agent_detected() {
        let dir = tempfile::tempdir().unwrap();
        setup_git_repo(&dir);
        std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
        with_process_state(
            Some(dir.path()),
            &[("BITLOOPS_TEST_TTY", Some("0"))],
            || {
                let mut out = Vec::new();
                let selected = detect_or_select_agent(dir.path(), &mut out, None).unwrap();
                assert_eq!(selected, vec![AGENT_CLAUDE_CODE.to_string()]);
            },
        );
    }

    #[test]
    fn detect_or_select_agent_single_detected_with_tty_uses_selector() {
        let dir = tempfile::tempdir().unwrap();
        setup_git_repo(&dir);
        std::fs::create_dir_all(dir.path().join(".claude")).unwrap();

        // In interactive mode, even a single detected agent should go through selection.
        let select = |_available: &[String]| -> std::result::Result<Vec<String>, String> {
            Ok(vec![AGENT_CURSOR.to_string()])
        };

        with_process_state(
            Some(dir.path()),
            &[("BITLOOPS_TEST_TTY", Some("1"))],
            || {
                let mut out = Vec::new();
                let selected = detect_or_select_agent(dir.path(), &mut out, Some(&select)).unwrap();
                assert_eq!(selected, vec![AGENT_CURSOR.to_string()]);
            },
        );
    }

    #[test]
    fn detect_or_select_agent_selection_cancelled() {
        let dir = tempfile::tempdir().unwrap();
        setup_git_repo(&dir);
        let select = |_available: &[String]| -> std::result::Result<Vec<String>, String> {
            Err("user cancelled".to_string())
        };
        with_process_state(
            Some(dir.path()),
            &[("BITLOOPS_TEST_TTY", Some("1"))],
            || {
                let mut out = Vec::new();
                let err = detect_or_select_agent(dir.path(), &mut out, Some(&select)).unwrap_err();
                assert!(format!("{err:#}").contains("user cancelled"));
            },
        );
    }

    #[test]
    fn detect_or_select_agent_none_selected_errors() {
        let dir = tempfile::tempdir().unwrap();
        setup_git_repo(&dir);
        let select =
            |_available: &[String]| -> std::result::Result<Vec<String>, String> { Ok(vec![]) };
        with_process_state(
            Some(dir.path()),
            &[("BITLOOPS_TEST_TTY", Some("1"))],
            || {
                let mut out = Vec::new();
                let err = detect_or_select_agent(dir.path(), &mut out, Some(&select)).unwrap_err();
                assert!(format!("{err:#}").contains("no agents selected"));
            },
        );
    }

    #[test]
    fn detect_or_select_agent_no_tty_returns_all_detected() {
        let dir = tempfile::tempdir().unwrap();
        setup_git_repo(&dir);
        std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
        std::fs::create_dir_all(dir.path().join(".gemini")).unwrap();
        with_process_state(
            Some(dir.path()),
            &[("BITLOOPS_TEST_TTY", Some("0"))],
            || {
                let mut out = Vec::new();
                let selected = detect_or_select_agent(dir.path(), &mut out, None).unwrap();
                assert_eq!(selected.len(), 2);
            },
        );
    }

    #[test]
    fn detect_or_select_agent_multiple_with_selector() {
        let dir = tempfile::tempdir().unwrap();
        setup_git_repo(&dir);
        std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
        std::fs::create_dir_all(dir.path().join(".gemini")).unwrap();
        let select = |_available: &[String]| -> std::result::Result<Vec<String>, String> {
            Ok(vec![
                AGENT_GEMINI_CLI.to_string(),
                AGENT_CLAUDE_CODE.to_string(),
            ])
        };
        with_process_state(
            Some(dir.path()),
            &[("BITLOOPS_TEST_TTY", Some("1"))],
            || {
                let mut out = Vec::new();
                let selected = detect_or_select_agent(dir.path(), &mut out, Some(&select)).unwrap();
                assert_eq!(
                    selected,
                    vec![AGENT_GEMINI_CLI.to_string(), AGENT_CLAUDE_CODE.to_string()]
                );
            },
        );
    }

    #[test]
    fn init_args_supports_telemetry_flag() {
        let parsed = Cli::try_parse_from(["bitloops", "init", "--telemetry=false"])
            .expect("parse init telemetry flag");
        let Some(Commands::Init(args)) = parsed.command else {
            panic!("expected init command");
        };
        assert!(!args.telemetry);
    }

    #[test]
    fn prompt_telemetry_consent_defaults_yes() {
        let mut out = Vec::new();
        let mut input = Cursor::new("\n");
        let consent = prompt_telemetry_consent(&mut out, &mut input).expect("telemetry prompt");
        assert!(consent);
    }

    #[test]
    fn prompt_telemetry_consent_accepts_no() {
        let mut out = Vec::new();
        let mut input = Cursor::new("no\n");
        let consent = prompt_telemetry_consent(&mut out, &mut input).expect("telemetry prompt");
        assert!(!consent);
    }

    #[test]
    fn maybe_capture_telemetry_consent_flag_false_disables() {
        let dir = tempfile::tempdir().unwrap();
        setup_git_repo(&dir);

        let mut out = Vec::new();
        maybe_capture_telemetry_consent(dir.path(), false, true, &mut out)
            .expect("telemetry config");

        let merged = settings::load_settings(dir.path()).expect("load settings");
        assert_eq!(merged.telemetry, Some(false));
    }

    #[test]
    fn maybe_capture_telemetry_consent_env_optout_disables() {
        with_env_var(TELEMETRY_OPTOUT_ENV, Some("1"), || {
            let dir = tempfile::tempdir().unwrap();
            setup_git_repo(&dir);

            let mut out = Vec::new();
            maybe_capture_telemetry_consent(dir.path(), true, true, &mut out)
                .expect("telemetry config");

            let merged = settings::load_settings(dir.path()).expect("load settings");
            assert_eq!(merged.telemetry, Some(false));
        });
    }

    #[test]
    fn maybe_capture_telemetry_consent_no_tty_leaves_unset() {
        with_env_var("BITLOOPS_TEST_TTY", Some("0"), || {
            let dir = tempfile::tempdir().unwrap();
            setup_git_repo(&dir);

            let mut out = Vec::new();
            maybe_capture_telemetry_consent(dir.path(), true, true, &mut out)
                .expect("telemetry config");

            let merged = settings::load_settings(dir.path()).expect("load settings");
            assert_eq!(merged.telemetry, None);
        });
    }
}

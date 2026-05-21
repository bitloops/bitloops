use std::fs;
use std::io::{BufRead, Read, Write};
use std::process::{Command, Stdio};

use crate::cli::telemetry_consent;
#[cfg(not(test))]
use crate::cli::terminal_picker::can_use_terminal_picker;
use crate::utils::branding::color_hex_if_enabled;

use anyhow::{Context, Result, bail};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InitFinalSetupSelection {
    pub sync: bool,
    pub ingest: bool,
    pub telemetry: bool,
    pub auto_start_daemon: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InitFinalSetupPromptOptions {
    pub show_telemetry: bool,
    pub show_auto_start_daemon: bool,
}

#[derive(Clone, Copy)]
enum InitFinalSetupOptionKind {
    Sync,
    Ingest,
    Telemetry,
    AutoStartDaemon,
}

#[derive(Clone, Copy)]
struct InitFinalSetupOptionSpec {
    kind: InitFinalSetupOptionKind,
    label: &'static str,
    insert_spacing_before: bool,
}

pub(crate) fn choose_final_setup_options(
    sync: Option<bool>,
    out: &mut dyn Write,
    input: &mut dyn BufRead,
    ingest: Option<bool>,
    prompt_options: InitFinalSetupPromptOptions,
) -> Result<InitFinalSetupSelection> {
    let can_prompt = telemetry_consent::can_prompt_interactively();
    let defaults = InitFinalSetupSelection {
        sync: sync.unwrap_or(true),
        ingest: ingest.unwrap_or(true),
        telemetry: prompt_options.show_telemetry,
        auto_start_daemon: prompt_options.show_auto_start_daemon && can_prompt,
    };
    let requires_prompt = sync.is_none()
        || ingest.is_none()
        || prompt_options.show_telemetry
        || (prompt_options.show_auto_start_daemon && can_prompt);

    if !requires_prompt {
        return Ok(defaults);
    }

    if !can_prompt {
        if sync.is_none() || ingest.is_none() {
            bail!(
                "`bitloops init` requires explicit `--sync=true|false` and `--ingest=true|false` choices when not running interactively."
            );
        }
        if prompt_options.show_telemetry {
            bail!(telemetry_consent::NON_INTERACTIVE_TELEMETRY_ERROR);
        }
        return Ok(defaults);
    }

    prompt_final_setup_selection(out, input, defaults, prompt_options)
}

fn prompt_final_setup_selection(
    out: &mut dyn Write,
    input: &mut dyn BufRead,
    defaults: InitFinalSetupSelection,
    prompt_options: InitFinalSetupPromptOptions,
) -> Result<InitFinalSetupSelection> {
    #[cfg(test)]
    let use_picker = false;
    #[cfg(not(test))]
    let use_picker = can_use_terminal_picker();

    if use_picker {
        return prompt_final_setup_selection_with_picker(out, defaults, prompt_options);
    }

    prompt_final_setup_selection_with_text_input(out, input, defaults, prompt_options)
}

fn prompt_final_setup_selection_with_picker(
    out: &mut dyn Write,
    defaults: InitFinalSetupSelection,
    prompt_options: InitFinalSetupPromptOptions,
) -> Result<InitFinalSetupSelection> {
    let options = final_setup_option_specs(prompt_options);
    let mut selected = options
        .iter()
        .map(|option| final_setup_selection_value(defaults, option.kind))
        .collect::<Vec<_>>();
    let mut cursor = 0usize;
    let mut tty_in = fs::OpenOptions::new().read(true).open("/dev/tty")?;
    let _raw_mode = InitPickerRawMode::enter()?;
    let mut rendered_lines = render_follow_up_picker(out, &options, &selected, cursor, None)?;

    loop {
        match read_follow_up_key(&mut tty_in)? {
            FollowUpKey::Up => {
                cursor = cursor.saturating_sub(1);
            }
            FollowUpKey::Down => {
                if cursor + 1 < options.len() {
                    cursor += 1;
                }
            }
            FollowUpKey::Toggle => {
                selected[cursor] = !selected[cursor];
            }
            FollowUpKey::Cancel => bail!("cancelled by user"),
            FollowUpKey::Submit => break,
            FollowUpKey::Unknown => {}
        }

        rendered_lines =
            render_follow_up_picker(out, &options, &selected, cursor, Some(rendered_lines))?;
    }

    writeln!(out)?;
    out.flush()?;

    let mut selection = InitFinalSetupSelection {
        sync: false,
        ingest: false,
        telemetry: false,
        auto_start_daemon: false,
    };
    for (option, is_selected) in options.iter().zip(selected) {
        set_final_setup_selection_value(&mut selection, option.kind, is_selected);
    }
    Ok(selection)
}

fn prompt_final_setup_selection_with_text_input(
    out: &mut dyn Write,
    input: &mut dyn BufRead,
    defaults: InitFinalSetupSelection,
    prompt_options: InitFinalSetupPromptOptions,
) -> Result<InitFinalSetupSelection> {
    let options = final_setup_option_specs(prompt_options);
    writeln!(out)?;
    writeln!(out, "Final setup")?;
    writeln!(out)?;
    writeln!(out, "And we made it to the last setup options 🎉")?;
    writeln!(
        out,
        "{}",
        style_follow_up_hint("Use space to select, enter to confirm.")
    )?;
    writeln!(out)?;
    for (index, option) in options.iter().enumerate() {
        if option.insert_spacing_before {
            writeln!(out)?;
        }
        writeln!(
            out,
            "{}. {}{}",
            index + 1,
            option.label,
            if final_setup_selection_value(defaults, option.kind) {
                " (selected)"
            } else {
                ""
            }
        )?;
    }

    loop {
        let available = (1..=options.len())
            .map(|index| index.to_string())
            .collect::<Vec<_>>()
            .join(",");
        writeln!(
            out,
            "Select options [{available}] (comma-separated, empty to accept defaults)"
        )?;
        write!(out, "> ")?;
        out.flush()?;

        let mut response = String::new();
        input
            .read_line(&mut response)
            .context("reading final setup selection for `bitloops init`")?;
        let response = response.trim().to_ascii_lowercase();
        if response.is_empty() {
            return Ok(defaults);
        }

        if matches!(response.as_str(), "none" | "skip") {
            return Ok(InitFinalSetupSelection {
                sync: false,
                ingest: false,
                telemetry: false,
                auto_start_daemon: false,
            });
        }

        let mut selection = InitFinalSetupSelection {
            sync: false,
            ingest: false,
            telemetry: false,
            auto_start_daemon: false,
        };
        let mut invalid = false;
        for token in response
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            if matches!(token, "all" | "everything") {
                for option in &options {
                    set_final_setup_selection_value(&mut selection, option.kind, true);
                }
                continue;
            }

            if token == "both" {
                selection.sync = true;
                selection.ingest = true;
                continue;
            }

            let Some(option) = option_for_final_setup_token(&options, token) else {
                invalid = true;
                break;
            };
            set_final_setup_selection_value(&mut selection, option.kind, true);
        }

        if !invalid {
            return Ok(selection);
        }

        writeln!(
            out,
            "Please choose option numbers, `all`, `none`, or press enter to accept the defaults."
        )?;
    }
}

fn final_setup_option_specs(
    prompt_options: InitFinalSetupPromptOptions,
) -> Vec<InitFinalSetupOptionSpec> {
    let mut options = vec![
        InitFinalSetupOptionSpec {
            kind: InitFinalSetupOptionKind::Sync,
            label: "Sync codebase",
            insert_spacing_before: false,
        },
        InitFinalSetupOptionSpec {
            kind: InitFinalSetupOptionKind::Ingest,
            label: "Import commit history",
            insert_spacing_before: false,
        },
    ];

    let mut first_setting = true;
    if prompt_options.show_telemetry {
        options.push(InitFinalSetupOptionSpec {
            kind: InitFinalSetupOptionKind::Telemetry,
            label: "Enable anonymous telemetry",
            insert_spacing_before: first_setting,
        });
        first_setting = false;
    }
    if prompt_options.show_auto_start_daemon {
        options.push(InitFinalSetupOptionSpec {
            kind: InitFinalSetupOptionKind::AutoStartDaemon,
            label: "Start Bitloops daemon automatically when you sign in",
            insert_spacing_before: first_setting,
        });
    }

    options
}

fn final_setup_selection_value(
    selection: InitFinalSetupSelection,
    kind: InitFinalSetupOptionKind,
) -> bool {
    match kind {
        InitFinalSetupOptionKind::Sync => selection.sync,
        InitFinalSetupOptionKind::Ingest => selection.ingest,
        InitFinalSetupOptionKind::Telemetry => selection.telemetry,
        InitFinalSetupOptionKind::AutoStartDaemon => selection.auto_start_daemon,
    }
}

fn set_final_setup_selection_value(
    selection: &mut InitFinalSetupSelection,
    kind: InitFinalSetupOptionKind,
    value: bool,
) {
    match kind {
        InitFinalSetupOptionKind::Sync => selection.sync = value,
        InitFinalSetupOptionKind::Ingest => selection.ingest = value,
        InitFinalSetupOptionKind::Telemetry => selection.telemetry = value,
        InitFinalSetupOptionKind::AutoStartDaemon => selection.auto_start_daemon = value,
    }
}

fn option_for_final_setup_token<'a>(
    options: &'a [InitFinalSetupOptionSpec],
    token: &str,
) -> Option<&'a InitFinalSetupOptionSpec> {
    if let Ok(index) = token.parse::<usize>() {
        return index.checked_sub(1).and_then(|index| options.get(index));
    }

    options.iter().find(|option| match option.kind {
        InitFinalSetupOptionKind::Sync => matches!(token, "sync" | "codebase"),
        InitFinalSetupOptionKind::Ingest => {
            matches!(token, "ingest" | "history" | "commit-history")
        }
        InitFinalSetupOptionKind::Telemetry => matches!(token, "telemetry"),
        InitFinalSetupOptionKind::AutoStartDaemon => matches!(
            token,
            "daemon" | "auto-start" | "autostart" | "startup" | "sign-in"
        ),
    })
}

#[derive(Clone, Copy)]
enum FollowUpKey {
    Up,
    Down,
    Toggle,
    Cancel,
    Submit,
    Unknown,
}

struct InitPickerRawMode {
    original_mode: String,
}

impl InitPickerRawMode {
    fn enter() -> Result<Self> {
        let tty = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/tty")
            .context("opening tty for final setup picker")?;

        let output = Command::new("stty")
            .arg("-g")
            .stdin(Stdio::from(
                tty.try_clone()
                    .context("cloning tty handle for final setup picker")?,
            ))
            .output()
            .context("reading tty mode for final setup picker")?;
        if !output.status.success() {
            bail!("failed to read tty mode");
        }

        let original_mode = String::from_utf8(output.stdout)
            .context("parsing tty mode for final setup picker")?
            .trim()
            .to_string();

        let status = Command::new("stty")
            .args(["-icanon", "-echo", "min", "1", "time", "0"])
            .stdin(Stdio::from(tty))
            .status()
            .context("setting raw tty mode for final setup picker")?;
        if !status.success() {
            bail!("failed to set raw tty mode");
        }

        Ok(Self { original_mode })
    }
}

impl Drop for InitPickerRawMode {
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

fn read_follow_up_key(input: &mut dyn Read) -> Result<FollowUpKey> {
    let mut first = [0u8; 1];
    input.read_exact(&mut first)?;
    match first[0] {
        3 => Ok(FollowUpKey::Cancel),
        b' ' => Ok(FollowUpKey::Toggle),
        b'\r' | b'\n' => Ok(FollowUpKey::Submit),
        b'k' => Ok(FollowUpKey::Up),
        b'j' => Ok(FollowUpKey::Down),
        27 => {
            let mut seq = [0u8; 2];
            if input.read_exact(&mut seq).is_err() {
                return Ok(FollowUpKey::Unknown);
            }
            if seq == [b'[', b'A'] {
                Ok(FollowUpKey::Up)
            } else if seq == [b'[', b'B'] {
                Ok(FollowUpKey::Down)
            } else {
                Ok(FollowUpKey::Unknown)
            }
        }
        _ => Ok(FollowUpKey::Unknown),
    }
}

fn render_follow_up_picker(
    out: &mut dyn Write,
    options: &[InitFinalSetupOptionSpec],
    selected: &[bool],
    cursor: usize,
    previous_lines: Option<usize>,
) -> Result<usize> {
    let mut lines = vec![
        "Final setup".to_string(),
        String::new(),
        "And we made it to the last setup options 🎉".to_string(),
        style_follow_up_hint("Use space to select, enter to confirm."),
        String::new(),
    ];

    for (idx, option) in options.iter().enumerate() {
        if option.insert_spacing_before {
            lines.push(String::new());
        }
        let pointer = if idx == cursor {
            color_hex_if_enabled(">", crate::utils::branding::BITLOOPS_PURPLE_HEX)
        } else {
            " ".to_string()
        };
        let checkbox = if selected[idx] {
            selected_follow_up_checkbox()
        } else {
            "[ ]".to_string()
        };
        let label = if selected[idx] {
            selected_follow_up_label(option.label)
        } else {
            option.label.to_string()
        };
        lines.push(format!("{pointer} {checkbox} {label}"));
    }

    lines.push(String::new());
    lines.push(format!(
        "space {} • ↑/↓ {} • enter {}",
        style_follow_up_hint("toggle"),
        style_follow_up_hint("move"),
        style_follow_up_hint("submit")
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

fn style_follow_up_hint(detail: &str) -> String {
    if crate::utils::branding::should_use_color_output() {
        format!("\x1b[2;3m{detail}\x1b[0m")
    } else {
        detail.to_string()
    }
}

fn selected_follow_up_checkbox() -> String {
    const SELECTION_WHITE_HEX: &str = "#ffffff";
    format!(
        "{}{}{}",
        color_hex_if_enabled("[", SELECTION_WHITE_HEX),
        color_hex_if_enabled("•", crate::utils::branding::BITLOOPS_PURPLE_HEX),
        color_hex_if_enabled("]", SELECTION_WHITE_HEX)
    )
}

fn selected_follow_up_label(label: &str) -> String {
    const SELECTION_WHITE_HEX: &str = "#ffffff";
    color_hex_if_enabled(label, SELECTION_WHITE_HEX)
}

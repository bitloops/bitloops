#[cfg(not(test))]
use std::env;
use std::fs;
#[cfg(not(test))]
use std::io;
#[cfg(not(test))]
use std::io::IsTerminal;
use std::io::{Read, Write};
#[cfg(not(test))]
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Result, anyhow, bail};

use crate::utils::branding::{BITLOOPS_PURPLE_HEX, color_hex_if_enabled};

#[cfg(test)]
use std::{cell::RefCell, rc::Rc};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SingleSelectOption {
    pub(crate) label: String,
    pub(crate) details: Vec<String>,
}

impl SingleSelectOption {
    pub(crate) fn new(label: impl Into<String>, details: Vec<String>) -> Self {
        Self {
            label: label.into(),
            details,
        }
    }
}

#[cfg(test)]
type SingleSelectHook = dyn Fn(&[SingleSelectOption], usize) -> Result<usize> + 'static;

#[cfg(test)]
thread_local! {
    static SINGLE_SELECT_HOOK: RefCell<Option<Rc<SingleSelectHook>>> = RefCell::new(None);
}

pub(crate) fn can_use_terminal_picker() -> bool {
    #[cfg(test)]
    if SINGLE_SELECT_HOOK.with(|cell| cell.borrow().is_some()) {
        return true;
    }

    #[cfg(test)]
    {
        false
    }

    #[cfg(not(test))]
    {
        io::stdin().is_terminal()
            && io::stdout().is_terminal()
            && command_exists("stty")
            && fs::OpenOptions::new().read(true).open("/dev/tty").is_ok()
    }
}

pub(crate) fn prompt_single_select(
    out: &mut dyn Write,
    title: &str,
    options: &[SingleSelectOption],
    default_index: usize,
    footer: &[String],
) -> Result<usize> {
    if options.is_empty() {
        bail!("single-select prompt requires at least one option");
    }

    let mut cursor = default_index.min(options.len() - 1);

    #[cfg(test)]
    if let Some(hook) = SINGLE_SELECT_HOOK.with(|cell| cell.borrow().clone()) {
        render_single_select(out, title, options, cursor, None, footer)?;
        let selected = hook(options, cursor)?;
        if selected >= options.len() {
            bail!("single-select hook returned out-of-bounds index {selected}");
        }
        writeln!(out)?;
        out.flush()?;
        return Ok(selected);
    }

    let mut tty_in = fs::OpenOptions::new().read(true).open("/dev/tty")?;
    let _raw_mode = SttyRawMode::enter()?;
    let mut rendered_lines = render_single_select(out, title, options, cursor, None, footer)?;

    loop {
        match read_key(&mut tty_in)? {
            Key::Up => {
                cursor = cursor.saturating_sub(1);
            }
            Key::Down => {
                if cursor + 1 < options.len() {
                    cursor += 1;
                }
            }
            Key::Cancel => bail!("cancelled by user"),
            Key::Submit => break,
            Key::Unknown => {}
        }
        rendered_lines =
            render_single_select(out, title, options, cursor, Some(rendered_lines), footer)?;
    }

    writeln!(out)?;
    out.flush()?;
    Ok(cursor)
}

#[cfg(test)]
pub(crate) fn with_single_select_hook<T>(
    hook: impl Fn(&[SingleSelectOption], usize) -> Result<usize> + 'static,
    f: impl FnOnce() -> T,
) -> T {
    SINGLE_SELECT_HOOK.with(|cell| {
        assert!(
            cell.borrow().is_none(),
            "single-select hook already installed"
        );
        *cell.borrow_mut() = Some(Rc::new(hook));
    });
    let result = f();
    SINGLE_SELECT_HOOK.with(|cell| {
        *cell.borrow_mut() = None;
    });
    result
}

fn render_single_select(
    out: &mut dyn Write,
    title: &str,
    options: &[SingleSelectOption],
    cursor: usize,
    previous_lines: Option<usize>,
    footer: &[String],
) -> Result<usize> {
    let mut lines = Vec::new();
    lines.push(title.to_string());
    lines.push("Use ↑/↓ to move and enter to confirm.".to_string());
    lines.push(String::new());

    for (index, option) in options.iter().enumerate() {
        let pointer = if index == cursor {
            color_hex_if_enabled(">", BITLOOPS_PURPLE_HEX)
        } else {
            " ".to_string()
        };
        let label = if index == cursor {
            color_hex_if_enabled(&option.label, BITLOOPS_PURPLE_HEX)
        } else {
            option.label.clone()
        };

        lines.push(format!("{pointer} {label}"));
        for detail in &option.details {
            lines.push(format!("  {detail}"));
        }
    }

    if !footer.is_empty() {
        lines.push(String::new());
        lines.extend(footer.iter().cloned());
    }

    lines.push(String::new());
    lines.push("ctrl+c cancel • ↑/↓ move • enter submit".to_string());

    if let Some(previous_lines) = previous_lines {
        if previous_lines > 1 {
            write!(out, "\x1b[{}F", previous_lines - 1)?;
        } else {
            write!(out, "\r")?;
        }
    }

    for (index, line) in lines.iter().enumerate() {
        write!(out, "\r\x1b[2K{line}")?;
        if index + 1 < lines.len() {
            writeln!(out)?;
        }
    }
    out.flush()?;
    Ok(lines.len())
}

#[cfg(not(test))]
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

#[cfg(not(test))]
fn executable_with_extensions(dir: &Path, program: &str) -> [PathBuf; 3] {
    [
        dir.join(format!("{program}.exe")),
        dir.join(format!("{program}.cmd")),
        dir.join(format!("{program}.bat")),
    ]
}

#[derive(Clone, Copy)]
enum Key {
    Up,
    Down,
    Cancel,
    Submit,
    Unknown,
}

fn read_key(input: &mut dyn Read) -> Result<Key> {
    let mut first = [0u8; 1];
    input.read_exact(&mut first)?;
    match first[0] {
        3 => Ok(Key::Cancel),
        b'\r' | b'\n' => Ok(Key::Submit),
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

struct SttyRawMode {
    original_mode: String,
}

impl SttyRawMode {
    fn enter() -> Result<Self> {
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

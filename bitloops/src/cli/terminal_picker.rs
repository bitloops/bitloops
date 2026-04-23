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

use crate::utils::branding::{BITLOOPS_PURPLE_HEX, color_hex_if_enabled, should_use_color_output};

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MultiSelectOption {
    pub(crate) label: String,
    pub(crate) details: Vec<String>,
    pub(crate) selected: bool,
}

impl MultiSelectOption {
    pub(crate) fn new(label: impl Into<String>, details: Vec<String>, selected: bool) -> Self {
        Self {
            label: label.into(),
            details,
            selected,
        }
    }
}

struct MultiSelectRenderState<'a> {
    title: &'a str,
    intro: &'a [String],
    options: &'a [MultiSelectOption],
    selected: &'a [bool],
    cursor: usize,
    previous_lines: Option<usize>,
    footer: &'a [String],
}

#[cfg(test)]
type SingleSelectHook = dyn Fn(&[SingleSelectOption], usize) -> Result<usize> + 'static;
#[cfg(test)]
type MultiSelectHook = dyn Fn(&[MultiSelectOption], usize) -> Result<Vec<usize>> + 'static;

#[cfg(test)]
thread_local! {
    static SINGLE_SELECT_HOOK: RefCell<Option<Rc<SingleSelectHook>>> = RefCell::new(None);
    static MULTI_SELECT_HOOK: RefCell<Option<Rc<MultiSelectHook>>> = RefCell::new(None);
}

pub(crate) fn can_use_terminal_picker() -> bool {
    #[cfg(test)]
    if SINGLE_SELECT_HOOK.with(|cell| cell.borrow().is_some())
        || MULTI_SELECT_HOOK.with(|cell| cell.borrow().is_some())
    {
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
    intro: &[String],
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
        render_single_select(out, title, intro, options, cursor, None, footer)?;
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
    let mut rendered_lines =
        render_single_select(out, title, intro, options, cursor, None, footer)?;

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
            Key::Toggle | Key::SelectAll => {}
            Key::Unknown => {}
        }
        rendered_lines = render_single_select(
            out,
            title,
            intro,
            options,
            cursor,
            Some(rendered_lines),
            footer,
        )?;
    }

    writeln!(out)?;
    out.flush()?;
    Ok(cursor)
}

pub(crate) fn prompt_multi_select(
    out: &mut dyn Write,
    title: &str,
    intro: &[String],
    options: &[MultiSelectOption],
    footer: &[String],
) -> Result<Vec<usize>> {
    if options.is_empty() {
        bail!("multi-select prompt requires at least one option");
    }

    let mut cursor = options
        .iter()
        .position(|option| option.selected)
        .unwrap_or(0);
    let mut selected = options
        .iter()
        .map(|option| option.selected)
        .collect::<Vec<_>>();

    #[cfg(test)]
    if let Some(hook) = MULTI_SELECT_HOOK.with(|cell| cell.borrow().clone()) {
        render_multi_select(
            out,
            MultiSelectRenderState {
                title,
                intro,
                options,
                selected: &selected,
                cursor,
                previous_lines: None,
                footer,
            },
        )?;
        let selected_indexes =
            normalize_multi_select_indexes(hook(options, cursor)?, options.len())?;
        writeln!(out)?;
        out.flush()?;
        return Ok(selected_indexes);
    }

    let mut tty_in = fs::OpenOptions::new().read(true).open("/dev/tty")?;
    let _raw_mode = SttyRawMode::enter()?;
    let mut rendered_lines = render_multi_select(
        out,
        MultiSelectRenderState {
            title,
            intro,
            options,
            selected: &selected,
            cursor,
            previous_lines: None,
            footer,
        },
    )?;

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
            Key::Toggle => {
                selected[cursor] = !selected[cursor];
            }
            Key::SelectAll => {
                let all_selected = selected.iter().all(|is_selected| *is_selected);
                selected.fill(!all_selected);
            }
            Key::Cancel => bail!("cancelled by user"),
            Key::Submit => break,
            Key::Unknown => {}
        }
        rendered_lines = render_multi_select(
            out,
            MultiSelectRenderState {
                title,
                intro,
                options,
                selected: &selected,
                cursor,
                previous_lines: Some(rendered_lines),
                footer,
            },
        )?;
    }

    writeln!(out)?;
    out.flush()?;

    let selected_indexes = selected
        .into_iter()
        .enumerate()
        .filter_map(|(index, is_selected)| is_selected.then_some(index))
        .collect::<Vec<_>>();
    if selected_indexes.is_empty() {
        bail!("no options selected");
    }

    Ok(selected_indexes)
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

#[cfg(test)]
pub(crate) fn with_multi_select_hook<T>(
    hook: impl Fn(&[MultiSelectOption], usize) -> Result<Vec<usize>> + 'static,
    f: impl FnOnce() -> T,
) -> T {
    MULTI_SELECT_HOOK.with(|cell| {
        assert!(
            cell.borrow().is_none(),
            "multi-select hook already installed"
        );
        *cell.borrow_mut() = Some(Rc::new(hook));
    });
    let result = f();
    MULTI_SELECT_HOOK.with(|cell| {
        *cell.borrow_mut() = None;
    });
    result
}

fn render_single_select(
    out: &mut dyn Write,
    title: &str,
    intro: &[String],
    options: &[SingleSelectOption],
    cursor: usize,
    previous_lines: Option<usize>,
    footer: &[String],
) -> Result<usize> {
    let mut lines = Vec::new();
    lines.push(title.to_string());
    lines.push(String::new());
    if !intro.is_empty() {
        lines.extend(intro.iter().cloned());
        lines.push(String::new());
    }

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
            lines.push(format!("  {}", style_single_select_detail(detail)));
        }
    }

    if !footer.is_empty() {
        lines.push(String::new());
        lines.extend(footer.iter().cloned());
    }

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

fn render_multi_select(out: &mut dyn Write, state: MultiSelectRenderState<'_>) -> Result<usize> {
    let mut lines = Vec::new();
    lines.push(state.title.to_string());
    lines.push(String::new());
    if !state.intro.is_empty() {
        lines.extend(state.intro.iter().cloned());
        lines.push(String::new());
    }

    for (index, option) in state.options.iter().enumerate() {
        let pointer = if index == state.cursor {
            color_hex_if_enabled(">", BITLOOPS_PURPLE_HEX)
        } else {
            " ".to_string()
        };
        let checkbox = if state.selected[index] {
            "[•]"
        } else {
            "[ ]"
        };
        let label = if index == state.cursor {
            color_hex_if_enabled(&option.label, BITLOOPS_PURPLE_HEX)
        } else {
            option.label.clone()
        };
        let line = if state.selected[index] {
            format!(
                "{pointer} {} {}",
                color_hex_if_enabled(checkbox, BITLOOPS_PURPLE_HEX),
                label
            )
        } else {
            format!("{pointer} {checkbox} {label}")
        };
        lines.push(line);
        for detail in &option.details {
            lines.push(format!("  {}", style_single_select_detail(detail)));
        }
    }

    if !state.footer.is_empty() {
        lines.push(String::new());
        lines.extend(state.footer.iter().cloned());
    }

    if let Some(previous_lines) = state.previous_lines {
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

fn style_single_select_detail(detail: &str) -> String {
    if should_use_color_output() {
        format!("\x1b[2m{detail}\x1b[0m")
    } else {
        detail.to_string()
    }
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
    Toggle,
    SelectAll,
    Cancel,
    Submit,
    Unknown,
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

#[cfg(test)]
fn normalize_multi_select_indexes(
    mut selected_indexes: Vec<usize>,
    len: usize,
) -> Result<Vec<usize>> {
    selected_indexes.sort_unstable();
    selected_indexes.dedup();
    if let Some(index) = selected_indexes.iter().copied().find(|index| *index >= len) {
        bail!("multi-select hook returned out-of-bounds index {index}");
    }
    Ok(selected_indexes)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_use_terminal_picker_when_multi_select_hook_is_installed() {
        with_multi_select_hook(
            |_options, _cursor| Ok(vec![0]),
            || {
                assert!(can_use_terminal_picker());
            },
        );
    }

    #[test]
    fn prompt_multi_select_uses_hook_selection() {
        let options = vec![
            MultiSelectOption::new("Capture", vec![], false),
            MultiSelectOption::new("DevQL Guidance", vec![], true),
        ];
        let mut out = Vec::new();

        let selected = with_multi_select_hook(
            |_options, cursor| {
                assert_eq!(cursor, 1);
                Ok(vec![0, 1])
            },
            || prompt_multi_select(&mut out, "Select", &[], &options, &[]).expect("select"),
        );

        assert_eq!(selected, vec![0, 1]);
    }

    #[test]
    fn prompt_multi_select_rejects_empty_options() {
        let mut out = Vec::new();
        let err = prompt_multi_select(&mut out, "Select", &[], &[], &[])
            .expect_err("empty options should fail");
        assert!(format!("{err:#}").contains("at least one option"));
    }
}

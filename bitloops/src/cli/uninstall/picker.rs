use std::collections::BTreeSet;
use std::fs;
use std::io::{Read, Write};

use anyhow::{Result, bail};

use super::targets::{ALL_TARGETS, UninstallTarget};
use super::tty::SttyRawMode;

pub(super) fn prompt_select_targets(
    out: &mut dyn Write,
) -> Result<Option<BTreeSet<UninstallTarget>>> {
    let labels = ALL_TARGETS
        .iter()
        .map(|target| target.picker_label().to_string())
        .collect::<Vec<_>>();
    let mut selected = vec![false; labels.len()];
    let mut cursor = 0usize;

    let mut tty_in = fs::OpenOptions::new().read(true).open("/dev/tty")?;
    let _raw_mode = SttyRawMode::enter()?;
    let mut rendered_lines = render_target_picker(out, &labels, &selected, cursor, None)?;

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
                let all_selected = selected.iter().all(|value| *value);
                selected.fill(!all_selected);
            }
            Key::Cancel => {
                writeln!(out)?;
                out.flush()?;
                return Ok(None);
            }
            Key::Submit => break,
            Key::Unknown => {}
        }

        rendered_lines =
            render_target_picker(out, &labels, &selected, cursor, Some(rendered_lines))?;
    }

    writeln!(out)?;
    out.flush()?;

    let selected_targets = selected
        .into_iter()
        .enumerate()
        .filter_map(|(index, is_selected)| is_selected.then_some(ALL_TARGETS[index]))
        .collect::<BTreeSet<_>>();

    if selected_targets.is_empty() {
        bail!("no uninstall targets selected");
    }

    Ok(Some(selected_targets))
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

fn render_target_picker(
    out: &mut dyn Write,
    labels: &[String],
    selected: &[bool],
    cursor: usize,
    previous_lines: Option<usize>,
) -> Result<usize> {
    let mut lines = Vec::new();
    lines.push("Select what to uninstall:".to_string());
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

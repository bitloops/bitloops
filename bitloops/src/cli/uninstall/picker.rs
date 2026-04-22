use std::collections::BTreeSet;
use std::fs;
use std::io::{Read, Write};

use anyhow::{Result, bail};

use super::targets::{ALL_TARGETS, UninstallTarget};
use super::tty::SttyRawMode;

pub(super) fn prompt_select_targets(
    out: &mut dyn Write,
) -> Result<Option<BTreeSet<UninstallTarget>>> {
    let mut tty_in = fs::OpenOptions::new().read(true).open("/dev/tty")?;
    let _raw_mode = SttyRawMode::enter()?;
    prompt_select_targets_with_input(out, &mut tty_in)
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

fn prompt_select_targets_with_input(
    out: &mut dyn Write,
    input: &mut dyn Read,
) -> Result<Option<BTreeSet<UninstallTarget>>> {
    let labels = ALL_TARGETS
        .iter()
        .map(|target| target.picker_label().to_string())
        .collect::<Vec<_>>();
    let mut selected = vec![false; labels.len()];
    let mut cursor = 0usize;
    let mut rendered_lines = render_target_picker(out, &labels, &selected, cursor, None)?;

    loop {
        match read_key(input)? {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn collect_selection(bytes: &[u8]) -> Result<Option<BTreeSet<UninstallTarget>>> {
        let mut out = Vec::new();
        let mut input = Cursor::new(bytes.to_vec());
        prompt_select_targets_with_input(&mut out, &mut input)
    }

    #[test]
    fn picker_toggle_and_submit_selects_requested_targets() {
        let selected = collect_selection(b"xjjx\n")
            .expect("selection should succeed")
            .expect("selection should not be cancelled");

        assert_eq!(
            selected,
            BTreeSet::from([UninstallTarget::AgentHooks, UninstallTarget::GitHooks])
        );
    }

    #[test]
    fn picker_ctrl_a_selects_everything() {
        let selected = collect_selection(&[1, b'\n'])
            .expect("selection should succeed")
            .expect("selection should not be cancelled");

        assert_eq!(selected, BTreeSet::from(ALL_TARGETS));
    }

    #[test]
    fn picker_cancel_returns_none() {
        let selected = collect_selection(&[3]).expect("cancel should not error");
        assert_eq!(selected, None);
    }

    #[test]
    fn picker_submit_without_selection_errors() {
        let err = collect_selection(b"\n").expect_err("empty selection should error");
        assert!(format!("{err:#}").contains("no uninstall targets selected"));
    }

    #[test]
    fn read_key_maps_supported_escape_sequences() {
        assert!(matches!(
            read_key(&mut Cursor::new(vec![27, b'[', b'A'])).expect("read key"),
            Key::Up
        ));
        assert!(matches!(
            read_key(&mut Cursor::new(vec![27, b'[', b'B'])).expect("read key"),
            Key::Down
        ));
        assert!(matches!(
            read_key(&mut Cursor::new(vec![27, b'['])).expect("read key"),
            Key::Unknown
        ));
        assert!(matches!(
            read_key(&mut Cursor::new(vec![b'x'])).expect("read key"),
            Key::Toggle
        ));
    }
}

use std::io::Write;

use anyhow::Result;

pub(crate) fn clear_rendered_lines(out: &mut dyn Write, line_count: usize) -> Result<()> {
    if line_count == 0 {
        return Ok(());
    }
    write!(out, "\r\x1b[2K")?;
    for _ in 1..line_count {
        write!(out, "\x1b[1A\r\x1b[2K")?;
    }
    Ok(())
}

pub(crate) fn rendered_terminal_line_count(frame: &str, terminal_width: Option<usize>) -> usize {
    let Some(width) = terminal_width.filter(|width| *width > 0) else {
        return frame.lines().count().max(1);
    };

    frame
        .split('\n')
        .map(|line| visible_terminal_width(line).max(1).div_ceil(width))
        .sum::<usize>()
        .max(1)
}

fn visible_terminal_width(text: &str) -> usize {
    let mut width = 0usize;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for next in chars.by_ref() {
                if ('@'..='~').contains(&next) {
                    break;
                }
            }
            continue;
        }
        width += 1;
    }
    width
}

pub(crate) fn fit_line(text: &str, available_width: Option<usize>) -> String {
    let Some(max_width) = available_width else {
        return text.to_string();
    };
    if max_width == 0 || text.chars().count() <= max_width {
        return text.to_string();
    }
    let prefix_len = (max_width.saturating_sub(1)) / 2;
    let suffix_len = max_width.saturating_sub(1).saturating_sub(prefix_len);
    let prefix = text.chars().take(prefix_len).collect::<String>();
    let suffix = text
        .chars()
        .rev()
        .take(suffix_len)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    format!("{prefix}…{suffix}")
}

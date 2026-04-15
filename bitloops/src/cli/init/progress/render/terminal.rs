use std::io::Write;

use anyhow::Result;

use crate::utils::branding::{BITLOOPS_PURPLE_HEX, color_hex_if_enabled};

use super::super::InitChecklistState;

pub(super) fn clear_rendered_lines(out: &mut dyn Write, line_count: usize) -> Result<()> {
    if line_count == 0 {
        return Ok(());
    }
    write!(out, "\r\x1b[2K")?;
    for _ in 1..line_count {
        write!(out, "\x1b[1A\r\x1b[2K")?;
    }
    Ok(())
}

pub(super) fn rendered_terminal_line_count(frame: &str, terminal_width: Option<usize>) -> usize {
    let Some(width) = terminal_width.filter(|width| *width > 0) else {
        return frame.lines().count().max(1);
    };

    frame
        .split('\n')
        .map(|line| {
            let visible_width = visible_terminal_width(line);
            visible_width.max(1).div_ceil(width)
        })
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

pub(super) fn render_init_determinate_progress_bar(width: usize, ratio: f64) -> String {
    let filled = ((width as f64) * ratio).round() as usize;
    let filled = filled.min(width);
    let fill = color_hex_if_enabled(&"█".repeat(filled), BITLOOPS_PURPLE_HEX);
    let empty = "░".repeat(width.saturating_sub(filled));
    format!("{fill}{empty}")
}

pub(super) fn render_init_indeterminate_progress_bar(width: usize, spinner_index: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let position = spinner_index % width;
    let prefix = "░".repeat(position);
    let pulse = color_hex_if_enabled("█", BITLOOPS_PURPLE_HEX);
    let suffix = "░".repeat(width.saturating_sub(position + 1));
    format!("{prefix}{pulse}{suffix}")
}

pub(super) fn format_count_i32(value: i32) -> String {
    format_count_u64(u64::try_from(value).unwrap_or_default())
}

pub(super) fn format_count_u64(value: u64) -> String {
    let digits = value.to_string();
    let mut formatted = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            formatted.push(',');
        }
        formatted.push(ch);
    }
    formatted
}

pub(super) fn format_megabytes(bytes: i64) -> String {
    let megabytes = bytes as f64 / (1024.0 * 1024.0);
    if megabytes >= 10.0 {
        format!("{megabytes:.0}")
    } else {
        format!("{megabytes:.1}")
    }
}

pub(super) fn remaining_init_dependencies(checklist: InitChecklistState) -> Option<&'static str> {
    match (
        checklist.show_sync && !checklist.sync_complete,
        checklist.show_ingest && !checklist.ingest_complete,
    ) {
        (true, true) => Some("sync and ingest"),
        (true, false) => Some("sync"),
        (false, true) => Some("ingest"),
        (false, false) => None,
    }
}

pub(in super::super) fn fit_init_plain_line(text: &str, terminal_width: Option<usize>) -> String {
    fit_init_status_text(text, terminal_width)
}

pub(super) fn fit_init_status_text(text: &str, available_width: Option<usize>) -> String {
    let Some(max_width) = available_width else {
        return text.to_string();
    };
    if max_width == 0 {
        return String::new();
    }
    if text.chars().count() <= max_width {
        return text.to_string();
    }
    elide_init_middle(text, max_width)
}

fn elide_init_middle(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if max_width == 1 {
        return "…".to_string();
    }
    if text.chars().count() <= max_width {
        return text.to_string();
    }
    let prefix_len = (max_width - 1) / 2;
    let suffix_len = max_width - 1 - prefix_len;
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

pub(super) fn humanise_init_sync_phase(phase: &str) -> &'static str {
    match phase {
        "queued" => "waiting in queue",
        "ensuring_schema" => "preparing schema",
        "inspecting_workspace" => "inspecting workspace",
        "building_manifest" => "building manifest",
        "loading_stored_state" => "loading stored state",
        "classifying_paths" => "classifying paths",
        "removing_paths" => "removing stale paths",
        "extracting_paths" => "extracting artefacts",
        "materialising_paths" => "materialising artefacts",
        "running_gc" => "cleaning caches",
        "complete" => "complete",
        "failed" => "failed",
        _ => "working",
    }
}

pub(super) fn humanise_init_ingest_phase(phase: &str) -> &'static str {
    match phase.to_ascii_lowercase().as_str() {
        "initializing" => "initialising",
        "extracting" => "extracting checkpoints",
        "persisting" => "persisting state",
        "complete" => "complete",
        "failed" => "failed",
        _ => "working",
    }
}

pub(super) fn humanise_init_bootstrap_phase(phase: &str) -> &'static str {
    match phase {
        "queued" => "waiting in queue",
        "preparing_config" => "preparing config",
        "resolving_release" => "resolving release",
        "downloading_runtime" => "downloading runtime",
        "extracting_runtime" => "extracting runtime",
        "rewriting_runtime" => "updating runtime config",
        "warming_profile" => "warming profile cache",
        "complete" => "complete",
        "failed" => "failed",
        _ => "working",
    }
}

pub(super) fn humanise_summary_setup_phase(
    phase: crate::cli::inference::SummarySetupPhase,
) -> &'static str {
    match phase {
        crate::cli::inference::SummarySetupPhase::Queued => "queued",
        crate::cli::inference::SummarySetupPhase::ResolvingRelease => "resolving release",
        crate::cli::inference::SummarySetupPhase::DownloadingRuntime => "downloading runtime",
        crate::cli::inference::SummarySetupPhase::ExtractingRuntime => "extracting runtime",
        crate::cli::inference::SummarySetupPhase::RewritingRuntime => "updating runtime config",
        crate::cli::inference::SummarySetupPhase::WritingProfile => "writing summary profile",
    }
}

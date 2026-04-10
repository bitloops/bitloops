use std::io::{self, IsTerminal, Write};
use std::time::Duration;

use anyhow::Result;
use terminal_size::{Width, terminal_size};

use super::types::SyncTaskGraphqlRecord;
use crate::utils::branding::{BITLOOPS_PURPLE_HEX, color_hex_if_enabled};

pub(super) const SYNC_PROGRESS_POLL_INTERVAL: Duration = Duration::from_secs(1);
pub(super) const SYNC_RENDER_TICK_INTERVAL: Duration = Duration::from_millis(120);
const SYNC_SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub(super) struct SyncProgressRenderer {
    interactive: bool,
    terminal_width: Option<usize>,
    spinner_index: usize,
    last_frame: Option<String>,
    wrote_in_place: bool,
}

impl SyncProgressRenderer {
    pub(super) fn new() -> Self {
        Self {
            interactive: std::io::stdout().is_terminal() && std::env::var("ACCESSIBLE").is_err(),
            terminal_width: terminal_size().map(|(Width(width), _)| width as usize),
            spinner_index: 0,
            last_frame: None,
            wrote_in_place: false,
        }
    }

    pub(super) fn is_interactive(&self) -> bool {
        self.interactive
    }

    pub(super) fn render(&mut self, task: &SyncTaskGraphqlRecord) -> Result<()> {
        let frame = self.render_frame(task);
        self.write_frame(frame, false)
    }

    pub(super) fn tick(&mut self, task: &SyncTaskGraphqlRecord) -> Result<()> {
        if !self.interactive || !matches!(task.status.as_str(), "queued" | "running") {
            return Ok(());
        }
        self.spinner_index = (self.spinner_index + 1) % SYNC_SPINNER_FRAMES.len();
        let frame = self.render_frame(task);
        self.write_frame(frame, true)
    }

    pub(super) fn finish(&mut self) -> Result<()> {
        if self.interactive && self.wrote_in_place {
            let mut stdout = io::stdout();
            writeln!(stdout)?;
            stdout.flush()?;
            self.wrote_in_place = false;
        }
        Ok(())
    }

    fn spinner_frame(&self) -> String {
        color_hex_if_enabled(SYNC_SPINNER_FRAMES[self.spinner_index], BITLOOPS_PURPLE_HEX)
    }

    fn render_frame(&self, task: &SyncTaskGraphqlRecord) -> String {
        if self.interactive {
            let bar =
                format_live_sync_progress_bar_line(task, self.spinner_index, self.terminal_width);
            let status =
                format_live_sync_task_status_line(task, &self.spinner_frame(), self.terminal_width);
            format!("{bar}\n{status}")
        } else {
            format_live_sync_task_status_line(task, &self.spinner_frame(), self.terminal_width)
        }
    }

    fn write_frame(&mut self, frame: String, force: bool) -> Result<()> {
        if self.interactive {
            if !force && self.last_frame.as_deref() == Some(frame.as_str()) {
                return Ok(());
            }
            let mut stdout = io::stdout();
            if self.wrote_in_place {
                write!(stdout, "\r\x1b[2K\x1b[1A\r\x1b[2K{frame}")?;
            } else {
                write!(stdout, "{frame}")?;
            }
            stdout.flush()?;
            self.last_frame = Some(frame);
            self.wrote_in_place = true;
            return Ok(());
        }

        if self.last_frame.as_deref() != Some(frame.as_str()) {
            println!("{frame}");
            self.last_frame = Some(frame);
        }
        Ok(())
    }
}

pub(super) fn format_live_sync_task_status_line(
    task: &SyncTaskGraphqlRecord,
    spinner: &str,
    terminal_width: Option<usize>,
) -> String {
    let status = match task.status.as_str() {
        "queued" => format!(
            "Sync queued for {} · mode={} · {} ahead",
            task.repo_name,
            task.mode,
            task.tasks_ahead.unwrap_or(0),
        ),
        "running" => {
            let mut line = format!(
                "Syncing {} · {}",
                task.repo_name,
                humanise_sync_phase(task.phase.as_str()),
            );
            if task.paths_total > 0 {
                line.push_str(&format!(" · {}/{}", task.paths_completed, task.paths_total));
            }
            if let Some(path) = task.current_path.as_ref() {
                line.push_str(&format!(" · {path}"));
            }
            line
        }
        "completed" => format!("✓ Sync complete for {}", task.repo_name),
        "failed" => format!("✖ Sync failed for {}", task.repo_name),
        "cancelled" => format!("✖ Sync cancelled for {}", task.repo_name),
        other => format!("Sync {other} for {}", task.repo_name),
    };
    let fitted = fit_live_status_text(
        status.as_str(),
        terminal_width.map(|width| width.saturating_sub(2)),
    );
    format!("{spinner} {fitted}")
}

pub(super) fn format_live_sync_progress_bar_line(
    task: &SyncTaskGraphqlRecord,
    spinner_index: usize,
    terminal_width: Option<usize>,
) -> String {
    let available_width = terminal_width.unwrap_or(80).max(16);
    let ratio = progress_ratio(task);
    let summary = if let Some(ratio) = ratio {
        format!(
            " {:>3}% {}/{}",
            (ratio * 100.0).round() as usize,
            task.paths_completed,
            task.paths_total
        )
    } else {
        format!(" {} ", humanise_sync_phase(task.phase.as_str()))
    };
    let reserved = summary.chars().count() + 2;
    if available_width <= reserved + 1 {
        return fit_live_status_text(summary.trim(), Some(available_width));
    }

    let bar_width = available_width - reserved;
    let bar = if let Some(ratio) = ratio {
        render_determinate_progress_bar(bar_width, ratio)
    } else {
        render_indeterminate_progress_bar(bar_width, spinner_index)
    };

    format!("[{bar}]{summary}")
}

fn progress_ratio(task: &SyncTaskGraphqlRecord) -> Option<f64> {
    match task.status.as_str() {
        "completed" => Some(1.0),
        "failed" | "cancelled" => {
            if task.paths_total > 0 {
                Some((task.paths_completed as f64 / task.paths_total as f64).clamp(0.0, 1.0))
            } else {
                Some(0.0)
            }
        }
        _ if task.paths_total > 0 => {
            Some((task.paths_completed as f64 / task.paths_total as f64).clamp(0.0, 1.0))
        }
        _ => None,
    }
}

fn render_determinate_progress_bar(width: usize, ratio: f64) -> String {
    let filled = ((width as f64) * ratio).round() as usize;
    let filled = filled.min(width);
    let fill = color_hex_if_enabled(&"█".repeat(filled), BITLOOPS_PURPLE_HEX);
    let empty = "░".repeat(width.saturating_sub(filled));
    format!("{fill}{empty}")
}

fn render_indeterminate_progress_bar(width: usize, spinner_index: usize) -> String {
    if width == 0 {
        return String::new();
    }

    let position = spinner_index % width;
    let prefix = "░".repeat(position);
    let pulse = color_hex_if_enabled("█", BITLOOPS_PURPLE_HEX);
    let suffix = "░".repeat(width.saturating_sub(position + 1));
    format!("{prefix}{pulse}{suffix}")
}

fn fit_live_status_text(text: &str, available_width: Option<usize>) -> String {
    let Some(max_width) = available_width else {
        return text.to_string();
    };
    if max_width == 0 {
        return String::new();
    }
    if text.chars().count() <= max_width {
        return text.to_string();
    }
    elide_middle(text, max_width)
}

fn elide_middle(text: &str, max_width: usize) -> String {
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

fn humanise_sync_phase(phase: &str) -> &'static str {
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

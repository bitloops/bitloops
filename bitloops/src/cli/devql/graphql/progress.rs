use std::io::{self, IsTerminal, Write};
use std::time::Duration;

use anyhow::Result;
use terminal_size::{Width, terminal_size};

use super::types::TaskGraphqlRecord;
use crate::utils::branding::{BITLOOPS_PURPLE_HEX, color_hex_if_enabled};

pub(super) const TASK_PROGRESS_POLL_INTERVAL: Duration = Duration::from_secs(1);
pub(super) const TASK_RENDER_TICK_INTERVAL: Duration = Duration::from_millis(120);
const TASK_SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub(super) struct TaskProgressRenderer {
    interactive: bool,
    terminal_width: Option<usize>,
    spinner_index: usize,
    last_frame: Option<String>,
    wrote_in_place: bool,
}

impl TaskProgressRenderer {
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

    pub(super) fn render(&mut self, task: &TaskGraphqlRecord) -> Result<()> {
        let frame = self.render_frame(task);
        self.write_frame(frame, false)
    }

    pub(super) fn tick(&mut self, task: &TaskGraphqlRecord) -> Result<()> {
        if !self.interactive || !matches!(status_key(task).as_str(), "queued" | "running") {
            return Ok(());
        }
        self.spinner_index = (self.spinner_index + 1) % TASK_SPINNER_FRAMES.len();
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
        color_hex_if_enabled(TASK_SPINNER_FRAMES[self.spinner_index], BITLOOPS_PURPLE_HEX)
    }

    fn render_frame(&self, task: &TaskGraphqlRecord) -> String {
        if self.interactive {
            let bar =
                format_live_task_progress_bar_line(task, self.spinner_index, self.terminal_width);
            let status =
                format_live_task_status_line(task, &self.spinner_frame(), self.terminal_width);
            format!("{bar}\n{status}")
        } else {
            format_live_task_status_line(task, &self.spinner_frame(), self.terminal_width)
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

pub(crate) fn format_live_task_status_line(
    task: &TaskGraphqlRecord,
    spinner: &str,
    terminal_width: Option<usize>,
) -> String {
    let status = match (kind_key(task).as_str(), status_key(task).as_str()) {
        ("sync", "queued") => format!(
            "Sync queued for {} · mode={} · {} ahead",
            task.repo_name,
            task.sync_spec
                .as_ref()
                .map(|spec| spec.mode.as_str())
                .unwrap_or("auto"),
            task.tasks_ahead.unwrap_or(0),
        ),
        ("sync", "running") => {
            let progress = task.sync_progress.as_ref();
            let mut line = format!(
                "Syncing {} · {}",
                task.repo_name,
                progress
                    .map(|progress| humanise_sync_phase(progress.phase.as_str()))
                    .unwrap_or("working"),
            );
            if let Some(progress) = progress {
                if progress.paths_total > 0 {
                    line.push_str(&format!(
                        " · {}/{}",
                        progress.paths_completed, progress.paths_total
                    ));
                }
                if let Some(path) = progress.current_path.as_ref() {
                    line.push_str(&format!(" · {path}"));
                }
            }
            line
        }
        ("ingest", "queued") => format!(
            "Ingest queued for {} · {} ahead",
            task.repo_name,
            task.tasks_ahead.unwrap_or(0),
        ),
        ("ingest", "running") => {
            let progress = task.ingest_progress.as_ref();
            let mut line = format!(
                "Ingesting {} · {}",
                task.repo_name,
                progress
                    .map(|progress| humanise_ingest_phase(progress.phase.as_str()))
                    .unwrap_or("working"),
            );
            if let Some(progress) = progress {
                if progress.commits_total > 0 {
                    line.push_str(&format!(
                        " · {}/{}",
                        progress.commits_processed, progress.commits_total
                    ));
                }
                if let Some(commit_sha) = progress.current_commit_sha.as_ref() {
                    line.push_str(&format!(" · {commit_sha}"));
                }
            }
            line
        }
        ("embeddings_bootstrap", "queued") => format!(
            "Embeddings bootstrap queued for {} · profile={} · {} ahead",
            task.repo_name,
            task.embeddings_bootstrap_spec
                .as_ref()
                .map(|spec| spec.profile_name.as_str())
                .unwrap_or("default"),
            task.tasks_ahead.unwrap_or(0),
        ),
        ("embeddings_bootstrap", "running") => {
            let progress = task.embeddings_bootstrap_progress.as_ref();
            let mut line = format!(
                "Bootstrapping embeddings for {} · {}",
                task.repo_name,
                progress
                    .map(|progress| humanise_bootstrap_phase(progress.phase.as_str()))
                    .unwrap_or("working"),
            );
            if let Some(progress) = progress {
                if let (downloaded, Some(total)) = (progress.bytes_downloaded, progress.bytes_total)
                    && total > 0
                {
                    line.push_str(&format!(" · {downloaded}/{total} bytes"));
                }
                if let Some(asset_name) = progress.asset_name.as_ref() {
                    line.push_str(&format!(" · {asset_name}"));
                } else if let Some(message) = progress.message.as_ref() {
                    line.push_str(&format!(" · {message}"));
                }
            }
            line
        }
        ("summary_bootstrap", "queued") => format!(
            "Summary bootstrap queued for {} · action={} · {} ahead",
            task.repo_name,
            task.summary_bootstrap_spec
                .as_ref()
                .map(|spec| spec.action.as_str())
                .unwrap_or("configure_cloud"),
            task.tasks_ahead.unwrap_or(0),
        ),
        ("summary_bootstrap", "running") => {
            let progress = task.summary_bootstrap_progress.as_ref();
            let mut line = format!(
                "Bootstrapping summaries for {} · {}",
                task.repo_name,
                progress
                    .map(|progress| humanise_summary_bootstrap_phase(progress.phase.as_str()))
                    .unwrap_or("working"),
            );
            if let Some(progress) = progress {
                if let (downloaded, Some(total)) = (progress.bytes_downloaded, progress.bytes_total)
                    && total > 0
                {
                    line.push_str(&format!(" · {downloaded}/{total} bytes"));
                }
                if let Some(asset_name) = progress.asset_name.as_ref() {
                    line.push_str(&format!(" · {asset_name}"));
                } else if let Some(message) = progress.message.as_ref() {
                    line.push_str(&format!(" · {message}"));
                }
            }
            line
        }
        ("sync", "completed") => format!("✓ Sync complete for {}", task.repo_name),
        ("ingest", "completed") => format!("✓ Ingest complete for {}", task.repo_name),
        ("embeddings_bootstrap", "completed") => {
            format!("✓ Embeddings bootstrap complete for {}", task.repo_name)
        }
        ("summary_bootstrap", "completed") => {
            format!("✓ Summary bootstrap complete for {}", task.repo_name)
        }
        ("sync", "failed") => format!("✖ Sync failed for {}", task.repo_name),
        ("ingest", "failed") => format!("✖ Ingest failed for {}", task.repo_name),
        ("embeddings_bootstrap", "failed") => {
            format!("✖ Embeddings bootstrap failed for {}", task.repo_name)
        }
        ("summary_bootstrap", "failed") => {
            format!("✖ Summary bootstrap failed for {}", task.repo_name)
        }
        ("sync", "cancelled") => format!("✖ Sync cancelled for {}", task.repo_name),
        ("ingest", "cancelled") => format!("✖ Ingest cancelled for {}", task.repo_name),
        ("embeddings_bootstrap", "cancelled") => {
            format!("✖ Embeddings bootstrap cancelled for {}", task.repo_name)
        }
        ("summary_bootstrap", "cancelled") => {
            format!("✖ Summary bootstrap cancelled for {}", task.repo_name)
        }
        _ => format!(
            "{} {} for {}",
            humanise_kind(task),
            status_key(task),
            task.repo_name
        ),
    };
    let fitted = fit_live_status_text(
        status.as_str(),
        terminal_width.map(|width| width.saturating_sub(2)),
    );
    format!("{spinner} {fitted}")
}

pub(crate) fn format_live_task_progress_bar_line(
    task: &TaskGraphqlRecord,
    spinner_index: usize,
    terminal_width: Option<usize>,
) -> String {
    let available_width = terminal_width.unwrap_or(80).max(16);
    let ratio = progress_ratio(task);
    let summary = if let Some((ratio, done, total)) = ratio {
        format!(" {:>3}% {done}/{total}", (ratio * 100.0).round() as usize)
    } else {
        format!(" {} ", humanise_phase(task))
    };
    let reserved = summary.chars().count() + 2;
    if available_width <= reserved + 1 {
        return fit_live_status_text(summary.trim(), Some(available_width));
    }

    let bar_width = available_width - reserved;
    let bar = if let Some((ratio, _, _)) = ratio {
        render_determinate_progress_bar(bar_width, ratio)
    } else {
        render_indeterminate_progress_bar(bar_width, spinner_index)
    };

    format!("[{bar}]{summary}")
}

fn progress_ratio(task: &TaskGraphqlRecord) -> Option<(f64, i32, i32)> {
    match kind_key(task).as_str() {
        "sync" => task.sync_progress.as_ref().and_then(|progress| {
            if progress.paths_total > 0 {
                Some((
                    (progress.paths_completed as f64 / progress.paths_total as f64).clamp(0.0, 1.0),
                    progress.paths_completed,
                    progress.paths_total,
                ))
            } else if status_key(task) == "completed" {
                Some((1.0, 1, 1))
            } else {
                None
            }
        }),
        "ingest" => task.ingest_progress.as_ref().and_then(|progress| {
            if progress.phase.eq_ignore_ascii_case("persisting") {
                None
            } else if progress.commits_total > 0 {
                Some((
                    (progress.commits_processed as f64 / progress.commits_total as f64)
                        .clamp(0.0, 1.0),
                    progress.commits_processed,
                    progress.commits_total,
                ))
            } else if status_key(task) == "completed" {
                Some((1.0, 1, 1))
            } else {
                None
            }
        }),
        "embeddings_bootstrap" => {
            task.embeddings_bootstrap_progress
                .as_ref()
                .and_then(|progress| {
                    progress
                        .bytes_total
                        .and_then(|total| {
                            (total > 0).then_some((
                                (progress.bytes_downloaded as f64 / total as f64).clamp(0.0, 1.0),
                                progress.bytes_downloaded as i32,
                                total as i32,
                            ))
                        })
                        .or_else(|| (status_key(task) == "completed").then_some((1.0, 1, 1)))
                })
        }
        "summary_bootstrap" => task
            .summary_bootstrap_progress
            .as_ref()
            .and_then(|progress| {
                progress
                    .bytes_total
                    .and_then(|total| {
                        (total > 0).then_some((
                            (progress.bytes_downloaded as f64 / total as f64).clamp(0.0, 1.0),
                            progress.bytes_downloaded as i32,
                            total as i32,
                        ))
                    })
                    .or_else(|| (status_key(task) == "completed").then_some((1.0, 1, 1)))
            }),
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

fn kind_key(task: &TaskGraphqlRecord) -> String {
    task.kind.to_ascii_lowercase()
}

fn status_key(task: &TaskGraphqlRecord) -> String {
    task.status.to_ascii_lowercase()
}

fn humanise_kind(task: &TaskGraphqlRecord) -> &'static str {
    match kind_key(task).as_str() {
        "sync" => "Sync",
        "ingest" => "Ingest",
        "embeddings_bootstrap" => "Embeddings bootstrap",
        "summary_bootstrap" => "Summary bootstrap",
        _ => "Task",
    }
}

fn humanise_phase(task: &TaskGraphqlRecord) -> &'static str {
    match kind_key(task).as_str() {
        "sync" => task
            .sync_progress
            .as_ref()
            .map(|progress| humanise_sync_phase(progress.phase.as_str()))
            .unwrap_or("working"),
        "ingest" => task
            .ingest_progress
            .as_ref()
            .map(|progress| humanise_ingest_phase(progress.phase.as_str()))
            .unwrap_or("working"),
        "embeddings_bootstrap" => task
            .embeddings_bootstrap_progress
            .as_ref()
            .map(|progress| humanise_bootstrap_phase(progress.phase.as_str()))
            .unwrap_or("working"),
        "summary_bootstrap" => task
            .summary_bootstrap_progress
            .as_ref()
            .map(|progress| humanise_summary_bootstrap_phase(progress.phase.as_str()))
            .unwrap_or("working"),
        _ => "working",
    }
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

fn humanise_ingest_phase(phase: &str) -> &'static str {
    match phase.to_ascii_lowercase().as_str() {
        "initializing" => "initialising",
        "extracting" => "extracting checkpoints",
        "persisting" => "persisting state",
        "complete" => "complete",
        "failed" => "failed",
        _ => "working",
    }
}

fn humanise_bootstrap_phase(phase: &str) -> &'static str {
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

fn humanise_summary_bootstrap_phase(phase: &str) -> &'static str {
    match phase {
        "queued" => "waiting in queue",
        "resolving_release" => "resolving release",
        "downloading_runtime" => "downloading runtime",
        "extracting_runtime" => "extracting runtime",
        "rewriting_runtime" => "updating runtime config",
        "writing_profile" => "writing profile",
        "complete" => "complete",
        "failed" => "failed",
        _ => "working",
    }
}

use std::io::{IsTerminal, Write};

use anyhow::Result;
use terminal_size::{Width, terminal_size};

use crate::utils::branding::{BITLOOPS_PURPLE_HEX, color_hex_if_enabled};

use super::super::{
    BottomProgressState, INIT_SPINNER_FRAMES, InitChecklistState, SUCCESS_GREEN_HEX,
    SummaryProgressState,
};
use super::queue::{
    format_embedding_queue_complete_progress_bar_line, format_embedding_queue_progress_bar_line,
    format_embedding_queue_status_line, format_embedding_waiting_status_line,
    format_queue_waiting_progress_bar_line, format_summary_progress_bar_line,
    format_summary_status_line,
};
use super::task::{
    format_init_task_progress_bar_line, format_init_task_status_line, init_task_description,
};
use super::terminal::{
    clear_rendered_lines, fit_init_plain_line, format_count_u64, rendered_terminal_line_count,
};

pub(in super::super) struct InitProgressRenderer {
    pub(super) interactive: bool,
    pub(super) terminal_width: Option<usize>,
    pub(super) spinner_index: usize,
    pub(super) last_frame: Option<String>,
    pub(super) wrote_in_place: bool,
    pub(super) rendered_lines: usize,
}

impl InitProgressRenderer {
    pub(in super::super) fn new() -> Self {
        Self {
            interactive: std::io::stdout().is_terminal() && std::env::var("ACCESSIBLE").is_err(),
            terminal_width: terminal_size().map(|(Width(width), _)| width as usize),
            spinner_index: 0,
            last_frame: None,
            wrote_in_place: false,
            rendered_lines: 0,
        }
    }

    pub(in super::super) fn terminal_width(&self) -> Option<usize> {
        self.terminal_width
    }

    pub(in super::super) fn is_interactive(&self) -> bool {
        self.interactive
    }

    pub(in super::super) fn render(
        &mut self,
        out: &mut dyn Write,
        checklist: InitChecklistState,
        top_task: Option<&crate::cli::devql::graphql::TaskGraphqlRecord>,
        bottom_state: &BottomProgressState,
        summary_state: &SummaryProgressState,
    ) -> Result<()> {
        let frame = self.render_frame(checklist, top_task, bottom_state, summary_state);
        self.write_frame(out, frame, false)
    }

    pub(in super::super) fn tick(
        &mut self,
        out: &mut dyn Write,
        checklist: InitChecklistState,
        top_task: Option<&crate::cli::devql::graphql::TaskGraphqlRecord>,
        bottom_state: &BottomProgressState,
        summary_state: &SummaryProgressState,
    ) -> Result<()> {
        if !self.interactive {
            return Ok(());
        }
        self.spinner_index = (self.spinner_index + 1) % INIT_SPINNER_FRAMES.len();
        let frame = self.render_frame(checklist, top_task, bottom_state, summary_state);
        self.write_frame(out, frame, true)
    }

    pub(in super::super) fn finish(&mut self, out: &mut dyn Write) -> Result<()> {
        if self.interactive && self.wrote_in_place {
            writeln!(out)?;
            out.flush()?;
            self.wrote_in_place = false;
        }
        Ok(())
    }

    pub(super) fn render_frame(
        &self,
        checklist: InitChecklistState,
        top_task: Option<&crate::cli::devql::graphql::TaskGraphqlRecord>,
        bottom_state: &BottomProgressState,
        summary_state: &SummaryProgressState,
    ) -> String {
        let mut lines = Vec::new();
        let spinner =
            color_hex_if_enabled(INIT_SPINNER_FRAMES[self.spinner_index], BITLOOPS_PURPLE_HEX);
        let tick = color_hex_if_enabled("✓", SUCCESS_GREEN_HEX);
        if checklist.show_sync {
            lines.push(render_init_checklist_item(
                if checklist.sync_complete {
                    InitChecklistItemState::Complete
                } else {
                    InitChecklistItemState::Active
                },
                spinner.as_str(),
                tick.as_str(),
                "Analysing your current branch to know what's what",
                self.terminal_width,
            ));
        }
        if checklist.show_ingest {
            lines.push(render_init_checklist_item(
                if checklist.ingest_complete {
                    InitChecklistItemState::Complete
                } else {
                    InitChecklistItemState::Active
                },
                spinner.as_str(),
                tick.as_str(),
                "Analysing your git history because you know... history is important",
                self.terminal_width,
            ));
        }
        if checklist.show_embeddings {
            lines.push(render_init_checklist_item(
                match bottom_state {
                    BottomProgressState::QueueComplete { failed_jobs, .. } if *failed_jobs == 0 => {
                        InitChecklistItemState::Complete
                    }
                    BottomProgressState::QueueComplete { .. }
                    | BottomProgressState::BootstrapFailed(_) => InitChecklistItemState::Failed,
                    _ => InitChecklistItemState::Active,
                },
                spinner.as_str(),
                tick.as_str(),
                "Creating code embeddings for fast search using our local embeddings provider",
                self.terminal_width,
            ));
        }
        if checklist.show_summaries {
            lines.push(render_init_checklist_item(
                match summary_state {
                    SummaryProgressState::Complete { failed_jobs, .. } if *failed_jobs == 0 => {
                        InitChecklistItemState::Complete
                    }
                    SummaryProgressState::Complete { .. } | SummaryProgressState::Failed { .. } => {
                        InitChecklistItemState::Failed
                    }
                    _ => InitChecklistItemState::Active,
                },
                spinner.as_str(),
                tick.as_str(),
                "Configuring local semantic summaries with bitloops-inference",
                self.terminal_width,
            ));
        }
        lines.push(String::new());

        if let Some(task) = top_task {
            lines.push(fit_init_plain_line(
                init_task_description(task),
                self.terminal_width,
            ));
            lines.push(format_init_task_progress_bar_line(
                task,
                self.spinner_index,
                self.terminal_width,
            ));
            lines.push(format_init_task_status_line(
                task,
                spinner.as_str(),
                self.terminal_width,
            ));
        }
        match bottom_state {
            BottomProgressState::Bootstrap(task) | BottomProgressState::BootstrapFailed(task) => {
                if top_task.is_some() {
                    lines.push(String::new());
                }
                lines.push(fit_init_plain_line(
                    "Creating code embeddings for fast search using our local embeddings provider",
                    self.terminal_width,
                ));
                lines.push(format_init_task_progress_bar_line(
                    task,
                    self.spinner_index,
                    self.terminal_width,
                ));
                lines.push(format_init_task_status_line(
                    task,
                    spinner.as_str(),
                    self.terminal_width,
                ));
            }
            BottomProgressState::Queue {
                snapshot,
                baseline_total,
                completed_floor: _,
            } => {
                if top_task.is_some() {
                    lines.push(String::new());
                }
                lines.push(fit_init_plain_line(
                    "Creating code embeddings for fast search using our local embeddings provider",
                    self.terminal_width,
                ));
                lines.push(format_embedding_queue_progress_bar_line(
                    *snapshot,
                    *baseline_total,
                    self.spinner_index,
                    self.terminal_width,
                ));
                lines.push(format_embedding_queue_status_line(
                    *snapshot,
                    spinner.as_str(),
                ));
            }
            BottomProgressState::WaitingForQueue {
                baseline_total: _,
                completed_floor: _,
                completed_jobs,
                failed_jobs,
            } => {
                if top_task.is_some() {
                    lines.push(String::new());
                }
                lines.push(fit_init_plain_line(
                    "Creating code embeddings for fast search using our local embeddings provider",
                    self.terminal_width,
                ));
                lines.push(format_queue_waiting_progress_bar_line(
                    checklist,
                    self.spinner_index,
                    self.terminal_width,
                ));
                lines.push(format_embedding_waiting_status_line(
                    checklist,
                    *completed_jobs,
                    *failed_jobs,
                    spinner.as_str(),
                    self.terminal_width,
                ));
            }
            BottomProgressState::QueueComplete {
                failed_jobs,
                baseline_total,
            } => {
                if top_task.is_some() {
                    lines.push(String::new());
                }
                lines.push(fit_init_plain_line(
                    "Creating code embeddings for fast search using our local embeddings provider",
                    self.terminal_width,
                ));
                lines.push(format_embedding_queue_complete_progress_bar_line(
                    *baseline_total,
                    self.terminal_width,
                ));
                if *failed_jobs > 0 {
                    lines.push(format!(
                        "✖ Embedding queue finished with {} failed job(s)",
                        format_count_u64(*failed_jobs)
                    ));
                } else {
                    lines.push("✓ Embedding queue complete".to_string());
                }
            }
            BottomProgressState::Hidden => {}
        }
        if !matches!(summary_state, SummaryProgressState::Hidden) {
            if top_task.is_some() || !matches!(bottom_state, BottomProgressState::Hidden) {
                lines.push(String::new());
            }
            lines.push(fit_init_plain_line(
                "Configuring local semantic summaries with bitloops-inference",
                self.terminal_width,
            ));
            lines.push(format_summary_progress_bar_line(
                summary_state,
                self.spinner_index,
                self.terminal_width,
            ));
            lines.push(format_summary_status_line(
                summary_state,
                checklist,
                spinner.as_str(),
                tick.as_str(),
                self.terminal_width,
            ));
        }
        lines.join("\n")
    }

    fn write_frame(&mut self, out: &mut dyn Write, frame: String, force: bool) -> Result<()> {
        if self.interactive {
            if !force && self.last_frame.as_deref() == Some(frame.as_str()) {
                return Ok(());
            }
            if self.wrote_in_place {
                clear_rendered_lines(out, self.rendered_lines)?;
            } else {
                write!(out, "{frame}")?;
                out.flush()?;
                self.last_frame = Some(frame.clone());
                self.wrote_in_place = true;
                self.rendered_lines =
                    rendered_terminal_line_count(&frame, self.terminal_width).max(1);
                return Ok(());
            }
            write!(out, "{frame}")?;
            out.flush()?;
            self.last_frame = Some(frame.clone());
            self.wrote_in_place = true;
            self.rendered_lines = rendered_terminal_line_count(&frame, self.terminal_width).max(1);
            return Ok(());
        }

        if self.last_frame.as_deref() != Some(frame.as_str()) {
            writeln!(out, "{frame}")?;
            out.flush()?;
            self.last_frame = Some(frame);
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum InitChecklistItemState {
    Active,
    Complete,
    Failed,
}

fn render_init_checklist_item(
    state: InitChecklistItemState,
    spinner: &str,
    tick: &str,
    text: &str,
    terminal_width: Option<usize>,
) -> String {
    let icon = match state {
        InitChecklistItemState::Active => spinner.to_string(),
        InitChecklistItemState::Complete => tick.to_string(),
        InitChecklistItemState::Failed => "✖".to_string(),
    };
    fit_init_plain_line(&format!("{icon} {text}"), terminal_width)
}

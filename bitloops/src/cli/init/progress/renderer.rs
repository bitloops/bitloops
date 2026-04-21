use std::io::{IsTerminal, Write};

use anyhow::Result;
use terminal_size::{Width, terminal_size};

use crate::runtime_presentation::{
    INIT_CODE_EMBEDDINGS_LANE_LABEL, INIT_CODE_EMBEDDINGS_SECTION_TITLE, INIT_INGEST_LANE_LABEL,
    INIT_INGEST_SECTION_TITLE, INIT_SUMMARIES_LANE_LABEL, INIT_SUMMARIES_SECTION_TITLE,
    INIT_SUMMARY_EMBEDDINGS_LANE_LABEL, INIT_SUMMARY_EMBEDDINGS_SECTION_TITLE,
    INIT_SYNC_LANE_LABEL, INIT_SYNC_SECTION_TITLE,
};
use crate::utils::branding::{BITLOOPS_PURPLE_HEX, color_hex_if_enabled};

use super::bars::SUCCESS_GREEN_HEX;
use super::compact::{LaneRenderContext, compact_selected_section_titles, render_compact_lane};
use super::session_status::compact_session_status_line;
use super::task_lookup::task_for_lane;
use super::viewport::{clear_rendered_lines, fit_line, rendered_terminal_line_count};

const INIT_SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub(crate) struct RuntimeInitRenderer {
    interactive: bool,
    pub(crate) terminal_width: Option<usize>,
    spinner_index: usize,
    last_frame: Option<String>,
    wrote_in_place: bool,
    rendered_lines: usize,
}

impl RuntimeInitRenderer {
    pub(crate) fn new() -> Self {
        Self {
            interactive: std::io::stdout().is_terminal() && std::env::var("ACCESSIBLE").is_err(),
            terminal_width: terminal_size().map(|(Width(width), _)| width as usize),
            spinner_index: 0,
            last_frame: None,
            wrote_in_place: false,
            rendered_lines: 0,
        }
    }

    pub(crate) fn advance_spinner(&mut self) {
        self.spinner_index = (self.spinner_index + 1) % INIT_SPINNER_FRAMES.len();
    }

    pub(crate) fn render(
        &mut self,
        out: &mut dyn Write,
        snapshot: &crate::cli::devql::graphql::RuntimeSnapshotGraphqlRecord,
        session_id: &str,
    ) -> Result<()> {
        let frame = self.frame(snapshot, session_id);
        if self.last_frame.as_ref() == Some(&frame) {
            return Ok(());
        }

        if self.interactive && self.wrote_in_place {
            clear_rendered_lines(out, self.rendered_lines)?;
        }
        write!(out, "{frame}")?;
        out.flush()?;

        self.rendered_lines = rendered_terminal_line_count(frame.as_str(), self.terminal_width);
        self.last_frame = Some(frame);
        self.wrote_in_place = self.interactive;
        Ok(())
    }

    pub(crate) fn finish(&mut self, out: &mut dyn Write) -> Result<()> {
        if self.interactive && self.wrote_in_place {
            writeln!(out)?;
            out.flush()?;
            self.wrote_in_place = false;
        }
        Ok(())
    }

    fn frame(
        &self,
        snapshot: &crate::cli::devql::graphql::RuntimeSnapshotGraphqlRecord,
        session_id: &str,
    ) -> String {
        let session = snapshot
            .current_init_session
            .as_ref()
            .filter(|session| session.init_session_id == session_id);
        let spinner =
            color_hex_if_enabled(INIT_SPINNER_FRAMES[self.spinner_index], BITLOOPS_PURPLE_HEX);
        let tick = color_hex_if_enabled("✓", SUCCESS_GREEN_HEX);
        let render_context = LaneRenderContext {
            spinner: spinner.as_str(),
            tick: tick.as_str(),
            spinner_index: self.spinner_index,
            terminal_width: self.terminal_width,
        };
        let mut lines = Vec::new();

        let Some(session) = session else {
            lines.push(format!("{spinner} Waiting for init session state"));
            return lines.join("\n");
        };

        let selected_titles = compact_selected_section_titles(session);
        let label_width = selected_titles
            .iter()
            .map(|title| title.chars().count())
            .max()
            .unwrap_or(0)
            + 12;

        if session.run_sync {
            lines.extend(render_compact_lane(
                INIT_SYNC_SECTION_TITLE,
                &session.sync_lane,
                INIT_SYNC_LANE_LABEL,
                task_for_lane(snapshot, &session.sync_lane),
                None,
                label_width,
                &render_context,
            ));
        }

        if session.run_ingest {
            if !lines.is_empty() {
                lines.push(String::new());
            }
            lines.extend(render_compact_lane(
                INIT_INGEST_SECTION_TITLE,
                &session.ingest_lane,
                INIT_INGEST_LANE_LABEL,
                task_for_lane(snapshot, &session.ingest_lane),
                None,
                label_width,
                &render_context,
            ));
        }

        if session.embeddings_selected {
            if !lines.is_empty() {
                lines.push(String::new());
            }
            lines.extend(render_compact_lane(
                INIT_CODE_EMBEDDINGS_SECTION_TITLE,
                &session.code_embeddings_lane,
                INIT_CODE_EMBEDDINGS_LANE_LABEL,
                task_for_lane(snapshot, &session.code_embeddings_lane),
                None,
                label_width,
                &render_context,
            ));
        }

        if session.summaries_selected {
            if !lines.is_empty() {
                lines.push(String::new());
            }
            let summary_run = snapshot
                .summaries_bootstrap
                .as_ref()
                .and_then(|run| (run.init_session_id == session.init_session_id).then_some(run));
            lines.extend(render_compact_lane(
                INIT_SUMMARIES_SECTION_TITLE,
                &session.summaries_lane,
                INIT_SUMMARIES_LANE_LABEL,
                None,
                summary_run,
                label_width,
                &render_context,
            ));
        }

        if session.summary_embeddings_selected {
            if !lines.is_empty() {
                lines.push(String::new());
            }
            lines.extend(render_compact_lane(
                INIT_SUMMARY_EMBEDDINGS_SECTION_TITLE,
                &session.summary_embeddings_lane,
                INIT_SUMMARY_EMBEDDINGS_LANE_LABEL,
                task_for_lane(snapshot, &session.summary_embeddings_lane),
                None,
                label_width,
                &render_context,
            ));
        }

        lines.push(String::new());
        lines.push(fit_line("Session Status", self.terminal_width));
        lines.push(format!(
            " {} {}",
            if matches!(
                session.status.to_ascii_lowercase().as_str(),
                "completed" | "completed_with_warnings"
            ) {
                tick.as_str()
            } else {
                spinner.as_str()
            },
            fit_line(
                &compact_session_status_line(snapshot, session),
                self.terminal_width.map(|width| width.saturating_sub(4)),
            )
        ));

        lines.join("\n")
    }
}

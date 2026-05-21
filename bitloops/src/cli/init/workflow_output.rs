use std::io::Write;

use anyhow::Result;

use crate::utils::branding::{BITLOOPS_PURPLE_HEX, color_hex_if_enabled};

const SUCCESS_GREEN_HEX: &str = "#22c55e";
const INTEGRATION_SPINNER_FRAME: &str = "⠋";

pub(super) fn write_integrations_installing(
    out: &mut dyn Write,
    integrations: &[crate::cli::agent_surfaces::AgentIntegrationReport],
) -> Result<Option<usize>> {
    let spinner = color_hex_if_enabled(INTEGRATION_SPINNER_FRAME, BITLOOPS_PURPLE_HEX);
    let label_width = integrations
        .iter()
        .map(|integration| integration.label.chars().count())
        .max()
        .unwrap_or(0)
        + 3;
    let mut lines = Vec::new();
    lines.push("Installing integrations…".to_string());
    lines.push(String::new());
    for integration in integrations {
        lines.push(format!(
            "  {} {:<label_width$}({} hooks)",
            spinner,
            integration.label,
            integration.hook_count,
            label_width = label_width
        ));
    }

    for (index, line) in lines.iter().enumerate() {
        write!(out, "{line}")?;
        if index + 1 < lines.len() {
            writeln!(out)?;
        }
    }
    out.flush()?;

    #[cfg(test)]
    {
        Ok(None)
    }

    #[cfg(not(test))]
    {
        if super::agent_selection::can_prompt_interactively() {
            Ok(Some(lines.len()))
        } else {
            Ok(None)
        }
    }
}

pub(super) fn write_integrations_installed(
    out: &mut dyn Write,
    integrations: &[crate::cli::agent_surfaces::AgentIntegrationReport],
    previous_lines: Option<usize>,
) -> Result<()> {
    let tick = color_hex_if_enabled("✓", SUCCESS_GREEN_HEX);
    let label_width = integrations
        .iter()
        .map(|integration| integration.label.chars().count())
        .max()
        .unwrap_or(0)
        + 3;
    let mut lines = Vec::new();
    lines.push("Integrations installed:".to_string());
    lines.push(String::new());
    for integration in integrations {
        let detail = if integration.state
            == crate::cli::agent_surfaces::AgentIntegrationState::AlreadyInstalled
        {
            format!("{} hooks were already installed", integration.hook_count)
        } else {
            format!("{} hooks", integration.hook_count)
        };
        lines.push(format!(
            "  {} {:<label_width$}({detail})",
            tick,
            integration.label,
            label_width = label_width
        ));
    }

    if let Some(previous_lines) = previous_lines {
        if previous_lines > 0 {
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
    writeln!(out)?;
    writeln!(out)?;
    out.flush()?;
    Ok(())
}

pub(super) fn planned_integrations(
    selected_agents: &[String],
) -> Vec<crate::cli::agent_surfaces::AgentIntegrationReport> {
    selected_agents
        .iter()
        .map(|agent| crate::cli::agent_surfaces::AgentIntegrationReport {
            agent: agent.clone(),
            label: super::agent_hooks::agent_display(agent),
            hook_count: planned_hook_count(agent),
            newly_installed_hook_count: 0,
            state: crate::cli::agent_surfaces::AgentIntegrationState::Installed,
        })
        .collect()
}

fn planned_hook_count(agent: &str) -> usize {
    match agent {
        crate::adapters::agents::AGENT_NAME_CLAUDE_CODE => 7,
        crate::adapters::agents::AGENT_NAME_COPILOT => 8,
        crate::adapters::agents::AGENT_NAME_CODEX => 5,
        crate::adapters::agents::AGENT_NAME_CURSOR => 9,
        crate::adapters::agents::AGENT_NAME_GEMINI => 12,
        crate::adapters::agents::AGENT_NAME_OPEN_CODE => 5,
        _ => 0,
    }
}

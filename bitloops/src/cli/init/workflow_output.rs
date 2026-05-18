use std::io::Write;
use std::path::Path;

use anyhow::Result;
#[cfg(test)]
use std::{cell::RefCell, rc::Rc};

use crate::utils::branding::{BITLOOPS_PURPLE_HEX, bitloops_wordmark, color_hex_if_enabled};
use crate::utils::platform_dirs::bitloops_home_dir;

const SUCCESS_GREEN_HEX: &str = "#22c55e";
const INTEGRATION_SPINNER_FRAME: &str = "⠋";
const INIT_CONFIGURATION_ROUTE: &str = "/settings/configuration";

#[cfg(test)]
type DashboardOpenHook = dyn Fn(&str) -> Result<()> + 'static;

#[cfg(test)]
thread_local! {
    static DASHBOARD_OPEN_HOOK: RefCell<Option<Rc<DashboardOpenHook>>> =
        RefCell::new(None);
}

fn shell_escape_display_path(path: &Path) -> String {
    let preferred = display_path_with_home(path);
    preferred
        .chars()
        .flat_map(|ch| match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '/' | '.' | '_' | '-' | '~' => [None, Some(ch)],
            _ => [Some('\\'), Some(ch)],
        })
        .flatten()
        .collect()
}

fn display_path_with_home(path: &Path) -> String {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let Ok(home) = bitloops_home_dir() else {
        return canonical.display().to_string();
    };
    if canonical == home {
        return "~".to_string();
    }
    if let Ok(relative) = canonical.strip_prefix(&home) {
        let relative = relative.to_string_lossy();
        if relative.is_empty() {
            "~".to_string()
        } else {
            format!("~/{}", relative)
        }
    } else {
        canonical.display().to_string()
    }
}

pub(super) fn write_default_daemon_bootstrap(
    out: &mut dyn Write,
    config_path: &Path,
    port: u16,
) -> Result<()> {
    writeln!(out, "Starting Bitloops daemon…")?;
    writeln!(out, "  config: {}", shell_escape_display_path(config_path))?;
    writeln!(out, "  port:   {port}")?;
    writeln!(out)?;
    out.flush()?;
    Ok(())
}

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

#[derive(Debug, Clone, Copy)]
pub(super) struct InitSetupHandoffOptions {
    pub(super) run_sync: bool,
    pub(super) run_ingest: bool,
    pub(super) run_code_embeddings: bool,
    pub(super) run_summaries: bool,
    pub(super) run_summary_embeddings: bool,
    pub(super) prepare_embeddings_runtime: bool,
    pub(super) prepare_summary_generation: bool,
}

pub(super) async fn write_init_setup_handoff(
    out: &mut dyn Write,
    options: InitSetupHandoffOptions,
) -> Result<()> {
    let tick = color_hex_if_enabled("✓", SUCCESS_GREEN_HEX);
    let dashboard_url = current_dashboard_url()
        .await?
        .unwrap_or_else(default_dashboard_url_for_init_handoff);
    let dashboard_opened = maybe_open_dashboard_for_init_handoff(&dashboard_url);
    let mut background_steps = Vec::new();
    if options.run_sync {
        background_steps.push("Syncing your current codebase");
    }
    if options.run_ingest {
        background_steps.push("Ingesting your git history");
    }
    if options.run_code_embeddings {
        background_steps.push("Creating code embeddings for semantic search");
    } else if options.prepare_embeddings_runtime {
        background_steps.push("Preparing the embeddings runtime");
    }
    if options.run_summaries {
        background_steps.push("Generating file and module summaries");
    } else if options.prepare_summary_generation {
        background_steps.push("Preparing summary generation");
    }
    if options.run_summary_embeddings {
        background_steps.push("Creating summary embeddings");
    }

    writeln!(out)?;
    writeln!(
        out,
        "{}",
        color_hex_if_enabled(&bitloops_wordmark(), BITLOOPS_PURPLE_HEX)
    )?;
    writeln!(out)?;
    writeln!(out, "{tick} Setup complete")?;
    writeln!(out)?;
    if background_steps.is_empty() {
        writeln!(
            out,
            "Bitloops is ready. No background indexing steps were selected during setup."
        )?;
        writeln!(out)?;
    } else {
        writeln!(
            out,
            "Bitloops is now continuing the setup you selected in the background."
        )?;
        writeln!(out)?;
        writeln!(out, "What’s happening:")?;
        for step in background_steps {
            writeln!(out, "  • {step}")?;
        }
        writeln!(out)?;
    }
    if dashboard_opened {
        writeln!(out, "Opening the dashboard in your browser…")?;
        writeln!(out)?;
    }
    writeln!(out, "You can:")?;
    writeln!(out, "  • View progress: {dashboard_url}")?;
    writeln!(out, "  • Check status anytime: bitloops init status")?;
    writeln!(
        out,
        "  • Close this terminal — setup will continue in the background"
    )?;
    writeln!(out)?;
    if should_render_local_http_mkcert_notice(&dashboard_url) {
        write_local_http_mkcert_notice(out)?;
    }
    writeln!(
        out,
        "──────────────────────────────────────────────────────────────────"
    )?;
    writeln!(out, "                   🔍 Live Progress")?;
    writeln!(
        out,
        " Feel free to close this terminal and continue with your day! 🌟"
    )?;
    writeln!(
        out,
        "──────────────────────────────────────────────────────────────────"
    )?;
    writeln!(out)?;
    out.flush()?;
    Ok(())
}

fn default_dashboard_url_for_init_handoff() -> String {
    let scheme = if crate::api::tls::mkcert_on_path() {
        "https"
    } else {
        "http"
    };
    format!(
        "{scheme}://127.0.0.1:{}{}",
        crate::api::DEFAULT_DASHBOARD_PORT,
        INIT_CONFIGURATION_ROUTE
    )
}

fn should_render_local_http_mkcert_notice(dashboard_url: &str) -> bool {
    dashboard_url.starts_with("http://") && !crate::api::tls::mkcert_on_path()
}

fn write_local_http_mkcert_notice(out: &mut dyn Write) -> Result<()> {
    writeln!(
        out,
        "Notice: local dashboard HTTPS is unavailable because `mkcert` is not on your PATH."
    )?;
    writeln!(
        out,
        "Install `mkcert`, run `mkcert -install`, then run `bitloops daemon start --recheck-local-dashboard-net`."
    )?;
    writeln!(
        out,
        "Guide: {}",
        crate::api::tls::LOCAL_HTTPS_SETUP_DOCS_URL
    )?;
    writeln!(out)?;
    Ok(())
}

async fn current_dashboard_url() -> Result<Option<String>> {
    Ok(crate::daemon::runtime_state()?.map(|runtime| configuration_dashboard_url(&runtime.url)))
}

fn configuration_dashboard_url(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.ends_with(INIT_CONFIGURATION_ROUTE) {
        trimmed.to_string()
    } else {
        format!("{trimmed}{INIT_CONFIGURATION_ROUTE}")
    }
}

fn maybe_open_dashboard_for_init_handoff(dashboard_url: &str) -> bool {
    if !crate::cli::telemetry_consent::can_prompt_interactively() {
        return false;
    }

    #[cfg(test)]
    if let Some(result) = maybe_run_dashboard_open_hook(dashboard_url) {
        return result.is_ok();
    }

    #[cfg(not(test))]
    {
        if let Err(err) = crate::api::open_in_default_browser(dashboard_url) {
            log::warn!(
                "failed to open dashboard after `bitloops init --install-default-daemon`: {err:#}"
            );
            return false;
        }
        true
    }

    #[cfg(test)]
    {
        false
    }
}

#[cfg(test)]
fn maybe_run_dashboard_open_hook(dashboard_url: &str) -> Option<Result<()>> {
    DASHBOARD_OPEN_HOOK.with(|cell: &RefCell<Option<Rc<DashboardOpenHook>>>| {
        cell.borrow()
            .as_ref()
            .map(|hook| hook(dashboard_url))
    })
}

#[cfg(test)]
pub(crate) fn with_dashboard_open_hook<T>(
    hook: impl Fn(&str) -> Result<()> + 'static,
    f: impl FnOnce() -> T,
) -> T {
    DASHBOARD_OPEN_HOOK.with(|cell: &RefCell<Option<Rc<DashboardOpenHook>>>| {
        assert!(
            cell.borrow().is_none(),
            "dashboard open hook already installed"
        );
        *cell.borrow_mut() = Some(Rc::new(hook));
    });
    let result = f();
    DASHBOARD_OPEN_HOOK.with(|cell: &RefCell<Option<Rc<DashboardOpenHook>>>| {
        *cell.borrow_mut() = None;
    });
    result
}

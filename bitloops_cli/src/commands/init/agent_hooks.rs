use std::path::Path;

use anyhow::{Result, bail};

use crate::engine::agent::HookSupport;
use crate::engine::agent::claude_code::hooks as claude_hooks;
use crate::engine::agent::codex::hooks as codex_hooks;
use crate::engine::agent::copilot_cli::agent::CopilotCliAgent;
use crate::engine::agent::cursor::agent::CursorAgent;
use crate::engine::agent::gemini_cli::agent::GeminiCliAgent;
use crate::engine::agent::open_code::agent::OpenCodeAgent;

pub(super) const AGENT_CLAUDE_CODE: &str = "claude-code";
pub(super) const AGENT_CODEX: &str = "codex";
pub(super) const AGENT_CURSOR: &str = "cursor";
pub(super) const AGENT_GEMINI_CLI: &str = "gemini-cli";
pub(super) const AGENT_OPEN_CODE: &str = "opencode";
pub(super) const DEFAULT_AGENT: &str = AGENT_CLAUDE_CODE;
pub(super) const AGENT_COPILOT: &str = "copilot";

pub(super) fn install_agent_hooks(
    repo_root: &Path,
    agent_name: &str,
    local_dev: bool,
    force: bool,
) -> Result<(String, usize)> {
    match agent_name {
        AGENT_CLAUDE_CODE => Ok((
            "Claude Code".to_string(),
            claude_hooks::install_hooks(repo_root, force)?,
        )),
        AGENT_COPILOT => Ok((
            "Copilot".to_string(),
            HookSupport::install_hooks(&CopilotCliAgent, local_dev, force)?,
        )),
        AGENT_CODEX => Ok((
            "Codex CLI".to_string(),
            codex_hooks::install_hooks_at(repo_root, local_dev, force)?,
        )),
        AGENT_CURSOR => Ok((
            "Cursor".to_string(),
            HookSupport::install_hooks(&CursorAgent, local_dev, force)?,
        )),
        AGENT_GEMINI_CLI => Ok((
            "Gemini CLI".to_string(),
            HookSupport::install_hooks(&GeminiCliAgent, local_dev, force)?,
        )),
        AGENT_OPEN_CODE => Ok((
            "OpenCode".to_string(),
            HookSupport::install_hooks(&OpenCodeAgent, local_dev, force)?,
        )),
        other => bail!("unknown agent name: {other}"),
    }
}

pub(super) fn normalize_agent_name(value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("missing agent name");
    }

    match trimmed {
        AGENT_CLAUDE_CODE => Ok(AGENT_CLAUDE_CODE.to_string()),
        AGENT_COPILOT | "copilot-cli" => Ok(AGENT_COPILOT.to_string()),
        AGENT_CODEX => Ok(AGENT_CODEX.to_string()),
        AGENT_CURSOR => Ok(AGENT_CURSOR.to_string()),
        AGENT_GEMINI_CLI | "gemini" => Ok(AGENT_GEMINI_CLI.to_string()),
        AGENT_OPEN_CODE | "open-code" => Ok(AGENT_OPEN_CODE.to_string()),
        _ => bail!("unknown agent name: {trimmed}"),
    }
}

pub(super) fn detect_agents(repo_root: &Path) -> Vec<String> {
    let mut detected = Vec::new();

    if repo_root.join(".claude").is_dir() {
        detected.push(AGENT_CLAUDE_CODE.to_string());
    }
    if HookSupport::are_hooks_installed(&CopilotCliAgent) {
        detected.push(AGENT_COPILOT.to_string());
    }
    if repo_root.join(".codex").is_dir() {
        detected.push(AGENT_CODEX.to_string());
    }
    if repo_root.join(".cursor").is_dir() {
        detected.push(AGENT_CURSOR.to_string());
    }
    if repo_root.join(".gemini").is_dir() {
        detected.push(AGENT_GEMINI_CLI.to_string());
    }
    if repo_root.join(".opencode").is_dir() {
        detected.push(AGENT_OPEN_CODE.to_string());
    }

    detected
}

pub(super) fn available_agents() -> Vec<String> {
    vec![
        AGENT_CLAUDE_CODE.to_string(),
        AGENT_COPILOT.to_string(),
        AGENT_CODEX.to_string(),
        AGENT_CURSOR.to_string(),
        AGENT_GEMINI_CLI.to_string(),
        AGENT_OPEN_CODE.to_string(),
    ]
}

pub(super) fn agent_display(agent: &str) -> &'static str {
    match agent {
        AGENT_CLAUDE_CODE => "Claude Code",
        AGENT_COPILOT => "Copilot",
        AGENT_CODEX => "Codex CLI",
        AGENT_CURSOR => "Cursor",
        AGENT_GEMINI_CLI => "Gemini CLI",
        AGENT_OPEN_CODE => "OpenCode",
        _ => "Unknown",
    }
}

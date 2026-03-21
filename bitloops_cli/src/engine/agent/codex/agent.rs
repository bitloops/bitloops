use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;

use anyhow::{Result, anyhow};

use crate::engine::agent::{
    AGENT_NAME_CODEX, AGENT_TYPE_CODEX, Agent, AgentSession, Event, HookInput, HookSupport,
    chunk_jsonl, reassemble_jsonl,
};

use super::hooks;
use super::lifecycle;

#[derive(Debug, Default, Clone, Copy)]
pub struct CodexAgent;

impl Agent for CodexAgent {
    fn name(&self) -> String {
        AGENT_NAME_CODEX.to_string()
    }

    fn agent_type(&self) -> String {
        AGENT_TYPE_CODEX.to_string()
    }

    fn description(&self) -> String {
        "Codex - OpenAI coding agent".to_string()
    }

    fn is_preview(&self) -> bool {
        true
    }

    fn detect_presence(&self) -> Result<bool> {
        let repo_root = crate::utils::paths::repo_root().unwrap_or_else(|_| PathBuf::from("."));
        Ok(repo_root.join(".codex").is_dir() || repo_root.join(".codex/hooks.json").exists())
    }

    fn get_session_id(&self, input: &HookInput) -> String {
        input.session_id.clone()
    }

    fn protected_dirs(&self) -> Vec<String> {
        vec![".codex".to_string()]
    }

    fn hook_names(&self) -> Vec<String> {
        vec![
            lifecycle::HOOK_NAME_SESSION_START.to_string(),
            lifecycle::HOOK_NAME_STOP.to_string(),
        ]
    }

    fn parse_hook_event(
        &self,
        hook_name: &str,
        stdin: &mut dyn std::io::Read,
    ) -> Result<Option<Event>> {
        if lifecycle::parse_hook_event(hook_name, stdin)?.is_some() {
            return Ok(Some(Event));
        }
        Ok(None)
    }

    fn read_transcript(&self, session_ref: &str) -> Result<Vec<u8>> {
        std::fs::read(session_ref).map_err(|err| anyhow!("failed to read transcript: {err}"))
    }

    fn chunk_transcript(&self, content: &[u8], max_size: usize) -> Result<Vec<Vec<u8>>> {
        chunk_jsonl(content, max_size)
            .map_err(|err| anyhow!("failed to chunk Codex transcript: {err}"))
    }

    fn reassemble_transcript(&self, chunks: &[Vec<u8>]) -> Result<Vec<u8>> {
        Ok(reassemble_jsonl(chunks))
    }

    fn resolve_session_file(&self, session_dir: &str, agent_session_id: &str) -> String {
        Path::new(session_dir)
            .join(format!("{agent_session_id}.jsonl"))
            .to_string_lossy()
            .to_string()
    }

    fn read_session(&self, input: &HookInput) -> Result<Option<AgentSession>> {
        if input.session_ref.is_empty() {
            return Err(anyhow!("session reference (transcript path) is required"));
        }

        let data = std::fs::read(&input.session_ref)
            .map_err(|err| anyhow!("failed to read transcript: {err}"))?;

        Ok(Some(AgentSession {
            session_id: input.session_id.clone(),
            agent_name: self.name(),
            session_ref: input.session_ref.clone(),
            start_time: SystemTime::now(),
            native_data: data,
            ..AgentSession::default()
        }))
    }

    fn write_session(&self, session: &AgentSession) -> Result<()> {
        if !session.agent_name.is_empty() && session.agent_name != self.name() {
            return Err(anyhow!(
                "session belongs to agent \"{}\", not \"{}\"",
                session.agent_name,
                self.name()
            ));
        }
        if session.session_ref.is_empty() {
            return Err(anyhow!("session reference (transcript path) is required"));
        }
        if session.native_data.is_empty() {
            return Err(anyhow!("session has no native data to write"));
        }
        std::fs::write(&session.session_ref, &session.native_data)
            .map_err(|err| anyhow!("failed to write transcript: {err}"))
    }

    fn format_resume_command(&self, session_id: &str) -> String {
        if session_id.trim().is_empty() {
            "codex".to_string()
        } else {
            format!("codex --resume {session_id}")
        }
    }
}

impl HookSupport for CodexAgent {
    fn install_hooks(&self, local_dev: bool, force: bool) -> Result<usize> {
        hooks::install_hooks(local_dev, force)
    }

    fn uninstall_hooks(&self) -> Result<()> {
        hooks::uninstall_hooks()
    }

    fn are_hooks_installed(&self) -> bool {
        hooks::are_hooks_installed()
    }
}

#[cfg(test)]
#[path = "agent_tests.rs"]
mod tests;

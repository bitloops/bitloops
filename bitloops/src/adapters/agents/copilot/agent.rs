use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Result, anyhow};

use crate::adapters::agents::{
    AGENT_NAME_COPILOT, AGENT_TYPE_COPILOT, Agent, AgentSession, HookInput, HookSupport,
    TokenCalculator, TokenUsage, TranscriptAnalyzer, chunk_jsonl, reassemble_jsonl,
};

use super::hooks;
use super::lifecycle;
use super::transcript::{
    calculate_token_usage_from_events, extract_modified_files_from_events,
    extract_prompts_from_events, extract_summary_from_events, get_transcript_position_from_bytes,
    parse_events_from_offset,
};

#[derive(Debug, Default, Clone, Copy)]
pub struct CopilotCliAgent;

impl Agent for CopilotCliAgent {
    fn name(&self) -> String {
        AGENT_NAME_COPILOT.to_string()
    }

    fn agent_type(&self) -> String {
        AGENT_TYPE_COPILOT.to_string()
    }

    fn description(&self) -> String {
        "Copilot".to_string()
    }

    fn is_preview(&self) -> bool {
        true
    }

    fn detect_presence(&self) -> Result<bool> {
        Ok(hooks::are_hooks_installed())
    }

    fn get_session_id(&self, input: &HookInput) -> String {
        input.session_id.clone()
    }

    fn protected_dirs(&self) -> Vec<String> {
        vec![".github/hooks".to_string()]
    }

    fn hook_names(&self) -> Vec<String> {
        vec![
            lifecycle::HOOK_NAME_USER_PROMPT_SUBMITTED.to_string(),
            lifecycle::HOOK_NAME_SESSION_START.to_string(),
            lifecycle::HOOK_NAME_AGENT_STOP.to_string(),
            lifecycle::HOOK_NAME_SESSION_END.to_string(),
            lifecycle::HOOK_NAME_SUBAGENT_STOP.to_string(),
            lifecycle::HOOK_NAME_PRE_TOOL_USE.to_string(),
            lifecycle::HOOK_NAME_POST_TOOL_USE.to_string(),
            lifecycle::HOOK_NAME_ERROR_OCCURRED.to_string(),
        ]
    }

    fn parse_hook_event(
        &self,
        hook_name: &str,
        stdin: &mut dyn std::io::Read,
    ) -> Result<Option<crate::adapters::agents::Event>> {
        if lifecycle::parse_hook_event(hook_name, stdin)?.is_some() {
            return Ok(Some(crate::adapters::agents::Event));
        }
        Ok(None)
    }

    fn read_transcript(&self, session_ref: &str) -> Result<Vec<u8>> {
        std::fs::read(session_ref).map_err(|err| anyhow!("failed to read transcript: {err}"))
    }

    fn chunk_transcript(&self, content: &[u8], max_size: usize) -> Result<Vec<Vec<u8>>> {
        chunk_jsonl(content, max_size).map_err(|err| anyhow!("failed to chunk transcript: {err}"))
    }

    fn reassemble_transcript(&self, chunks: &[Vec<u8>]) -> Result<Vec<u8>> {
        Ok(reassemble_jsonl(chunks))
    }

    fn get_session_dir(&self, _repo_path: &str) -> Result<String> {
        Self::session_dir_from_override_or_home(
            std::env::var("BITLOOPS_TEST_COPILOT_SESSION_DIR")
                .ok()
                .as_deref(),
            std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .as_deref()
                .map(Path::new),
        )
    }

    fn resolve_session_file(&self, session_dir: &str, agent_session_id: &str) -> String {
        Path::new(session_dir)
            .join(agent_session_id)
            .join("events.jsonl")
            .to_string_lossy()
            .to_string()
    }

    fn read_session(&self, input: &HookInput) -> Result<Option<AgentSession>> {
        if input.session_ref.is_empty() {
            return Err(anyhow!("session reference (transcript path) is required"));
        }

        let data = std::fs::read(&input.session_ref)
            .map_err(|err| anyhow!("failed to read transcript: {err}"))?;

        let (events, _) = parse_events_from_offset(&data, 0)?;
        let modified_files = extract_modified_files_from_events(&events);

        Ok(Some(AgentSession {
            session_id: input.session_id.clone(),
            agent_name: self.name(),
            session_ref: input.session_ref.clone(),
            start_time: SystemTime::now(),
            native_data: data,
            modified_files,
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

        let path = Path::new(&session.session_ref);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| anyhow!("failed to create transcript directory: {err}"))?;
        }
        std::fs::write(path, &session.native_data)
            .map_err(|err| anyhow!("failed to write transcript: {err}"))
    }

    fn format_resume_command(&self, session_id: &str) -> String {
        format!("copilot --resume {session_id}")
    }

    fn as_transcript_entry_deriver(
        &self,
    ) -> Option<&dyn crate::adapters::agents::TranscriptEntryDeriver> {
        Some(self)
    }
}

impl HookSupport for CopilotCliAgent {
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

impl TranscriptAnalyzer for CopilotCliAgent {
    fn get_transcript_position(&self, path: &str) -> Result<usize> {
        Self::get_transcript_position_impl(path)
    }

    fn extract_modified_files_from_offset(
        &self,
        path: &str,
        start_offset: usize,
    ) -> Result<(Vec<String>, usize)> {
        Self::extract_modified_files_from_offset_impl(path, start_offset)
    }

    fn extract_prompts(&self, session_ref: &str, from_offset: usize) -> Result<Vec<String>> {
        Self::extract_prompts_impl(session_ref, from_offset)
    }

    fn extract_summary(&self, session_ref: &str) -> Result<String> {
        Self::extract_summary_impl(session_ref)
    }
}

impl TokenCalculator for CopilotCliAgent {
    fn calculate_token_usage(&self, session_ref: &str, from_offset: usize) -> Result<TokenUsage> {
        Self::calculate_token_usage_impl(session_ref, from_offset)
    }
}

impl CopilotCliAgent {
    fn session_dir_from_override_or_home(
        override_path: Option<&str>,
        home_dir: Option<&Path>,
    ) -> Result<String> {
        if let Some(override_path) = override_path
            && !override_path.is_empty()
        {
            return Ok(override_path.to_string());
        }

        let home_dir = home_dir.ok_or_else(|| anyhow!("failed to get home directory"))?;
        Ok(home_dir
            .join(".copilot")
            .join("session-state")
            .to_string_lossy()
            .to_string())
    }

    pub fn hooks_file_path(&self) -> Result<PathBuf> {
        let repo_root = crate::utils::paths::repo_root().or_else(|_| {
            std::env::current_dir().map_err(|err| anyhow!("failed to get current directory: {err}"))
        })?;
        Ok(repo_root.join(".github/hooks/bitloops.json"))
    }

    fn get_transcript_position_impl(path: &str) -> Result<usize> {
        if path.is_empty() {
            return Ok(0);
        }

        let data = match std::fs::read(path) {
            Ok(data) => data,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(err) => return Err(anyhow!("failed to read transcript: {err}")),
        };

        if data.is_empty() {
            return Ok(0);
        }

        get_transcript_position_from_bytes(&data)
    }

    fn extract_modified_files_from_offset_impl(
        path: &str,
        start_offset: usize,
    ) -> Result<(Vec<String>, usize)> {
        if path.is_empty() {
            return Ok((Vec::new(), 0));
        }

        let data = match std::fs::read(path) {
            Ok(data) => data,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok((Vec::new(), 0)),
            Err(err) => return Err(anyhow!("failed to read transcript: {err}")),
        };

        let (events, position) = parse_events_from_offset(&data, start_offset)?;
        Ok((extract_modified_files_from_events(&events), position))
    }

    fn extract_prompts_impl(session_ref: &str, from_offset: usize) -> Result<Vec<String>> {
        let data = std::fs::read(session_ref)
            .map_err(|err| anyhow!("failed to read transcript: {err}"))?;
        let (events, _) = parse_events_from_offset(&data, from_offset)?;
        Ok(extract_prompts_from_events(&events))
    }

    fn extract_summary_impl(session_ref: &str) -> Result<String> {
        let data = std::fs::read(session_ref)
            .map_err(|err| anyhow!("failed to read transcript: {err}"))?;
        let (events, _) = parse_events_from_offset(&data, 0)?;
        Ok(extract_summary_from_events(&events))
    }

    fn calculate_token_usage_impl(session_ref: &str, from_offset: usize) -> Result<TokenUsage> {
        let data = std::fs::read(session_ref)
            .map_err(|err| anyhow!("failed to read transcript: {err}"))?;
        let (events, _) = parse_events_from_offset(&data, from_offset)?;
        Ok(calculate_token_usage_from_events(&events))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_and_preview() {
        let agent = CopilotCliAgent;
        assert_eq!(agent.name(), AGENT_NAME_COPILOT);
        assert_eq!(agent.agent_type(), AGENT_TYPE_COPILOT);
        assert!(agent.is_preview());
        assert_eq!(agent.protected_dirs(), vec![".github/hooks".to_string()]);
    }

    #[test]
    fn get_session_dir_uses_override() {
        let dir = CopilotCliAgent::session_dir_from_override_or_home(
            Some("/tmp/copilot-override"),
            Some(Path::new("/tmp/home")),
        )
        .expect("session dir");
        assert_eq!(dir, "/tmp/copilot-override");
    }

    #[test]
    fn resolve_session_file_targets_events_jsonl() {
        let agent = CopilotCliAgent;
        let path = agent.resolve_session_file("/tmp/copilot", "session-1");
        assert_eq!(path, "/tmp/copilot/session-1/events.jsonl");
    }

    #[test]
    fn read_and_write_session_roundtrip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("events.jsonl");
        let payload = br#"{"type":"user.message","data":{"content":"hello"}}"#.to_vec();

        let agent = CopilotCliAgent;
        let session = AgentSession {
            session_id: "session-1".to_string(),
            agent_name: AGENT_NAME_COPILOT.to_string(),
            session_ref: path.to_string_lossy().to_string(),
            native_data: payload.clone(),
            ..AgentSession::default()
        };
        agent.write_session(&session).expect("write");

        let input = HookInput {
            session_id: "session-1".to_string(),
            session_ref: path.to_string_lossy().to_string(),
            ..HookInput::default()
        };
        let loaded = agent.read_session(&input).expect("read").expect("session");
        assert_eq!(loaded.native_data, payload);
    }
}

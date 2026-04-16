use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;

use anyhow::{Result, anyhow};
use serde::Deserialize;
use walkdir::WalkDir;

use crate::adapters::agents::{
    AGENT_NAME_CODEX, AGENT_TYPE_CODEX, Agent, AgentSession, Event, HookInput, HookSupport,
    TranscriptAnalyzer, chunk_jsonl, reassemble_jsonl,
};
use crate::host::checkpoints::transcript::metadata::{
    extract_prompts_from_transcript_bytes, extract_summary_from_transcript_bytes,
};
use crate::host::interactions::transcript_fragment::{
    transcript_fragment_from_bytes, transcript_position_from_bytes,
};

use super::hooks;
use super::lifecycle;

#[derive(Debug, Default, Clone, Copy)]
pub struct CodexAgent;

#[derive(Debug, Deserialize)]
struct CodexSessionIndexEntry {
    id: String,
    updated_at: String,
}

impl CodexAgent {
    pub(crate) fn detect_presence_at(&self, repo_root: &Path) -> bool {
        repo_root.join(".codex").is_dir() || repo_root.join(".codex/hooks.json").exists()
    }

    fn session_home_from_override_or_home(
        override_path: Option<&str>,
        home_dir: Option<&Path>,
    ) -> Result<PathBuf> {
        if let Some(override_path) = override_path
            && !override_path.is_empty()
        {
            return Ok(PathBuf::from(override_path));
        }

        let home_dir = home_dir.ok_or_else(|| anyhow!("failed to get home directory"))?;
        Ok(home_dir.join(".codex").join("sessions"))
    }

    fn session_index_path(session_dir: &str) -> Option<PathBuf> {
        let session_dir = Path::new(session_dir);
        session_dir
            .parent()
            .map(|parent| parent.join("session_index.jsonl"))
    }

    fn narrow_search_dir_from_session_index(
        session_dir: &str,
        agent_session_id: &str,
    ) -> Option<PathBuf> {
        let index_path = Self::session_index_path(session_dir)?;
        let raw = std::fs::read_to_string(index_path).ok()?;
        let latest = raw
            .lines()
            .filter_map(|line| serde_json::from_str::<CodexSessionIndexEntry>(line).ok())
            .rev()
            .find(|entry| entry.id == agent_session_id)?;
        let date = latest.updated_at.get(..10)?;
        let mut parts = date.split('-');
        let year = parts.next()?;
        let month = parts.next()?;
        let day = parts.next()?;
        Some(Path::new(session_dir).join(year).join(month).join(day))
    }

    fn is_matching_session_file(path: &Path, agent_session_id: &str) -> bool {
        let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
            return false;
        };

        file_name == format!("{agent_session_id}.jsonl")
            || file_name == format!("rollout-{agent_session_id}.jsonl")
            || file_name.ends_with(&format!("-{agent_session_id}.jsonl"))
    }

    fn newest_matching_rollout(search_root: &Path, agent_session_id: &str) -> Option<PathBuf> {
        if !search_root.exists() {
            return None;
        }

        WalkDir::new(search_root)
            .follow_links(false)
            .into_iter()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_type().is_file())
            .filter(|entry| Self::is_matching_session_file(entry.path(), agent_session_id))
            .max_by_key(|entry| {
                entry
                    .metadata()
                    .ok()
                    .and_then(|metadata| metadata.modified().ok())
                    .unwrap_or(SystemTime::UNIX_EPOCH)
            })
            .map(|entry| entry.into_path())
    }

    fn read_transcript_position(path: &str) -> Result<usize> {
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

        Ok(transcript_position_from_bytes(&data))
    }
}

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
        Ok(self.detect_presence_at(&repo_root))
    }

    fn get_session_id(&self, input: &HookInput) -> String {
        input.session_id.clone()
    }

    fn protected_dirs(&self) -> Vec<String> {
        vec![".codex".to_string()]
    }

    fn hook_names(&self) -> Vec<String> {
        use crate::host::checkpoints::lifecycle::adapters::{
            CODEX_HOOK_POST_TOOL_USE, CODEX_HOOK_PRE_TOOL_USE, CODEX_HOOK_SESSION_START,
            CODEX_HOOK_STOP, CODEX_HOOK_USER_PROMPT_SUBMIT,
        };

        vec![
            CODEX_HOOK_SESSION_START.to_string(),
            CODEX_HOOK_USER_PROMPT_SUBMIT.to_string(),
            CODEX_HOOK_PRE_TOOL_USE.to_string(),
            CODEX_HOOK_POST_TOOL_USE.to_string(),
            CODEX_HOOK_STOP.to_string(),
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

    fn get_session_dir(&self, _repo_path: &str) -> Result<String> {
        Self::session_home_from_override_or_home(
            std::env::var("BITLOOPS_TEST_CODEX_SESSION_DIR")
                .ok()
                .as_deref(),
            std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .as_deref()
                .map(Path::new),
        )
        .map(|path| path.to_string_lossy().to_string())
    }

    fn resolve_session_file(&self, session_dir: &str, agent_session_id: &str) -> String {
        if agent_session_id.trim().is_empty() {
            return String::new();
        }

        let session_root = Path::new(session_dir);
        let flat_path = session_root.join(format!("{agent_session_id}.jsonl"));
        if flat_path.is_file() {
            return flat_path.to_string_lossy().to_string();
        }

        if let Some(day_dir) =
            Self::narrow_search_dir_from_session_index(session_dir, agent_session_id)
            && let Some(path) = Self::newest_matching_rollout(&day_dir, agent_session_id)
        {
            return path.to_string_lossy().to_string();
        }

        if let Some(path) = Self::newest_matching_rollout(session_root, agent_session_id) {
            return path.to_string_lossy().to_string();
        }

        flat_path.to_string_lossy().to_string()
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

impl TranscriptAnalyzer for CodexAgent {
    fn get_transcript_position(&self, path: &str) -> Result<usize> {
        Self::read_transcript_position(path)
    }

    fn extract_modified_files_from_offset(
        &self,
        path: &str,
        _start_offset: usize,
    ) -> Result<(Vec<String>, usize)> {
        Ok((Vec::new(), Self::read_transcript_position(path)?))
    }

    fn extract_prompts(&self, session_ref: &str, from_offset: usize) -> Result<Vec<String>> {
        let data = match std::fs::read(session_ref) {
            Ok(data) => data,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(anyhow!("failed to read transcript: {err}")),
        };

        let end_offset = transcript_position_from_bytes(&data);
        let fragment = transcript_fragment_from_bytes(&data, from_offset, end_offset);
        Ok(extract_prompts_from_transcript_bytes(fragment.as_bytes()))
    }

    fn extract_summary(&self, session_ref: &str) -> Result<String> {
        let data = match std::fs::read(session_ref) {
            Ok(data) => data,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
            Err(err) => return Err(anyhow!("failed to read transcript: {err}")),
        };

        Ok(extract_summary_from_transcript_bytes(&data))
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

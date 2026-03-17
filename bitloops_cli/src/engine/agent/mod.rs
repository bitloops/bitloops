pub mod chunking;
pub mod claude_code;
pub mod codex;
pub mod copilot_cli;
pub mod cursor;
pub mod gemini_cli;
pub mod open_code;
pub mod registry;
pub mod session;
pub mod types;

use anyhow::Result;
use std::io::Read;

pub use chunking::*;
pub use registry::AgentRegistry;
pub use session::*;
pub use types::*;

pub trait Agent: Send + Sync {
    fn name(&self) -> String;
    fn agent_type(&self) -> String;

    fn description(&self) -> String {
        String::from("TODO")
    }

    fn is_preview(&self) -> bool {
        true
    }

    fn detect_presence(&self) -> Result<bool> {
        Ok(false)
    }

    fn get_session_id(&self, _input: &HookInput) -> String {
        String::new()
    }

    fn protected_dirs(&self) -> Vec<String> {
        Vec::new()
    }

    fn hook_names(&self) -> Vec<String> {
        Vec::new()
    }

    fn parse_hook_event(&self, _hook_name: &str, _stdin: &mut dyn Read) -> Result<Option<Event>> {
        Ok(None)
    }

    fn read_transcript(&self, _session_ref: &str) -> Result<Vec<u8>> {
        Ok(Vec::new())
    }

    fn chunk_transcript(&self, content: &[u8], _max_size: usize) -> Result<Vec<Vec<u8>>> {
        Ok(vec![content.to_vec()])
    }

    fn reassemble_transcript(&self, chunks: &[Vec<u8>]) -> Result<Vec<u8>> {
        Ok(chunks.concat())
    }

    fn get_session_dir(&self, _repo_path: &str) -> Result<String> {
        Ok(String::new())
    }

    fn resolve_session_file(&self, session_dir: &str, agent_session_id: &str) -> String {
        format!("{session_dir}/{agent_session_id}.jsonl")
    }

    fn read_session(&self, _input: &HookInput) -> Result<Option<AgentSession>> {
        Ok(None)
    }

    fn write_session(&self, _session: &AgentSession) -> Result<()> {
        Ok(())
    }

    fn format_resume_command(&self, _session_id: &str) -> String {
        String::new()
    }
}

pub trait HookSupport: Agent {
    fn install_hooks(&self, _local_dev: bool, _force: bool) -> Result<usize> {
        Ok(0)
    }

    fn uninstall_hooks(&self) -> Result<()> {
        Ok(())
    }

    fn are_hooks_installed(&self) -> bool {
        false
    }
}

pub trait FileWatcher: Agent {
    fn get_watch_paths(&self) -> Result<Vec<String>> {
        Ok(Vec::new())
    }

    fn on_file_change(&self, _path: &str) -> Result<Option<SessionChange>> {
        Ok(None)
    }
}

/// Provides format-specific transcript parsing for agents that support it.
/// Agents implementing this get richer checkpoints: file lists, prompts, summaries.
pub trait TranscriptAnalyzer: Agent {
    /// Returns the current position (message count for JSON, line count for JSONL).
    /// Returns 0 if the file does not exist or is empty.
    fn get_transcript_position(&self, path: &str) -> Result<usize>;

    /// Extracts files modified since the given offset.
    /// Returns (files, current_position).
    fn extract_modified_files_from_offset(
        &self,
        path: &str,
        start_offset: usize,
    ) -> Result<(Vec<String>, usize)>;

    /// Extracts user prompts from the transcript starting at the given message offset.
    fn extract_prompts(&self, session_ref: &str, from_offset: usize) -> Result<Vec<String>>;

    /// Extracts the last assistant message as a session summary.
    fn extract_summary(&self, session_ref: &str) -> Result<String>;
}

/// Provides token usage calculation for a session.
pub trait TokenCalculator: Agent {
    /// Computes token usage from the transcript starting at the given message offset.
    fn calculate_token_usage(&self, session_ref: &str, from_offset: usize) -> Result<TokenUsage>;
}

/// Provides transcript position (e.g. message count or line count) for lifecycle orchestration.
/// Used by capture_pre_prompt_state so TurnEnd can parse from the correct offset.
pub trait TranscriptPositionProvider: Send + Sync {
    fn get_transcript_position(&self, path: &str) -> Result<usize>;
}

#[cfg(test)]
mod agent_test;
#[cfg(test)]
mod chunking_test;
#[cfg(test)]
mod registry_test;
#[cfg(test)]
mod session_test;

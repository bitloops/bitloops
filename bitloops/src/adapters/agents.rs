pub mod adapters;
pub mod canonical;
pub mod chunking;
pub mod claude_code;
pub mod codex;
pub mod copilot;
pub mod cursor;
pub mod gemini;
pub mod open_code;
pub mod policy;
pub mod registry;
pub mod session;
pub mod types;

use anyhow::Result;
use std::io::Read;
use std::path::PathBuf;

pub use adapters::*;
pub use canonical::*;
pub use chunking::*;
pub use policy::*;
pub use registry::AgentRegistry;
pub use session::*;
pub use types::*;

pub(crate) fn managed_hook_command(command: &str) -> String {
    match std::env::var_os(crate::config::ENV_DAEMON_CONFIG_PATH_OVERRIDE) {
        Some(path) => format!(
            "{}={} {}",
            crate::config::ENV_DAEMON_CONFIG_PATH_OVERRIDE,
            shell_single_quote(&PathBuf::from(path).to_string_lossy()),
            command,
        ),
        None => command.to_string(),
    }
}

pub(crate) fn managed_hook_env_export_script() -> String {
    match std::env::var_os(crate::config::ENV_DAEMON_CONFIG_PATH_OVERRIDE) {
        Some(path) => format!(
            "export {}={}\n",
            crate::config::ENV_DAEMON_CONFIG_PATH_OVERRIDE,
            shell_single_quote(&PathBuf::from(path).to_string_lossy()),
        ),
        None => String::new(),
    }
}

pub(crate) fn is_managed_hook_command(command: &str, prefixes: &[&str]) -> bool {
    let stripped = strip_leading_shell_env_assignments(command);
    prefixes.iter().any(|prefix| stripped.starts_with(prefix))
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn strip_leading_shell_env_assignments(mut input: &str) -> &str {
    loop {
        let trimmed = input.trim_start();
        let Some(token_end) = first_unquoted_whitespace(trimmed) else {
            return trimmed;
        };
        let token = &trimmed[..token_end];
        if !looks_like_shell_env_assignment(token) {
            return trimmed;
        }
        input = &trimmed[token_end..];
    }
}

fn first_unquoted_whitespace(input: &str) -> Option<usize> {
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;

    for (idx, ch) in input.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if !in_single => escaped = true,
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            _ if ch.is_whitespace() && !in_single && !in_double => return Some(idx),
            _ => {}
        }
    }

    None
}

fn looks_like_shell_env_assignment(token: &str) -> bool {
    let Some((name, _value)) = token.split_once('=') else {
        return false;
    };
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

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

#[cfg(test)]
mod hook_command_tests {
    use super::{is_managed_hook_command, managed_hook_command};
    use crate::test_support::process_state::with_env_var;

    #[test]
    fn managed_hook_command_prefixes_explicit_daemon_config_override() {
        with_env_var(
            crate::config::ENV_DAEMON_CONFIG_PATH_OVERRIDE,
            Some("/tmp/config root/config.toml"),
            || {
                let command = managed_hook_command("bitloops hooks claude-code session-start");
                assert_eq!(
                    command,
                    "BITLOOPS_DAEMON_CONFIG_PATH_OVERRIDE='/tmp/config root/config.toml' bitloops hooks claude-code session-start"
                );
            },
        );
    }

    #[test]
    fn is_managed_hook_command_accepts_env_prefixed_commands() {
        let command =
            "BITLOOPS_DAEMON_CONFIG_PATH_OVERRIDE='/tmp/config root/config.toml' bitloops hooks claude-code session-start";
        assert!(is_managed_hook_command(command, &["bitloops hooks claude-code "]));
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
mod adapters_test;
#[cfg(test)]
mod agent_test;
#[cfg(test)]
mod chunking_test;
#[cfg(test)]
mod registry_test;
#[cfg(test)]
mod session_test;

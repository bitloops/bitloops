use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Result, anyhow};

use crate::adapters::agents::{
    AGENT_NAME_CURSOR, AGENT_TYPE_CURSOR, Agent, AgentSession, HookInput, HookSupport, chunk_jsonl,
    reassemble_jsonl,
};

use super::hooks;
use super::lifecycle;

#[derive(Debug, Default, Clone, Copy)]
pub struct CursorAgent;

impl Agent for CursorAgent {
    fn name(&self) -> String {
        AGENT_NAME_CURSOR.to_string()
    }

    fn agent_type(&self) -> String {
        AGENT_TYPE_CURSOR.to_string()
    }

    fn description(&self) -> String {
        "Cursor - AI-powered code editor".to_string()
    }

    fn is_preview(&self) -> bool {
        true
    }

    fn detect_presence(&self) -> Result<bool> {
        let repo_root = crate::utils::paths::repo_root().unwrap_or_else(|_| PathBuf::from("."));
        Ok(repo_root.join(".cursor").is_dir())
    }

    fn get_session_id(&self, input: &HookInput) -> String {
        input.session_id.clone()
    }

    fn protected_dirs(&self) -> Vec<String> {
        vec![".cursor".to_string()]
    }

    fn hook_names(&self) -> Vec<String> {
        vec![
            lifecycle::HOOK_NAME_SESSION_START.to_string(),
            lifecycle::HOOK_NAME_SESSION_END.to_string(),
            lifecycle::HOOK_NAME_BEFORE_SUBMIT_PROMPT.to_string(),
            lifecycle::HOOK_NAME_BEFORE_SHELL_EXECUTION.to_string(),
            lifecycle::HOOK_NAME_AFTER_SHELL_EXECUTION.to_string(),
            lifecycle::HOOK_NAME_STOP.to_string(),
            lifecycle::HOOK_NAME_PRE_COMPACT.to_string(),
            lifecycle::HOOK_NAME_SUBAGENT_START.to_string(),
            lifecycle::HOOK_NAME_SUBAGENT_STOP.to_string(),
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
        chunk_jsonl(content, max_size)
            .map_err(|err| anyhow!("failed to chunk JSONL transcript: {err}"))
    }

    fn reassemble_transcript(&self, chunks: &[Vec<u8>]) -> Result<Vec<u8>> {
        Ok(reassemble_jsonl(chunks))
    }

    fn get_session_dir(&self, repo_path: &str) -> Result<String> {
        if let Ok(override_path) = std::env::var("BITLOOPS_TEST_CURSOR_PROJECT_DIR")
            && !override_path.is_empty()
        {
            return Ok(override_path);
        }

        let home_dir = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .ok_or_else(|| anyhow!("failed to get home directory"))?;

        let project_dir = sanitize_path_for_cursor(repo_path);
        Ok(Path::new(&home_dir)
            .join(".cursor")
            .join("projects")
            .join(project_dir)
            .join("agent-transcripts")
            .to_string_lossy()
            .to_string())
    }

    fn resolve_session_file(&self, session_dir: &str, agent_session_id: &str) -> String {
        let nested_dir = Path::new(session_dir).join(agent_session_id);
        let nested_file = nested_dir.join(format!("{agent_session_id}.jsonl"));
        if nested_file.exists() {
            return nested_file.to_string_lossy().to_string();
        }
        if nested_dir.is_dir() {
            return nested_file.to_string_lossy().to_string();
        }

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
        if session.agent_name != self.name() && !session.agent_name.is_empty() {
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

    fn format_resume_command(&self, _session_id: &str) -> String {
        "Open this project in Cursor to continue the session.".to_string()
    }
}

impl HookSupport for CursorAgent {
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

fn sanitize_path_for_cursor(path: &str) -> String {
    path.trim_start_matches('/')
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::process_state::with_env_var;

    #[test]
    fn identity_and_preview() {
        let agent = CursorAgent;
        assert_eq!(agent.name(), AGENT_NAME_CURSOR);
        assert_eq!(agent.agent_type(), AGENT_TYPE_CURSOR);
        assert!(agent.is_preview());
        assert_eq!(agent.protected_dirs(), vec![".cursor".to_string()]);
    }

    #[test]
    fn resolve_session_file_prefers_nested() {
        let dir = tempfile::tempdir().expect("tempdir");
        let session_dir = dir.path().join("sessions");
        std::fs::create_dir_all(&session_dir).expect("create session dir");

        let nested_dir = session_dir.join("abc123");
        std::fs::create_dir_all(&nested_dir).expect("create nested");

        let agent = CursorAgent;
        let got = agent.resolve_session_file(session_dir.to_string_lossy().as_ref(), "abc123");
        assert!(got.ends_with("abc123/abc123.jsonl"));
    }

    #[test]
    fn get_session_dir_uses_override() {
        with_env_var(
            "BITLOOPS_TEST_CURSOR_PROJECT_DIR",
            Some("/tmp/cursor-override"),
            || {
                let agent = CursorAgent;
                let dir = agent.get_session_dir("/repo/test").expect("session dir");
                assert_eq!(dir, "/tmp/cursor-override");
            },
        );
    }

    #[test]
    fn read_and_write_session_roundtrip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("session.jsonl");

        let agent = CursorAgent;
        let session = AgentSession {
            session_id: "s1".to_string(),
            agent_name: AGENT_NAME_CURSOR.to_string(),
            session_ref: path.to_string_lossy().to_string(),
            native_data: br#"{"role":"user","content":"hello"}"#.to_vec(),
            ..AgentSession::default()
        };
        agent.write_session(&session).expect("write");

        let input = HookInput {
            session_id: "s1".to_string(),
            session_ref: path.to_string_lossy().to_string(),
            ..HookInput::default()
        };
        let read = agent.read_session(&input).expect("read").expect("session");
        assert_eq!(read.native_data, session.native_data);
    }
}

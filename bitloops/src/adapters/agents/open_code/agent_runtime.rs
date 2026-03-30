use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Result, anyhow};

use crate::adapters::agents::{
    AGENT_NAME_OPEN_CODE, AGENT_TYPE_OPEN_CODE, Agent, AgentSession, Event, HookInput, chunk_jsonl,
    reassemble_jsonl,
};
use crate::host::checkpoints::lifecycle::read_and_parse_hook_input;

use super::agent_api::{
    HOOK_NAME_COMPACTION, HOOK_NAME_SESSION_END, HOOK_NAME_SESSION_START, HOOK_NAME_TURN_END,
    HOOK_NAME_TURN_START, OpenCodeAgent, SessionInfoRaw, TurnStartRaw, sanitize_path_for_opencode,
};
use super::transcript::extract_modified_files;

impl Agent for OpenCodeAgent {
    fn name(&self) -> String {
        AGENT_NAME_OPEN_CODE.to_string()
    }

    fn agent_type(&self) -> String {
        AGENT_TYPE_OPEN_CODE.to_string()
    }

    fn description(&self) -> String {
        "OpenCode - AI-powered terminal coding agent".to_string()
    }

    fn is_preview(&self) -> bool {
        true
    }

    fn detect_presence(&self) -> Result<bool> {
        let repo_root = crate::utils::paths::repo_root().unwrap_or_else(|_| PathBuf::from("."));
        Ok(repo_root.join(".opencode").is_dir() || repo_root.join("opencode.json").is_file())
    }

    fn get_session_id(&self, input: &HookInput) -> String {
        input.session_id.clone()
    }

    fn protected_dirs(&self) -> Vec<String> {
        vec![".opencode".to_string()]
    }

    fn hook_names(&self) -> Vec<String> {
        vec![
            HOOK_NAME_SESSION_START.to_string(),
            HOOK_NAME_SESSION_END.to_string(),
            HOOK_NAME_TURN_START.to_string(),
            HOOK_NAME_TURN_END.to_string(),
            HOOK_NAME_COMPACTION.to_string(),
        ]
    }

    fn parse_hook_event(&self, hook_name: &str, stdin: &mut dyn Read) -> Result<Option<Event>> {
        match hook_name {
            HOOK_NAME_SESSION_START => {
                let raw: SessionInfoRaw = read_and_parse_hook_input(stdin)?;
                let _ = raw.session_id;
                let _ = raw.transcript_path;
                Ok(Some(Event))
            }
            HOOK_NAME_TURN_START => {
                let raw: TurnStartRaw = read_and_parse_hook_input(stdin)?;
                let _ = raw.session_id;
                let _ = raw.transcript_path;
                let _ = raw.prompt;
                Ok(Some(Event))
            }
            HOOK_NAME_TURN_END | HOOK_NAME_COMPACTION | HOOK_NAME_SESSION_END => {
                let raw: SessionInfoRaw = read_and_parse_hook_input(stdin)?;
                let _ = raw.session_id;
                let _ = raw.transcript_path;
                Ok(Some(Event))
            }
            _ => Ok(None),
        }
    }

    fn read_transcript(&self, session_ref: &str) -> Result<Vec<u8>> {
        fs::read(session_ref).map_err(|err| anyhow!("failed to read opencode transcript: {err}"))
    }

    fn chunk_transcript(&self, content: &[u8], max_size: usize) -> Result<Vec<Vec<u8>>> {
        chunk_jsonl(content, max_size)
            .map_err(|err| anyhow!("failed to chunk opencode transcript: {err}"))
    }

    fn reassemble_transcript(&self, chunks: &[Vec<u8>]) -> Result<Vec<u8>> {
        Ok(reassemble_jsonl(chunks))
    }

    fn get_session_dir(&self, repo_path: &str) -> Result<String> {
        if let Ok(override_path) = std::env::var("BITLOOPS_TEST_OPENCODE_PROJECT_DIR")
            && !override_path.is_empty()
        {
            return Ok(override_path);
        }

        let project_dir = sanitize_path_for_opencode(repo_path);
        Ok(std::env::temp_dir()
            .join("bitloops-opencode")
            .join(project_dir)
            .to_string_lossy()
            .to_string())
    }

    fn resolve_session_file(&self, session_dir: &str, agent_session_id: &str) -> String {
        Path::new(session_dir)
            .join(format!("{agent_session_id}.jsonl"))
            .to_string_lossy()
            .to_string()
    }

    fn read_session(&self, input: &HookInput) -> Result<Option<AgentSession>> {
        if input.session_ref.is_empty() {
            return Err(anyhow!("no session ref provided"));
        }

        let data =
            fs::read(&input.session_ref).map_err(|err| anyhow!("failed to read session: {err}"))?;

        let modified_files = extract_modified_files(&data).unwrap_or_default();

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
        if session.session_ref.is_empty() {
            return Err(anyhow!("no session ref to write to"));
        }
        if session.native_data.is_empty() {
            return Err(anyhow!("no session data to write"));
        }

        let parent = Path::new(&session.session_ref)
            .parent()
            .ok_or_else(|| anyhow!("failed to resolve session directory from session ref"))?;
        fs::create_dir_all(parent)
            .map_err(|err| anyhow!("failed to create session directory: {err}"))?;
        fs::write(&session.session_ref, &session.native_data)
            .map_err(|err| anyhow!("failed to write session data: {err}"))?;

        if session.export_data.is_empty() {
            return Ok(());
        }

        if let Err(err) =
            self.import_session_into_opencode(&session.session_id, &session.export_data)
        {
            eprintln!("warning: could not import session into OpenCode: {err}");
        }
        Ok(())
    }

    fn format_resume_command(&self, session_id: &str) -> String {
        if session_id.trim().is_empty() {
            "opencode".to_string()
        } else {
            format!("opencode -s {session_id}")
        }
    }
}

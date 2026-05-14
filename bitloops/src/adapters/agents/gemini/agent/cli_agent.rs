use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Result, anyhow};

use super::config::{
    HOOK_NAME_AFTER_AGENT, HOOK_NAME_AFTER_MODEL, HOOK_NAME_AFTER_TOOL, HOOK_NAME_BEFORE_AGENT,
    HOOK_NAME_BEFORE_MODEL, HOOK_NAME_BEFORE_TOOL, HOOK_NAME_BEFORE_TOOL_SELECTION,
    HOOK_NAME_NOTIFICATION, HOOK_NAME_PRE_COMPRESS, HOOK_NAME_SESSION_END, HOOK_NAME_SESSION_START,
};
use crate::adapters::agents::gemini::lifecycle;
use crate::adapters::agents::gemini::transcript::{
    GeminiMessage, GeminiTranscript, extract_modified_files,
};
use crate::adapters::agents::{
    AGENT_NAME_GEMINI, AGENT_TYPE_GEMINI, Agent, AgentSession, Event, HookInput, chunk_jsonl,
};

#[derive(Debug, Default, Clone, Copy)]
pub struct GeminiCliAgent;

pub fn new_gemini_agent() -> Box<dyn Agent + Send + Sync> {
    Box::new(GeminiCliAgent)
}

impl Agent for GeminiCliAgent {
    fn name(&self) -> String {
        AGENT_NAME_GEMINI.to_string()
    }

    fn agent_type(&self) -> String {
        AGENT_TYPE_GEMINI.to_string()
    }

    fn description(&self) -> String {
        "Gemini - Google's AI coding assistant".to_string()
    }

    fn is_preview(&self) -> bool {
        true
    }

    fn detect_presence(&self) -> Result<bool> {
        let repo_root = crate::utils::paths::repo_root().unwrap_or_else(|_| PathBuf::from("."));
        self.detect_presence_at(&repo_root)
    }

    fn get_session_id(&self, input: &HookInput) -> String {
        input.session_id.clone()
    }

    fn protected_dirs(&self) -> Vec<String> {
        vec![".gemini".to_string()]
    }

    fn hook_names(&self) -> Vec<String> {
        vec![
            HOOK_NAME_SESSION_START.to_string(),
            HOOK_NAME_SESSION_END.to_string(),
            HOOK_NAME_BEFORE_AGENT.to_string(),
            HOOK_NAME_AFTER_AGENT.to_string(),
            HOOK_NAME_BEFORE_MODEL.to_string(),
            HOOK_NAME_AFTER_MODEL.to_string(),
            HOOK_NAME_BEFORE_TOOL_SELECTION.to_string(),
            HOOK_NAME_BEFORE_TOOL.to_string(),
            HOOK_NAME_AFTER_TOOL.to_string(),
            HOOK_NAME_PRE_COMPRESS.to_string(),
            HOOK_NAME_NOTIFICATION.to_string(),
        ]
    }

    fn parse_hook_event(&self, hook_name: &str, stdin: &mut dyn Read) -> Result<Option<Event>> {
        let event = lifecycle::parse_hook_event(hook_name, stdin)?;
        Ok(event.map(|_| Event))
    }

    fn read_transcript(&self, session_ref: &str) -> Result<Vec<u8>> {
        std::fs::read(session_ref).map_err(|err| anyhow!("failed to read transcript: {err}"))
    }

    fn chunk_transcript(&self, content: &[u8], max_size: usize) -> Result<Vec<Vec<u8>>> {
        let transcript = match serde_json::from_slice::<GeminiTranscript>(content) {
            Ok(transcript) => transcript,
            Err(_) => {
                return chunk_jsonl(content, max_size)
                    .map_err(|err| anyhow!("failed to chunk as JSONL: {err}"));
            }
        };

        if transcript.messages.is_empty() {
            return Ok(vec![content.to_vec()]);
        }

        let mut chunks = Vec::new();
        let mut current_messages: Vec<GeminiMessage> = Vec::new();
        let mut current_size = br#"{"messages":[]}"#.len();

        for msg in transcript.messages {
            let msg_size = match serde_json::to_vec(&msg) {
                Ok(bytes) => bytes.len() + 1,
                Err(_) => {
                    continue;
                }
            };

            if current_size + msg_size > max_size && !current_messages.is_empty() {
                let chunk = GeminiTranscript {
                    messages: current_messages,
                };
                let chunk_data = serde_json::to_vec(&chunk)
                    .map_err(|err| anyhow!("failed to marshal chunk: {err}"))?;
                chunks.push(chunk_data);
                current_messages = Vec::new();
                current_size = br#"{"messages":[]}"#.len();
            }

            current_messages.push(msg);
            current_size += msg_size;
        }

        if !current_messages.is_empty() {
            let chunk = GeminiTranscript {
                messages: current_messages,
            };
            let chunk_data = serde_json::to_vec(&chunk)
                .map_err(|err| anyhow!("failed to marshal final chunk: {err}"))?;
            chunks.push(chunk_data);
        }

        if chunks.is_empty() {
            return Err(anyhow!(
                "failed to create any chunks: all messages failed to marshal"
            ));
        }

        Ok(chunks)
    }

    fn reassemble_transcript(&self, chunks: &[Vec<u8>]) -> Result<Vec<u8>> {
        let mut all_messages: Vec<GeminiMessage> = Vec::new();

        for chunk in chunks {
            let transcript: GeminiTranscript = serde_json::from_slice(chunk)
                .map_err(|err| anyhow!("failed to unmarshal chunk: {err}"))?;
            all_messages.extend(transcript.messages);
        }

        let merged = GeminiTranscript {
            messages: all_messages,
        };
        serde_json::to_vec(&merged)
            .map_err(|err| anyhow!("failed to marshal reassembled transcript: {err}"))
    }

    fn get_session_dir(&self, repo_path: &str) -> Result<String> {
        if let Ok(override_path) = std::env::var("BITLOOPS_TEST_GEMINI_PROJECT_DIR")
            && !override_path.is_empty()
        {
            return Ok(override_path);
        }

        let home_dir = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .ok_or_else(|| anyhow!("failed to get home directory"))?;
        let project_dir = Self::get_project_hash(repo_path);
        Ok(Path::new(&home_dir)
            .join(".gemini")
            .join("tmp")
            .join(project_dir)
            .join("chats")
            .to_string_lossy()
            .to_string())
    }

    fn resolve_session_file(&self, session_dir: &str, agent_session_id: &str) -> String {
        let short_id = if agent_session_id.len() > 8 {
            &agent_session_id[..8]
        } else {
            agent_session_id
        };

        let mut matches = Vec::new();
        if let Ok(entries) = std::fs::read_dir(session_dir) {
            for entry in entries.flatten() {
                let file_name = entry.file_name().to_string_lossy().to_string();
                if file_name.starts_with("session-")
                    && file_name.ends_with(&format!("-{short_id}.json"))
                {
                    matches.push(entry.path().to_string_lossy().to_string());
                }
            }
        }

        if !matches.is_empty() {
            matches.sort();
            return matches.last().cloned().unwrap_or_default();
        }

        let ts = Self::current_utc_session_timestamp();
        Path::new(session_dir)
            .join(format!("session-{ts}-{short_id}.json"))
            .to_string_lossy()
            .to_string()
    }

    fn read_session(&self, input: &HookInput) -> Result<Option<AgentSession>> {
        if input.session_ref.is_empty() {
            return Err(anyhow!("session reference (transcript path) is required"));
        }

        let data = std::fs::read(&input.session_ref)
            .map_err(|err| anyhow!("failed to read transcript: {err}"))?;

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
            .map_err(|err| anyhow!("failed to write transcript: {err}"))?;
        Ok(())
    }

    fn format_resume_command(&self, session_id: &str) -> String {
        format!("gemini --resume {session_id}")
    }

    fn as_transcript_entry_deriver(&self) -> Option<&dyn crate::adapters::agents::TranscriptEntryDeriver> {
        Some(self)
    }

    /// Override the default JSONL line slice. For Gemini, the host's
    /// `TranscriptAnalyzer::get_transcript_position` returns the post-dedup
    /// message count (not raw line count), so `transcript_offset_*` markers
    /// are message indices. Parse the live transcript (handles both JSONL and
    /// the legacy JSON-document shape), slice the messages array, and emit
    /// each as a JSONL line so downstream `parse_transcript` re-reads it.
    fn slice_transcript_by_position(
        &self,
        transcript: &str,
        start: usize,
        end: usize,
    ) -> String {
        use crate::adapters::agents::gemini::transcript::parse_transcript;

        if end <= start || transcript.is_empty() {
            return String::new();
        }
        let parsed = match parse_transcript(transcript.as_bytes()) {
            Ok(parsed) => parsed,
            Err(_) => return String::new(),
        };
        let total = parsed.messages.len();
        if start >= total {
            return String::new();
        }
        let bounded_end = end.min(total);
        let mut lines = Vec::new();
        for msg in &parsed.messages[start..bounded_end] {
            if let Ok(line) = serde_json::to_string(msg) {
                lines.push(line);
            }
        }
        lines.join("\n")
    }
}

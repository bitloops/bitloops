use std::collections::HashSet;

use anyhow::{Result, anyhow};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::cli_agent::GeminiCliAgent;
use crate::adapters::agents::gemini::transcript::{
    self, calculate_token_usage_from_file, extract_last_assistant_message, parse_transcript,
};
use crate::adapters::agents::{
    TokenCalculator, TokenUsage, TranscriptAnalyzer, TranscriptPositionProvider,
};

impl TranscriptPositionProvider for GeminiCliAgent {
    fn get_transcript_position(&self, path: &str) -> Result<usize> {
        Self::get_transcript_position_impl(path)
    }
}

impl TranscriptAnalyzer for GeminiCliAgent {
    fn get_transcript_position(&self, path: &str) -> Result<usize> {
        Self::get_transcript_position_impl(path)
    }

    fn extract_modified_files_from_offset(
        &self,
        path: &str,
        start_offset: usize,
    ) -> Result<(Vec<String>, usize)> {
        Self::extract_modified_files_from_offset_impl(self, path, start_offset)
    }

    fn extract_prompts(&self, session_ref: &str, from_offset: usize) -> Result<Vec<String>> {
        Self::extract_prompts_impl(self, session_ref, from_offset)
    }

    fn extract_summary(&self, session_ref: &str) -> Result<String> {
        Self::extract_summary_impl(self, session_ref)
    }
}

impl TokenCalculator for GeminiCliAgent {
    fn calculate_token_usage(&self, session_ref: &str, from_offset: usize) -> Result<TokenUsage> {
        Self::calculate_token_usage_impl(self, session_ref, from_offset)
    }
}

impl GeminiCliAgent {
    pub fn get_project_hash(project_root: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(project_root.as_bytes());
        let digest = hasher.finalize();
        digest.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    pub fn get_transcript_position(&self, path: &str) -> Result<usize> {
        Self::get_transcript_position_impl(path)
    }

    pub(crate) fn get_transcript_position_impl(path: &str) -> Result<usize> {
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

        let transcript = parse_transcript(&data)?;
        Ok(transcript.messages.len())
    }

    pub fn extract_modified_files_from_offset(
        &self,
        path: &str,
        start_offset: usize,
    ) -> Result<(Vec<String>, usize)> {
        Self::extract_modified_files_from_offset_impl(self, path, start_offset)
    }

    pub(crate) fn extract_modified_files_from_offset_impl(
        _agent: &Self,
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
        if data.is_empty() {
            return Ok((Vec::new(), 0));
        }

        let transcript = parse_transcript(&data)?;
        let total_messages = transcript.messages.len();
        let mut files = Vec::new();
        let mut seen = HashSet::new();

        for msg in transcript.messages.iter().skip(start_offset) {
            if msg.r#type != "gemini" {
                continue;
            }

            for tool_call in &msg.tool_calls {
                if !transcript::FILE_MODIFICATION_TOOLS
                    .iter()
                    .any(|tool| *tool == tool_call.name)
                {
                    continue;
                }

                let file = tool_call
                    .args
                    .get("file_path")
                    .and_then(Value::as_str)
                    .or_else(|| tool_call.args.get("path").and_then(Value::as_str))
                    .or_else(|| tool_call.args.get("filename").and_then(Value::as_str))
                    .unwrap_or_default();
                if file.is_empty() {
                    continue;
                }

                if seen.insert(file.to_string()) {
                    files.push(file.to_string());
                }
            }
        }

        Ok((files, total_messages))
    }

    pub fn read_and_parse_hook_input<T: for<'de> Deserialize<'de>>(raw: &str) -> Result<T> {
        if raw.trim().is_empty() {
            return Err(anyhow!("empty hook input"));
        }
        serde_json::from_str(raw).map_err(|err| anyhow!("failed to parse hook input: {err}"))
    }

    pub fn extract_prompts(&self, session_ref: &str, from_offset: usize) -> Result<Vec<String>> {
        Self::extract_prompts_impl(self, session_ref, from_offset)
    }

    pub(crate) fn extract_prompts_impl(
        _agent: &Self,
        session_ref: &str,
        from_offset: usize,
    ) -> Result<Vec<String>> {
        let data = std::fs::read(session_ref)
            .map_err(|err| anyhow!("failed to read transcript: {err}"))?;
        let transcript = parse_transcript(&data)?;

        let mut prompts = Vec::new();
        for (idx, msg) in transcript.messages.iter().enumerate() {
            if idx < from_offset {
                continue;
            }
            if msg.r#type == "user" && !msg.content.is_empty() {
                prompts.push(msg.content.clone());
            }
        }
        Ok(prompts)
    }

    pub fn extract_summary(&self, session_ref: &str) -> Result<String> {
        Self::extract_summary_impl(self, session_ref)
    }

    pub(crate) fn extract_summary_impl(_agent: &Self, session_ref: &str) -> Result<String> {
        let data = std::fs::read(session_ref)
            .map_err(|err| anyhow!("failed to read transcript: {err}"))?;
        extract_last_assistant_message(&data)
    }

    pub fn calculate_token_usage(
        &self,
        session_ref: &str,
        from_offset: usize,
    ) -> Result<TokenUsage> {
        Self::calculate_token_usage_impl(self, session_ref, from_offset)
    }

    pub(crate) fn calculate_token_usage_impl(
        _agent: &Self,
        session_ref: &str,
        from_offset: usize,
    ) -> Result<TokenUsage> {
        calculate_token_usage_from_file(session_ref, from_offset)
    }
}

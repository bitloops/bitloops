use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

use anyhow::{Result, anyhow};
use uuid::Uuid;

use crate::adapters::agents::TokenUsage;

use super::agent_api::OpenCodeAgent;
use super::cli_commands::{run_opencode_import, run_opencode_session_delete};
use super::transcript::{
    calculate_token_usage_from_bytes, extract_file_path_from_input, parse_messages_from_file,
};
use super::types::{FILE_MODIFICATION_TOOLS, ROLE_ASSISTANT, ROLE_USER};

struct TempFileCleanup(PathBuf);

impl Drop for TempFileCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

impl OpenCodeAgent {
    pub fn get_transcript_position(&self, path: &str) -> Result<usize> {
        match parse_messages_from_file(path) {
            Ok(messages) => Ok(messages.len()),
            Err(err) => {
                if is_not_found(&err) {
                    return Ok(0);
                }
                Err(err)
            }
        }
    }

    pub fn extract_modified_files_from_offset(
        &self,
        path: &str,
        start_offset: usize,
    ) -> Result<(Vec<String>, usize)> {
        let messages = match parse_messages_from_file(path) {
            Ok(messages) => messages,
            Err(err) => {
                if is_not_found(&err) {
                    return Ok((Vec::new(), 0));
                }
                return Err(err);
            }
        };

        let mut seen = HashSet::new();
        let mut files = Vec::new();

        for message in messages.iter().skip(start_offset) {
            if message.role != ROLE_ASSISTANT {
                continue;
            }

            for part in &message.parts {
                let Some(state) = part.state.as_ref() else {
                    continue;
                };
                if part.part_type != "tool" {
                    continue;
                }
                if !FILE_MODIFICATION_TOOLS.contains(&part.tool.as_str()) {
                    continue;
                }

                let file_path = extract_file_path_from_input(&state.input);
                if !file_path.is_empty() && seen.insert(file_path.clone()) {
                    files.push(file_path);
                }
            }
        }

        Ok((files, messages.len()))
    }

    pub fn extract_prompts(&self, session_ref: &str, from_offset: usize) -> Result<Vec<String>> {
        let messages = match parse_messages_from_file(session_ref) {
            Ok(messages) => messages,
            Err(err) => {
                if is_not_found(&err) {
                    return Ok(Vec::new());
                }
                return Err(err);
            }
        };

        let mut prompts = Vec::new();
        for message in messages.into_iter().skip(from_offset) {
            if message.role == ROLE_USER && !message.content.is_empty() {
                prompts.push(message.content);
            }
        }
        Ok(prompts)
    }

    pub fn extract_summary(&self, session_ref: &str) -> Result<String> {
        let messages = match parse_messages_from_file(session_ref) {
            Ok(messages) => messages,
            Err(err) => {
                if is_not_found(&err) {
                    return Ok(String::new());
                }
                return Err(err);
            }
        };

        for message in messages.into_iter().rev() {
            if message.role == ROLE_ASSISTANT && !message.content.is_empty() {
                return Ok(message.content);
            }
        }
        Ok(String::new())
    }

    pub fn calculate_token_usage(
        &self,
        session_ref: &str,
        from_offset: usize,
    ) -> Result<Option<TokenUsage>> {
        let data = match fs::read(session_ref) {
            Ok(data) => data,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => {
                return Err(anyhow!("failed to parse transcript for token usage: {err}"));
            }
        };

        Ok(Some(calculate_token_usage_from_bytes(&data, from_offset)))
    }

    pub fn import_session_into_opencode(&self, session_id: &str, export_data: &[u8]) -> Result<()> {
        if session_id.trim().is_empty() {
            return Err(anyhow!("session id is required"));
        }
        if export_data.is_empty() {
            return Err(anyhow!("export data is required"));
        }

        run_opencode_session_delete(session_id)
            .map_err(|err| anyhow!("failed to delete existing session: {err}"))?;

        let temp_file_path =
            std::env::temp_dir().join(format!("bitloops-opencode-export-{}.json", Uuid::new_v4()));
        let _cleanup = TempFileCleanup(temp_file_path.clone());

        let mut temp_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_file_path)
            .map_err(|err| anyhow!("failed to create temp file: {err}"))?;
        temp_file
            .write_all(export_data)
            .map_err(|err| anyhow!("failed to write export data: {err}"))?;
        temp_file
            .flush()
            .map_err(|err| anyhow!("failed to close temp file: {err}"))?;
        drop(temp_file);

        run_opencode_import(temp_file_path.to_string_lossy().as_ref())
    }
}

fn is_not_found(err: &anyhow::Error) -> bool {
    err.downcast_ref::<std::io::Error>()
        .is_some_and(|io_err| io_err.kind() == std::io::ErrorKind::NotFound)
}

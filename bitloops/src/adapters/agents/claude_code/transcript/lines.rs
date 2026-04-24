use anyhow::{Context, Result};

use crate::host::checkpoints::transcript::parse::parse_from_bytes;
use crate::host::checkpoints::transcript::utils::{
    extract_last_user_prompt as extract_last_user_prompt_shared,
    extract_modified_files as extract_modified_files_shared,
    find_checkpoint_uuid as find_checkpoint_uuid_shared,
    truncate_transcript_at_uuid as truncate_transcript_at_uuid_shared,
};

use super::TranscriptLine;

pub fn parse_transcript(content: &[u8]) -> Result<Vec<TranscriptLine>> {
    parse_from_bytes(content)
}

pub fn serialize_transcript(lines: &[TranscriptLine]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for line in lines {
        let data = serde_json::to_vec(line).context("failed to marshal line")?;
        out.extend_from_slice(&data);
        out.push(b'\n');
    }
    Ok(out)
}

pub fn extract_modified_files(lines: &[TranscriptLine]) -> Vec<String> {
    extract_modified_files_shared(lines)
}

pub fn extract_last_user_prompt(lines: &[TranscriptLine]) -> String {
    extract_last_user_prompt_shared(lines)
}

pub fn truncate_at_uuid(lines: &[TranscriptLine], uuid: &str) -> Vec<TranscriptLine> {
    truncate_transcript_at_uuid_shared(lines, uuid)
}

pub fn find_checkpoint_uuid(lines: &[TranscriptLine], tool_use_id: &str) -> (String, bool) {
    match find_checkpoint_uuid_shared(lines, tool_use_id) {
        Some(uuid) => (uuid, true),
        None => (String::new(), false),
    }
}

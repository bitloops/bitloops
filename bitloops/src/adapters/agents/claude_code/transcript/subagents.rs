use std::collections::{HashMap, HashSet};

use crate::host::checkpoints::transcript::parse::parse_from_file_at_line;
use crate::host::checkpoints::transcript::types::TYPE_USER;
use crate::host::checkpoints::transcript::utils::agent_transcript_path;
use anyhow::{Context, Result};

use super::TranscriptLine;
use super::lines::extract_modified_files;
use super::message_types::{TextContentBlock, ToolResultContentBlock, ToolResultMessage};

pub fn extract_spawned_agent_ids(transcript: &[TranscriptLine]) -> HashMap<String, String> {
    let mut agent_ids = HashMap::new();

    for line in transcript {
        if line.r#type != TYPE_USER {
            continue;
        }

        let Ok(msg) = serde_json::from_value::<ToolResultMessage>(line.message.clone()) else {
            continue;
        };

        let Ok(content_blocks) = serde_json::from_value::<Vec<ToolResultContentBlock>>(msg.content)
        else {
            continue;
        };

        for block in content_blocks {
            if block.kind != "tool_result" {
                continue;
            }

            let text_content = if let Ok(text_blocks) =
                serde_json::from_value::<Vec<TextContentBlock>>(block.content.clone())
            {
                let mut text = String::new();
                for item in text_blocks {
                    if item.kind == "text" {
                        text.push_str(&item.text);
                        text.push('\n');
                    }
                }
                text
            } else {
                block.content.as_str().unwrap_or_default().to_string()
            };

            let agent_id = extract_agent_id_from_text(&text_content);
            if !agent_id.is_empty() {
                agent_ids.insert(agent_id, block.tool_use_id);
            }
        }
    }

    agent_ids
}

pub(crate) fn extract_agent_id_from_text(text: &str) -> String {
    const PREFIX: &str = "agentId: ";
    let Some(start_idx) = text.find(PREFIX) else {
        return String::new();
    };

    let rest = &text[start_idx + PREFIX.len()..];
    let end = rest
        .char_indices()
        .find_map(|(idx, ch)| (!ch.is_ascii_alphanumeric()).then_some(idx))
        .unwrap_or(rest.len());

    if end == 0 {
        return String::new();
    }

    rest[..end].to_string()
}

pub fn extract_all_modified_files(
    transcript_path: &str,
    start_line: usize,
    subagents_dir: &str,
) -> Result<Vec<String>> {
    if transcript_path.is_empty() {
        return Ok(Vec::new());
    }

    let (parsed, _) = parse_from_file_at_line(transcript_path, start_line)
        .with_context(|| format!("failed to parse transcript: {transcript_path}"))?;

    let mut seen_files = HashSet::new();
    let mut files = Vec::new();
    for path in extract_modified_files(&parsed) {
        if seen_files.insert(path.clone()) {
            files.push(path);
        }
    }

    let spawned_agents = extract_spawned_agent_ids(&parsed);
    let mut spawned_agent_ids: Vec<String> = spawned_agents.keys().cloned().collect();
    spawned_agent_ids.sort();

    for agent_id in spawned_agent_ids {
        let path = agent_transcript_path(subagents_dir, &agent_id);
        let Ok((agent_lines, _)) = parse_from_file_at_line(&path, 0) else {
            continue;
        };

        for file in extract_modified_files(&agent_lines) {
            if seen_files.insert(file.clone()) {
                files.push(file);
            }
        }
    }

    Ok(files)
}

use std::collections::HashMap;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::adapters::agents::TokenUsage;
use crate::host::checkpoints::transcript::parse::parse_from_file_at_line;
use crate::host::checkpoints::transcript::types::TYPE_ASSISTANT;
use crate::host::checkpoints::transcript::utils::agent_transcript_path;

use super::TranscriptLine;
use super::subagents::extract_spawned_agent_ids;

#[derive(Debug, Clone, Deserialize, Default, Copy)]
struct MessageUsage {
    #[serde(default)]
    input_tokens: i32,
    #[serde(default)]
    cache_creation_input_tokens: i32,
    #[serde(default)]
    cache_read_input_tokens: i32,
    #[serde(default)]
    output_tokens: i32,
}

#[derive(Debug, Deserialize, Default)]
struct MessageWithUsage {
    #[serde(default)]
    id: String,
    #[serde(default)]
    usage: MessageUsage,
}

pub fn calculate_token_usage(transcript: &[TranscriptLine]) -> TokenUsage {
    let mut usage_by_message_id: HashMap<String, MessageUsage> = HashMap::new();

    for line in transcript {
        if line.r#type != TYPE_ASSISTANT {
            continue;
        }

        let Ok(msg) = serde_json::from_value::<MessageWithUsage>(line.message.clone()) else {
            continue;
        };

        if msg.id.is_empty() {
            continue;
        }

        let should_replace = match usage_by_message_id.get(&msg.id) {
            None => true,
            Some(current) => msg.usage.output_tokens > current.output_tokens,
        };
        if should_replace {
            usage_by_message_id.insert(msg.id, msg.usage);
        }
    }

    let mut usage = TokenUsage {
        api_call_count: usage_by_message_id.len() as i32,
        ..TokenUsage::default()
    };

    for current in usage_by_message_id.values() {
        usage.input_tokens += current.input_tokens;
        usage.cache_creation_tokens += current.cache_creation_input_tokens;
        usage.cache_read_tokens += current.cache_read_input_tokens;
        usage.output_tokens += current.output_tokens;
    }

    usage
}

pub fn calculate_token_usage_from_file(path: &str, start_line: usize) -> Result<TokenUsage> {
    if path.is_empty() {
        return Ok(TokenUsage::default());
    }

    let (lines, _) = parse_from_file_at_line(path, start_line)
        .with_context(|| format!("failed to parse transcript: {path}"))?;
    Ok(calculate_token_usage(&lines))
}

pub fn calculate_total_token_usage(
    transcript_path: &str,
    start_line: usize,
    subagents_dir: &str,
) -> Result<TokenUsage> {
    if transcript_path.is_empty() {
        return Ok(TokenUsage::default());
    }

    let (parsed, _) = parse_from_file_at_line(transcript_path, start_line)
        .with_context(|| format!("failed to parse transcript: {transcript_path}"))?;
    let mut main_usage = calculate_token_usage(&parsed);

    let spawned_agents = extract_spawned_agent_ids(&parsed);
    if spawned_agents.is_empty() {
        return Ok(main_usage);
    }

    let mut subagent_usage = TokenUsage::default();
    let mut spawned_agent_ids: Vec<String> = spawned_agents.keys().cloned().collect();
    spawned_agent_ids.sort();

    for agent_id in spawned_agent_ids {
        let path = agent_transcript_path(subagents_dir, &agent_id);
        let Ok(usage) = calculate_token_usage_from_file(&path, 0) else {
            continue;
        };

        subagent_usage.input_tokens += usage.input_tokens;
        subagent_usage.cache_creation_tokens += usage.cache_creation_tokens;
        subagent_usage.cache_read_tokens += usage.cache_read_tokens;
        subagent_usage.output_tokens += usage.output_tokens;
        subagent_usage.api_call_count += usage.api_call_count;
    }

    if subagent_usage.api_call_count > 0 {
        main_usage.subagent_tokens = Some(Box::new(subagent_usage));
    }

    Ok(main_usage)
}

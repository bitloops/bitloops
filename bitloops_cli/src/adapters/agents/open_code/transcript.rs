use std::collections::HashSet;
use std::fs;

use anyhow::Result;
use serde_json::{Map, Value};

use crate::adapters::agents::TokenUsage;

use super::types::{FILE_MODIFICATION_TOOLS, Message, ROLE_ASSISTANT, ROLE_USER};

pub fn parse_messages(data: &[u8]) -> Result<Vec<Message>> {
    if data.is_empty() {
        return Ok(Vec::new());
    }

    let mut messages = Vec::new();
    for line in data.split(|b| *b == b'\n') {
        if line.iter().all(|byte| byte.is_ascii_whitespace()) {
            continue;
        }

        if let Ok(message) = serde_json::from_slice::<Message>(line) {
            messages.push(message);
        }
    }

    Ok(messages)
}

pub fn parse_messages_from_file(path: &str) -> Result<Vec<Message>> {
    let data = fs::read(path)?;
    parse_messages(&data)
}

pub fn extract_modified_files(data: &[u8]) -> Result<Vec<String>> {
    let messages = parse_messages(data)?;
    let mut seen = HashSet::new();
    let mut files = Vec::new();

    for message in messages {
        if message.role != ROLE_ASSISTANT {
            continue;
        }

        for part in message.parts {
            let Some(state) = part.state else {
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

    Ok(files)
}

pub fn extract_all_user_prompts(data: &[u8]) -> Result<Vec<String>> {
    let messages = parse_messages(data)?;
    let mut prompts = Vec::new();
    for message in messages {
        if message.role == ROLE_USER && !message.content.is_empty() {
            prompts.push(message.content);
        }
    }
    Ok(prompts)
}

pub fn extract_file_path_from_input(input: &Map<String, Value>) -> String {
    for key in ["file_path", "path", "file", "filename"] {
        if let Some(Value::String(path)) = input.get(key)
            && !path.is_empty()
        {
            return path.clone();
        }
    }
    String::new()
}

pub fn calculate_token_usage_from_bytes(data: &[u8], start_message_index: usize) -> TokenUsage {
    let Ok(messages) = parse_messages(data) else {
        return TokenUsage::default();
    };

    let mut usage = TokenUsage::default();
    for message in messages.into_iter().skip(start_message_index) {
        let Some(tokens) = message.tokens else {
            continue;
        };
        if message.role != ROLE_ASSISTANT {
            continue;
        }

        usage.input_tokens += tokens.input;
        usage.output_tokens += tokens.output;
        usage.cache_read_tokens += tokens.cache.read;
        usage.cache_creation_tokens += tokens.cache.write;
        usage.api_call_count += 1;
    }

    usage
}

use super::AGENT_TYPE_GEMINI;
use super::registry::AgentRegistry;
use anyhow::{Result, anyhow};

pub const MAX_CHUNK_SIZE: usize = 50 * 1024 * 1024;
pub const CHUNK_SUFFIX: &str = ".%03d";

pub fn chunk_transcript(
    content: &[u8],
    agent_type: &str,
    registry: &AgentRegistry,
) -> Result<Vec<Vec<u8>>> {
    if content.len() <= MAX_CHUNK_SIZE {
        return Ok(vec![content.to_vec()]);
    }

    if !agent_type.is_empty()
        && let Ok(agent) = registry.get_by_agent_type(agent_type)
    {
        return agent
            .chunk_transcript(content, MAX_CHUNK_SIZE)
            .map_err(|err| anyhow!("agent chunking failed: {err}"));
    }

    chunk_jsonl(content, MAX_CHUNK_SIZE)
}

pub fn reassemble_transcript(
    chunks: Vec<Vec<u8>>,
    agent_type: &str,
    registry: &AgentRegistry,
) -> Result<Option<Vec<u8>>> {
    if chunks.is_empty() {
        return Ok(None);
    }

    if chunks.len() == 1 {
        return Ok(Some(chunks[0].clone()));
    }

    if !agent_type.is_empty()
        && let Ok(agent) = registry.get_by_agent_type(agent_type)
    {
        return agent
            .reassemble_transcript(&chunks)
            .map(Some)
            .map_err(|err| anyhow!("agent reassembly failed: {err}"));
    }

    Ok(Some(reassemble_jsonl(&chunks)))
}

pub fn chunk_jsonl(content: &[u8], max_size: usize) -> Result<Vec<Vec<u8>>> {
    if content.is_empty() {
        return Ok(Vec::new());
    }

    let mut chunks: Vec<Vec<u8>> = Vec::new();
    let mut current_chunk: Vec<u8> = Vec::new();

    for (i, line) in content.split(|b| *b == b'\n').enumerate() {
        let line_with_newline_len = line.len() + 1;
        if line_with_newline_len > max_size {
            return Err(anyhow!(
                "JSONL line {} exceeds maximum chunk size ({} bytes > {} bytes); cannot split a single JSON object",
                i + 1,
                line_with_newline_len,
                max_size
            ));
        }

        if current_chunk.len() + line_with_newline_len > max_size && !current_chunk.is_empty() {
            if current_chunk.last() == Some(&b'\n') {
                current_chunk.pop();
            }
            chunks.push(current_chunk);
            current_chunk = Vec::new();
        }

        current_chunk.extend_from_slice(line);
        current_chunk.push(b'\n');
    }

    if !current_chunk.is_empty() {
        if current_chunk.last() == Some(&b'\n') {
            current_chunk.pop();
        }
        chunks.push(current_chunk);
    }

    Ok(chunks)
}

pub fn reassemble_jsonl(chunks: &[Vec<u8>]) -> Vec<u8> {
    let mut result: Vec<u8> = Vec::new();
    for (i, chunk) in chunks.iter().enumerate() {
        result.extend_from_slice(chunk);
        if i + 1 < chunks.len() {
            result.push(b'\n');
        }
    }
    result
}

pub fn chunk_file_name(base_name: &str, index: usize) -> String {
    if index == 0 {
        return base_name.to_string();
    }
    format!("{base_name}.{index:03}")
}

pub fn parse_chunk_index(filename: &str, base_name: &str) -> i32 {
    if filename == base_name {
        return 0;
    }

    let prefix = format!("{base_name}.");
    if !filename.starts_with(&prefix) {
        return -1;
    }

    let suffix = &filename[prefix.len()..];
    if suffix.is_empty() || !suffix.bytes().all(|b| b.is_ascii_digit()) {
        return -1;
    }

    suffix.parse::<i32>().unwrap_or(-1)
}

pub fn sort_chunk_files(files: &[String], base_name: &str) -> Vec<String> {
    let mut sorted = files.to_vec();
    sorted.sort_by(|a, b| {
        let idx_a = parse_chunk_index(a, base_name);
        let idx_b = parse_chunk_index(b, base_name);
        idx_a.cmp(&idx_b)
    });
    sorted
}

pub fn detect_agent_type_from_content(content: &[u8]) -> String {
    let trimmed = String::from_utf8_lossy(content).trim().to_string();
    if !trimmed.starts_with('{') {
        return String::new();
    }

    let Ok(value) = serde_json::from_slice::<serde_json::Value>(content) else {
        return String::new();
    };

    match value.get("messages") {
        Some(serde_json::Value::Array(messages)) if !messages.is_empty() => {
            AGENT_TYPE_GEMINI.to_string()
        }
        _ => String::new(),
    }
}

use anyhow::{Result, anyhow};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::io::Read;

use super::types::PrePromptState;

pub fn resolve_transcript_offset(
    pre_prompt_state: Option<&PrePromptState>,
    _session_id: &str,
) -> usize {
    if let Some(pre_prompt_state) = pre_prompt_state
        && pre_prompt_state.transcript_offset > 0
    {
        return pre_prompt_state.transcript_offset;
    }
    0
}

pub fn create_context_file(
    path: &std::path::Path,
    commit_message: &str,
    session_id: &str,
    prompts: &[String],
    summary: &str,
) -> Result<()> {
    let mut output = String::new();
    output.push_str("# Session Context\n\n");
    output.push_str(&format!("Session ID: {session_id}\n"));
    output.push_str(&format!("Commit Message: {commit_message}\n\n"));

    if !prompts.is_empty() {
        output.push_str("## Prompts\n\n");
        for (idx, prompt) in prompts.iter().enumerate() {
            output.push_str(&format!("### Prompt {}\n\n{prompt}\n\n", idx + 1));
        }
    }

    if !summary.is_empty() {
        output.push_str("## Summary\n\n");
        output.push_str(summary);
        output.push('\n');
    }

    std::fs::write(path, output).map_err(|err| anyhow!("failed to write context file: {err}"))
}

pub fn read_and_parse_hook_input<T: DeserializeOwned>(stdin: &mut dyn Read) -> Result<T> {
    let mut raw = String::new();
    stdin.read_to_string(&mut raw)?;
    if raw.trim().is_empty() {
        return Err(anyhow!("empty hook input"));
    }

    let mut parsed: Value =
        serde_json::from_str(&raw).map_err(|err| anyhow!("failed to parse hook input: {err}"))?;

    for _ in 0..16 {
        match serde_json::from_value::<T>(parsed.clone()) {
            Ok(result) => return Ok(result),
            Err(err) => {
                let Some(missing_field) = extract_missing_field_name(&err) else {
                    return Err(anyhow!("failed to parse hook input: {err}"));
                };

                let Some(object) = parsed.as_object_mut() else {
                    return Err(anyhow!("failed to parse hook input: {err}"));
                };

                if object.contains_key(&missing_field) {
                    return Err(anyhow!("failed to parse hook input: {err}"));
                }
                object.insert(missing_field, Value::String(String::new()));
            }
        }
    }

    Err(anyhow!(
        "failed to parse hook input: exceeded missing-field fallback attempts"
    ))
}

fn extract_missing_field_name(err: &serde_json::Error) -> Option<String> {
    let message = err.to_string();
    let prefix = "missing field `";
    let start = message.find(prefix)? + prefix.len();
    let tail = &message[start..];
    let end = tail.find('`')?;
    Some(tail[..end].to_string())
}

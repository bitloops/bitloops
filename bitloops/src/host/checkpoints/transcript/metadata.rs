use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionMetadataBundle {
    pub transcript: Vec<u8>,
    pub prompts: Vec<String>,
    pub summary: String,
    pub context: Vec<u8>,
}

impl SessionMetadataBundle {
    pub fn prompt_text(&self) -> String {
        self.prompts.join("\n\n---\n\n")
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskCheckpointMetadataBundle {
    pub checkpoint_json: Option<Vec<u8>>,
    pub subagent_transcript: Option<Vec<u8>>,
    pub incremental_checkpoint: Option<Vec<u8>>,
    pub prompt: Option<Vec<u8>>,
}

pub fn build_session_metadata_bundle(
    session_id: &str,
    commit_message: &str,
    transcript: &[u8],
) -> Result<SessionMetadataBundle> {
    let prompts = extract_prompts_from_transcript_bytes(transcript);
    let summary = extract_summary_from_transcript_bytes(transcript);
    let context =
        build_context_markdown(session_id, commit_message, &prompts, &summary).into_bytes();

    Ok(SessionMetadataBundle {
        transcript: transcript.to_vec(),
        prompts,
        summary,
        context,
    })
}

pub fn build_task_checkpoint_payload(
    session_id: &str,
    tool_use_id: &str,
    checkpoint_uuid: &str,
    agent_id: &str,
) -> Result<Vec<u8>> {
    let payload = serde_json::json!({
        "session_id": session_id,
        "tool_use_id": tool_use_id,
        "checkpoint_uuid": checkpoint_uuid,
        "agent_id": agent_id,
    });
    serde_json::to_vec_pretty(&payload).context("serializing task checkpoint payload")
}

pub fn build_incremental_checkpoint_payload(
    tool_use_id: &str,
    incremental_type: &str,
    timestamp: &str,
    data: &serde_json::Value,
) -> Result<Vec<u8>> {
    let payload = serde_json::json!({
        "type": incremental_type,
        "tool_use_id": tool_use_id,
        "timestamp": timestamp,
        "data": data,
    });
    serde_json::to_vec_pretty(&payload).context("serializing incremental checkpoint payload")
}

pub fn build_context_markdown(
    session_id: &str,
    commit_message: &str,
    prompts: &[String],
    summary: &str,
) -> String {
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

    if !summary.trim().is_empty() {
        output.push_str("## Summary\n\n");
        output.push_str(summary);
        output.push('\n');
    }

    output
}

pub fn extract_prompts_from_transcript_bytes(transcript: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(transcript);
    let mut prompts = extract_user_prompts_from_jsonl(&text);
    if !prompts.is_empty() {
        return prompts;
    }

    let Ok(value) = serde_json::from_slice::<serde_json::Value>(transcript) else {
        return Vec::new();
    };
    prompts = extract_prompts_from_json_value(&value);
    prompts
}

pub fn extract_summary_from_transcript_bytes(transcript: &[u8]) -> String {
    let text = String::from_utf8_lossy(transcript);
    let summary = extract_summary_from_jsonl(&text);
    if !summary.trim().is_empty() {
        return summary;
    }

    let Ok(value) = serde_json::from_slice::<serde_json::Value>(transcript) else {
        return String::new();
    };
    extract_summary_from_json_value(&value)
}

pub fn extract_user_prompts_from_jsonl(jsonl: &str) -> Vec<String> {
    let mut prompts = Vec::new();
    for line in jsonl.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if !is_user_role(transcript_line_role(&value)) {
            continue;
        }
        let Some(content) = transcript_line_content(&value) else {
            continue;
        };
        let text = content_to_text(content);
        if !text.trim().is_empty() {
            prompts.push(text);
        }
    }
    prompts
}

pub fn extract_summary_from_jsonl(jsonl: &str) -> String {
    let mut last_summary = String::new();
    for line in jsonl.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if !is_assistant_role(transcript_line_role(&value)) {
            continue;
        }
        let Some(content) = transcript_line_content(&value) else {
            continue;
        };
        let text = content_to_text(content);
        if !text.trim().is_empty() {
            last_summary = text;
        }
    }
    last_summary
}

pub fn transcript_line_role(value: &serde_json::Value) -> Option<&str> {
    value
        .get("message")
        .and_then(|message| message.get("role"))
        .and_then(|role| role.as_str())
        .or_else(|| value.get("role").and_then(|role| role.as_str()))
        .or_else(|| value.get("type").and_then(|kind| kind.as_str()))
}

pub fn transcript_line_content(value: &serde_json::Value) -> Option<&serde_json::Value> {
    let message_content = value
        .get("message")
        .and_then(|message| message.get("content"));
    if message_content.is_some() {
        return message_content;
    }

    let data_content = value.get("data").and_then(|data| data.get("content"));
    if let Some(content) = data_content {
        let text = content_to_text(content);
        if !text.trim().is_empty() {
            return Some(content);
        }
    }

    value
        .get("data")
        .and_then(|data| data.get("transformedContent"))
        .or_else(|| value.get("content"))
}

pub fn content_to_text(content: &serde_json::Value) -> String {
    match content {
        serde_json::Value::String(text) => text.trim().to_string(),
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(|item| {
                if item.get("type").and_then(|kind| kind.as_str()) == Some("text") {
                    item.get("text")
                        .and_then(|text| text.as_str())
                        .map(str::to_owned)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
            .trim()
            .to_string(),
        _ => String::new(),
    }
}

fn extract_prompts_from_json_value(value: &serde_json::Value) -> Vec<String> {
    value
        .get("messages")
        .and_then(|messages| messages.as_array())
        .map(|messages| {
            messages
                .iter()
                .filter_map(|message| {
                    let role = message
                        .get("type")
                        .and_then(|kind| kind.as_str())
                        .or_else(|| {
                            message
                                .get("role")
                                .and_then(|role| role.as_str())
                                .or_else(|| transcript_line_role(message))
                        });
                    if !is_user_role(role) {
                        return None;
                    }
                    transcript_line_content(message)
                        .map(content_to_text)
                        .filter(|text| !text.trim().is_empty())
                })
                .collect()
        })
        .unwrap_or_default()
}

fn extract_summary_from_json_value(value: &serde_json::Value) -> String {
    value
        .get("messages")
        .and_then(|messages| messages.as_array())
        .and_then(|messages| {
            messages.iter().rev().find_map(|message| {
                let role = message
                    .get("type")
                    .and_then(|kind| kind.as_str())
                    .or_else(|| {
                        message
                            .get("role")
                            .and_then(|role| role.as_str())
                            .or_else(|| transcript_line_role(message))
                    });
                if !is_assistant_role(role) {
                    return None;
                }
                transcript_line_content(message)
                    .map(content_to_text)
                    .filter(|text| !text.trim().is_empty())
            })
        })
        .unwrap_or_default()
}

fn is_user_role(role: Option<&str>) -> bool {
    matches!(role, Some("user") | Some("human") | Some("user.message"))
}

fn is_assistant_role(role: Option<&str>) -> bool {
    matches!(
        role,
        Some("assistant") | Some("assistant.message") | Some("gemini")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_session_metadata_bundle_handles_gemini_messages_json() {
        let transcript = br#"{"messages":[
            {"type":"user","content":"What is 2+2?"},
            {"type":"gemini","content":"2+2 equals 4."}
        ]}"#;

        let bundle =
            build_session_metadata_bundle("session-1", "What is 2+2?", transcript).unwrap();
        assert_eq!(bundle.prompts, vec!["What is 2+2?".to_string()]);
        assert_eq!(bundle.summary, "2+2 equals 4.");
        assert!(String::from_utf8_lossy(&bundle.context).contains("Prompt 1"));
    }

    #[test]
    fn extract_user_prompts_supports_copilot_transformed_content() {
        let jsonl = r#"{"type":"user.message","data":{"content":"Create hello.txt"}}
{"type":"user.message","data":{"content":"","transformedContent":"Refactor parser"}}
"#;
        assert_eq!(
            extract_user_prompts_from_jsonl(jsonl),
            vec!["Create hello.txt", "Refactor parser"]
        );
    }

    #[test]
    fn extract_summary_supports_nested_message_content() {
        let jsonl = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"first summary"}]}}
{"type":"assistant","message":{"content":[{"type":"text","text":"final summary"},{"type":"tool_use","name":"Edit","input":{"file_path":"a.txt"}}]}}"#;
        assert_eq!(extract_summary_from_jsonl(jsonl), "final summary");
    }
}

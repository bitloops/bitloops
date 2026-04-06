use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct CodexSessionInfoRaw {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub transcript_path: String,
    #[serde(default)]
    pub model: String,
}

pub fn parse_codex_session_info(raw: &str) -> Result<CodexSessionInfoRaw> {
    if raw.trim().is_empty() {
        return Err(anyhow!("empty codex hook input"));
    }

    let value: Value = serde_json::from_str(raw).context("failed to parse codex hook input")?;

    Ok(CodexSessionInfoRaw {
        session_id: first_non_empty_string(
            &value,
            &[
                "/session_id",
                "/sessionId",
                "/conversation_id",
                "/conversationId",
                "/session/id",
                "/session/session_id",
                "/session/sessionId",
                "/id",
            ],
        )
        .unwrap_or_default(),
        transcript_path: first_non_empty_string(
            &value,
            &[
                "/transcript_path",
                "/transcriptPath",
                "/transcript",
                "/transcript_file",
                "/log_path",
                "/session/transcript_path",
                "/session/transcriptPath",
                "/session/transcript",
                "/paths/transcript",
            ],
        )
        .unwrap_or_default(),
        model: first_non_empty_string(
            &value,
            &[
                "/model",
                "/modelName",
                "/model_name",
                "/modelSlug",
                "/model_slug",
                "/modelId",
                "/model_id",
                "/newModel",
                "/new_model",
                "/session/model",
                "/session/modelName",
                "/session/modelSlug",
                "/session/modelId",
            ],
        )
        .unwrap_or_default(),
    })
}

fn first_non_empty_string(value: &Value, pointers: &[&str]) -> Option<String> {
    for pointer in pointers {
        if let Some(found) = value.pointer(pointer).and_then(Value::as_str) {
            let trimmed = found.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct CodexHooksFile {
    #[serde(default)]
    pub hooks: CodexHooks,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct CodexHooks {
    #[serde(rename = "SessionStart", default)]
    pub session_start: Vec<CodexHookMatcher>,
    #[serde(rename = "Stop", default)]
    pub stop: Vec<CodexHookMatcher>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct CodexHookMatcher {
    #[serde(default)]
    pub hooks: Vec<CodexHookCommand>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct CodexHookCommand {
    #[serde(default)]
    pub command: String,
}

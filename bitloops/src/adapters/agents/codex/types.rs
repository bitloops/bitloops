use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
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
    let value = parse_codex_hook_input(raw)?;

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

#[derive(Debug, Clone, Default)]
pub struct CodexUserPromptSubmitRaw {
    pub session_id: String,
    pub transcript_path: String,
    pub model: String,
    pub prompt: String,
}

pub fn parse_codex_user_prompt_submit(raw: &str) -> Result<CodexUserPromptSubmitRaw> {
    let value = parse_codex_hook_input(raw)?;

    Ok(CodexUserPromptSubmitRaw {
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
        prompt: first_non_empty_string(&value, &["/prompt", "/input/prompt"]).unwrap_or_default(),
    })
}

#[derive(Debug, Clone, Default)]
pub struct CodexToolHookRaw {
    pub session_id: String,
    pub transcript_path: String,
    pub model: String,
    pub tool_name: String,
    pub tool_use_id: String,
    pub command: String,
    pub tool_input: Option<Value>,
    pub tool_response: Option<Value>,
}

pub fn parse_codex_tool_hook(raw: &str) -> Result<CodexToolHookRaw> {
    let value = parse_codex_hook_input(raw)?;

    Ok(CodexToolHookRaw {
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
        tool_name: first_non_empty_string(&value, &["/tool_name", "/toolName", "/tool/name"])
            .unwrap_or_default(),
        tool_use_id: first_non_empty_string(&value, &["/tool_use_id", "/toolUseId"])
            .unwrap_or_default(),
        command: first_non_empty_string(&value, &["/tool_input/command", "/toolInput/command"])
            .unwrap_or_default(),
        tool_input: value
            .pointer("/tool_input")
            .cloned()
            .or_else(|| value.pointer("/toolInput").cloned())
            .filter(|inner| !inner.is_null()),
        tool_response: value
            .pointer("/tool_response")
            .cloned()
            .or_else(|| value.pointer("/toolResponse").cloned())
            .filter(|inner| !inner.is_null()),
    })
}

fn parse_codex_hook_input(raw: &str) -> Result<Value> {
    if raw.trim().is_empty() {
        return Err(anyhow!("empty codex hook input"));
    }
    serde_json::from_str(raw).context("failed to parse codex hook input")
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CodexHooksFile {
    #[serde(default)]
    pub hooks: CodexHooks,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CodexHooks {
    #[serde(rename = "SessionStart", default)]
    pub session_start: Vec<CodexHookMatcher>,
    #[serde(rename = "UserPromptSubmit", default)]
    pub user_prompt_submit: Vec<CodexHookMatcher>,
    #[serde(rename = "PreToolUse", default)]
    pub pre_tool_use: Vec<CodexHookMatcher>,
    #[serde(rename = "PostToolUse", default)]
    pub post_tool_use: Vec<CodexHookMatcher>,
    #[serde(rename = "Stop", default)]
    pub stop: Vec<CodexHookMatcher>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CodexHookMatcher {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub matcher: String,
    #[serde(default)]
    pub hooks: Vec<CodexHookCommand>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CodexHookCommand {
    #[serde(default)]
    pub command: String,
}

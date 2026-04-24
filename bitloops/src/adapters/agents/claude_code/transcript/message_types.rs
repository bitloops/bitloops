use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize, Default)]
pub(super) struct ToolResultMessage {
    #[serde(default)]
    pub(super) content: Value,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct ToolResultContentBlock {
    #[serde(rename = "type", default)]
    pub(super) kind: String,
    #[serde(default)]
    pub(super) tool_use_id: String,
    #[serde(default)]
    pub(super) content: Value,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct TextContentBlock {
    #[serde(rename = "type", default)]
    pub(super) kind: String,
    #[serde(default)]
    pub(super) text: String,
}

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const TYPE_USER: &str = "user";
pub const TYPE_ASSISTANT: &str = "assistant";

pub const CONTENT_TYPE_TEXT: &str = "text";
pub const CONTENT_TYPE_TOOL_USE: &str = "tool_use";

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Line {
    #[serde(rename = "type")]
    pub r#type: String,
    pub uuid: String,
    pub message: Value,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct UserMessage {
    pub content: Value,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct AssistantMessage {
    pub content: Vec<ContentBlock>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ContentBlock {
    #[serde(rename = "type")]
    pub r#type: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub input: Value,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ToolInput {
    #[serde(default)]
    pub file_path: String,
    #[serde(default)]
    pub notebook_path: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub pattern: String,
    #[serde(default)]
    pub skill: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub prompt: String,
}

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const ROLE_ASSISTANT: &str = "assistant";
pub const ROLE_USER: &str = "user";

pub const FILE_MODIFICATION_TOOLS: [&str; 3] = ["edit", "write", "patch"];

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Message {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub time: MessageTime,
    #[serde(default)]
    pub tokens: Option<MessageTokens>,
    #[serde(default)]
    pub cost: f64,
    #[serde(default)]
    pub parts: Vec<MessagePart>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct MessageTime {
    #[serde(default)]
    pub created: i64,
    #[serde(default)]
    pub completed: i64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct MessageTokens {
    #[serde(default)]
    pub input: i32,
    #[serde(default)]
    pub output: i32,
    #[serde(default)]
    pub reasoning: i32,
    #[serde(default)]
    pub cache: MessageTokenCache,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct MessageTokenCache {
    #[serde(default)]
    pub read: i32,
    #[serde(default)]
    pub write: i32,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct MessagePart {
    #[serde(default, rename = "type")]
    pub part_type: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub tool: String,
    #[serde(default, rename = "callID")]
    pub call_id: String,
    #[serde(default)]
    pub state: Option<ToolState>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ToolState {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub input: Map<String, Value>,
    #[serde(default)]
    pub output: Value,
}

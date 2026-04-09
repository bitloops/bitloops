use crate::graphql::types::{ChatRole, DateTimeScalar};

#[derive(Debug, Clone)]
pub(super) struct CheckpointChatEvent {
    pub(super) checkpoint_id: String,
    pub(super) session_id: String,
    pub(super) agent: String,
    pub(super) event_time: DateTimeScalar,
    pub(super) commit_sha: Option<String>,
    pub(super) branch: Option<String>,
    pub(super) strategy: Option<String>,
    pub(super) files_touched: Vec<String>,
    pub(super) payload: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub(super) struct SessionMessageRecord {
    pub(super) role: ChatRole,
    pub(super) raw_role: Option<String>,
    pub(super) timestamp: DateTimeScalar,
    pub(super) content: String,
}

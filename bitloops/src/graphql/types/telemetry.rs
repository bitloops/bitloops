use async_graphql::{ID, SimpleObject};

use super::{DateTimeScalar, JsonScalar};

#[derive(Debug, Clone, SimpleObject)]
pub struct TelemetryEvent {
    pub id: ID,
    pub session_id: String,
    pub event_type: String,
    pub agent: Option<String>,
    pub event_time: DateTimeScalar,
    pub commit_sha: Option<String>,
    pub branch: Option<String>,
    pub payload: Option<JsonScalar>,
}

impl TelemetryEvent {
    pub fn cursor(&self) -> String {
        self.id.to_string()
    }
}

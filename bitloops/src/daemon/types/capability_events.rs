use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityEventRunStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl fmt::Display for CapabilityEventRunStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Queued => write!(f, "queued"),
            Self::Running => write!(f, "running"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapabilityEventRunRecord {
    pub run_id: String,
    pub repo_id: String,
    pub capability_id: String,
    #[serde(default)]
    pub init_session_id: Option<String>,
    #[serde(default)]
    pub consumer_id: String,
    #[serde(default)]
    pub handler_id: String,
    #[serde(default)]
    pub from_generation_seq: u64,
    #[serde(default)]
    pub to_generation_seq: u64,
    #[serde(default)]
    pub reconcile_mode: String,
    #[serde(default)]
    pub event_kind: String,
    #[serde(default)]
    pub lane_key: String,
    #[serde(default)]
    pub event_payload_json: String,
    pub status: CapabilityEventRunStatus,
    pub attempts: u32,
    pub submitted_at_unix: u64,
    pub started_at_unix: Option<u64>,
    pub updated_at_unix: u64,
    pub completed_at_unix: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityEventQueueState {
    pub version: u8,
    pub pending_runs: u64,
    pub running_runs: u64,
    pub failed_runs: u64,
    pub completed_recent_runs: u64,
    pub last_action: Option<String>,
    pub last_updated_unix: u64,
}

impl Default for CapabilityEventQueueState {
    fn default() -> Self {
        Self {
            version: 1,
            pending_runs: 0,
            running_runs: 0,
            failed_runs: 0,
            completed_recent_runs: 0,
            last_action: Some("initialized".to_string()),
            last_updated_unix: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CapabilityEventQueueStatus {
    pub state: CapabilityEventQueueState,
    pub persisted: bool,
    pub current_repo_run: Option<CapabilityEventRunRecord>,
}

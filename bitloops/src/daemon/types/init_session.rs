use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::devql_task::DevqlTaskStatus;
use super::embeddings_bootstrap::InitEmbeddingsBootstrapRequest;
use super::summary_bootstrap::SummaryBootstrapRequest;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StartInitSessionSelections {
    pub run_sync: bool,
    pub run_ingest: bool,
    #[serde(default)]
    pub run_code_embeddings: bool,
    #[serde(default)]
    pub run_summaries: bool,
    #[serde(default)]
    pub run_summary_embeddings: bool,
    pub ingest_backfill: Option<usize>,
    pub embeddings_bootstrap: Option<InitEmbeddingsBootstrapRequest>,
    pub summaries_bootstrap: Option<SummaryBootstrapRequest>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InitSessionTerminalStatus {
    Completed,
    CompletedWithWarnings,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InitSessionTaskTerminalSnapshot {
    pub task_id: String,
    pub status: DevqlTaskStatus,
    pub updated_at_unix: u64,
    pub completed_at_unix: Option<u64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InitSessionRecord {
    pub init_session_id: String,
    pub repo_id: String,
    pub repo_root: PathBuf,
    pub daemon_config_root: PathBuf,
    pub selections: StartInitSessionSelections,
    pub initial_sync_task_id: Option<String>,
    #[serde(default)]
    pub initial_sync_terminal: Option<InitSessionTaskTerminalSnapshot>,
    pub ingest_task_id: Option<String>,
    #[serde(default)]
    pub ingest_terminal: Option<InitSessionTaskTerminalSnapshot>,
    pub embeddings_bootstrap_task_id: Option<String>,
    #[serde(default)]
    pub embeddings_bootstrap_terminal: Option<InitSessionTaskTerminalSnapshot>,
    #[serde(alias = "summary_bootstrap_run_id", default)]
    pub summary_bootstrap_task_id: Option<String>,
    #[serde(default)]
    pub summary_bootstrap_terminal: Option<InitSessionTaskTerminalSnapshot>,
    pub follow_up_sync_required: bool,
    pub follow_up_sync_task_id: Option<String>,
    #[serde(default)]
    pub follow_up_sync_terminal: Option<InitSessionTaskTerminalSnapshot>,
    #[serde(default)]
    pub next_completion_seq: u64,
    #[serde(default)]
    pub initial_sync_completion_seq: Option<u64>,
    #[serde(default)]
    pub embeddings_bootstrap_completion_seq: Option<u64>,
    #[serde(default)]
    pub summary_bootstrap_completion_seq: Option<u64>,
    #[serde(default)]
    pub follow_up_sync_completion_seq: Option<u64>,
    pub submitted_at_unix: u64,
    pub updated_at_unix: u64,
    pub terminal_status: Option<InitSessionTerminalStatus>,
    pub terminal_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InitSessionState {
    pub version: u8,
    pub sessions: Vec<InitSessionRecord>,
    pub last_action: Option<String>,
    pub updated_at_unix: u64,
}

impl Default for InitSessionState {
    fn default() -> Self {
        Self {
            version: 1,
            sessions: Vec::new(),
            last_action: Some("initialized".to_string()),
            updated_at_unix: 0,
        }
    }
}

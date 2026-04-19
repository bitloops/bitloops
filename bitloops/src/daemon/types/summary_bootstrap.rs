use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SummaryBootstrapPhase {
    Queued,
    ResolvingRelease,
    DownloadingRuntime,
    ExtractingRuntime,
    RewritingRuntime,
    WritingProfile,
    Complete,
    Failed,
}

impl fmt::Display for SummaryBootstrapPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Queued => write!(f, "queued"),
            Self::ResolvingRelease => write!(f, "resolving_release"),
            Self::DownloadingRuntime => write!(f, "downloading_runtime"),
            Self::ExtractingRuntime => write!(f, "extracting_runtime"),
            Self::RewritingRuntime => write!(f, "rewriting_runtime"),
            Self::WritingProfile => write!(f, "writing_profile"),
            Self::Complete => write!(f, "complete"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SummaryBootstrapProgress {
    pub phase: SummaryBootstrapPhase,
    pub asset_name: Option<String>,
    pub bytes_downloaded: u64,
    pub bytes_total: Option<u64>,
    pub version: Option<String>,
    pub message: Option<String>,
}

impl Default for SummaryBootstrapProgress {
    fn default() -> Self {
        Self {
            phase: SummaryBootstrapPhase::Queued,
            asset_name: None,
            bytes_downloaded: 0,
            bytes_total: None,
            version: None,
            message: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SummaryBootstrapStatus {
    Queued,
    Running,
    Completed,
    Failed,
}

impl fmt::Display for SummaryBootstrapStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Queued => write!(f, "queued"),
            Self::Running => write!(f, "running"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SummaryBootstrapAction {
    InstallRuntimeOnly,
    InstallRuntimeOnlyPendingProbe,
    ConfigureLocal,
    ConfigureCloud,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SummaryBootstrapRequest {
    pub action: SummaryBootstrapAction,
    pub message: Option<String>,
    pub model_name: Option<String>,
    pub gateway_url_override: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SummaryBootstrapResultRecord {
    pub outcome_kind: String,
    pub model_name: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SummaryBootstrapRunRecord {
    pub run_id: String,
    pub repo_id: String,
    pub repo_root: PathBuf,
    pub init_session_id: String,
    pub request: SummaryBootstrapRequest,
    pub status: SummaryBootstrapStatus,
    pub progress: SummaryBootstrapProgress,
    pub result: Option<SummaryBootstrapResultRecord>,
    pub error: Option<String>,
    pub submitted_at_unix: u64,
    pub started_at_unix: Option<u64>,
    pub updated_at_unix: u64,
    pub completed_at_unix: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SummaryBootstrapState {
    pub version: u8,
    pub runs: Vec<SummaryBootstrapRunRecord>,
    pub last_action: Option<String>,
    pub updated_at_unix: u64,
}

impl Default for SummaryBootstrapState {
    fn default() -> Self {
        Self {
            version: 1,
            runs: Vec::new(),
            last_action: Some("initialized".to_string()),
            updated_at_unix: 0,
        }
    }
}

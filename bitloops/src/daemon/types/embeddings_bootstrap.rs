use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingsBootstrapReadiness {
    Pending,
    Ready,
    Failed,
}

impl fmt::Display for EmbeddingsBootstrapReadiness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Ready => write!(f, "ready"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmbeddingsBootstrapGateEntry {
    pub config_path: PathBuf,
    pub profile_name: String,
    pub readiness: EmbeddingsBootstrapReadiness,
    pub active_task_id: Option<String>,
    pub last_error: Option<String>,
    pub last_updated_unix: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmbeddingsBootstrapState {
    pub version: u8,
    pub entries: BTreeMap<String, EmbeddingsBootstrapGateEntry>,
    pub last_action: Option<String>,
    pub updated_at_unix: u64,
}

impl Default for EmbeddingsBootstrapState {
    fn default() -> Self {
        Self {
            version: 1,
            entries: BTreeMap::new(),
            last_action: Some("initialized".to_string()),
            updated_at_unix: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct EmbeddingsBootstrapGateStatus {
    pub blocked: bool,
    pub readiness: Option<EmbeddingsBootstrapReadiness>,
    pub reason: Option<String>,
    pub active_task_id: Option<String>,
    pub profile_name: Option<String>,
    pub config_path: Option<PathBuf>,
    pub last_error: Option<String>,
    pub last_updated_unix: u64,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingsBootstrapMode {
    #[default]
    Local,
    Platform,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmbeddingsBootstrapTaskSpec {
    pub config_path: PathBuf,
    pub profile_name: String,
    #[serde(default)]
    pub mode: EmbeddingsBootstrapMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway_url_override: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InitEmbeddingsBootstrapRequest {
    pub config_path: PathBuf,
    pub profile_name: String,
    #[serde(default)]
    pub mode: EmbeddingsBootstrapMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway_url_override: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingsBootstrapPhase {
    Queued,
    PreparingConfig,
    ResolvingRelease,
    DownloadingRuntime,
    ExtractingRuntime,
    RewritingRuntime,
    WarmingProfile,
    Complete,
    Failed,
}

impl EmbeddingsBootstrapPhase {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::PreparingConfig => "preparing_config",
            Self::ResolvingRelease => "resolving_release",
            Self::DownloadingRuntime => "downloading_runtime",
            Self::ExtractingRuntime => "extracting_runtime",
            Self::RewritingRuntime => "rewriting_runtime",
            Self::WarmingProfile => "warming_profile",
            Self::Complete => "complete",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmbeddingsBootstrapProgress {
    pub phase: EmbeddingsBootstrapPhase,
    pub asset_name: Option<String>,
    pub bytes_downloaded: u64,
    pub bytes_total: Option<u64>,
    pub version: Option<String>,
    pub message: Option<String>,
}

impl Default for EmbeddingsBootstrapProgress {
    fn default() -> Self {
        Self {
            phase: EmbeddingsBootstrapPhase::Queued,
            asset_name: None,
            bytes_downloaded: 0,
            bytes_total: None,
            version: None,
            message: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmbeddingsBootstrapResult {
    pub version: Option<String>,
    pub binary_path: Option<PathBuf>,
    pub cache_dir: Option<PathBuf>,
    pub runtime_name: Option<String>,
    pub model_name: Option<String>,
    pub freshly_installed: bool,
    pub message: String,
}

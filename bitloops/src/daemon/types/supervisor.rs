use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::api::DashboardServerConfig;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(in crate::daemon) struct SupervisorStartRequest {
    pub(in crate::daemon) config_path: PathBuf,
    pub(in crate::daemon) config: DashboardServerConfig,
    pub(in crate::daemon) telemetry: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(in crate::daemon) struct SupervisorStopRequest {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::daemon) struct SupervisorHealthResponse {
    pub(in crate::daemon) status: String,
}

#[derive(Clone)]
pub(in crate::daemon) struct SupervisorAppState {
    pub(in crate::daemon) operation_lock: Arc<Mutex<()>>,
}

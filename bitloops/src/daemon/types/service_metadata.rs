use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::api::DashboardServerConfig;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ServiceManagerKind {
    Launchd,
    SystemdUser,
    WindowsTask,
}

impl fmt::Display for ServiceManagerKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Launchd => write!(f, "launchd"),
            Self::SystemdUser => write!(f, "systemd --user"),
            Self::WindowsTask => write!(f, "Windows Scheduled Task"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonServiceMetadata {
    pub version: u8,
    pub config_path: PathBuf,
    pub config_root: PathBuf,
    pub manager: ServiceManagerKind,
    pub service_name: String,
    pub service_file: Option<PathBuf>,
    pub config: DashboardServerConfig,
    pub last_url: Option<String>,
    pub last_pid: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupervisorServiceMetadata {
    pub version: u8,
    pub manager: ServiceManagerKind,
    pub service_name: String,
    pub service_file: Option<PathBuf>,
}

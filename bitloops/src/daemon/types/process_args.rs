use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;

use clap::{Args, ValueEnum};
use serde::{Deserialize, Serialize};

use crate::api::DashboardServerConfig;

use super::constants::INTERNAL_DAEMON_COMMAND_NAME;
use super::resolved_config::ResolvedDaemonConfig;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DaemonMode {
    Foreground,
    Detached,
    Service,
}

impl fmt::Display for DaemonMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Foreground => write!(f, "foreground"),
            Self::Detached => write!(f, "detached"),
            Self::Service => write!(f, "always-on service"),
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum DaemonProcessModeArg {
    Detached,
    Service,
}

impl From<DaemonProcessModeArg> for DaemonMode {
    fn from(value: DaemonProcessModeArg) -> Self {
        match value {
            DaemonProcessModeArg::Detached => Self::Detached,
            DaemonProcessModeArg::Service => Self::Service,
        }
    }
}

#[derive(Debug, Clone, Args)]
pub struct InternalDaemonProcessArgs {
    #[arg(long)]
    pub config_path: PathBuf,

    #[arg(long, value_enum)]
    pub mode: DaemonProcessModeArg,

    #[arg(long)]
    pub host: Option<String>,

    #[arg(long, default_value_t = crate::api::DEFAULT_DASHBOARD_PORT)]
    pub port: u16,

    #[arg(long, default_value_t = false)]
    pub http: bool,

    #[arg(long = "recheck-local-dashboard-net", default_value_t = false)]
    pub recheck_local_dashboard_net: bool,

    #[arg(long = "bundle-dir")]
    pub bundle_dir: Option<PathBuf>,

    #[arg(long)]
    pub service_name: Option<String>,

    #[arg(long)]
    pub telemetry: Option<bool>,
}

#[derive(Debug, Clone, Args, Default)]
pub struct InternalDaemonSupervisorArgs {}

impl InternalDaemonProcessArgs {
    pub fn from_server_config(
        daemon_config: &ResolvedDaemonConfig,
        mode: DaemonMode,
        service_name: Option<String>,
        config: &DashboardServerConfig,
        telemetry: Option<bool>,
    ) -> Self {
        Self {
            config_path: daemon_config.config_path.clone(),
            mode: match mode {
                DaemonMode::Detached => DaemonProcessModeArg::Detached,
                DaemonMode::Service => DaemonProcessModeArg::Service,
                DaemonMode::Foreground => DaemonProcessModeArg::Detached,
            },
            host: config.host.clone(),
            port: config.port,
            http: config.force_http,
            recheck_local_dashboard_net: config.recheck_local_dashboard_net,
            bundle_dir: config.bundle_dir.clone(),
            service_name,
            telemetry,
        }
    }

    pub fn server_config(&self) -> DashboardServerConfig {
        DashboardServerConfig {
            host: self.host.clone(),
            port: self.port,
            no_open: true,
            force_http: self.http,
            recheck_local_dashboard_net: self.recheck_local_dashboard_net,
            bundle_dir: self.bundle_dir.clone(),
        }
    }

    pub fn argv(&self) -> Vec<OsString> {
        let mut argv = vec![
            OsString::from(INTERNAL_DAEMON_COMMAND_NAME),
            OsString::from("--config-path"),
            self.config_path.clone().into_os_string(),
            OsString::from("--mode"),
            OsString::from(match self.mode {
                DaemonProcessModeArg::Detached => "detached",
                DaemonProcessModeArg::Service => "service",
            }),
        ];
        if let Some(host) = &self.host {
            argv.push(OsString::from("--host"));
            argv.push(OsString::from(host));
        }
        argv.push(OsString::from("--port"));
        argv.push(OsString::from(self.port.to_string()));
        if self.http {
            argv.push(OsString::from("--http"));
        }
        if self.recheck_local_dashboard_net {
            argv.push(OsString::from("--recheck-local-dashboard-net"));
        }
        if let Some(bundle_dir) = &self.bundle_dir {
            argv.push(OsString::from("--bundle-dir"));
            argv.push(bundle_dir.clone().into_os_string());
        }
        if let Some(service_name) = &self.service_name {
            argv.push(OsString::from("--service-name"));
            argv.push(OsString::from(service_name));
        }
        if let Some(telemetry) = self.telemetry {
            argv.push(OsString::from("--telemetry"));
            argv.push(OsString::from(telemetry.to_string()));
        }
        argv
    }
}

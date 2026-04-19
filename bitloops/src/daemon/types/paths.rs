use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;

use super::constants::{
    RUNTIME_STATE_FILE_NAME, SERVICE_STATE_FILE_NAME, SUPERVISOR_SERVICE_STATE_FILE_NAME,
};

#[cfg(test)]
pub fn runtime_state_path(repo_root: &Path) -> PathBuf {
    if repo_root.as_os_str().is_empty() || repo_root == Path::new(".") {
        return global_daemon_dir_fallback().join(RUNTIME_STATE_FILE_NAME);
    }
    crate::utils::paths::default_runtime_state_dir(repo_root).join(RUNTIME_STATE_FILE_NAME)
}

#[cfg(not(test))]
pub fn runtime_state_path(_repo_root: &Path) -> PathBuf {
    global_daemon_dir_fallback().join(RUNTIME_STATE_FILE_NAME)
}

#[cfg(test)]
pub fn service_metadata_path(repo_root: &Path) -> PathBuf {
    if repo_root.as_os_str().is_empty() || repo_root == Path::new(".") {
        return global_daemon_dir_fallback().join(SERVICE_STATE_FILE_NAME);
    }
    crate::utils::paths::default_runtime_state_dir(repo_root).join(SERVICE_STATE_FILE_NAME)
}

#[cfg(not(test))]
pub fn service_metadata_path(_repo_root: &Path) -> PathBuf {
    global_daemon_dir_fallback().join(SERVICE_STATE_FILE_NAME)
}

pub(in crate::daemon) fn global_daemon_dir() -> Result<PathBuf> {
    crate::utils::platform_dirs::bitloops_state_dir().map(|dir| dir.join("daemon"))
}

pub(in crate::daemon) fn global_daemon_dir_fallback() -> PathBuf {
    crate::utils::platform_dirs::bitloops_state_dir()
        .unwrap_or_else(|_| std::env::temp_dir().join("bitloops").join("state"))
        .join("daemon")
}

pub(in crate::daemon) fn supervisor_service_metadata_path() -> Result<PathBuf> {
    Ok(global_daemon_dir()?.join(SUPERVISOR_SERVICE_STATE_FILE_NAME))
}

pub(in crate::daemon) fn unix_timestamp_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

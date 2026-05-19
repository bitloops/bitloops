use std::path::Path;

use anyhow::Result;

use crate::cli::embeddings::{
    managed_platform_runtime_command_is_eligible, managed_platform_runtime_version_for_command,
    managed_runtime_command_is_eligible, managed_runtime_version_for_command,
};
use crate::host::inference::{
    BITLOOPS_EMBEDDINGS_IPC_DRIVER, BITLOOPS_LOCAL_EMBEDDINGS_RUNTIME_ID,
    BITLOOPS_PLATFORM_EMBEDDINGS_RUNTIME_ID,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ManagedEmbeddingsRuntimeKind {
    Local,
    Platform,
}

pub(super) fn managed_runtime_kind_for_profile(
    profile: &crate::config::InferenceProfileConfig,
) -> Option<ManagedEmbeddingsRuntimeKind> {
    if profile.driver != BITLOOPS_EMBEDDINGS_IPC_DRIVER {
        return None;
    }

    match profile.runtime.as_deref() {
        Some(BITLOOPS_LOCAL_EMBEDDINGS_RUNTIME_ID) => Some(ManagedEmbeddingsRuntimeKind::Local),
        Some(BITLOOPS_PLATFORM_EMBEDDINGS_RUNTIME_ID) => {
            Some(ManagedEmbeddingsRuntimeKind::Platform)
        }
        _ => None,
    }
}

pub(super) fn managed_runtime_command_is_eligible_for_kind(
    kind: ManagedEmbeddingsRuntimeKind,
    config_path: &Path,
) -> Result<bool> {
    match kind {
        ManagedEmbeddingsRuntimeKind::Local => managed_runtime_command_is_eligible(config_path),
        ManagedEmbeddingsRuntimeKind::Platform => {
            managed_platform_runtime_command_is_eligible(config_path)
        }
    }
}

pub(super) fn managed_runtime_version_for_kind(
    kind: ManagedEmbeddingsRuntimeKind,
    command: &str,
) -> Result<Option<String>> {
    match kind {
        ManagedEmbeddingsRuntimeKind::Local => managed_runtime_version_for_command(command),
        ManagedEmbeddingsRuntimeKind::Platform => {
            managed_platform_runtime_version_for_command(command)
        }
    }
}

pub(super) fn managed_runtime_command_is_available(command: &str) -> bool {
    let command = command.trim();
    if command.is_empty() {
        return false;
    }
    let candidate = Path::new(command);
    if candidate.is_absolute() || candidate.components().count() > 1 {
        return candidate.is_file();
    }
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| runtime_command_exists_in_dir(&dir, command))
}

fn runtime_command_exists_in_dir(dir: &Path, command: &str) -> bool {
    let direct = dir.join(command);
    if direct.is_file() {
        return true;
    }
    #[cfg(windows)]
    {
        let path_ext = std::env::var_os("PATHEXT")
            .unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".into())
            .to_string_lossy()
            .to_string();
        for ext in path_ext.split(';').filter(|ext| !ext.is_empty()) {
            let ext = ext.trim_start_matches('.');
            let with_ext = dir.join(format!("{command}.{ext}"));
            if with_ext.is_file() {
                return true;
            }
        }
    }
    false
}

use std::path::{Path, PathBuf};

use crate::utils::platform_dirs::bitloops_state_dir;
#[cfg(test)]
use crate::utils::platform_dirs::{bitloops_cache_dir, bitloops_data_dir};
#[cfg(not(test))]
use crate::utils::platform_dirs::{bitloops_cache_dir, bitloops_data_dir};
#[cfg(not(test))]
use sha2::{Digest, Sha256};

use super::constants::{
    BITLOOPS_CHECKPOINT_ARTEFACTS_DIR, EVENTS_DB_FILE_NAME, LEGACY_BITLOOPS_METADATA_DIR,
    RELATIONAL_DB_FILE_NAME, RUNTIME_DB_FILE_NAME,
};

fn platform_path_fallback(category: &str) -> PathBuf {
    std::env::temp_dir().join("bitloops").join(category)
}

pub fn legacy_session_metadata_dir_from_session_id(session_id: &str) -> String {
    format!("{LEGACY_BITLOOPS_METADATA_DIR}/{session_id}")
}

pub fn checkpoint_artifacts_session_dir(session_id: &str) -> String {
    format!("{BITLOOPS_CHECKPOINT_ARTEFACTS_DIR}/sessions/{session_id}")
}

pub fn checkpoint_artifacts_task_dir(session_id: &str, tool_use_id: &str) -> String {
    format!(
        "{}/tasks/{}",
        checkpoint_artifacts_session_dir(session_id),
        tool_use_id
    )
}

#[cfg(test)]
fn should_use_test_app_dirs(repo_root: &Path) -> bool {
    repo_root.is_relative()
}

#[cfg(test)]
pub fn default_relational_db_path(repo_root: &Path) -> PathBuf {
    if should_use_test_app_dirs(repo_root) {
        return bitloops_data_dir()
            .unwrap_or_else(|_| platform_path_fallback("data"))
            .join("stores")
            .join("relational")
            .join(RELATIONAL_DB_FILE_NAME);
    }
    repo_root
        .join(".bitloops")
        .join("stores")
        .join("relational")
        .join(RELATIONAL_DB_FILE_NAME)
}

#[cfg(not(test))]
pub fn default_relational_db_path(_repo_root: &Path) -> PathBuf {
    bitloops_data_dir()
        .unwrap_or_else(|_| platform_path_fallback("data"))
        .join("stores")
        .join("relational")
        .join(RELATIONAL_DB_FILE_NAME)
}

#[cfg(test)]
pub fn default_events_db_path(repo_root: &Path) -> PathBuf {
    if should_use_test_app_dirs(repo_root) {
        return bitloops_data_dir()
            .unwrap_or_else(|_| platform_path_fallback("data"))
            .join("stores")
            .join("event")
            .join(EVENTS_DB_FILE_NAME);
    }
    repo_root
        .join(".bitloops")
        .join("stores")
        .join("event")
        .join(EVENTS_DB_FILE_NAME)
}

#[cfg(not(test))]
pub fn default_events_db_path(_repo_root: &Path) -> PathBuf {
    bitloops_data_dir()
        .unwrap_or_else(|_| platform_path_fallback("data"))
        .join("stores")
        .join("event")
        .join(EVENTS_DB_FILE_NAME)
}

#[cfg(test)]
pub fn default_blob_store_path(repo_root: &Path) -> PathBuf {
    if should_use_test_app_dirs(repo_root) {
        return bitloops_data_dir()
            .unwrap_or_else(|_| platform_path_fallback("data"))
            .join("stores")
            .join("blob");
    }
    repo_root.join(".bitloops").join("stores").join("blob")
}

#[cfg(not(test))]
pub fn default_blob_store_path(_repo_root: &Path) -> PathBuf {
    bitloops_data_dir()
        .unwrap_or_else(|_| platform_path_fallback("data"))
        .join("stores")
        .join("blob")
}

#[cfg(test)]
pub fn default_embedding_model_cache_dir(repo_root: &Path) -> PathBuf {
    if should_use_test_app_dirs(repo_root) {
        return bitloops_cache_dir()
            .unwrap_or_else(|_| platform_path_fallback("cache"))
            .join("embeddings")
            .join("models");
    }
    repo_root
        .join(".bitloops")
        .join("embeddings")
        .join("models")
}

#[cfg(not(test))]
pub fn default_embedding_model_cache_dir(_repo_root: &Path) -> PathBuf {
    bitloops_cache_dir()
        .unwrap_or_else(|_| platform_path_fallback("cache"))
        .join("embeddings")
        .join("models")
}

pub fn default_runtime_state_dir(_repo_root: &Path) -> PathBuf {
    bitloops_state_dir()
        .unwrap_or_else(|_| platform_path_fallback("state"))
        .join("daemon")
}

pub fn default_repo_runtime_db_path(repo_root: &Path) -> PathBuf {
    repo_root
        .join(".bitloops")
        .join("stores")
        .join("runtime")
        .join(RUNTIME_DB_FILE_NAME)
}

pub fn default_global_runtime_db_path() -> PathBuf {
    default_runtime_state_dir(Path::new(".")).join(RUNTIME_DB_FILE_NAME)
}

#[cfg(test)]
pub fn default_session_tmp_dir(repo_root: &Path) -> PathBuf {
    if should_use_test_app_dirs(repo_root) {
        return default_runtime_state_dir(repo_root).join("tmp");
    }
    repo_root.join(".bitloops").join("tmp")
}

#[cfg(not(test))]
pub fn default_session_tmp_dir(repo_root: &Path) -> PathBuf {
    default_runtime_state_dir(repo_root)
        .join("repos")
        .join(repo_state_key(repo_root))
        .join("tmp")
}

#[cfg(not(test))]
fn repo_state_key(repo_root: &Path) -> String {
    let canonical = repo_root
        .canonicalize()
        .unwrap_or_else(|_| repo_root.to_path_buf());
    let digest = Sha256::digest(canonical.to_string_lossy().as_bytes());
    digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

pub fn extract_session_id_from_transcript_path(transcript_path: &str) -> String {
    let normalized = transcript_path.replace('\\', "/");
    let parts: Vec<&str> = normalized.split('/').collect();
    for (idx, part) in parts.iter().enumerate() {
        if *part != "sessions" || idx + 1 >= parts.len() {
            continue;
        }
        let filename = parts[idx + 1];
        return filename
            .strip_suffix(".jsonl")
            .unwrap_or(filename)
            .to_string();
    }
    String::new()
}

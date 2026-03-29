use std::path::{Path, PathBuf};

#[cfg(not(test))]
use crate::utils::platform_dirs::bitloops_data_dir;
use crate::utils::platform_dirs::bitloops_state_dir;

use super::constants::{BITLOOPS_METADATA_DIR, EVENTS_DB_FILE_NAME, RELATIONAL_DB_FILE_NAME};

pub fn session_metadata_dir_from_session_id(session_id: &str) -> String {
    format!("{BITLOOPS_METADATA_DIR}/{session_id}")
}

#[cfg(test)]
pub fn default_relational_db_path(repo_root: &Path) -> PathBuf {
    repo_root
        .join(".bitloops")
        .join("stores")
        .join("relational")
        .join(RELATIONAL_DB_FILE_NAME)
}

#[cfg(not(test))]
pub fn default_relational_db_path(_repo_root: &Path) -> PathBuf {
    bitloops_data_dir()
        .unwrap_or_else(|_| PathBuf::from(".bitloops"))
        .join("stores")
        .join("relational")
        .join(RELATIONAL_DB_FILE_NAME)
}

#[cfg(test)]
pub fn default_events_db_path(repo_root: &Path) -> PathBuf {
    repo_root
        .join(".bitloops")
        .join("stores")
        .join("event")
        .join(EVENTS_DB_FILE_NAME)
}

#[cfg(not(test))]
pub fn default_events_db_path(_repo_root: &Path) -> PathBuf {
    bitloops_data_dir()
        .unwrap_or_else(|_| PathBuf::from(".bitloops"))
        .join("stores")
        .join("event")
        .join(EVENTS_DB_FILE_NAME)
}

#[cfg(test)]
pub fn default_blob_store_path(repo_root: &Path) -> PathBuf {
    repo_root.join(".bitloops").join("stores").join("blob")
}

#[cfg(not(test))]
pub fn default_blob_store_path(_repo_root: &Path) -> PathBuf {
    bitloops_data_dir()
        .unwrap_or_else(|_| PathBuf::from(".bitloops"))
        .join("stores")
        .join("blob")
}

#[cfg(test)]
pub fn default_embedding_model_cache_dir(repo_root: &Path) -> PathBuf {
    repo_root
        .join(".bitloops")
        .join("embeddings")
        .join("models")
}

#[cfg(not(test))]
pub fn default_embedding_model_cache_dir(_repo_root: &Path) -> PathBuf {
    bitloops_data_dir()
        .unwrap_or_else(|_| PathBuf::from(".bitloops"))
        .join("embeddings")
        .join("models")
}

pub fn default_runtime_state_dir(_repo_root: &Path) -> PathBuf {
    bitloops_state_dir()
        .unwrap_or_else(|_| PathBuf::from(".bitloops"))
        .join("daemon")
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

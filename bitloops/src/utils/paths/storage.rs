use std::path::{Path, PathBuf};

use super::constants::{
    BITLOOPS_BLOB_STORE_DIR, BITLOOPS_EMBEDDING_MODELS_DIR, BITLOOPS_EVENT_STORE_DIR,
    BITLOOPS_METADATA_DIR, BITLOOPS_RELATIONAL_STORE_DIR, EVENTS_DB_FILE_NAME,
    RELATIONAL_DB_FILE_NAME,
};

pub fn session_metadata_dir_from_session_id(session_id: &str) -> String {
    format!("{BITLOOPS_METADATA_DIR}/{session_id}")
}

pub fn default_relational_db_path(repo_root: &Path) -> PathBuf {
    repo_root
        .join(BITLOOPS_RELATIONAL_STORE_DIR)
        .join(RELATIONAL_DB_FILE_NAME)
}

pub fn default_events_db_path(repo_root: &Path) -> PathBuf {
    repo_root
        .join(BITLOOPS_EVENT_STORE_DIR)
        .join(EVENTS_DB_FILE_NAME)
}

pub fn default_blob_store_path(repo_root: &Path) -> PathBuf {
    repo_root.join(BITLOOPS_BLOB_STORE_DIR)
}

pub fn default_embedding_model_cache_dir(repo_root: &Path) -> PathBuf {
    repo_root.join(BITLOOPS_EMBEDDING_MODELS_DIR)
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

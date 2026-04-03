use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::super::types::{
    SYNC_STATE_FILE_NAME, SYNC_STATE_LOCK_FILE_NAME, SyncTaskRecord, global_daemon_dir_fallback,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct PersistedSyncQueueState {
    pub(super) version: u8,
    pub(super) tasks: Vec<SyncTaskRecord>,
    pub(super) last_action: Option<String>,
    pub(super) updated_at_unix: u64,
}

impl Default for PersistedSyncQueueState {
    fn default() -> Self {
        Self {
            version: 1,
            tasks: Vec::new(),
            last_action: Some("initialized".to_string()),
            updated_at_unix: 0,
        }
    }
}

pub(super) fn sync_state_path() -> PathBuf {
    global_daemon_dir_fallback().join(SYNC_STATE_FILE_NAME)
}

pub(super) fn sync_state_lock_path() -> PathBuf {
    global_daemon_dir_fallback().join(SYNC_STATE_LOCK_FILE_NAME)
}

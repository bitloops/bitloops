use std::path::PathBuf;

use anyhow::{Result, anyhow};

pub mod backend;
pub mod db_backend;
pub mod local_backend;
pub mod phase;
pub mod state;

pub use backend::SessionBackend;
pub use db_backend::DbSessionBackend;
use state::{PrePromptState, PreTaskState, SessionState};

#[derive(Debug)]
struct UnavailableSessionBackend {
    reason: String,
}

impl UnavailableSessionBackend {
    fn err<T>(&self) -> Result<T> {
        Err(anyhow!(
            "session backend is unavailable; no legacy local fallback is supported: {}",
            self.reason
        ))
    }
}

impl SessionBackend for UnavailableSessionBackend {
    fn list_sessions(&self) -> Result<Vec<SessionState>> {
        self.err()
    }

    fn load_session(&self, _session_id: &str) -> Result<Option<SessionState>> {
        self.err()
    }

    fn save_session(&self, _state: &SessionState) -> Result<()> {
        self.err()
    }

    fn delete_session(&self, _session_id: &str) -> Result<()> {
        self.err()
    }

    fn load_pre_prompt(&self, _session_id: &str) -> Result<Option<PrePromptState>> {
        self.err()
    }

    fn save_pre_prompt(&self, _state: &PrePromptState) -> Result<()> {
        self.err()
    }

    fn delete_pre_prompt(&self, _session_id: &str) -> Result<()> {
        self.err()
    }

    fn create_pre_task_marker(&self, _state: &PreTaskState) -> Result<()> {
        self.err()
    }

    fn load_pre_task_marker(&self, _tool_use_id: &str) -> Result<Option<PreTaskState>> {
        self.err()
    }

    fn delete_pre_task_marker(&self, _tool_use_id: &str) -> Result<()> {
        self.err()
    }

    fn find_active_pre_task(&self) -> Result<Option<String>> {
        self.err()
    }
}

pub fn create_session_backend(repo_root: impl Into<PathBuf>) -> Result<Box<dyn SessionBackend>> {
    let root = repo_root.into();
    let backend = DbSessionBackend::for_repo_root(&root)?;
    Ok(Box::new(backend))
}

pub fn create_session_backend_or_local(repo_root: impl Into<PathBuf>) -> Box<dyn SessionBackend> {
    let root = repo_root.into();
    match create_session_backend(root.clone()) {
        Ok(backend) => backend,
        Err(err) => {
            let reason = format!(
                "failed to initialise configured DbSessionBackend for {}: {err:#}",
                root.display()
            );
            log::warn!("{reason}; legacy LocalFileBackend fallback is disabled");
            Box::new(UnavailableSessionBackend { reason })
        }
    }
}

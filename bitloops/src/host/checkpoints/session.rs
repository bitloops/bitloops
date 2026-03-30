use std::path::PathBuf;

use anyhow::Result;

pub mod backend;
pub mod db_backend;
pub mod local_backend;
pub mod phase;
pub mod state;

pub use backend::SessionBackend;
pub use db_backend::DbSessionBackend;

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
            let reason = format!("failed to initialise DbSessionBackend: {err:#}");
            log::error!("{reason}");
            Box::new(UnavailableSessionBackend::new(reason))
        }
    }
}

struct UnavailableSessionBackend {
    reason: String,
}

impl UnavailableSessionBackend {
    fn new(reason: String) -> Self {
        Self { reason }
    }

    fn unavailable<T>(&self) -> Result<T> {
        Err(anyhow::anyhow!(
            "session backend unavailable: {}",
            self.reason
        ))
    }
}

impl SessionBackend for UnavailableSessionBackend {
    fn list_sessions(&self) -> Result<Vec<state::SessionState>> {
        self.unavailable()
    }

    fn load_session(&self, _session_id: &str) -> Result<Option<state::SessionState>> {
        self.unavailable()
    }

    fn save_session(&self, _state: &state::SessionState) -> Result<()> {
        self.unavailable()
    }

    fn delete_session(&self, _session_id: &str) -> Result<()> {
        self.unavailable()
    }

    fn load_pre_prompt(&self, _session_id: &str) -> Result<Option<state::PrePromptState>> {
        self.unavailable()
    }

    fn save_pre_prompt(&self, _state: &state::PrePromptState) -> Result<()> {
        self.unavailable()
    }

    fn delete_pre_prompt(&self, _session_id: &str) -> Result<()> {
        self.unavailable()
    }

    fn create_pre_task_marker(&self, _state: &state::PreTaskState) -> Result<()> {
        self.unavailable()
    }

    fn load_pre_task_marker(&self, _tool_use_id: &str) -> Result<Option<state::PreTaskState>> {
        self.unavailable()
    }

    fn delete_pre_task_marker(&self, _tool_use_id: &str) -> Result<()> {
        self.unavailable()
    }

    fn find_active_pre_task(&self) -> Result<Option<String>> {
        self.unavailable()
    }
}

use std::path::PathBuf;

use anyhow::{Context, Result};

pub mod backend;
pub mod db_backend;
pub mod local_backend;
pub mod phase;
pub mod state;

pub use backend::SessionBackend;
pub use db_backend::DbSessionBackend;

pub const LEGACY_LOCAL_BACKEND_ENV: &str = "BITLOOPS_ENABLE_LEGACY_LOCAL_BACKEND";

#[cfg(test)]
pub fn legacy_local_backend_enabled() -> bool {
    true
}

#[cfg(not(test))]
pub fn legacy_local_backend_enabled() -> bool {
    std::env::var(LEGACY_LOCAL_BACKEND_ENV)
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

pub fn list_legacy_local_session_ids(repo_root: impl Into<PathBuf>) -> Result<Vec<String>> {
    if !legacy_local_backend_enabled() {
        return Ok(vec![]);
    }
    let backend = local_backend::LocalFileBackend::new(repo_root.into());
    let dir = backend.sessions_dir();

    let mut session_ids = Vec::new();
    match std::fs::read_dir(&dir) {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(session_ids),
        Err(err) => {
            return Err(err).with_context(|| format!("reading sessions dir {}", dir.display()));
        }
        Ok(entries) => {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                    continue;
                }
                let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
                    continue;
                };
                if stem.is_empty() {
                    continue;
                }
                session_ids.push(stem.to_string());
            }
        }
    }

    session_ids.sort();
    session_ids.dedup();
    Ok(session_ids)
}

pub fn delete_legacy_local_session_state(
    repo_root: impl Into<PathBuf>,
    session_id: &str,
) -> Result<()> {
    if !legacy_local_backend_enabled() {
        return Ok(());
    }
    let backend = local_backend::LocalFileBackend::new(repo_root.into());
    backend.delete_session(session_id)
}

pub fn create_session_backend(repo_root: impl Into<PathBuf>) -> Result<Box<dyn SessionBackend>> {
    let root = repo_root.into();
    let backend = DbSessionBackend::for_repo_root(&root)?;
    Ok(Box::new(backend))
}

#[cfg(test)]
pub fn create_session_backend_or_local(repo_root: impl Into<PathBuf>) -> Box<dyn SessionBackend> {
    Box::new(local_backend::LocalFileBackend::new(repo_root.into()))
}

#[cfg(not(test))]
pub fn create_session_backend_or_local(repo_root: impl Into<PathBuf>) -> Box<dyn SessionBackend> {
    let root = repo_root.into();
    match create_session_backend(root.clone()) {
        Ok(backend) => backend,
        Err(err) => {
            if legacy_local_backend_enabled() {
                log::warn!(
                    "failed to initialise DbSessionBackend; falling back to LocalFileBackend because {LEGACY_LOCAL_BACKEND_ENV}=1: {err:#}"
                );
                return Box::new(local_backend::LocalFileBackend::new(root));
            }

            let reason = format!(
                "failed to initialise DbSessionBackend while legacy fallback is disabled ({LEGACY_LOCAL_BACKEND_ENV}=1 enables it): {err:#}"
            );
            log::error!("{reason}");
            Box::new(UnavailableSessionBackend::new(reason))
        }
    }
}

#[cfg(not(test))]
struct UnavailableSessionBackend {
    reason: String,
}

#[cfg(not(test))]
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

#[cfg(not(test))]
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

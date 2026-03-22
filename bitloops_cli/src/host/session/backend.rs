//! SessionBackend trait — dependency-injection interface for session persistence.

use anyhow::Result;

use super::state::{PrePromptState, PreTaskState, SessionState};

/// Storage interface for session lifecycle data.
///
/// Production code uses `DbSessionBackend` by default.
/// `LocalFileBackend` is legacy compatibility storage.
pub trait SessionBackend: Send + Sync {
    // ── Session state (<git-common-dir>/bitloops-sessions/<id>.json) ─────

    /// Return all persisted session states.
    fn list_sessions(&self) -> Result<Vec<SessionState>>;

    /// Load session state. Returns `None` if no state file exists yet.
    fn load_session(&self, session_id: &str) -> Result<Option<SessionState>>;

    /// Persist session state (creates parent directories if needed).
    fn save_session(&self, state: &SessionState) -> Result<()>;

    /// Delete session state (no-op if already absent).
    fn delete_session(&self, session_id: &str) -> Result<()>;

    // ── Pre-prompt state (.bitloops/tmp/pre-prompt-<id>.json) ────────────

    /// Load pre-prompt state. Returns `None` if file doesn't exist.
    fn load_pre_prompt(&self, session_id: &str) -> Result<Option<PrePromptState>>;

    /// Persist pre-prompt state.
    fn save_pre_prompt(&self, state: &PrePromptState) -> Result<()>;

    /// Delete pre-prompt state file (no-op if already absent).
    fn delete_pre_prompt(&self, session_id: &str) -> Result<()>;

    // ── Pre-task markers (.bitloops/tmp/pre-task-<tool-use-id>.json) ─────

    /// Create a pre-task marker file.
    fn create_pre_task_marker(&self, state: &PreTaskState) -> Result<()>;

    /// Load a pre-task marker state by tool-use ID.
    fn load_pre_task_marker(&self, tool_use_id: &str) -> Result<Option<PreTaskState>>;

    /// Remove a pre-task marker file (no-op if already absent).
    fn delete_pre_task_marker(&self, tool_use_id: &str) -> Result<()>;

    /// Scan for any active pre-task marker and return its `tool_use_id`.
    /// Returns `None` if no marker is found (i.e. not inside a subagent turn).
    fn find_active_pre_task(&self) -> Result<Option<String>>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::session::local_backend::LocalFileBackend;
    use crate::host::session::phase::SessionPhase;
    use tempfile::TempDir;

    #[test]
    fn list_sessions_is_available_via_trait_object() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();

        let backend: Box<dyn SessionBackend> = Box::new(LocalFileBackend::new(dir.path()));
        let session = SessionState {
            session_id: "session-trait-object".to_string(),
            phase: SessionPhase::Active,
            ..Default::default()
        };
        backend.save_session(&session).unwrap();

        let sessions = backend.list_sessions().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "session-trait-object");
    }

    #[test]
    fn delete_session_is_available_via_trait_object() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();

        let backend: Box<dyn SessionBackend> = Box::new(LocalFileBackend::new(dir.path()));
        let session = SessionState {
            session_id: "session-delete-trait-object".to_string(),
            phase: SessionPhase::Active,
            ..Default::default()
        };
        backend.save_session(&session).unwrap();
        assert!(backend.load_session(&session.session_id).unwrap().is_some());

        backend.delete_session(&session.session_id).unwrap();
        assert!(backend.load_session(&session.session_id).unwrap().is_none());
    }
}

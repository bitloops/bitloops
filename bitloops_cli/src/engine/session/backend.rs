//! SessionBackend trait — dependency-injection interface for session persistence.

use anyhow::Result;

use super::state::{PrePromptState, PreTaskState, SessionState};

/// Storage interface for session lifecycle data.
///
/// Production code uses `LocalFileBackend`; tests inject this directly.
pub trait SessionBackend: Send + Sync {
    // ── Session state (<git-common-dir>/bitloops-sessions/<id>.json) ─────

    /// Load session state. Returns `None` if no state file exists yet.
    fn load_session(&self, session_id: &str) -> Result<Option<SessionState>>;

    /// Persist session state (creates parent directories if needed).
    fn save_session(&self, state: &SessionState) -> Result<()>;

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

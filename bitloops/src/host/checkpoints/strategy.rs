//! Strategy trait — hooks delegate checkpoint creation to the active strategy.

pub mod attribution;
pub mod auto_commit;
pub mod manual_commit;
pub mod messages;
pub mod noop;
pub mod redact;
pub mod registry;

#[cfg(test)]
mod redact_test;

use std::path::Path;

use crate::adapters::agents::TokenUsage;
use crate::host::checkpoints::session::state::SessionState;
use anyhow::Result;

/// Context passed to `save_step` when a turn ends.
///
#[derive(Debug, Clone, Default)]
pub struct StepContext {
    pub session_id: String,
    pub modified_files: Vec<String>,
    pub new_files: Vec<String>,
    pub deleted_files: Vec<String>,
    /// Repo-relative path to the session metadata directory.
    pub metadata_dir: String,
    /// Absolute path to the session metadata directory.
    pub metadata_dir_abs: String,
    pub commit_message: String,
    pub transcript_path: String,
    pub author_name: String,
    pub author_email: String,
    pub agent_type: String,
    pub step_transcript_identifier: String,
    pub step_transcript_start: i64,
    pub token_usage: Option<TokenUsage>,
}

/// Context passed to `save_task_step` for subagent / incremental checkpoints.
///
#[derive(Debug, Clone, Default)]
pub struct TaskStepContext {
    pub session_id: String,
    pub tool_use_id: String,
    pub agent_id: String,
    pub modified_files: Vec<String>,
    pub new_files: Vec<String>,
    pub deleted_files: Vec<String>,
    pub transcript_path: String,
    pub subagent_transcript_path: String,
    pub checkpoint_uuid: String,
    pub author_name: String,
    pub author_email: String,
    pub subagent_type: String,
    pub task_description: String,
    pub agent_type: String,
    /// True for incremental TodoWrite checkpoints.
    pub is_incremental: bool,
    /// 1-based counter of incremental checkpoints within a task.
    pub incremental_sequence: u32,
    /// Tool name for incremental events (e.g. TodoWrite).
    pub incremental_type: String,
    /// Serialized incremental payload.
    pub incremental_data: String,
    /// Extracted todo item content (for incremental checkpoints).
    pub todo_content: String,
    /// Explicit commit message override.
    pub commit_message: String,
}

/// Strategy interface: creates checkpoints from session turn/task data.
///
pub trait Strategy: Send + Sync {
    /// Returns the strategy registration name.
    fn name(&self) -> &str;

    /// Called at turn start (`user-prompt-submit`) to initialize or refresh session state.
    fn initialize_session(
        &self,
        _session_id: &str,
        _agent_type: &str,
        _transcript_path: &str,
        _user_prompt: &str,
    ) -> Result<()> {
        Ok(())
    }

    /// Called by the `stop` hook when a turn ends (creates a shadow-branch checkpoint).
    fn save_step(&self, ctx: &StepContext) -> Result<()>;

    /// Called by `post-task` and `post-todo` hooks (creates task/incremental checkpoints).
    fn save_task_step(&self, ctx: &TaskStepContext) -> Result<()>;

    /// Called at turn end (`stop`) for strategy-specific finalization.
    fn handle_turn_end(&self, _state: &mut SessionState) -> Result<()> {
        Ok(())
    }

    // ── git hook handlers ─────────────────────────────────────────────────

    /// Called by the `prepare-commit-msg` git hook.
    /// Default implementation is a no-op.
    ///
    fn prepare_commit_msg(&self, _commit_msg_file: &Path, _source: Option<&str>) -> Result<()> {
        Ok(())
    }

    /// Called by the `commit-msg` git hook.
    /// Default implementation is a no-op.
    ///
    fn commit_msg(&self, _commit_msg_file: &Path) -> Result<()> {
        Ok(())
    }

    /// Called by the `post-commit` git hook.
    /// Reads the checkpoint mapping from DB and condenses session data
    /// onto the `bitloops/checkpoints/v1` branch.
    ///
    fn post_commit(&self) -> Result<()>;

    /// Called by the `pre-push` git hook.
    /// Receives raw stdin lines from git containing ref updates in the format:
    /// `<local_ref> <local_sha> <remote_ref> <remote_sha>`.
    /// Default implementation is a no-op.
    ///
    fn pre_push(&self, _remote: &str, _stdin_lines: &[String]) -> Result<()> {
        Ok(())
    }

    /// Called by the `post-merge` git hook.
    /// Default implementation is a no-op.
    ///
    fn post_merge(&self, _is_squash: bool) -> Result<()> {
        Ok(())
    }

    /// Called by the `post-checkout` git hook.
    /// Default implementation is a no-op.
    ///
    fn post_checkout(
        &self,
        _previous_head: &str,
        _new_head: &str,
        _is_branch_checkout: bool,
    ) -> Result<()> {
        Ok(())
    }

    /// Called by the `reference-transaction` git hook.
    /// Default implementation is a no-op.
    ///
    fn reference_transaction(&self, _state: &str, _stdin_lines: &[String]) -> Result<()> {
        Ok(())
    }
}

//! ManualCommitStrategy — checkpoints via shadow git branches.
//!
//! # Workflow
//! 1. `stop` hook → `save_step()` → shadow branch commit (`refs/heads/bitloops/<head[:7]>`)
//! 2. `git commit` → `prepare_commit_msg()` → appends `Bitloops-Checkpoint: <12hexid>` trailer
//! 3. `commit-msg` hook → `commit_msg()` → strips trailer if no user content (empty commit abort)
//! 4. `post-commit` hook → `post_commit()` → condenses session onto `bitloops/checkpoints/v1`
//! 5. `git push` → `pre_push()` → pushes `bitloops/checkpoints/v1` alongside
//!
//! Git operations use shell `git` + `GIT_INDEX_FILE` for temp-index tree construction.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::engine::agent::claude_code::transcript as claude_transcript;
use crate::engine::agent::{
    AGENT_TYPE_CLAUDE_CODE, AGENT_TYPE_CODEX, AGENT_TYPE_GEMINI, AGENT_TYPE_OPEN_CODE, TokenUsage,
    canonical_agent_key,
};
use crate::engine::paths;
use crate::engine::session::backend::SessionBackend;
use crate::engine::session::local_backend::LocalFileBackend;
use crate::engine::session::phase::{
    Action, Event, NoOpActionHandler, SessionPhase, TransitionContext, apply_transition,
    transition_with_context,
};
use crate::engine::session::state::{PromptAttribution as SessionPromptAttribution, SessionState};
use crate::engine::stringutil;
use crate::engine::trailers::{
    AGENT_TRAILER_KEY, CHECKPOINT_TRAILER_KEY, SESSION_TRAILER_KEY, STRATEGY_TRAILER_KEY,
    is_valid_checkpoint_id,
};
use crate::engine::transcript::commit_message;
use crate::engine::validation::validators::{
    validate_agent_id, validate_session_id, validate_tool_use_id,
};

use super::attribution::{
    PromptAttribution as StrategyPromptAttribution, TreeSnapshot,
    calculate_attribution_with_accumulated, calculate_prompt_attribution,
};
use super::{StepContext, Strategy, TaskStepContext, redact};

// ── Constants ─────────────────────────────────────────────────────────────────

const CLI_VERSION: &str = env!("CARGO_PKG_VERSION");

// ── Strategy struct ───────────────────────────────────────────────────────────

pub struct ManualCommitStrategy {
    repo_root: PathBuf,
    backend: LocalFileBackend,
}

impl ManualCommitStrategy {
    pub fn new(repo_root: impl Into<PathBuf>) -> Self {
        let root = repo_root.into();
        let backend = LocalFileBackend::new(&root);
        Self {
            repo_root: root,
            backend,
        }
    }

    /// Condenses a specific session immediately, used by `bitloops doctor`.
    ///
    pub fn condense_session_by_id(&self, session_id: &str) -> Result<()> {
        let Some(mut state) = self.backend.load_session(session_id)? else {
            anyhow::bail!("session not found: {session_id}");
        };
        if state.base_commit.trim().is_empty() {
            anyhow::bail!("session {session_id} has no base commit");
        }
        let Some(head) = try_head_hash(&self.repo_root)? else {
            anyhow::bail!("HEAD not found");
        };
        let checkpoint_id = generate_checkpoint_id();
        self.condense_session(&mut state, &checkpoint_id, &head)
    }
}

include!("manual_commit/strategy_impl.rs");
include!("manual_commit/strategy_helpers.rs");
include!("manual_commit/support.rs");

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "manual_commit_tests.rs"]
mod tests;

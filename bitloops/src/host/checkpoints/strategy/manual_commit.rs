//! ManualCommitStrategy — checkpoints via temporary trees and DevQL storage.
//!
//! # Workflow
//! 1. `stop` hook → `save_step()` → temporary checkpoint tree row in SQLite
//! 2. `git commit` → `prepare_commit_msg()` → no-op
//! 3. `commit-msg` hook → `commit_msg()` → no-op
//! 4. `post-commit` hook → `post_commit()` → condenses session into checkpoint rows/blobs
//! 5. `post-merge` hook → `post_merge()` → refreshes DevQL for pull/merge changed files
//! 6. `post-checkout` hook → `post_checkout()` → seeds DevQL branch current-state when needed
//! 7. `reference-transaction` hook → `reference_transaction()` → branch-deletion cleanup
//! 8. `git push` → `pre_push()` → replicates DevQL historical/current state to Postgres
//!
//! Git operations use shell `git` + `GIT_INDEX_FILE` for temp-index tree construction.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::adapters::agents::claude_code::transcript as claude_transcript;
use crate::adapters::agents::{
    AGENT_TYPE_CLAUDE_CODE, AGENT_TYPE_CODEX, AGENT_TYPE_GEMINI, AGENT_TYPE_OPEN_CODE, TokenUsage,
    canonical_agent_key,
};
#[cfg(test)]
use crate::host::checkpoints::checkpoint_id::CHECKPOINT_KEY;
use crate::host::checkpoints::checkpoint_id::is_valid_checkpoint_id;
use crate::host::checkpoints::session::phase::{
    Action, Event, NoOpActionHandler, SessionPhase, TransitionContext, apply_transition,
    transition_with_context,
};
use crate::host::checkpoints::session::state::{
    PromptAttribution as SessionPromptAttribution, SessionState,
};
use crate::host::checkpoints::session::{SessionBackend, create_session_backend_or_local};
use crate::host::checkpoints::transcript::commit_message;
use crate::host::validation::validators::{
    validate_agent_id, validate_session_id, validate_tool_use_id,
};
use crate::utils::paths;
use crate::utils::strings;

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
    backend: Box<dyn SessionBackend>,
}

impl ManualCommitStrategy {
    pub fn new(repo_root: impl Into<PathBuf>) -> Self {
        let root = repo_root.into();
        let backend = create_session_backend_or_local(&root);
        Self::with_backend(root, backend)
    }

    pub fn with_backend(repo_root: impl Into<PathBuf>, backend: Box<dyn SessionBackend>) -> Self {
        let root = repo_root.into();
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

#[path = "manual_commit/strategy_helpers.rs"]
mod strategy_helpers;
#[path = "manual_commit/strategy_impl.rs"]
mod strategy_impl;
#[path = "manual_commit/support.rs"]
mod support;

pub(crate) use self::strategy_impl::*;
pub use self::support::*;
pub(crate) use CommittedMetadata;
pub(crate) use WriteCommittedOptions;
pub(crate) use persist_committed_checkpoint_db_and_blobs;
pub(crate) use redact_bytes;
pub(crate) use redact_jsonl_bytes_with_fallback;
pub(crate) use redact_text;

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "manual_commit_tests.rs"]
mod tests;

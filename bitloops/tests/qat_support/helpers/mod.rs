use super::world::QatWorld;
use anyhow::{Context, Result, anyhow, bail, ensure};
use bitloops::adapters::agents::AgentAdapterRegistry;
use bitloops::adapters::agents::AGENT_NAME_CLAUDE_CODE;
use bitloops::adapters::agents::claude_code::git_hooks;
use bitloops::config::{
    REPO_POLICY_LOCAL_FILE_NAME, discover_repo_policy, resolve_duckdb_db_path_for_repo,
    resolve_sqlite_db_path_for_repo, resolve_store_backend_config_for_repo,
};
use bitloops::daemon::resolve_daemon_config;
use bitloops::config::settings::{
    DEFAULT_STRATEGY, load_settings, set_capture_enabled, write_project_bootstrap_settings,
};
use bitloops::host::checkpoints::session::create_session_backend_or_local;
use bitloops::host::checkpoints::strategy::manual_commit::{
    read_commit_checkpoint_mappings, read_committed,
};
use serde::Serialize;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration as StdDuration, Instant};
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime, PrimitiveDateTime, Time, UtcOffset};
use uuid::Uuid;

pub const BITLOOPS_REPO_NAME: &str = "bitloops";
const DEFAULT_CLAUDE_CODE_COMMAND: &str =
    "claude --model haiku --permission-mode bypassPermissions -p";
const FIRST_CLAUDE_PROMPT: &str =
    "Remove the Vite example code from the project and replace it with a simple hello world page";
const SECOND_CLAUDE_PROMPT: &str = "Change the hello world color to blue";
const COMMAND_TIMEOUT_ENV: &str = "BITLOOPS_QAT_COMMAND_TIMEOUT_SECS";
const DEFAULT_COMMAND_TIMEOUT_SECS: u64 = 180;
const CLAUDE_TIMEOUT_ENV: &str = "BITLOOPS_QAT_CLAUDE_TIMEOUT_SECS";
const DEFAULT_CLAUDE_TIMEOUT_SECS: u64 = 30;
const CLAUDE_AUTH_TIMEOUT_ENV: &str = "BITLOOPS_QAT_CLAUDE_AUTH_TIMEOUT_SECS";
const DEFAULT_CLAUDE_AUTH_TIMEOUT_SECS: u64 = 300;
const CLAUDE_AUTH_STATUS_COMMAND_ENV: &str = "BITLOOPS_QAT_CLAUDE_AUTH_STATUS_CMD";
const DEFAULT_CLAUDE_AUTH_STATUS_COMMAND: &str = "claude auth status --json";
const CLAUDE_AUTH_LOGIN_COMMAND_ENV: &str = "BITLOOPS_QAT_CLAUDE_AUTH_LOGIN_CMD";
const DEFAULT_CLAUDE_AUTH_LOGIN_COMMAND: &str = "claude auth login --claudeai";
const CLAUDE_FALLBACK_MARKER: &str = ".qat-claude-fallback";
const SEMANTIC_CLONES_FALLBACK_MARKER: &str = ".qat-semantic-clones-fallback";
const KNOWLEDGE_FALLBACK_MARKER: &str = ".qat-knowledge-fallback";

#[derive(Debug, Serialize)]
struct RunMetadata<'a> {
    scenario_name: &'a str,
    scenario_slug: &'a str,
    flow_name: &'a str,
    run_dir: String,
    repo_dir: String,
    terminal_log: String,
    binary_path: String,
    created_at: String,
}

include!("core.rs");
include!("deps_and_testlens.rs");
include!("semantic_clones.rs");
include!("knowledge.rs");
include!("internals.rs");

#[cfg(test)]
mod tests;

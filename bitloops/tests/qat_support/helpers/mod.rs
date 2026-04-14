use super::world::QatWorld;
use anyhow::{Context, Result, anyhow, bail, ensure};
use bitloops::adapters::agents::{
    AGENT_NAME_CLAUDE_CODE, AGENT_NAME_CODEX, AGENT_NAME_COPILOT, AGENT_NAME_CURSOR,
    AGENT_NAME_GEMINI, AGENT_NAME_OPEN_CODE,
};
use bitloops::config::settings::load_settings;
use bitloops::config::{
    resolve_duckdb_db_path_for_repo, resolve_sqlite_db_path_for_repo,
    resolve_store_backend_config_for_repo,
};
use bitloops::daemon::resolve_daemon_config;
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
use std::sync::{Mutex, MutexGuard, OnceLock};
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
const TESTLENS_EVENTUAL_TIMEOUT_ENV: &str = "BITLOOPS_QAT_TESTLENS_EVENTUAL_TIMEOUT_SECS";
const DEFAULT_TESTLENS_EVENTUAL_TIMEOUT_SECS: u64 = 15;
const TESTLENS_EVENTUAL_POLL_INTERVAL_MILLIS: u64 = 250;
const SEMANTIC_CLONES_EVENTUAL_TIMEOUT_ENV: &str =
    "BITLOOPS_QAT_SEMANTIC_CLONES_EVENTUAL_TIMEOUT_SECS";
const DEFAULT_SEMANTIC_CLONES_EVENTUAL_TIMEOUT_SECS: u64 = 60;
const SEMANTIC_CLONES_EVENTUAL_POLL_INTERVAL_MILLIS: u64 = 250;
const CLAUDE_TIMEOUT_ENV: &str = "BITLOOPS_QAT_CLAUDE_TIMEOUT_SECS";
const DEFAULT_CLAUDE_TIMEOUT_SECS: u64 = 30;
const CLAUDE_AUTH_TIMEOUT_ENV: &str = "BITLOOPS_QAT_CLAUDE_AUTH_TIMEOUT_SECS";
const DEFAULT_CLAUDE_AUTH_TIMEOUT_SECS: u64 = 300;
const CLAUDE_AUTH_STATUS_COMMAND_ENV: &str = "BITLOOPS_QAT_CLAUDE_AUTH_STATUS_CMD";
const DEFAULT_CLAUDE_AUTH_STATUS_COMMAND: &str = "claude auth status --json";
const CLAUDE_AUTH_LOGIN_COMMAND_ENV: &str = "BITLOOPS_QAT_CLAUDE_AUTH_LOGIN_CMD";
const DEFAULT_CLAUDE_AUTH_LOGIN_COMMAND: &str = "claude auth login --claudeai";
const CLAUDE_FALLBACK_MARKER: &str = ".qat-claude-fallback";

fn qat_env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct ScenarioAppEnvGuard {
    _lock_guard: MutexGuard<'static, ()>,
    previous_env: Vec<(String, Option<OsString>)>,
}

impl Drop for ScenarioAppEnvGuard {
    fn drop(&mut self) {
        restore_scenario_env_vars(&self.previous_env);
    }
}

fn apply_scenario_env_vars(
    vars: &[(&str, Option<&std::ffi::OsStr>)],
) -> Vec<(String, Option<OsString>)> {
    let mut previous = Vec::with_capacity(vars.len());
    for (key, value) in vars {
        previous.push(((*key).to_string(), std::env::var_os(key)));
        // SAFETY: QAT assertions serialise process env mutation through qat_env_lock().
        unsafe {
            match value {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
    }
    previous
}

fn restore_scenario_env_vars(previous: &[(String, Option<OsString>)]) {
    for (key, value) in previous.iter().rev() {
        // SAFETY: QAT assertions serialise process env mutation through qat_env_lock().
        unsafe {
            match value {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
    }
}

fn enter_scenario_app_env(world: &QatWorld) -> ScenarioAppEnvGuard {
    let home = world.run_dir().join("home");
    let xdg_config = home.join("xdg");
    let xdg_data = home.join("xdg-data");
    let xdg_cache = home.join("xdg-cache");
    let xdg_state = home.join("xdg-state");
    let lock_guard = qat_env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let previous_env = apply_scenario_env_vars(&[
        ("HOME", Some(home.as_os_str())),
        ("USERPROFILE", Some(home.as_os_str())),
        ("XDG_CONFIG_HOME", Some(xdg_config.as_os_str())),
        ("XDG_DATA_HOME", Some(xdg_data.as_os_str())),
        ("XDG_CACHE_HOME", Some(xdg_cache.as_os_str())),
        ("XDG_STATE_HOME", Some(xdg_state.as_os_str())),
        ("BITLOOPS_DEVQL_PG_DSN", None),
        ("BITLOOPS_DEVQL_CH_URL", None),
        ("BITLOOPS_DEVQL_CH_DATABASE", None),
        ("BITLOOPS_DEVQL_CH_USER", None),
        ("BITLOOPS_DEVQL_CH_PASSWORD", None),
    ]);
    ScenarioAppEnvGuard {
        _lock_guard: lock_guard,
        previous_env,
    }
}

fn with_scenario_app_env<T>(world: &QatWorld, f: impl FnOnce() -> T) -> T {
    let _guard = enter_scenario_app_env(world);
    f()
}

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
include!("daemon_harness.rs");
include!("capability_runtime.rs");
include!("deps_and_testlens.rs");
include!("semantic_clones.rs");
include!("knowledge.rs");
include!("internals.rs");

#[cfg(test)]
mod tests;

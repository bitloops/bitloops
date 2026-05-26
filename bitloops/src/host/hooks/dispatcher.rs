//! `bitloops hooks ...` — shared dispatcher for agent and git hook commands.
use std::io::{self, Read};
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime};

use anyhow::{Context, Result};
use clap::{Args, Subcommand};

use crate::adapters::agents::{
    AGENT_NAME_CLAUDE_CODE, AGENT_NAME_CODEX, AGENT_NAME_COPILOT, AGENT_NAME_CURSOR,
    AGENT_NAME_GEMINI, AGENT_NAME_OPEN_CODE,
};
use crate::config::settings;
use crate::host::checkpoints::lifecycle::adapters::{
    CLAUDE_HOOK_POST_TASK, CLAUDE_HOOK_POST_TODO, CLAUDE_HOOK_POST_TOOL_USE, CLAUDE_HOOK_PRE_TASK,
    CLAUDE_HOOK_PRE_TOOL_USE, CLAUDE_HOOK_SESSION_END, CLAUDE_HOOK_SESSION_START, CLAUDE_HOOK_STOP,
    CLAUDE_HOOK_USER_PROMPT_SUBMIT, CODEX_HOOK_POST_TOOL_USE, CODEX_HOOK_PRE_TOOL_USE,
    CODEX_HOOK_SESSION_START, CODEX_HOOK_STOP, CODEX_HOOK_USER_PROMPT_SUBMIT,
    COPILOT_HOOK_AGENT_STOP, COPILOT_HOOK_POST_TOOL_USE, COPILOT_HOOK_PRE_TOOL_USE,
    COPILOT_HOOK_SESSION_END, COPILOT_HOOK_SESSION_START, COPILOT_HOOK_SUBAGENT_STOP,
    COPILOT_HOOK_USER_PROMPT_SUBMITTED, CURSOR_HOOK_BEFORE_SUBMIT_PROMPT, CURSOR_HOOK_PRE_COMPACT,
    CURSOR_HOOK_SESSION_END, CURSOR_HOOK_SESSION_START, CURSOR_HOOK_STOP,
    CURSOR_HOOK_SUBAGENT_START, CURSOR_HOOK_SUBAGENT_STOP, ClaudeCodeLifecycleAdapter,
    CodexLifecycleAdapter, CopilotCliLifecycleAdapter, CursorLifecycleAdapter,
    GEMINI_HOOK_AFTER_AGENT, GEMINI_HOOK_AFTER_TOOL, GEMINI_HOOK_BEFORE_AGENT,
    GEMINI_HOOK_BEFORE_TOOL, GEMINI_HOOK_PRE_COMPRESS, GEMINI_HOOK_SESSION_END,
    GEMINI_HOOK_SESSION_START, GeminiCliLifecycleAdapter, OPENCODE_HOOK_COMPACTION,
    OPENCODE_HOOK_SESSION_END, OPENCODE_HOOK_SESSION_START, OPENCODE_HOOK_TURN_END,
    OPENCODE_HOOK_TURN_START, OpenCodeLifecycleAdapter, route_hook_command_to_lifecycle,
};
use crate::host::checkpoints::lifecycle::{
    LifecycleAgentAdapter, LifecycleEvent, LifecycleEventType,
};
use crate::host::checkpoints::session::create_session_backend_or_local;
use crate::host::checkpoints::strategy::registry::{self, StrategyRegistry};
use crate::telemetry::logging;
use crate::utils::paths;

use super::git;

#[derive(Args)]
pub struct HooksArgs {
    #[command(subcommand)]
    pub agent: HooksAgent,
}

#[derive(Subcommand)]
pub enum HooksAgent {
    #[command(name = "claude-code")]
    ClaudeCode(ClaudeCodeHooksArgs),
    #[command(name = "codex")]
    Codex(CodexHooksArgs),
    #[command(name = "cursor")]
    Cursor(CursorHooksArgs),
    #[command(name = "copilot")]
    Copilot(CopilotHooksArgs),
    #[command(name = "gemini")]
    Gemini(GeminiHooksArgs),
    #[command(name = "opencode")]
    OpenCode(OpenCodeHooksArgs),
    /// Git hook handlers (called by git hooks, not users).
    #[command(name = "git")]
    Git(git::GitHooksArgs),
}

#[derive(Args)]
pub struct ClaudeCodeHooksArgs {
    #[command(subcommand)]
    pub verb: ClaudeCodeHookVerb,
}

#[derive(Subcommand)]
pub enum ClaudeCodeHookVerb {
    #[command(name = "session-start")]
    SessionStart,
    #[command(name = "session-end")]
    SessionEnd,
    #[command(name = "stop")]
    Stop,
    #[command(name = "user-prompt-submit")]
    UserPromptSubmit,
    #[command(name = "pre-task")]
    PreTask,
    #[command(name = "post-task")]
    PostTask,
    #[command(name = "pre-tool-use")]
    PreToolUse,
    #[command(name = "post-tool-use")]
    PostToolUse,
    #[command(name = "post-todo")]
    PostTodo,
}

#[derive(Args)]
pub struct CodexHooksArgs {
    #[command(subcommand)]
    pub verb: CodexHookVerb,
}

#[derive(Subcommand)]
pub enum CodexHookVerb {
    #[command(name = "session-start")]
    SessionStart,
    #[command(name = "user-prompt-submit")]
    UserPromptSubmit,
    #[command(name = "pre-tool-use")]
    PreToolUse,
    #[command(name = "post-tool-use")]
    PostToolUse,
    #[command(name = "stop")]
    Stop,
}

#[derive(Args)]
pub struct GeminiHooksArgs {
    #[command(subcommand)]
    pub verb: GeminiHookVerb,
}

#[derive(Subcommand)]
pub enum GeminiHookVerb {
    #[command(name = "session-start")]
    SessionStart,
    #[command(name = "session-end")]
    SessionEnd,
    #[command(name = "before-agent")]
    BeforeAgent,
    #[command(name = "after-agent")]
    AfterAgent,
    #[command(name = "pre-compress")]
    PreCompress,
    #[command(name = "before-tool")]
    BeforeTool,
    #[command(name = "after-tool")]
    AfterTool,
    #[command(name = "before-model")]
    BeforeModel,
    #[command(name = "after-model")]
    AfterModel,
    #[command(name = "before-tool-selection")]
    BeforeToolSelection,
    #[command(name = "notification")]
    Notification,
}

#[derive(Args)]
pub struct CursorHooksArgs {
    #[command(subcommand)]
    pub verb: CursorHookVerb,
}

#[derive(Args)]
pub struct CopilotHooksArgs {
    #[command(subcommand)]
    pub verb: CopilotHookVerb,
}

#[derive(Args)]
pub struct OpenCodeHooksArgs {
    #[command(subcommand)]
    pub verb: OpenCodeHookVerb,
}

#[derive(Subcommand)]
pub enum CursorHookVerb {
    #[command(name = "session-start")]
    SessionStart,
    #[command(name = "before-submit-prompt")]
    BeforeSubmitPrompt,
    #[command(name = "before-shell-execution")]
    BeforeShellExecution,
    #[command(name = "after-shell-execution")]
    AfterShellExecution,
    #[command(name = "stop")]
    Stop,
    #[command(name = "session-end")]
    SessionEnd,
    #[command(name = "pre-compact")]
    PreCompact,
    #[command(name = "subagent-start")]
    SubagentStart,
    #[command(name = "subagent-stop")]
    SubagentStop,
}

#[derive(Subcommand)]
pub enum CopilotHookVerb {
    #[command(name = "user-prompt-submitted")]
    UserPromptSubmitted,
    #[command(name = "session-start")]
    SessionStart,
    #[command(name = "agent-stop")]
    AgentStop,
    #[command(name = "session-end")]
    SessionEnd,
    #[command(name = "subagent-stop")]
    SubagentStop,
    #[command(name = "pre-tool-use")]
    PreToolUse,
    #[command(name = "post-tool-use")]
    PostToolUse,
    #[command(name = "error-occurred")]
    ErrorOccurred,
}

#[derive(Subcommand)]
pub enum OpenCodeHookVerb {
    #[command(name = "session-start")]
    SessionStart,
    #[command(name = "turn-start")]
    TurnStart,
    #[command(name = "turn-end")]
    TurnEnd,
    #[command(name = "compaction")]
    Compaction,
    #[command(name = "session-end")]
    SessionEnd,
}

impl ClaudeCodeHookVerb {
    pub fn hook_name(&self) -> &'static str {
        match self {
            Self::SessionStart => "session-start",
            Self::SessionEnd => "session-end",
            Self::Stop => "stop",
            Self::UserPromptSubmit => "user-prompt-submit",
            Self::PreTask => "pre-task",
            Self::PostTask => "post-task",
            Self::PreToolUse => "pre-tool-use",
            Self::PostToolUse => "post-tool-use",
            Self::PostTodo => "post-todo",
        }
    }
}

impl CodexHookVerb {
    pub fn hook_name(&self) -> &'static str {
        match self {
            Self::SessionStart => {
                crate::host::checkpoints::lifecycle::adapters::CODEX_HOOK_SESSION_START
            }
            Self::UserPromptSubmit => {
                crate::host::checkpoints::lifecycle::adapters::CODEX_HOOK_USER_PROMPT_SUBMIT
            }
            Self::PreToolUse => {
                crate::host::checkpoints::lifecycle::adapters::CODEX_HOOK_PRE_TOOL_USE
            }
            Self::PostToolUse => {
                crate::host::checkpoints::lifecycle::adapters::CODEX_HOOK_POST_TOOL_USE
            }
            Self::Stop => crate::host::checkpoints::lifecycle::adapters::CODEX_HOOK_STOP,
        }
    }
}

impl GeminiHookVerb {
    pub fn hook_name(&self) -> &'static str {
        match self {
            Self::SessionStart => "session-start",
            Self::SessionEnd => "session-end",
            Self::BeforeAgent => "before-agent",
            Self::AfterAgent => "after-agent",
            Self::PreCompress => "pre-compress",
            Self::BeforeTool => "before-tool",
            Self::AfterTool => "after-tool",
            Self::BeforeModel => "before-model",
            Self::AfterModel => "after-model",
            Self::BeforeToolSelection => "before-tool-selection",
            Self::Notification => "notification",
        }
    }
}

impl CursorHookVerb {
    pub fn hook_name(&self) -> &'static str {
        match self {
            Self::SessionStart => "session-start",
            Self::BeforeSubmitPrompt => "before-submit-prompt",
            Self::BeforeShellExecution => "before-shell-execution",
            Self::AfterShellExecution => "after-shell-execution",
            Self::Stop => "stop",
            Self::SessionEnd => "session-end",
            Self::PreCompact => "pre-compact",
            Self::SubagentStart => "subagent-start",
            Self::SubagentStop => "subagent-stop",
        }
    }
}

impl CopilotHookVerb {
    pub fn hook_name(&self) -> &'static str {
        match self {
            Self::UserPromptSubmitted => {
                crate::adapters::agents::copilot::lifecycle::HOOK_NAME_USER_PROMPT_SUBMITTED
            }
            Self::SessionStart => {
                crate::adapters::agents::copilot::lifecycle::HOOK_NAME_SESSION_START
            }
            Self::AgentStop => crate::adapters::agents::copilot::lifecycle::HOOK_NAME_AGENT_STOP,
            Self::SessionEnd => crate::adapters::agents::copilot::lifecycle::HOOK_NAME_SESSION_END,
            Self::SubagentStop => {
                crate::adapters::agents::copilot::lifecycle::HOOK_NAME_SUBAGENT_STOP
            }
            Self::PreToolUse => crate::adapters::agents::copilot::lifecycle::HOOK_NAME_PRE_TOOL_USE,
            Self::PostToolUse => {
                crate::adapters::agents::copilot::lifecycle::HOOK_NAME_POST_TOOL_USE
            }
            Self::ErrorOccurred => {
                crate::adapters::agents::copilot::lifecycle::HOOK_NAME_ERROR_OCCURRED
            }
        }
    }
}

impl OpenCodeHookVerb {
    pub fn hook_name(&self) -> &'static str {
        match self {
            Self::SessionStart => {
                crate::host::checkpoints::lifecycle::adapters::OPENCODE_HOOK_SESSION_START
            }
            Self::TurnStart => {
                crate::host::checkpoints::lifecycle::adapters::OPENCODE_HOOK_TURN_START
            }
            Self::TurnEnd => crate::host::checkpoints::lifecycle::adapters::OPENCODE_HOOK_TURN_END,
            Self::Compaction => {
                crate::host::checkpoints::lifecycle::adapters::OPENCODE_HOOK_COMPACTION
            }
            Self::SessionEnd => {
                crate::host::checkpoints::lifecycle::adapters::OPENCODE_HOOK_SESSION_END
            }
        }
    }
}

fn current_hook_agent_name_store() -> &'static Mutex<String> {
    static STORE: OnceLock<Mutex<String>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(String::new()))
}

fn set_current_hook_agent_name(agent_name: &str) {
    let mut guard = current_hook_agent_name_store()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *guard = agent_name.to_string();
}

fn clear_current_hook_agent_name() {
    let mut guard = current_hook_agent_name_store()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.clear();
}

#[cfg(test)]
pub(crate) fn current_hook_agent_name_for_tests() -> String {
    current_hook_agent_name_store()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

fn get_hook_type(agent_name: &str, hook_name: &str) -> &'static str {
    match (agent_name, hook_name) {
        (
            AGENT_NAME_CLAUDE_CODE,
            CLAUDE_HOOK_PRE_TASK | CLAUDE_HOOK_POST_TASK | CLAUDE_HOOK_POST_TODO,
        ) => "subagent",
        (AGENT_NAME_CLAUDE_CODE, CLAUDE_HOOK_PRE_TOOL_USE | CLAUDE_HOOK_POST_TOOL_USE) => "tool",
        (
            AGENT_NAME_CURSOR,
            crate::adapters::agents::cursor::lifecycle::HOOK_NAME_SUBAGENT_START
            | crate::adapters::agents::cursor::lifecycle::HOOK_NAME_SUBAGENT_STOP,
        ) => "subagent",
        (
            AGENT_NAME_COPILOT,
            crate::adapters::agents::copilot::lifecycle::HOOK_NAME_SUBAGENT_STOP,
        ) => "subagent",
        (AGENT_NAME_GEMINI, GEMINI_HOOK_BEFORE_TOOL | GEMINI_HOOK_AFTER_TOOL) => "tool",
        (
            AGENT_NAME_CODEX,
            crate::adapters::agents::codex::lifecycle::HOOK_NAME_PRE_TOOL_USE
            | crate::adapters::agents::codex::lifecycle::HOOK_NAME_POST_TOOL_USE,
        ) => "tool",
        (AGENT_NAME_COPILOT, COPILOT_HOOK_PRE_TOOL_USE | COPILOT_HOOK_POST_TOOL_USE) => "tool",
        _ => "agent",
    }
}

fn find_most_recent_session_id(repo_root: &Path) -> String {
    let backend = create_session_backend_or_local(repo_root);
    let sessions = backend.list_sessions().unwrap_or_default();
    crate::host::checkpoints::session::state::find_most_recent_session(
        &sessions,
        &repo_root.to_string_lossy(),
    )
    .map(|s| s.session_id)
    .unwrap_or_default()
}

fn init_hook_logging(repo_root: &Path) {
    let session_id = find_most_recent_session_id(repo_root);
    let _ = logging::init(&session_id);
}

fn hook_action_descriptor(
    agent_name: &str,
    hook_name: &str,
) -> crate::telemetry::analytics::ActionDescriptor {
    let mut properties = std::collections::HashMap::new();
    properties.insert(
        "command".to_string(),
        serde_json::Value::String("bitloops hook".to_string()),
    );
    properties.insert(
        "agent".to_string(),
        serde_json::Value::String(agent_name.to_string()),
    );
    properties.insert(
        "hook".to_string(),
        serde_json::Value::String(hook_name.to_string()),
    );
    properties.insert(
        "hook_type".to_string(),
        serde_json::Value::String(get_hook_type(agent_name, hook_name).to_string()),
    );

    crate::telemetry::analytics::ActionDescriptor {
        event: "bitloops hook".to_string(),
        surface: "hook",
        properties,
    }
}

fn track_hook_action(
    repo_root: &Path,
    dispatch_context: Option<&crate::telemetry::analytics::TelemetryDispatchContext>,
    agent_name: &str,
    hook_name: &str,
    success: bool,
    duration_ms: u128,
) {
    let Some(dispatch_context) = dispatch_context else {
        return;
    };
    let descriptor = hook_action_descriptor(agent_name, hook_name);
    crate::telemetry::analytics::track_action_detached(
        Some(&descriptor),
        dispatch_context,
        env!("CARGO_PKG_VERSION"),
        Some(repo_root),
        success,
        duration_ms,
    );
}

pub(crate) fn run_agent_hook_with_logging<F, T>(
    repo_root: &Path,
    agent_name: &str,
    hook_name: &str,
    strategy_name: &str,
    handler: F,
) -> Result<T>
where
    F: FnOnce() -> Result<T>,
{
    init_hook_logging(repo_root);

    let start = SystemTime::now();
    let ctx = logging::with_agent(
        logging::with_component(logging::background(), "hooks"),
        agent_name,
    );
    let hook_type = get_hook_type(agent_name, hook_name);

    logging::debug(
        &ctx,
        "hook invoked",
        &[
            logging::string_attr("hook", hook_name),
            logging::string_attr("hook_type", hook_type),
            logging::string_attr("strategy", strategy_name),
        ],
    );
    logging::info(
        &ctx,
        "hook invoked",
        &[
            logging::string_attr("hook", hook_name),
            logging::string_attr("hook_type", hook_type),
            logging::string_attr("strategy", strategy_name),
        ],
    );

    set_current_hook_agent_name(agent_name);
    let result = handler();
    clear_current_hook_agent_name();

    if let Err(err) = result.as_ref() {
        logging::warn(
            &ctx,
            "hook failed",
            &[
                logging::string_attr("hook", hook_name),
                logging::string_attr("hook_type", hook_type),
                logging::string_attr("strategy", strategy_name),
                logging::string_attr("error", &format!("{err:#}")),
            ],
        );
    }

    logging::log_duration(
        &ctx,
        logging::LogLevel::Debug,
        "hook completed",
        start,
        &[
            logging::string_attr("hook", hook_name),
            logging::string_attr("hook_type", hook_type),
            logging::string_attr("strategy", strategy_name),
            logging::bool_attr("success", result.is_ok()),
        ],
    );
    logging::log_duration(
        &ctx,
        logging::LogLevel::Info,
        "hook completed",
        start,
        &[
            logging::string_attr("hook", hook_name),
            logging::string_attr("hook_type", hook_type),
            logging::string_attr("strategy", strategy_name),
            logging::bool_attr("success", result.is_ok()),
        ],
    );

    logging::close();
    result
}

fn emit_hook_stdout_if_present(
    outcome: &crate::host::checkpoints::lifecycle::adapters::HookCommandOutcome,
) -> Result<()> {
    if let Some(stdout) = &outcome.stdout {
        print!("{stdout}");
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LifecycleHookDispatchMode {
    Sync,
    AsyncNoSnapshot,
    AsyncPreBoundarySnapshot,
    AsyncWorkspaceSnapshot,
    AsyncWorkspaceAndBranchSnapshot,
}

impl LifecycleHookDispatchMode {
    const fn is_async(self) -> bool {
        !matches!(self, Self::Sync)
    }
}

fn lifecycle_hook_dispatch_mode(agent_name: &str, hook_name: &str) -> LifecycleHookDispatchMode {
    use LifecycleHookDispatchMode::*;

    match (agent_name, hook_name) {
        (AGENT_NAME_CLAUDE_CODE, CLAUDE_HOOK_SESSION_START)
        | (AGENT_NAME_CLAUDE_CODE, CLAUDE_HOOK_SESSION_END)
        | (AGENT_NAME_CLAUDE_CODE, CLAUDE_HOOK_PRE_TOOL_USE)
        | (AGENT_NAME_CLAUDE_CODE, CLAUDE_HOOK_POST_TOOL_USE)
        | (AGENT_NAME_CODEX, CODEX_HOOK_SESSION_START)
        | (AGENT_NAME_GEMINI, GEMINI_HOOK_SESSION_START)
        | (AGENT_NAME_GEMINI, GEMINI_HOOK_SESSION_END)
        | (AGENT_NAME_GEMINI, GEMINI_HOOK_PRE_COMPRESS)
        | (AGENT_NAME_CURSOR, CURSOR_HOOK_SESSION_START)
        | (AGENT_NAME_CURSOR, CURSOR_HOOK_PRE_COMPACT)
        | (AGENT_NAME_COPILOT, COPILOT_HOOK_SESSION_START)
        | (AGENT_NAME_COPILOT, COPILOT_HOOK_SESSION_END)
        | (AGENT_NAME_OPEN_CODE, OPENCODE_HOOK_SESSION_START)
        | (AGENT_NAME_OPEN_CODE, OPENCODE_HOOK_SESSION_END)
        | (AGENT_NAME_OPEN_CODE, OPENCODE_HOOK_COMPACTION) => AsyncNoSnapshot,

        (AGENT_NAME_CLAUDE_CODE, CLAUDE_HOOK_USER_PROMPT_SUBMIT)
        | (AGENT_NAME_CLAUDE_CODE, CLAUDE_HOOK_PRE_TASK)
        | (AGENT_NAME_CODEX, CODEX_HOOK_USER_PROMPT_SUBMIT)
        | (AGENT_NAME_CODEX, CODEX_HOOK_PRE_TOOL_USE)
        | (AGENT_NAME_GEMINI, GEMINI_HOOK_BEFORE_AGENT)
        | (AGENT_NAME_CURSOR, CURSOR_HOOK_BEFORE_SUBMIT_PROMPT)
        | (
            AGENT_NAME_CURSOR,
            crate::adapters::agents::cursor::lifecycle::HOOK_NAME_BEFORE_SHELL_EXECUTION,
        )
        | (AGENT_NAME_CURSOR, CURSOR_HOOK_SUBAGENT_START)
        | (AGENT_NAME_COPILOT, COPILOT_HOOK_USER_PROMPT_SUBMITTED)
        | (AGENT_NAME_OPEN_CODE, OPENCODE_HOOK_TURN_START) => AsyncPreBoundarySnapshot,

        (AGENT_NAME_CODEX, CODEX_HOOK_STOP)
        | (AGENT_NAME_CODEX, CODEX_HOOK_POST_TOOL_USE)
        | (AGENT_NAME_CLAUDE_CODE, CLAUDE_HOOK_STOP)
        | (AGENT_NAME_CLAUDE_CODE, CLAUDE_HOOK_POST_TASK)
        | (AGENT_NAME_GEMINI, GEMINI_HOOK_AFTER_AGENT)
        | (AGENT_NAME_CURSOR, CURSOR_HOOK_STOP)
        | (
            AGENT_NAME_CURSOR,
            crate::adapters::agents::cursor::lifecycle::HOOK_NAME_AFTER_SHELL_EXECUTION,
        )
        | (AGENT_NAME_CURSOR, CURSOR_HOOK_SESSION_END)
        | (AGENT_NAME_CURSOR, CURSOR_HOOK_SUBAGENT_STOP)
        | (AGENT_NAME_COPILOT, COPILOT_HOOK_AGENT_STOP)
        | (AGENT_NAME_COPILOT, COPILOT_HOOK_SUBAGENT_STOP)
        | (AGENT_NAME_OPEN_CODE, OPENCODE_HOOK_TURN_END) => AsyncWorkspaceSnapshot,

        (AGENT_NAME_CLAUDE_CODE, CLAUDE_HOOK_POST_TODO) => AsyncWorkspaceAndBranchSnapshot,

        _ => Sync,
    }
}

fn parse_lifecycle_event_for_enqueue_offset(
    repo_root: &Path,
    adapter: &dyn LifecycleAgentAdapter,
    hook_name: &str,
    stdin: &str,
) -> Result<Option<LifecycleEvent>> {
    let mut input = std::io::Cursor::new(stdin.as_bytes());
    if adapter.agent_name() == AGENT_NAME_CODEX {
        crate::adapters::agents::codex::lifecycle::parse_hook_event_for_repo(
            hook_name, &mut input, repo_root,
        )
    } else if adapter.agent_name() == AGENT_NAME_CURSOR {
        crate::adapters::agents::cursor::lifecycle::parse_hook_event_for_repo(
            hook_name, &mut input, repo_root,
        )
    } else {
        adapter.parse_hook_event(hook_name, &mut input)
    }
}

fn lifecycle_adapter_for_enqueue_offset(
    agent_name: &str,
) -> Option<Box<dyn LifecycleAgentAdapter>> {
    match agent_name {
        AGENT_NAME_CLAUDE_CODE => Some(Box::new(ClaudeCodeLifecycleAdapter)),
        AGENT_NAME_CODEX => Some(Box::new(CodexLifecycleAdapter)),
        AGENT_NAME_COPILOT => Some(Box::new(CopilotCliLifecycleAdapter)),
        AGENT_NAME_CURSOR => Some(Box::new(CursorLifecycleAdapter)),
        AGENT_NAME_GEMINI => Some(Box::new(GeminiCliLifecycleAdapter)),
        AGENT_NAME_OPEN_CODE => Some(Box::new(OpenCodeLifecycleAdapter)),
        _ => None,
    }
}

fn capture_pre_boundary_transcript_offset(
    repo_root: &Path,
    agent_name: &str,
    hook_name: &str,
    stdin: &str,
) -> Option<i64> {
    let adapter = lifecycle_adapter_for_enqueue_offset(agent_name)?;
    let event = match parse_lifecycle_event_for_enqueue_offset(
        repo_root,
        adapter.as_ref(),
        hook_name,
        stdin,
    ) {
        Ok(Some(event)) => event,
        Ok(None) => return None,
        Err(err) => {
            log::debug!(
                "failed to parse lifecycle hook while capturing pre-boundary transcript offset for agent={} hook={}: {err:#}",
                agent_name,
                hook_name
            );
            return None;
        }
    };
    if event.event_type.as_ref() != Some(&LifecycleEventType::TurnStart) {
        return None;
    }
    let transcript_ref = event.session_ref.trim();
    if transcript_ref.is_empty() {
        return None;
    }
    adapter
        .as_transcript_analyzer()
        .and_then(|analyzer| analyzer.get_transcript_position(transcript_ref).ok())
        .and_then(|offset| i64::try_from(offset).ok())
}

fn enqueue_lifecycle_hook_from_hook(
    repo_root: &Path,
    agent_name: &str,
    hook_name: &str,
    stdin: &str,
    mode: LifecycleHookDispatchMode,
) -> Result<crate::host::checkpoints::lifecycle::spool::LifecycleHookEnqueueResult> {
    let repo = crate::host::devql::resolve_repo_identity(repo_root)
        .context("resolving repo identity for lifecycle hook spool")?;
    let config_root = crate::config::resolve_bound_daemon_config_root_for_repo(repo_root)
        .context("resolving daemon config root for lifecycle hook spool")?;
    let db_path = crate::config::resolve_repo_runtime_db_path_for_config_root(&config_root);
    let cwd = std::env::current_dir().unwrap_or_else(|_| repo_root.to_path_buf());
    let boundary_snapshot = match mode {
        LifecycleHookDispatchMode::Sync | LifecycleHookDispatchMode::AsyncNoSnapshot => None,
        LifecycleHookDispatchMode::AsyncPreBoundarySnapshot => {
            let transcript_offset =
                capture_pre_boundary_transcript_offset(repo_root, agent_name, hook_name, stdin);
            Some(
                crate::host::checkpoints::lifecycle::capture_pre_boundary_snapshot(
                    repo_root,
                    transcript_offset,
                ),
            )
        }
        LifecycleHookDispatchMode::AsyncWorkspaceSnapshot => Some(
            crate::host::checkpoints::lifecycle::capture_workspace_boundary_snapshot(repo_root),
        ),
        LifecycleHookDispatchMode::AsyncWorkspaceAndBranchSnapshot => Some(
            crate::host::checkpoints::lifecycle::capture_workspace_and_branch_snapshot(repo_root),
        ),
    };
    let workspace_snapshot = boundary_snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.workspace.clone());
    let insert = crate::host::checkpoints::lifecycle::spool::LifecycleJobInsert {
        repo_id: repo.repo_id,
        repo_root: repo_root.to_path_buf(),
        config_root,
        agent_name: agent_name.to_string(),
        hook_name: hook_name.to_string(),
        raw_stdin: stdin.to_string(),
        workspace_snapshot,
        boundary_snapshot,
        cwd,
        received_at_unix: crate::host::checkpoints::lifecycle::spool::unix_timestamp_now(),
    };
    crate::host::checkpoints::lifecycle::spool::enqueue_lifecycle_job_hook_safe_at(&db_path, insert)
}

fn route_or_enqueue_lifecycle_hook(
    repo_root: &Path,
    agent_name: &str,
    hook_name: &str,
    stdin: &str,
) -> Result<crate::host::checkpoints::lifecycle::adapters::HookCommandOutcome> {
    let mode = lifecycle_hook_dispatch_mode(agent_name, hook_name);
    if mode.is_async() {
        enqueue_lifecycle_hook_from_hook(repo_root, agent_name, hook_name, stdin, mode)
            .map(|_| crate::host::checkpoints::lifecycle::adapters::HookCommandOutcome::default())
    } else {
        route_hook_command_to_lifecycle(repo_root, agent_name, hook_name, stdin)
    }
}

pub async fn run(args: HooksArgs, strategy_registry: &StrategyRegistry) -> Result<()> {
    let agent = match args.agent {
        HooksAgent::Git(git_args) => return git::run(git_args, strategy_registry).await,
        other => other,
    };

    if crate::host::hooks::agent_hooks_suppressed_by_env() {
        return Ok(());
    }

    let repo_root = match paths::repo_root() {
        Ok(r) => r,
        Err(_) => return Ok(()),
    };
    let config_start = std::env::current_dir().unwrap_or_else(|_| repo_root.clone());

    if !settings::is_enabled_for_hooks(&config_start) {
        return Ok(());
    }

    let strategy_name = settings::load_settings(&config_start)
        .map(|s| s.strategy)
        .unwrap_or_else(|_| registry::STRATEGY_NAME_MANUAL_COMMIT.to_string());
    let dispatch_context = crate::telemetry::analytics::load_dispatch_context_for_repo(&repo_root);

    match agent {
        HooksAgent::ClaudeCode(cc) => {
            let hook_name = cc.verb.hook_name();
            let stdin = read_stdin()?;
            let started = Instant::now();
            let result = run_agent_hook_with_logging(
                &repo_root,
                AGENT_NAME_CLAUDE_CODE,
                hook_name,
                &strategy_name,
                || {
                    route_or_enqueue_lifecycle_hook(
                        &repo_root,
                        AGENT_NAME_CLAUDE_CODE,
                        hook_name,
                        &stdin,
                    )
                },
            );
            track_hook_action(
                &repo_root,
                dispatch_context.as_ref(),
                AGENT_NAME_CLAUDE_CODE,
                hook_name,
                result.is_ok(),
                started.elapsed().as_millis(),
            );
            result.and_then(|outcome| emit_hook_stdout_if_present(&outcome))
        }
        HooksAgent::Codex(codex) => {
            let hook_name = codex.verb.hook_name();
            let stdin = read_stdin()?;
            let started = Instant::now();
            let result = run_agent_hook_with_logging(
                &repo_root,
                AGENT_NAME_CODEX,
                hook_name,
                &strategy_name,
                || route_or_enqueue_lifecycle_hook(&repo_root, AGENT_NAME_CODEX, hook_name, &stdin),
            );
            track_hook_action(
                &repo_root,
                dispatch_context.as_ref(),
                AGENT_NAME_CODEX,
                hook_name,
                result.is_ok(),
                started.elapsed().as_millis(),
            );
            result.and_then(|outcome| emit_hook_stdout_if_present(&outcome))
        }
        HooksAgent::Gemini(gemini) => {
            let hook_name = gemini.verb.hook_name();
            let stdin = read_stdin()?;
            let started = Instant::now();
            let result = run_agent_hook_with_logging(
                &repo_root,
                AGENT_NAME_GEMINI,
                hook_name,
                &strategy_name,
                || {
                    route_or_enqueue_lifecycle_hook(
                        &repo_root,
                        AGENT_NAME_GEMINI,
                        hook_name,
                        &stdin,
                    )
                },
            );
            track_hook_action(
                &repo_root,
                dispatch_context.as_ref(),
                AGENT_NAME_GEMINI,
                hook_name,
                result.is_ok(),
                started.elapsed().as_millis(),
            );
            result.and_then(|outcome| emit_hook_stdout_if_present(&outcome))
        }
        HooksAgent::Cursor(cursor) => {
            let hook_name = cursor.verb.hook_name();
            let stdin = read_stdin()?;
            let started = Instant::now();
            let result = run_agent_hook_with_logging(
                &repo_root,
                AGENT_NAME_CURSOR,
                hook_name,
                &strategy_name,
                || {
                    route_or_enqueue_lifecycle_hook(
                        &repo_root,
                        AGENT_NAME_CURSOR,
                        hook_name,
                        &stdin,
                    )
                },
            );
            track_hook_action(
                &repo_root,
                dispatch_context.as_ref(),
                AGENT_NAME_CURSOR,
                hook_name,
                result.is_ok(),
                started.elapsed().as_millis(),
            );
            result.and_then(|outcome| emit_hook_stdout_if_present(&outcome))
        }
        HooksAgent::Copilot(copilot) => {
            let hook_name = copilot.verb.hook_name();
            let stdin = read_stdin()?;
            let started = Instant::now();
            let result = run_agent_hook_with_logging(
                &repo_root,
                AGENT_NAME_COPILOT,
                hook_name,
                &strategy_name,
                || {
                    route_or_enqueue_lifecycle_hook(
                        &repo_root,
                        AGENT_NAME_COPILOT,
                        hook_name,
                        &stdin,
                    )
                },
            );
            track_hook_action(
                &repo_root,
                dispatch_context.as_ref(),
                AGENT_NAME_COPILOT,
                hook_name,
                result.is_ok(),
                started.elapsed().as_millis(),
            );
            result.and_then(|outcome| emit_hook_stdout_if_present(&outcome))
        }
        HooksAgent::OpenCode(opencode) => {
            let hook_name = opencode.verb.hook_name();
            let stdin = read_stdin()?;
            let started = Instant::now();
            let result = run_agent_hook_with_logging(
                &repo_root,
                AGENT_NAME_OPEN_CODE,
                hook_name,
                &strategy_name,
                || {
                    route_or_enqueue_lifecycle_hook(
                        &repo_root,
                        AGENT_NAME_OPEN_CODE,
                        hook_name,
                        &stdin,
                    )
                },
            );
            track_hook_action(
                &repo_root,
                dispatch_context.as_ref(),
                AGENT_NAME_OPEN_CODE,
                hook_name,
                result.is_ok(),
                started.elapsed().as_millis(),
            );
            result.and_then(|outcome| emit_hook_stdout_if_present(&outcome))
        }
        HooksAgent::Git(_) => unreachable!(),
    }
}

fn read_stdin() -> Result<String> {
    let mut buf = String::new();
    io::stdin()
        .read_to_string(&mut buf)
        .context("reading stdin")?;
    Ok(buf)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) use tests::dispatch_cursor_hook;

#[cfg(test)]
#[path = "dispatcher_telemetry_tests.rs"]
mod telemetry_tests;

#[cfg(test)]
#[path = "dispatcher_logging_tests.rs"]
mod logging_tests;

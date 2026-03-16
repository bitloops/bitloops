//! `bitloops hooks ...` — shared dispatcher for agent and git hook commands.
use std::io::{self, Read};
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use anyhow::{Context, Result};
use clap::{Args, Subcommand};

use crate::engine::agent::codex::types::{CodexSessionInfoRaw, parse_codex_session_info};
use crate::engine::agent::cursor::types::{
    CursorAfterShellExecutionRaw, CursorBeforeShellExecutionRaw, CursorBeforeSubmitPromptRaw,
    CursorSessionInfoRaw,
};
use crate::engine::agent::{
    AGENT_NAME_CLAUDE_CODE, AGENT_NAME_CODEX, AGENT_NAME_CURSOR, AGENT_NAME_GEMINI,
};
use crate::engine::lifecycle::adapters::{
    CLAUDE_HOOK_POST_TASK, CLAUDE_HOOK_POST_TODO, CLAUDE_HOOK_PRE_TASK, GEMINI_HOOK_AFTER_TOOL,
    GEMINI_HOOK_BEFORE_TOOL, route_hook_command_to_lifecycle,
};
use crate::engine::logging;
use crate::engine::paths;
use crate::engine::session::backend::SessionBackend;
use crate::engine::session::create_session_backend_or_local;
use crate::engine::session::phase::SessionPhase;
use crate::engine::session::state::PRE_PROMPT_SOURCE_CURSOR_SHELL;
use crate::engine::settings;
use crate::engine::strategy::Strategy;
use crate::engine::strategy::manual_commit::ManualCommitStrategy;
use crate::engine::strategy::registry::{self, StrategyRegistry};

use super::git;
use crate::engine::agent::claude_code::hooks_cmd::{
    PostTaskInput, PostTodoInput, SessionInfoInput, TaskHookInput, UserPromptSubmitInput,
    handle_post_task, handle_post_todo, handle_pre_task, handle_session_end, handle_session_start,
    handle_stop, handle_user_prompt_submit_with_strategy,
};
use crate::engine::agent::codex::hooks_cmd::{handle_session_start_codex, handle_stop_codex};
use crate::engine::agent::cursor::hooks_cmd::{
    handle_before_submit_prompt_cursor, handle_session_end_cursor, handle_session_start_cursor,
    handle_stop_cursor,
};

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
    #[command(name = "gemini")]
    Gemini(GeminiHooksArgs),
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

impl ClaudeCodeHookVerb {
    pub fn hook_name(&self) -> &'static str {
        match self {
            Self::SessionStart => "session-start",
            Self::SessionEnd => "session-end",
            Self::Stop => "stop",
            Self::UserPromptSubmit => "user-prompt-submit",
            Self::PreTask => "pre-task",
            Self::PostTask => "post-task",
            Self::PostTodo => "post-todo",
        }
    }
}

impl CodexHookVerb {
    pub fn hook_name(&self) -> &'static str {
        match self {
            Self::SessionStart => crate::engine::agent::codex::lifecycle::HOOK_NAME_SESSION_START,
            Self::Stop => crate::engine::agent::codex::lifecycle::HOOK_NAME_STOP,
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
        (
            AGENT_NAME_CURSOR,
            crate::engine::agent::cursor::lifecycle::HOOK_NAME_SUBAGENT_START
            | crate::engine::agent::cursor::lifecycle::HOOK_NAME_SUBAGENT_STOP,
        ) => "subagent",
        (AGENT_NAME_GEMINI, GEMINI_HOOK_BEFORE_TOOL | GEMINI_HOOK_AFTER_TOOL) => "tool",
        _ => "agent",
    }
}

fn find_most_recent_session_id(repo_root: &Path) -> String {
    let backend = create_session_backend_or_local(repo_root);
    let sessions = backend.list_sessions().unwrap_or_default();
    crate::engine::session::state::find_most_recent_session(&sessions, &repo_root.to_string_lossy())
        .map(|s| s.session_id)
        .unwrap_or_default()
}

fn init_hook_logging(repo_root: &Path) {
    let session_id = find_most_recent_session_id(repo_root);
    let _ = logging::init(&session_id);
}

pub(crate) fn run_agent_hook_with_logging<F>(
    repo_root: &Path,
    agent_name: &str,
    hook_name: &str,
    strategy_name: &str,
    handler: F,
) -> Result<()>
where
    F: FnOnce() -> Result<()>,
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

pub async fn run(args: HooksArgs, strategy_registry: &StrategyRegistry) -> Result<()> {
    let agent = match args.agent {
        HooksAgent::Git(git_args) => return git::run(git_args, strategy_registry).await,
        other => other,
    };

    let repo_root = match paths::repo_root() {
        Ok(r) => r,
        Err(_) => return Ok(()),
    };

    if !settings::is_enabled(&repo_root).unwrap_or(true) {
        return Ok(());
    }

    let strategy_name = settings::load_settings(&repo_root)
        .map(|s| s.strategy)
        .unwrap_or_else(|_| registry::STRATEGY_NAME_MANUAL_COMMIT.to_string());

    match agent {
        HooksAgent::ClaudeCode(cc) => {
            let backend = create_session_backend_or_local(&repo_root);
            let strategy: Box<dyn Strategy> = strategy_registry
                .get(&strategy_name, &repo_root)
                .unwrap_or_else(|_| Box::new(ManualCommitStrategy::new(&repo_root)));
            let hook_name = cc.verb.hook_name();
            let stdin = read_stdin()?;

            run_agent_hook_with_logging(
                &repo_root,
                AGENT_NAME_CLAUDE_CODE,
                hook_name,
                &strategy_name,
                || match cc.verb {
                    ClaudeCodeHookVerb::SessionStart => {
                        let input: SessionInfoInput =
                            serde_json::from_str(&stdin).context("parsing session-start input")?;
                        handle_session_start(input, backend.as_ref(), Some(&repo_root))
                    }
                    ClaudeCodeHookVerb::UserPromptSubmit => {
                        let input: UserPromptSubmitInput = serde_json::from_str(&stdin)
                            .context("parsing user-prompt-submit input")?;
                        handle_user_prompt_submit_with_strategy(
                            input,
                            backend.as_ref(),
                            strategy.as_ref(),
                            Some(&repo_root),
                        )
                    }
                    ClaudeCodeHookVerb::Stop => {
                        let input: SessionInfoInput =
                            serde_json::from_str(&stdin).context("parsing stop input")?;
                        handle_stop(input, backend.as_ref(), strategy.as_ref(), Some(&repo_root))
                    }
                    ClaudeCodeHookVerb::SessionEnd => {
                        let input: SessionInfoInput =
                            serde_json::from_str(&stdin).context("parsing session-end input")?;
                        handle_session_end(input, backend.as_ref())
                    }
                    ClaudeCodeHookVerb::PreTask => {
                        let input: TaskHookInput =
                            serde_json::from_str(&stdin).context("parsing pre-task input")?;
                        handle_pre_task(input, backend.as_ref(), Some(&repo_root))
                    }
                    ClaudeCodeHookVerb::PostTask => {
                        let input: PostTaskInput =
                            serde_json::from_str(&stdin).context("parsing post-task input")?;
                        handle_post_task(
                            input,
                            backend.as_ref(),
                            strategy.as_ref(),
                            Some(&repo_root),
                        )
                    }
                    ClaudeCodeHookVerb::PostTodo => {
                        let input: PostTodoInput =
                            serde_json::from_str(&stdin).context("parsing post-todo input")?;
                        handle_post_todo(
                            input,
                            backend.as_ref(),
                            strategy.as_ref(),
                            Some(&repo_root),
                        )
                    }
                },
            )
        }
        HooksAgent::Codex(codex) => {
            let backend = create_session_backend_or_local(&repo_root);
            let strategy: Box<dyn Strategy> = strategy_registry
                .get(&strategy_name, &repo_root)
                .unwrap_or_else(|_| Box::new(ManualCommitStrategy::new(&repo_root)));
            let hook_name = codex.verb.hook_name();
            let stdin = read_stdin()?;

            run_agent_hook_with_logging(
                &repo_root,
                AGENT_NAME_CODEX,
                hook_name,
                &strategy_name,
                || {
                    let raw: CodexSessionInfoRaw = parse_codex_session_info(&stdin)?;
                    let session_policy = match codex.verb {
                        CodexHookVerb::SessionStart => {
                            crate::engine::lifecycle::SessionIdPolicy::Strict
                        }
                        CodexHookVerb::Stop => {
                            crate::engine::lifecycle::SessionIdPolicy::PreserveEmpty
                        }
                    };
                    let input = SessionInfoInput {
                        session_id: crate::engine::lifecycle::apply_session_id_policy(
                            &raw.session_id,
                            session_policy,
                        )
                        .context("validating codex session_id")?,
                        transcript_path: raw.transcript_path,
                    };
                    match codex.verb {
                        CodexHookVerb::SessionStart => {
                            handle_session_start_codex(input, backend.as_ref(), Some(&repo_root))
                        }
                        CodexHookVerb::Stop => handle_stop_codex(
                            input,
                            backend.as_ref(),
                            strategy.as_ref(),
                            Some(&repo_root),
                        ),
                    }
                },
            )
        }
        HooksAgent::Gemini(gemini) => {
            let hook_name = gemini.verb.hook_name();
            let stdin = read_stdin()?;
            run_agent_hook_with_logging(
                &repo_root,
                AGENT_NAME_GEMINI,
                hook_name,
                &strategy_name,
                || route_hook_command_to_lifecycle(AGENT_NAME_GEMINI, hook_name, &stdin),
            )
        }
        HooksAgent::Cursor(cursor) => {
            let hook_name = cursor.verb.hook_name();
            let stdin = read_stdin()?;
            let backend = create_session_backend_or_local(&repo_root);
            let strategy: Box<dyn Strategy> = strategy_registry
                .get(&strategy_name, &repo_root)
                .unwrap_or_else(|_| Box::new(ManualCommitStrategy::new(&repo_root)));
            run_agent_hook_with_logging(
                &repo_root,
                AGENT_NAME_CURSOR,
                hook_name,
                &strategy_name,
                || {
                    dispatch_cursor_hook(
                        &cursor.verb,
                        &stdin,
                        backend.as_ref(),
                        strategy.as_ref(),
                        &repo_root,
                        hook_name,
                    )
                },
            )
        }
        HooksAgent::Git(_) => unreachable!(),
    }
}

pub(crate) fn dispatch_cursor_hook(
    verb: &CursorHookVerb,
    stdin: &str,
    backend: &dyn SessionBackend,
    strategy: &dyn Strategy,
    repo_root: &Path,
    hook_name: &str,
) -> Result<()> {
    match verb {
        CursorHookVerb::SessionStart => {
            let raw: CursorSessionInfoRaw =
                serde_json::from_str(stdin).context("parsing session-start input")?;
            let session_id = crate::engine::lifecycle::apply_session_id_policy(
                &raw.conversation_id,
                crate::engine::lifecycle::SessionIdPolicy::Strict,
            )?;
            let input = SessionInfoInput {
                session_id: session_id.clone(),
                transcript_path: crate::engine::agent::cursor::lifecycle::resolve_transcript_ref(
                    &session_id,
                    raw.transcript_path.as_deref(),
                ),
            };
            handle_session_start_cursor(input, backend, Some(repo_root))
        }
        CursorHookVerb::BeforeSubmitPrompt => {
            let raw: CursorBeforeSubmitPromptRaw =
                serde_json::from_str(stdin).context("parsing before-submit-prompt input")?;
            let session_id = crate::engine::lifecycle::apply_session_id_policy(
                &raw.conversation_id,
                crate::engine::lifecycle::SessionIdPolicy::Strict,
            )?;
            let input = UserPromptSubmitInput {
                session_id: session_id.clone(),
                transcript_path: crate::engine::agent::cursor::lifecycle::resolve_transcript_ref(
                    &session_id,
                    raw.transcript_path.as_deref(),
                ),
                prompt: raw.prompt,
            };
            handle_before_submit_prompt_cursor(input, backend, strategy, Some(repo_root))
        }
        CursorHookVerb::BeforeShellExecution => {
            let raw: CursorBeforeShellExecutionRaw =
                serde_json::from_str(stdin).context("parsing before-shell-execution input")?;
            let session_id = crate::engine::lifecycle::apply_session_id_policy(
                &raw.conversation_id,
                crate::engine::lifecycle::SessionIdPolicy::Strict,
            )?;

            // Fallback-only behavior: if turn-start already captured pre-prompt state,
            // skip shell fallback to avoid duplicate turn boundaries.
            if backend.load_pre_prompt(&session_id)?.is_some() {
                return Ok(());
            }

            let input = UserPromptSubmitInput {
                session_id: session_id.clone(),
                transcript_path: crate::engine::agent::cursor::lifecycle::resolve_transcript_ref(
                    &session_id,
                    raw.transcript_path.as_deref(),
                ),
                prompt: shell_command_to_prompt(&raw.command),
            };
            handle_before_submit_prompt_cursor(input, backend, strategy, Some(repo_root))?;

            if let Some(mut pre_prompt) = backend.load_pre_prompt(&session_id)? {
                pre_prompt.source = PRE_PROMPT_SOURCE_CURSOR_SHELL.to_string();
                backend.save_pre_prompt(&pre_prompt)?;
            }
            Ok(())
        }
        CursorHookVerb::AfterShellExecution => {
            let raw: CursorAfterShellExecutionRaw =
                serde_json::from_str(stdin).context("parsing after-shell-execution input")?;
            let session_id = crate::engine::lifecycle::apply_session_id_policy(
                &raw.conversation_id,
                crate::engine::lifecycle::SessionIdPolicy::PreserveEmpty,
            )?;

            // Only complete turns that were started by shell fallback.
            let Some(pre_prompt) = backend.load_pre_prompt(&session_id)? else {
                return Ok(());
            };
            if pre_prompt.source != PRE_PROMPT_SOURCE_CURSOR_SHELL {
                return Ok(());
            }

            let input = SessionInfoInput {
                session_id: session_id.clone(),
                transcript_path: crate::engine::agent::cursor::lifecycle::resolve_transcript_ref(
                    &session_id,
                    raw.transcript_path.as_deref(),
                ),
            };
            handle_stop_cursor(input, backend, strategy, Some(repo_root))
        }
        CursorHookVerb::Stop => {
            let raw: CursorSessionInfoRaw =
                serde_json::from_str(stdin).context("parsing stop input")?;
            let session_id = crate::engine::lifecycle::apply_session_id_policy(
                &raw.conversation_id,
                crate::engine::lifecycle::SessionIdPolicy::PreserveEmpty,
            )?;
            let input = SessionInfoInput {
                session_id: session_id.clone(),
                transcript_path: crate::engine::agent::cursor::lifecycle::resolve_transcript_ref(
                    &session_id,
                    raw.transcript_path.as_deref(),
                ),
            };
            handle_stop_cursor(input, backend, strategy, Some(repo_root))
        }
        CursorHookVerb::SessionEnd => {
            let raw: CursorSessionInfoRaw =
                serde_json::from_str(stdin).context("parsing session-end input")?;
            let session_id = crate::engine::lifecycle::apply_session_id_policy(
                &raw.conversation_id,
                crate::engine::lifecycle::SessionIdPolicy::PreserveEmpty,
            )?;
            let transcript_path = crate::engine::agent::cursor::lifecycle::resolve_transcript_ref(
                &session_id,
                raw.transcript_path.as_deref(),
            );

            let pre_prompt = backend.load_pre_prompt(&session_id)?;
            let session = backend.load_session(&session_id)?;
            let should_finalize_turn = !session_id.is_empty()
                && (pre_prompt.is_some()
                    || session.is_none()
                    || session.as_ref().is_some_and(|state| {
                        state.phase == SessionPhase::Active
                            || (state.phase == SessionPhase::Idle && state.step_count == 0)
                    }));

            if should_finalize_turn {
                handle_stop_cursor(
                    SessionInfoInput {
                        session_id: session_id.clone(),
                        transcript_path: transcript_path.clone(),
                    },
                    backend,
                    strategy,
                    Some(repo_root),
                )?;
            }

            let input = SessionInfoInput {
                session_id,
                transcript_path,
            };
            handle_session_end_cursor(input, backend)
        }
        CursorHookVerb::PreCompact
        | CursorHookVerb::SubagentStart
        | CursorHookVerb::SubagentStop => {
            route_hook_command_to_lifecycle(AGENT_NAME_CURSOR, hook_name, stdin)
        }
    }
}

fn shell_command_to_prompt(command: &str) -> String {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        "Run shell command".to_string()
    } else {
        format!("Run shell command: {trimmed}")
    }
}

fn read_stdin() -> Result<String> {
    let mut buf = String::new();
    io::stdin()
        .read_to_string(&mut buf)
        .context("reading stdin")?;
    Ok(buf)
}

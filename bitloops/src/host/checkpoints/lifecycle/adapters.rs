use anyhow::{Result, anyhow};
use serde::Deserialize;
use serde_json::Value;

use crate::adapters::agents::AgentAdapterRegistry;
use crate::adapters::agents::copilot::agent::CopilotCliAgent;
use crate::adapters::agents::gemini::agent::GeminiCliAgent;
use crate::adapters::agents::{TokenCalculator, TranscriptAnalyzer};

use super::{
    LifecycleAgentAdapter, LifecycleEvent, LifecycleEventType, dispatch_lifecycle_event,
    read_and_parse_hook_input,
};

pub const CLAUDE_HOOK_SESSION_START: &str = "session-start";
pub const CLAUDE_HOOK_SESSION_END: &str = "session-end";
pub const CLAUDE_HOOK_USER_PROMPT_SUBMIT: &str = "user-prompt-submit";
pub const CLAUDE_HOOK_STOP: &str = "stop";
pub const CLAUDE_HOOK_PRE_TASK: &str = "pre-task";
pub const CLAUDE_HOOK_POST_TASK: &str = "post-task";
pub const CLAUDE_HOOK_POST_TODO: &str = "post-todo";

pub const GEMINI_HOOK_SESSION_START: &str = "session-start";
pub const GEMINI_HOOK_SESSION_END: &str = "session-end";
pub const GEMINI_HOOK_BEFORE_AGENT: &str = "before-agent";
pub const GEMINI_HOOK_AFTER_AGENT: &str = "after-agent";
pub const GEMINI_HOOK_PRE_COMPRESS: &str = "pre-compress";
pub const GEMINI_HOOK_BEFORE_TOOL: &str = "before-tool";
pub const GEMINI_HOOK_AFTER_TOOL: &str = "after-tool";
pub const GEMINI_HOOK_BEFORE_MODEL: &str = "before-model";
pub const GEMINI_HOOK_AFTER_MODEL: &str = "after-model";
pub const GEMINI_HOOK_BEFORE_TOOL_SELECTION: &str = "before-tool-selection";
pub const GEMINI_HOOK_NOTIFICATION: &str = "notification";

pub const OPENCODE_HOOK_SESSION_START: &str = "session-start";
pub const OPENCODE_HOOK_TURN_START: &str = "turn-start";
pub const OPENCODE_HOOK_TURN_END: &str = "turn-end";
pub const OPENCODE_HOOK_COMPACTION: &str = "compaction";
pub const OPENCODE_HOOK_SESSION_END: &str = "session-end";

pub const CURSOR_HOOK_SESSION_START: &str = "session-start";
pub const CURSOR_HOOK_BEFORE_SUBMIT_PROMPT: &str = "before-submit-prompt";
pub const CURSOR_HOOK_STOP: &str = "stop";
pub const CURSOR_HOOK_SESSION_END: &str = "session-end";
pub const CURSOR_HOOK_PRE_COMPACT: &str = "pre-compact";
pub const CURSOR_HOOK_SUBAGENT_START: &str = "subagent-start";
pub const CURSOR_HOOK_SUBAGENT_STOP: &str = "subagent-stop";

pub const COPILOT_HOOK_USER_PROMPT_SUBMITTED: &str = "user-prompt-submitted";
pub const COPILOT_HOOK_SESSION_START: &str = "session-start";
pub const COPILOT_HOOK_AGENT_STOP: &str = "agent-stop";
pub const COPILOT_HOOK_SESSION_END: &str = "session-end";
pub const COPILOT_HOOK_SUBAGENT_STOP: &str = "subagent-stop";
pub const COPILOT_HOOK_PRE_TOOL_USE: &str = "pre-tool-use";
pub const COPILOT_HOOK_POST_TOOL_USE: &str = "post-tool-use";
pub const COPILOT_HOOK_ERROR_OCCURRED: &str = "error-occurred";

pub const CODEX_HOOK_SESSION_START: &str = "session-start";
pub const CODEX_HOOK_STOP: &str = "stop";

#[derive(Default)]
pub struct ClaudeCodeLifecycleAdapter;

impl LifecycleAgentAdapter for ClaudeCodeLifecycleAdapter {
    fn agent_name(&self) -> &'static str {
        crate::adapters::agents::AGENT_NAME_CLAUDE_CODE
    }

    fn parse_hook_event(
        &self,
        hook_name: &str,
        stdin: &mut dyn std::io::Read,
    ) -> Result<Option<LifecycleEvent>> {
        match hook_name {
            CLAUDE_HOOK_SESSION_START => {
                let raw: SessionInfoRaw = read_and_parse_hook_input(stdin)?;
                Ok(Some(LifecycleEvent {
                    event_type: Some(LifecycleEventType::SessionStart),
                    session_id: raw.session_id,
                    session_ref: raw.transcript_path,
                    ..LifecycleEvent::default()
                }))
            }
            CLAUDE_HOOK_USER_PROMPT_SUBMIT => {
                let raw: TurnStartRaw = read_and_parse_hook_input(stdin)?;
                Ok(Some(LifecycleEvent {
                    event_type: Some(LifecycleEventType::TurnStart),
                    session_id: raw.session_id,
                    session_ref: raw.transcript_path,
                    prompt: raw.prompt,
                    ..LifecycleEvent::default()
                }))
            }
            CLAUDE_HOOK_STOP => {
                let raw: SessionInfoRaw = read_and_parse_hook_input(stdin)?;
                Ok(Some(LifecycleEvent {
                    event_type: Some(LifecycleEventType::TurnEnd),
                    session_id: raw.session_id,
                    session_ref: raw.transcript_path,
                    ..LifecycleEvent::default()
                }))
            }
            CLAUDE_HOOK_SESSION_END => {
                let raw: SessionInfoRaw = read_and_parse_hook_input(stdin)?;
                Ok(Some(LifecycleEvent {
                    event_type: Some(LifecycleEventType::SessionEnd),
                    session_id: raw.session_id,
                    session_ref: raw.transcript_path,
                    ..LifecycleEvent::default()
                }))
            }
            CLAUDE_HOOK_PRE_TASK => {
                let raw: TaskHookInputRaw = read_and_parse_hook_input(stdin)?;
                Ok(Some(LifecycleEvent {
                    event_type: Some(LifecycleEventType::SubagentStart),
                    session_id: raw.session_id,
                    session_ref: raw.transcript_path,
                    tool_use_id: raw.tool_use_id,
                    ..LifecycleEvent::default()
                }))
            }
            CLAUDE_HOOK_POST_TASK => {
                let raw: PostTaskHookInputRaw = read_and_parse_hook_input(stdin)?;
                Ok(Some(LifecycleEvent {
                    event_type: Some(LifecycleEventType::SubagentEnd),
                    session_id: raw.session_id,
                    session_ref: raw.transcript_path,
                    tool_use_id: raw.tool_use_id,
                    subagent_id: raw.tool_response.agent_id,
                    ..LifecycleEvent::default()
                }))
            }
            CLAUDE_HOOK_POST_TODO => Ok(None),
            _ => Ok(None),
        }
    }

    fn hook_names(&self) -> Vec<&'static str> {
        vec![
            CLAUDE_HOOK_SESSION_START,
            CLAUDE_HOOK_SESSION_END,
            CLAUDE_HOOK_USER_PROMPT_SUBMIT,
            CLAUDE_HOOK_STOP,
            CLAUDE_HOOK_PRE_TASK,
            CLAUDE_HOOK_POST_TASK,
            CLAUDE_HOOK_POST_TODO,
        ]
    }

    fn format_resume_command(&self, _session_id: &str) -> String {
        String::from("claude")
    }
}

static GEMINI_AGENT_FOR_LIFECYCLE: GeminiCliAgent = GeminiCliAgent;

#[derive(Default)]
pub struct GeminiCliLifecycleAdapter;

impl LifecycleAgentAdapter for GeminiCliLifecycleAdapter {
    fn agent_name(&self) -> &'static str {
        crate::adapters::agents::AGENT_NAME_GEMINI
    }

    fn parse_hook_event(
        &self,
        hook_name: &str,
        stdin: &mut dyn std::io::Read,
    ) -> Result<Option<LifecycleEvent>> {
        crate::adapters::agents::gemini::lifecycle::parse_hook_event(hook_name, stdin)
    }

    fn hook_names(&self) -> Vec<&'static str> {
        vec![
            GEMINI_HOOK_SESSION_START,
            GEMINI_HOOK_SESSION_END,
            GEMINI_HOOK_BEFORE_AGENT,
            GEMINI_HOOK_AFTER_AGENT,
            GEMINI_HOOK_PRE_COMPRESS,
            GEMINI_HOOK_BEFORE_TOOL,
            GEMINI_HOOK_AFTER_TOOL,
            GEMINI_HOOK_BEFORE_MODEL,
            GEMINI_HOOK_AFTER_MODEL,
            GEMINI_HOOK_BEFORE_TOOL_SELECTION,
            GEMINI_HOOK_NOTIFICATION,
        ]
    }

    fn format_resume_command(&self, _session_id: &str) -> String {
        String::from("gemini")
    }

    fn as_transcript_analyzer(&self) -> Option<&dyn TranscriptAnalyzer> {
        Some(&GEMINI_AGENT_FOR_LIFECYCLE)
    }

    fn as_token_calculator(&self) -> Option<&dyn TokenCalculator> {
        Some(&GEMINI_AGENT_FOR_LIFECYCLE)
    }
}

#[derive(Default)]
pub struct OpenCodeLifecycleAdapter;

impl LifecycleAgentAdapter for OpenCodeLifecycleAdapter {
    fn agent_name(&self) -> &'static str {
        crate::adapters::agents::AGENT_NAME_OPEN_CODE
    }

    fn parse_hook_event(
        &self,
        hook_name: &str,
        stdin: &mut dyn std::io::Read,
    ) -> Result<Option<LifecycleEvent>> {
        match hook_name {
            OPENCODE_HOOK_SESSION_START => {
                let raw: SessionInfoRaw = read_and_parse_hook_input(stdin)?;
                Ok(Some(LifecycleEvent {
                    event_type: Some(LifecycleEventType::SessionStart),
                    session_id: raw.session_id,
                    session_ref: raw.transcript_path,
                    ..LifecycleEvent::default()
                }))
            }
            OPENCODE_HOOK_TURN_START => {
                let raw: TurnStartRaw = read_and_parse_hook_input(stdin)?;
                Ok(Some(LifecycleEvent {
                    event_type: Some(LifecycleEventType::TurnStart),
                    session_id: raw.session_id,
                    session_ref: raw.transcript_path,
                    prompt: raw.prompt,
                    ..LifecycleEvent::default()
                }))
            }
            OPENCODE_HOOK_TURN_END => {
                let raw: SessionInfoRaw = read_and_parse_hook_input(stdin)?;
                Ok(Some(LifecycleEvent {
                    event_type: Some(LifecycleEventType::TurnEnd),
                    session_id: raw.session_id,
                    session_ref: raw.transcript_path,
                    ..LifecycleEvent::default()
                }))
            }
            OPENCODE_HOOK_COMPACTION => {
                let raw: SessionInfoRaw = read_and_parse_hook_input(stdin)?;
                Ok(Some(LifecycleEvent {
                    event_type: Some(LifecycleEventType::Compaction),
                    session_id: raw.session_id,
                    session_ref: raw.transcript_path,
                    ..LifecycleEvent::default()
                }))
            }
            OPENCODE_HOOK_SESSION_END => {
                let raw: SessionInfoRaw = read_and_parse_hook_input(stdin)?;
                Ok(Some(LifecycleEvent {
                    event_type: Some(LifecycleEventType::SessionEnd),
                    session_id: raw.session_id,
                    session_ref: raw.transcript_path,
                    ..LifecycleEvent::default()
                }))
            }
            _ => Ok(None),
        }
    }

    fn hook_names(&self) -> Vec<&'static str> {
        vec![
            OPENCODE_HOOK_SESSION_START,
            OPENCODE_HOOK_SESSION_END,
            OPENCODE_HOOK_TURN_START,
            OPENCODE_HOOK_TURN_END,
            OPENCODE_HOOK_COMPACTION,
        ]
    }

    fn format_resume_command(&self, session_id: &str) -> String {
        if session_id.is_empty() {
            String::from("opencode")
        } else {
            format!("opencode -s {session_id}")
        }
    }
}

static COPILOT_AGENT_FOR_LIFECYCLE: CopilotCliAgent = CopilotCliAgent;

#[derive(Default)]
pub struct CopilotCliLifecycleAdapter;

impl LifecycleAgentAdapter for CopilotCliLifecycleAdapter {
    fn agent_name(&self) -> &'static str {
        crate::adapters::agents::AGENT_NAME_COPILOT
    }

    fn parse_hook_event(
        &self,
        hook_name: &str,
        stdin: &mut dyn std::io::Read,
    ) -> Result<Option<LifecycleEvent>> {
        crate::adapters::agents::copilot::lifecycle::parse_hook_event(hook_name, stdin)
    }

    fn hook_names(&self) -> Vec<&'static str> {
        vec![
            COPILOT_HOOK_USER_PROMPT_SUBMITTED,
            COPILOT_HOOK_SESSION_START,
            COPILOT_HOOK_AGENT_STOP,
            COPILOT_HOOK_SESSION_END,
            COPILOT_HOOK_SUBAGENT_STOP,
            COPILOT_HOOK_PRE_TOOL_USE,
            COPILOT_HOOK_POST_TOOL_USE,
            COPILOT_HOOK_ERROR_OCCURRED,
        ]
    }

    fn format_resume_command(&self, session_id: &str) -> String {
        format!("copilot --resume {session_id}")
    }

    fn as_transcript_analyzer(&self) -> Option<&dyn TranscriptAnalyzer> {
        Some(&COPILOT_AGENT_FOR_LIFECYCLE)
    }

    fn as_token_calculator(&self) -> Option<&dyn TokenCalculator> {
        Some(&COPILOT_AGENT_FOR_LIFECYCLE)
    }
}

#[derive(Default)]
pub struct CursorLifecycleAdapter;

impl LifecycleAgentAdapter for CursorLifecycleAdapter {
    fn agent_name(&self) -> &'static str {
        crate::adapters::agents::AGENT_NAME_CURSOR
    }

    fn parse_hook_event(
        &self,
        hook_name: &str,
        stdin: &mut dyn std::io::Read,
    ) -> Result<Option<LifecycleEvent>> {
        crate::adapters::agents::cursor::lifecycle::parse_hook_event(hook_name, stdin)
    }

    fn hook_names(&self) -> Vec<&'static str> {
        vec![
            CURSOR_HOOK_SESSION_START,
            CURSOR_HOOK_BEFORE_SUBMIT_PROMPT,
            CURSOR_HOOK_STOP,
            CURSOR_HOOK_SESSION_END,
            CURSOR_HOOK_PRE_COMPACT,
            CURSOR_HOOK_SUBAGENT_START,
            CURSOR_HOOK_SUBAGENT_STOP,
        ]
    }

    fn format_resume_command(&self, _session_id: &str) -> String {
        String::from("Open this project in Cursor to continue the session.")
    }
}

#[derive(Default)]
pub struct CodexLifecycleAdapter;

impl LifecycleAgentAdapter for CodexLifecycleAdapter {
    fn agent_name(&self) -> &'static str {
        crate::adapters::agents::AGENT_NAME_CODEX
    }

    fn parse_hook_event(
        &self,
        hook_name: &str,
        stdin: &mut dyn std::io::Read,
    ) -> Result<Option<LifecycleEvent>> {
        crate::adapters::agents::codex::lifecycle::parse_hook_event(hook_name, stdin)
    }

    fn hook_names(&self) -> Vec<&'static str> {
        vec![CODEX_HOOK_SESSION_START, CODEX_HOOK_STOP]
    }

    fn format_resume_command(&self, session_id: &str) -> String {
        if session_id.trim().is_empty() {
            "codex".to_string()
        } else {
            format!("codex --resume {session_id}")
        }
    }
}

pub fn route_hook_command_to_lifecycle(
    agent_name: &str,
    hook_name: &str,
    stdin: &str,
) -> Result<()> {
    let resolved = AgentAdapterRegistry::builtin().resolve_with_trace(agent_name, None)?;
    let descriptor = resolved.registration.descriptor();
    let family = descriptor.protocol_family.id;
    let profile = descriptor.target_profile.id;
    let correlation_id = resolved.trace.correlation_id;

    let adapter: Box<dyn LifecycleAgentAdapter> = match (family, profile) {
        ("jsonl-cli", crate::adapters::agents::AGENT_NAME_CLAUDE_CODE) => {
            Box::new(ClaudeCodeLifecycleAdapter)
        }
        ("json-event", crate::adapters::agents::AGENT_NAME_COPILOT) => {
            Box::new(CopilotCliLifecycleAdapter)
        }
        ("jsonl-cli", crate::adapters::agents::AGENT_NAME_CODEX) => Box::new(CodexLifecycleAdapter),
        ("jsonl-cli", crate::adapters::agents::AGENT_NAME_CURSOR) => {
            Box::new(CursorLifecycleAdapter)
        }
        ("json-event", crate::adapters::agents::AGENT_TYPE_GEMINI) => {
            Box::new(GeminiCliLifecycleAdapter)
        }
        ("jsonl-cli", crate::adapters::agents::AGENT_NAME_OPEN_CODE) => {
            Box::new(OpenCodeLifecycleAdapter)
        }
        _ => return Err(anyhow!("unsupported lifecycle agent: {agent_name}")),
    };

    let mut input = std::io::Cursor::new(stdin.as_bytes());
    let event = adapter.parse_hook_event(hook_name, &mut input).map_err(|err| {
        anyhow!(
            "failed to parse lifecycle hook '{hook_name}' for family '{family}' profile '{profile}' (correlation_id={correlation_id}): {err}"
        )
    })?;
    if let Some(event) = event {
        dispatch_lifecycle_event(Some(adapter.as_ref()), Some(&event)).map_err(|err| {
            anyhow!(
                "failed to dispatch lifecycle event for family '{family}' profile '{profile}' (correlation_id={correlation_id}): {err}"
            )
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod route_tests {
    use super::*;
    use crate::adapters::agents::AGENT_NAME_CODEX;
    use crate::host::interactions::db_store::interaction_spool_db_path;
    use crate::test_support::process_state::{git_command, with_process_state};
    use anyhow::Result;
    use tempfile::TempDir;

    fn git_ok(repo_root: &std::path::Path, args: &[&str]) {
        let output = git_command()
            .args(args)
            .current_dir(repo_root)
            .output()
            .unwrap_or_else(|err| panic!("failed to start git {:?}: {err}", args));
        assert!(
            output.status.success(),
            "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn seed_repo() -> TempDir {
        let dir = TempDir::new().expect("temp dir");
        let root = dir.path();
        git_ok(root, &["init"]);
        git_ok(root, &["checkout", "-B", "main"]);
        git_ok(root, &["config", "user.name", "Bitloops Test"]);
        git_ok(root, &["config", "user.email", "bitloops-test@example.com"]);
        std::fs::write(root.join("tracked.txt"), "one\n").expect("write tracked file");
        git_ok(root, &["add", "tracked.txt"]);
        git_ok(root, &["commit", "-m", "initial"]);
        dir
    }

    #[test]
    fn route_codex_hooks_persist_interactions_to_event_db_even_if_checkpoint_save_fails(
    ) -> Result<()> {
        let repo = seed_repo();
        let transcript_path = repo.path().join("codex-transcript.json");
        let transcript_path_str = transcript_path.to_string_lossy().to_string();
        std::fs::write(
            &transcript_path,
            r#"{"messages":[{"type":"user","content":"Refactor tracked file"},{"type":"gemini","content":"Updated tracked file"}]}"#,
        )
        .expect("write transcript");
        std::fs::write(repo.path().join("tracked.txt"), "two\n").expect("modify tracked file");

        with_process_state(Some(repo.path()), &[], || -> Result<()> {
            let session_payload = serde_json::json!({
                "session_id": "codex-session-1",
                "transcript_path": transcript_path_str.clone(),
            })
            .to_string();
            route_hook_command_to_lifecycle(
                AGENT_NAME_CODEX,
                CODEX_HOOK_SESSION_START,
                &session_payload,
            )?;

            let stop_payload = serde_json::json!({
                "sessionId": "codex-session-1",
                "transcriptPath": transcript_path_str,
            })
            .to_string();
            let err = route_hook_command_to_lifecycle(AGENT_NAME_CODEX, CODEX_HOOK_STOP, &stop_payload)
                .expect_err("stop should still fail when the relational checkpoint store is absent");
            assert!(
                err.to_string().contains("opening temporary checkpoint SQLite database"),
                "unexpected stop error: {err:#}"
            );
            Ok(())
        })?;

        let backends = crate::config::resolve_store_backend_config_for_repo(repo.path())?;
        let event_db_path = backends.events.resolve_duckdb_db_path_for_repo(repo.path());
        assert!(
            event_db_path.is_file(),
            "expected events DuckDB at {}",
            event_db_path.display()
        );

        let duckdb = duckdb::Connection::open(&event_db_path).expect("open events duckdb");
        let session_count: i64 = duckdb
            .query_row("SELECT COUNT(*) FROM interaction_sessions", [], |row| row.get(0))
            .expect("count interaction sessions");
        let turn_count: i64 = duckdb
            .query_row("SELECT COUNT(*) FROM interaction_turns", [], |row| row.get(0))
            .expect("count interaction turns");
        let event_count: i64 = duckdb
            .query_row("SELECT COUNT(*) FROM interaction_events", [], |row| row.get(0))
            .expect("count interaction events");
        assert_eq!(session_count, 1);
        assert_eq!(turn_count, 1);
        assert_eq!(event_count, 2);

        let spool = rusqlite::Connection::open(interaction_spool_db_path(repo.path()))
            .expect("open interaction spool");
        let local_session_count: i64 = spool
            .query_row("SELECT COUNT(*) FROM interaction_sessions", [], |row| row.get(0))
            .expect("count local interaction sessions");
        let local_turn_count: i64 = spool
            .query_row("SELECT COUNT(*) FROM interaction_turns", [], |row| row.get(0))
            .expect("count local interaction turns");
        let local_event_count: i64 = spool
            .query_row("SELECT COUNT(*) FROM interaction_events", [], |row| row.get(0))
            .expect("count local interaction events");
        let queued_mutations: i64 = spool
            .query_row("SELECT COUNT(*) FROM interaction_spool_queue", [], |row| row.get(0))
            .expect("count queued interaction mutations");
        assert_eq!(local_session_count, 1);
        assert_eq!(local_turn_count, 1);
        assert_eq!(local_event_count, 2);
        assert_eq!(queued_mutations, 0);

        Ok(())
    }
}

#[derive(Debug, Deserialize, Default)]
struct SessionInfoRaw {
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    transcript_path: String,
}

#[derive(Debug, Deserialize, Default)]
struct TurnStartRaw {
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    transcript_path: String,
    #[serde(default)]
    prompt: String,
}

#[derive(Debug, Deserialize, Default)]
struct TaskHookInputRaw {
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    transcript_path: String,
    #[serde(default)]
    tool_use_id: String,
    #[serde(default, rename = "tool_input")]
    _tool_input: Value,
}

#[derive(Debug, Deserialize, Default)]
struct PostTaskHookInputRaw {
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    transcript_path: String,
    #[serde(default)]
    tool_use_id: String,
    #[serde(default, rename = "tool_input")]
    _tool_input: Value,
    #[serde(default)]
    tool_response: PostTaskResponseRaw,
}

#[derive(Debug, Deserialize, Default)]
struct PostTaskResponseRaw {
    #[serde(default, rename = "agentId")]
    agent_id: String,
}

use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::SystemTime;

use anyhow::{Result, anyhow};
use regex::Regex;
use serde::Deserialize;
use uuid::Uuid;

use crate::engine::agent::{
    AGENT_NAME_OPEN_CODE, AGENT_TYPE_OPEN_CODE, Agent, AgentSession, Event, HookInput, HookSupport,
    HookType, TokenUsage, chunk_jsonl, reassemble_jsonl,
};
use crate::engine::lifecycle::read_and_parse_hook_input;

use super::cli_commands::{run_opencode_import, run_opencode_session_delete};
use super::hooks::{BITLOOPS_MARKER, get_plugin_path, render_plugin_template};
use super::transcript::{
    calculate_token_usage_from_bytes, extract_modified_files, parse_messages_from_file,
};
use super::types::{FILE_MODIFICATION_TOOLS, ROLE_ASSISTANT, ROLE_USER};

pub const HOOK_NAME_SESSION_START: &str = "session-start";
pub const HOOK_NAME_SESSION_END: &str = "session-end";
pub const HOOK_NAME_TURN_START: &str = "turn-start";
pub const HOOK_NAME_TURN_END: &str = "turn-end";
pub const HOOK_NAME_COMPACTION: &str = "compaction";

#[derive(Debug, Default, Deserialize)]
struct SessionInfoRaw {
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    transcript_path: String,
}

#[derive(Debug, Default, Deserialize)]
struct TurnStartRaw {
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    transcript_path: String,
    #[serde(default)]
    prompt: String,
}

struct TempFileCleanup(PathBuf);

impl Drop for TempFileCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct OpenCodeAgent;

pub fn new_open_code_agent() -> Box<dyn Agent + Send + Sync> {
    Box::new(OpenCodeAgent)
}

impl Agent for OpenCodeAgent {
    fn name(&self) -> String {
        AGENT_NAME_OPEN_CODE.to_string()
    }

    fn agent_type(&self) -> String {
        AGENT_TYPE_OPEN_CODE.to_string()
    }

    fn description(&self) -> String {
        "OpenCode - AI-powered terminal coding agent".to_string()
    }

    fn is_preview(&self) -> bool {
        true
    }

    fn detect_presence(&self) -> Result<bool> {
        let repo_root = crate::utils::paths::repo_root().unwrap_or_else(|_| PathBuf::from("."));
        Ok(repo_root.join(".opencode").is_dir() || repo_root.join("opencode.json").is_file())
    }

    fn get_session_id(&self, input: &HookInput) -> String {
        input.session_id.clone()
    }

    fn protected_dirs(&self) -> Vec<String> {
        vec![".opencode".to_string()]
    }

    fn hook_names(&self) -> Vec<String> {
        vec![
            HOOK_NAME_SESSION_START.to_string(),
            HOOK_NAME_SESSION_END.to_string(),
            HOOK_NAME_TURN_START.to_string(),
            HOOK_NAME_TURN_END.to_string(),
            HOOK_NAME_COMPACTION.to_string(),
        ]
    }

    fn parse_hook_event(&self, hook_name: &str, stdin: &mut dyn Read) -> Result<Option<Event>> {
        match hook_name {
            HOOK_NAME_SESSION_START => {
                let raw: SessionInfoRaw = read_and_parse_hook_input(stdin)?;
                let _ = raw.session_id;
                let _ = raw.transcript_path;
                Ok(Some(Event))
            }
            HOOK_NAME_TURN_START => {
                let raw: TurnStartRaw = read_and_parse_hook_input(stdin)?;
                let _ = raw.session_id;
                let _ = raw.transcript_path;
                let _ = raw.prompt;
                Ok(Some(Event))
            }
            HOOK_NAME_TURN_END | HOOK_NAME_COMPACTION | HOOK_NAME_SESSION_END => {
                let raw: SessionInfoRaw = read_and_parse_hook_input(stdin)?;
                let _ = raw.session_id;
                let _ = raw.transcript_path;
                Ok(Some(Event))
            }
            _ => Ok(None),
        }
    }

    fn read_transcript(&self, session_ref: &str) -> Result<Vec<u8>> {
        fs::read(session_ref).map_err(|err| anyhow!("failed to read opencode transcript: {err}"))
    }

    fn chunk_transcript(&self, content: &[u8], max_size: usize) -> Result<Vec<Vec<u8>>> {
        chunk_jsonl(content, max_size)
            .map_err(|err| anyhow!("failed to chunk opencode transcript: {err}"))
    }

    fn reassemble_transcript(&self, chunks: &[Vec<u8>]) -> Result<Vec<u8>> {
        Ok(reassemble_jsonl(chunks))
    }

    fn get_session_dir(&self, repo_path: &str) -> Result<String> {
        if let Ok(override_path) = std::env::var("BITLOOPS_TEST_OPENCODE_PROJECT_DIR")
            && !override_path.is_empty()
        {
            return Ok(override_path);
        }

        let project_dir = sanitize_path_for_opencode(repo_path);
        Ok(std::env::temp_dir()
            .join("bitloops-opencode")
            .join(project_dir)
            .to_string_lossy()
            .to_string())
    }

    fn resolve_session_file(&self, session_dir: &str, agent_session_id: &str) -> String {
        Path::new(session_dir)
            .join(format!("{agent_session_id}.jsonl"))
            .to_string_lossy()
            .to_string()
    }

    fn read_session(&self, input: &HookInput) -> Result<Option<AgentSession>> {
        if input.session_ref.is_empty() {
            return Err(anyhow!("no session ref provided"));
        }

        let data =
            fs::read(&input.session_ref).map_err(|err| anyhow!("failed to read session: {err}"))?;

        let modified_files = extract_modified_files(&data).unwrap_or_default();

        Ok(Some(AgentSession {
            session_id: input.session_id.clone(),
            agent_name: self.name(),
            session_ref: input.session_ref.clone(),
            start_time: SystemTime::now(),
            native_data: data,
            modified_files,
            ..AgentSession::default()
        }))
    }

    fn write_session(&self, session: &AgentSession) -> Result<()> {
        if session.session_ref.is_empty() {
            return Err(anyhow!("no session ref to write to"));
        }
        if session.native_data.is_empty() {
            return Err(anyhow!("no session data to write"));
        }

        let parent = Path::new(&session.session_ref)
            .parent()
            .ok_or_else(|| anyhow!("failed to resolve session directory from session ref"))?;
        fs::create_dir_all(parent)
            .map_err(|err| anyhow!("failed to create session directory: {err}"))?;
        fs::write(&session.session_ref, &session.native_data)
            .map_err(|err| anyhow!("failed to write session data: {err}"))?;

        if session.export_data.is_empty() {
            return Ok(());
        }

        if let Err(err) =
            self.import_session_into_opencode(&session.session_id, &session.export_data)
        {
            eprintln!("warning: could not import session into OpenCode: {err}");
        }
        Ok(())
    }

    fn format_resume_command(&self, session_id: &str) -> String {
        if session_id.trim().is_empty() {
            "opencode".to_string()
        } else {
            format!("opencode -s {session_id}")
        }
    }
}

impl HookSupport for OpenCodeAgent {
    fn install_hooks(&self, local_dev: bool, force: bool) -> Result<usize> {
        let plugin_path = self.plugin_path()?;

        if !force
            && plugin_path.exists()
            && let Ok(content) = fs::read_to_string(&plugin_path)
            && Self::ensure_plugin_marker(&content).is_ok()
        {
            return Ok(0);
        }

        let content = self.render_plugin(local_dev)?;
        if let Some(parent) = plugin_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| anyhow!("failed to create plugin directory: {err}"))?;
        }
        fs::write(&plugin_path, content)
            .map_err(|err| anyhow!("failed to write plugin file: {err}"))?;

        Ok(1)
    }

    fn uninstall_hooks(&self) -> Result<()> {
        let plugin_path = self.plugin_path()?;
        match fs::remove_file(plugin_path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(anyhow!("failed to remove plugin file: {err}")),
        }
    }

    fn are_hooks_installed(&self) -> bool {
        let Ok(plugin_path) = self.plugin_path() else {
            return false;
        };
        let Ok(content) = fs::read_to_string(plugin_path) else {
            return false;
        };
        Self::ensure_plugin_marker(&content).is_ok()
    }
}

impl OpenCodeAgent {
    pub fn get_supported_hooks(&self) -> Vec<HookType> {
        vec![
            HookType::SessionStart,
            HookType::SessionEnd,
            HookType::UserPromptSubmit,
            HookType::Stop,
        ]
    }

    pub fn plugin_path(&self) -> Result<PathBuf> {
        let repo_root = crate::utils::paths::repo_root().or_else(|_| {
            std::env::current_dir().map_err(|err| anyhow!("failed to get current directory: {err}"))
        })?;
        Ok(get_plugin_path(&repo_root))
    }

    pub fn ensure_plugin_marker(content: &str) -> Result<()> {
        if content.contains(BITLOOPS_MARKER) {
            return Ok(());
        }
        Err(anyhow!("plugin file does not contain Bitloops marker"))
    }

    pub fn render_plugin(&self, local_dev: bool) -> Result<String> {
        render_plugin_template(local_dev)
    }

    pub fn get_transcript_position(&self, path: &str) -> Result<usize> {
        match parse_messages_from_file(path) {
            Ok(messages) => Ok(messages.len()),
            Err(err) => {
                if is_not_found(&err) {
                    return Ok(0);
                }
                Err(err)
            }
        }
    }

    pub fn extract_modified_files_from_offset(
        &self,
        path: &str,
        start_offset: usize,
    ) -> Result<(Vec<String>, usize)> {
        let messages = match parse_messages_from_file(path) {
            Ok(messages) => messages,
            Err(err) => {
                if is_not_found(&err) {
                    return Ok((Vec::new(), 0));
                }
                return Err(err);
            }
        };

        let mut seen = HashSet::new();
        let mut files = Vec::new();

        for message in messages.iter().skip(start_offset) {
            if message.role != ROLE_ASSISTANT {
                continue;
            }

            for part in &message.parts {
                let Some(state) = part.state.as_ref() else {
                    continue;
                };
                if part.part_type != "tool" {
                    continue;
                }
                if !FILE_MODIFICATION_TOOLS.contains(&part.tool.as_str()) {
                    continue;
                }

                let file_path = super::transcript::extract_file_path_from_input(&state.input);
                if !file_path.is_empty() && seen.insert(file_path.clone()) {
                    files.push(file_path);
                }
            }
        }

        Ok((files, messages.len()))
    }

    pub fn extract_prompts(&self, session_ref: &str, from_offset: usize) -> Result<Vec<String>> {
        let messages = match parse_messages_from_file(session_ref) {
            Ok(messages) => messages,
            Err(err) => {
                if is_not_found(&err) {
                    return Ok(Vec::new());
                }
                return Err(err);
            }
        };

        let mut prompts = Vec::new();
        for message in messages.into_iter().skip(from_offset) {
            if message.role == ROLE_USER && !message.content.is_empty() {
                prompts.push(message.content);
            }
        }
        Ok(prompts)
    }

    pub fn extract_summary(&self, session_ref: &str) -> Result<String> {
        let messages = match parse_messages_from_file(session_ref) {
            Ok(messages) => messages,
            Err(err) => {
                if is_not_found(&err) {
                    return Ok(String::new());
                }
                return Err(err);
            }
        };

        for message in messages.into_iter().rev() {
            if message.role == ROLE_ASSISTANT && !message.content.is_empty() {
                return Ok(message.content);
            }
        }
        Ok(String::new())
    }

    pub fn calculate_token_usage(
        &self,
        session_ref: &str,
        from_offset: usize,
    ) -> Result<Option<TokenUsage>> {
        let data = match fs::read(session_ref) {
            Ok(data) => data,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => {
                return Err(anyhow!("failed to parse transcript for token usage: {err}"));
            }
        };

        Ok(Some(calculate_token_usage_from_bytes(&data, from_offset)))
    }

    pub fn import_session_into_opencode(&self, session_id: &str, export_data: &[u8]) -> Result<()> {
        if session_id.trim().is_empty() {
            return Err(anyhow!("session id is required"));
        }
        if export_data.is_empty() {
            return Err(anyhow!("export data is required"));
        }

        run_opencode_session_delete(session_id)
            .map_err(|err| anyhow!("failed to delete existing session: {err}"))?;

        let temp_file_path =
            std::env::temp_dir().join(format!("bitloops-opencode-export-{}.json", Uuid::new_v4()));
        let _cleanup = TempFileCleanup(temp_file_path.clone());

        let mut temp_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_file_path)
            .map_err(|err| anyhow!("failed to create temp file: {err}"))?;
        temp_file
            .write_all(export_data)
            .map_err(|err| anyhow!("failed to write export data: {err}"))?;
        temp_file
            .flush()
            .map_err(|err| anyhow!("failed to close temp file: {err}"))?;
        drop(temp_file);

        run_opencode_import(temp_file_path.to_string_lossy().as_ref())
    }
}

fn is_not_found(err: &anyhow::Error) -> bool {
    err.downcast_ref::<std::io::Error>()
        .is_some_and(|io_err| io_err.kind() == std::io::ErrorKind::NotFound)
}

pub fn sanitize_path_for_opencode(path: &str) -> String {
    static NON_ALPHANUMERIC: OnceLock<Regex> = OnceLock::new();
    NON_ALPHANUMERIC
        .get_or_init(|| Regex::new(r"[^a-zA-Z0-9]").expect("regex must compile"))
        .replace_all(path, "-")
        .to_string()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use tempfile::TempDir;

    use crate::engine::agent::{Agent, HookSupport};
    use crate::test_support::process_state::with_cwd;

    use super::*;
    use crate::engine::agent::open_code::transcript::{extract_modified_files, parse_messages};

    const TEST_TRANSCRIPT_JSONL: &str = r#"{"id":"msg-1","role":"user","content":"Fix the bug in main.rs","time":{"created":1708300000}}
{"id":"msg-2","role":"assistant","content":"I'll fix the bug.","time":{"created":1708300001,"completed":1708300005},"tokens":{"input":150,"output":80,"reasoning":10,"cache":{"read":5,"write":15}},"cost":0.003,"parts":[{"type":"text","text":"I'll fix the bug."},{"type":"tool","tool":"edit","callID":"call-1","state":{"status":"completed","input":{"file_path":"main.rs"},"output":"Applied edit"}}]}
{"id":"msg-3","role":"user","content":"Also fix util.rs","time":{"created":1708300010}}
{"id":"msg-4","role":"assistant","content":"Done fixing util.rs.","time":{"created":1708300011,"completed":1708300015},"tokens":{"input":200,"output":100,"reasoning":5,"cache":{"read":10,"write":20}},"cost":0.005,"parts":[{"type":"tool","tool":"write","callID":"call-2","state":{"status":"completed","input":{"file_path":"util.rs"},"output":"File written"}},{"type":"text","text":"Done fixing util.rs."}]}
"#;

    fn write_test_transcript(content: &str) -> (TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let path = dir.path().join("test-session.jsonl");
        fs::write(&path, content).expect("failed to write test transcript");
        (dir, path)
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestInstallHooks_FreshInstall() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        with_cwd(dir.path(), || {
            let agent = OpenCodeAgent;

            let count = agent.install_hooks(false, false).expect("unexpected error");
            assert_eq!(count, 1, "expected one hook install");

            let plugin_path = dir
                .path()
                .join(".opencode")
                .join("plugins")
                .join("bitloops.ts");
            let content = fs::read_to_string(&plugin_path).expect("plugin file not created");
            assert!(
                content.contains(r#"const BITLOOPS_CMD = "bitloops""#),
                "plugin file does not contain production command constant"
            );
            assert!(
                content.contains("hooks opencode"),
                "plugin file does not contain 'hooks opencode'"
            );
            assert!(
                content.contains("BitloopsPlugin"),
                "plugin file does not contain BitloopsPlugin export"
            );
            assert!(
                !content.contains("cargo run --"),
                "plugin file should not contain cargo run in production mode"
            );
        });
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestInstallHooks_Idempotent() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        with_cwd(dir.path(), || {
            let agent = OpenCodeAgent;

            let count1 = agent
                .install_hooks(false, false)
                .expect("first install failed");
            assert_eq!(count1, 1, "first install should create one hook");

            let count2 = agent
                .install_hooks(false, false)
                .expect("second install failed");
            assert_eq!(count2, 0, "second install should be idempotent");
        });
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestInstallHooks_LocalDev() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        with_cwd(dir.path(), || {
            let agent = OpenCodeAgent;

            let count = agent.install_hooks(true, false).expect("unexpected error");
            assert_eq!(count, 1, "expected one hook install");

            let plugin_path = dir
                .path()
                .join(".opencode")
                .join("plugins")
                .join("bitloops.ts");
            let content = fs::read_to_string(&plugin_path).expect("plugin file not created");
            assert!(
                content.contains("cargo run --"),
                "local dev mode plugin should contain cargo run command"
            );
        });
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestInstallHooks_ForceReinstall() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        with_cwd(dir.path(), || {
            let agent = OpenCodeAgent;

            agent
                .install_hooks(false, false)
                .expect("first install failed");

            let count = agent
                .install_hooks(false, true)
                .expect("force reinstall failed");
            assert_eq!(count, 1, "force reinstall should write plugin again");
        });
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestUninstallHooks() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        with_cwd(dir.path(), || {
            let agent = OpenCodeAgent;

            agent
                .install_hooks(false, false)
                .expect("install should succeed before uninstall");
            agent
                .uninstall_hooks()
                .expect("uninstall should remove plugin");

            let plugin_path = dir
                .path()
                .join(".opencode")
                .join("plugins")
                .join("bitloops.ts");
            assert!(
                !plugin_path.exists(),
                "plugin file should not exist after uninstall"
            );
        });
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestUninstallHooks_NoFile() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        with_cwd(dir.path(), || {
            let agent = OpenCodeAgent;

            agent
                .uninstall_hooks()
                .expect("uninstall should be non-fatal when plugin does not exist");
        });
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestAreHooksInstalled() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        with_cwd(dir.path(), || {
            let agent = OpenCodeAgent;

            assert!(
                !agent.are_hooks_installed(),
                "hooks should not be installed initially"
            );

            agent
                .install_hooks(false, false)
                .expect("install hooks should succeed");
            assert!(
                agent.are_hooks_installed(),
                "hooks should be installed after InstallHooks"
            );

            agent
                .uninstall_hooks()
                .expect("uninstall hooks should succeed");
            assert!(
                !agent.are_hooks_installed(),
                "hooks should not be installed after UninstallHooks"
            );
        });
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestParseMessages() {
        let messages = parse_messages(TEST_TRANSCRIPT_JSONL.as_bytes()).expect("unexpected error");
        assert_eq!(messages.len(), 4, "expected 4 messages");
        assert_eq!(messages[0].id, "msg-1", "first message id mismatch");
        assert_eq!(messages[0].role, "user", "first message role mismatch");
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestParseMessages_Empty() {
        let messages = parse_messages(b"").expect("unexpected error for empty transcript");
        assert!(
            messages.is_empty(),
            "expected no messages for empty transcript"
        );
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestParseMessages_InvalidLines() {
        let data = b"not json\n{\"id\":\"msg-1\",\"role\":\"user\",\"content\":\"hello\"}\n";
        let messages = parse_messages(data).expect("unexpected error for invalid-line transcript");
        assert_eq!(messages.len(), 1, "expected one valid message");
        assert_eq!(
            messages[0].content, "hello",
            "expected valid line content to be parsed"
        );
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestGetTranscriptPosition() {
        let (_dir, path) = write_test_transcript(TEST_TRANSCRIPT_JSONL);
        let agent = OpenCodeAgent;

        let pos = agent
            .get_transcript_position(path.to_string_lossy().as_ref())
            .expect("unexpected error");
        assert_eq!(pos, 4, "expected 4-message position");
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestGetTranscriptPosition_NonexistentFile() {
        let agent = OpenCodeAgent;
        let pos = agent
            .get_transcript_position("/nonexistent/path.jsonl")
            .expect("unexpected error for nonexistent transcript");
        assert_eq!(pos, 0, "expected 0 for nonexistent transcript");
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestExtractModifiedFilesFromOffset() {
        let (_dir, path) = write_test_transcript(TEST_TRANSCRIPT_JSONL);
        let agent = OpenCodeAgent;

        let (files, pos) = agent
            .extract_modified_files_from_offset(path.to_string_lossy().as_ref(), 0)
            .expect("unexpected error");
        assert_eq!(pos, 4, "expected position 4");
        assert_eq!(files.len(), 2, "expected two modified files at offset 0");

        let (files, pos) = agent
            .extract_modified_files_from_offset(path.to_string_lossy().as_ref(), 2)
            .expect("unexpected error");
        assert_eq!(pos, 4, "expected position 4");
        assert_eq!(files.len(), 1, "expected one modified file at offset 2");
        assert_eq!(files[0], "util.rs", "expected util.rs from offset 2");
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestExtractPrompts() {
        let (_dir, path) = write_test_transcript(TEST_TRANSCRIPT_JSONL);
        let agent = OpenCodeAgent;

        let prompts = agent
            .extract_prompts(path.to_string_lossy().as_ref(), 0)
            .expect("unexpected error");
        assert_eq!(prompts.len(), 2, "expected two prompts from offset 0");
        assert_eq!(prompts[0], "Fix the bug in main.rs");

        let prompts = agent
            .extract_prompts(path.to_string_lossy().as_ref(), 2)
            .expect("unexpected error");
        assert_eq!(prompts.len(), 1, "expected one prompt from offset 2");
        assert_eq!(prompts[0], "Also fix util.rs");
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestExtractSummary() {
        let (_dir, path) = write_test_transcript(TEST_TRANSCRIPT_JSONL);
        let agent = OpenCodeAgent;

        let summary = agent
            .extract_summary(path.to_string_lossy().as_ref())
            .expect("unexpected error");
        assert_eq!(summary, "Done fixing util.rs.");
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestExtractSummary_EmptyTranscript() {
        let (_dir, path) = write_test_transcript("");
        let agent = OpenCodeAgent;

        let summary = agent
            .extract_summary(path.to_string_lossy().as_ref())
            .expect("unexpected error");
        assert_eq!(summary, "", "expected empty summary");
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestCalculateTokenUsage() {
        let (_dir, path) = write_test_transcript(TEST_TRANSCRIPT_JSONL);
        let agent = OpenCodeAgent;

        let usage = agent
            .calculate_token_usage(path.to_string_lossy().as_ref(), 0)
            .expect("unexpected error")
            .expect("expected non-nil token usage");
        assert_eq!(usage.input_tokens, 350, "input token count mismatch");
        assert_eq!(usage.output_tokens, 180, "output token count mismatch");
        assert_eq!(usage.cache_read_tokens, 15, "cache read token mismatch");
        assert_eq!(
            usage.cache_creation_tokens, 35,
            "cache creation token mismatch"
        );
        assert_eq!(usage.api_call_count, 2, "api call count mismatch");
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestCalculateTokenUsage_FromOffset() {
        let (_dir, path) = write_test_transcript(TEST_TRANSCRIPT_JSONL);
        let agent = OpenCodeAgent;

        let usage = agent
            .calculate_token_usage(path.to_string_lossy().as_ref(), 2)
            .expect("unexpected error")
            .expect("expected non-nil token usage");
        assert_eq!(usage.input_tokens, 200);
        assert_eq!(usage.output_tokens, 100);
        assert_eq!(usage.api_call_count, 1);
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestCalculateTokenUsage_NonexistentFile() {
        let agent = OpenCodeAgent;

        let usage = agent
            .calculate_token_usage("/nonexistent/path.jsonl", 0)
            .expect("unexpected error for nonexistent transcript");
        assert!(usage.is_none(), "expected nil usage for missing transcript");
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestChunkTranscript_SmallContent() {
        let agent = OpenCodeAgent;
        let content = TEST_TRANSCRIPT_JSONL.as_bytes();

        let chunks = agent
            .chunk_transcript(content, content.len() + 1000)
            .expect("unexpected chunking error");
        assert_eq!(chunks.len(), 1, "small content should remain single chunk");
        assert_eq!(chunks[0], content, "single chunk should match original");
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestChunkTranscript_SplitsLargeContent() {
        let agent = OpenCodeAgent;
        let content = TEST_TRANSCRIPT_JSONL.as_bytes();

        let chunks = agent
            .chunk_transcript(content, 500)
            .expect("unexpected chunking error");
        assert!(chunks.len() >= 2, "large content should split into chunks");

        for (idx, chunk) in chunks.iter().enumerate() {
            let messages = parse_messages(chunk)
                .unwrap_or_else(|err| panic!("chunk {idx} should parse as JSONL: {err}"));
            assert!(
                !messages.is_empty(),
                "chunk {idx} should contain at least one message"
            );
        }
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestChunkTranscript_RoundTrip() {
        let agent = OpenCodeAgent;
        let content = TEST_TRANSCRIPT_JSONL.as_bytes();

        let chunks = agent
            .chunk_transcript(content, 500)
            .expect("chunking should succeed");
        let reassembled = agent
            .reassemble_transcript(&chunks)
            .expect("reassembly should succeed");

        let original = parse_messages(content).expect("failed to parse original transcript");
        let result = parse_messages(&reassembled).expect("failed to parse reassembled transcript");

        assert_eq!(
            result.len(),
            original.len(),
            "message count must round-trip"
        );
        for (idx, msg) in result.iter().enumerate() {
            assert_eq!(
                msg.id, original[idx].id,
                "message ID mismatch at index {idx}"
            );
        }
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestChunkTranscript_EmptyContent() {
        let agent = OpenCodeAgent;
        let chunks = agent
            .chunk_transcript(b"", 100)
            .expect("chunking empty content should succeed");
        assert_eq!(chunks.len(), 0, "expected zero chunks for empty content");
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestReassembleTranscript_SingleChunk() {
        let agent = OpenCodeAgent;
        let content = TEST_TRANSCRIPT_JSONL.as_bytes().to_vec();
        let result = agent
            .reassemble_transcript(std::slice::from_ref(&content))
            .expect("single-chunk reassembly should succeed");
        assert_eq!(
            result, content,
            "single chunk reassembly should preserve original"
        );
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestReassembleTranscript_Empty() {
        let agent = OpenCodeAgent;
        let result = agent
            .reassemble_transcript(&[])
            .expect("empty reassembly should succeed");
        assert!(
            result.is_empty(),
            "expected empty output for empty reassembly input"
        );
    }

    #[test]
    #[allow(non_snake_case)]
    fn TestExtractModifiedFiles() {
        let files =
            extract_modified_files(TEST_TRANSCRIPT_JSONL.as_bytes()).expect("unexpected error");
        assert_eq!(files.len(), 2, "expected two modified files");
        assert_eq!(files[0], "main.rs", "first modified file mismatch");
        assert_eq!(files[1], "util.rs", "second modified file mismatch");
    }
}

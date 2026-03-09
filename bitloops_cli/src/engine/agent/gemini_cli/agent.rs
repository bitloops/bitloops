use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use super::transcript::{
    GeminiMessage, GeminiTranscript, calculate_token_usage_from_file,
    extract_last_assistant_message, extract_modified_files, parse_transcript,
};
use crate::engine::agent::{
    AGENT_NAME_GEMINI, AGENT_TYPE_GEMINI, Agent, AgentSession, Event, HookInput, HookSupport,
    TokenCalculator, TokenUsage, TranscriptAnalyzer, TranscriptPositionProvider, chunk_jsonl,
};

pub const HOOK_NAME_SESSION_START: &str = "session-start";
pub const HOOK_NAME_SESSION_END: &str = "session-end";
pub const HOOK_NAME_BEFORE_AGENT: &str = "before-agent";
pub const HOOK_NAME_AFTER_AGENT: &str = "after-agent";
pub const HOOK_NAME_BEFORE_MODEL: &str = "before-model";
pub const HOOK_NAME_AFTER_MODEL: &str = "after-model";
pub const HOOK_NAME_BEFORE_TOOL_SELECTION: &str = "before-tool-selection";
pub const HOOK_NAME_BEFORE_TOOL: &str = "before-tool";
pub const HOOK_NAME_AFTER_TOOL: &str = "after-tool";
pub const HOOK_NAME_PRE_COMPRESS: &str = "pre-compress";
pub const HOOK_NAME_NOTIFICATION: &str = "notification";

pub const GEMINI_SETTINGS_FILE_NAME: &str = "settings.json";

const BITLOOPS_HOOK_PREFIXES: [&str; 2] = ["bitloops ", "cargo run -- "];

#[derive(Debug, Default, Clone, Copy)]
pub struct GeminiCliAgent;

pub fn new_gemini_cli_agent() -> Box<dyn Agent + Send + Sync> {
    Box::new(GeminiCliAgent)
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GeminiSettings {
    #[serde(rename = "hooksConfig", default)]
    pub hooks_config: GeminiHooksConfig,
    #[serde(default)]
    pub hooks: GeminiHooks,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GeminiHooksConfig {
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GeminiHooks {
    #[serde(rename = "SessionStart", default)]
    pub session_start: Vec<GeminiHookMatcher>,
    #[serde(rename = "SessionEnd", default)]
    pub session_end: Vec<GeminiHookMatcher>,
    #[serde(rename = "BeforeAgent", default)]
    pub before_agent: Vec<GeminiHookMatcher>,
    #[serde(rename = "AfterAgent", default)]
    pub after_agent: Vec<GeminiHookMatcher>,
    #[serde(rename = "BeforeModel", default)]
    pub before_model: Vec<GeminiHookMatcher>,
    #[serde(rename = "AfterModel", default)]
    pub after_model: Vec<GeminiHookMatcher>,
    #[serde(rename = "BeforeToolSelection", default)]
    pub before_tool_selection: Vec<GeminiHookMatcher>,
    #[serde(rename = "BeforeTool", default)]
    pub before_tool: Vec<GeminiHookMatcher>,
    #[serde(rename = "AfterTool", default)]
    pub after_tool: Vec<GeminiHookMatcher>,
    #[serde(rename = "PreCompress", default)]
    pub pre_compress: Vec<GeminiHookMatcher>,
    #[serde(rename = "Notification", default)]
    pub notification: Vec<GeminiHookMatcher>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GeminiHookMatcher {
    #[serde(default)]
    pub matcher: String,
    #[serde(default)]
    pub hooks: Vec<GeminiHookEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GeminiHookEntry {
    #[serde(default)]
    pub name: String,
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub command: String,
}

impl Agent for GeminiCliAgent {
    fn name(&self) -> String {
        AGENT_NAME_GEMINI.to_string()
    }

    fn agent_type(&self) -> String {
        AGENT_TYPE_GEMINI.to_string()
    }

    fn description(&self) -> String {
        "Gemini CLI - Google's AI coding assistant".to_string()
    }

    fn is_preview(&self) -> bool {
        true
    }

    fn detect_presence(&self) -> Result<bool> {
        let repo_root = crate::engine::paths::repo_root().unwrap_or_else(|_| PathBuf::from("."));

        let gemini_dir = repo_root.join(".gemini");
        if gemini_dir.exists() {
            return Ok(true);
        }

        let settings_file = gemini_dir.join(GEMINI_SETTINGS_FILE_NAME);
        if settings_file.exists() {
            return Ok(true);
        }

        Ok(false)
    }

    fn get_session_id(&self, input: &HookInput) -> String {
        input.session_id.clone()
    }

    fn protected_dirs(&self) -> Vec<String> {
        vec![".gemini".to_string()]
    }

    fn hook_names(&self) -> Vec<String> {
        vec![
            HOOK_NAME_SESSION_START.to_string(),
            HOOK_NAME_SESSION_END.to_string(),
            HOOK_NAME_BEFORE_AGENT.to_string(),
            HOOK_NAME_AFTER_AGENT.to_string(),
            HOOK_NAME_BEFORE_MODEL.to_string(),
            HOOK_NAME_AFTER_MODEL.to_string(),
            HOOK_NAME_BEFORE_TOOL_SELECTION.to_string(),
            HOOK_NAME_BEFORE_TOOL.to_string(),
            HOOK_NAME_AFTER_TOOL.to_string(),
            HOOK_NAME_PRE_COMPRESS.to_string(),
            HOOK_NAME_NOTIFICATION.to_string(),
        ]
    }

    fn parse_hook_event(&self, hook_name: &str, stdin: &mut dyn Read) -> Result<Option<Event>> {
        let event = super::lifecycle::parse_hook_event(hook_name, stdin)?;
        Ok(event.map(|_| Event))
    }

    fn read_transcript(&self, session_ref: &str) -> Result<Vec<u8>> {
        std::fs::read(session_ref).map_err(|err| anyhow!("failed to read transcript: {err}"))
    }

    fn chunk_transcript(&self, content: &[u8], max_size: usize) -> Result<Vec<Vec<u8>>> {
        let transcript = match serde_json::from_slice::<GeminiTranscript>(content) {
            Ok(transcript) => transcript,
            Err(_) => {
                return chunk_jsonl(content, max_size)
                    .map_err(|err| anyhow!("failed to chunk as JSONL: {err}"));
            }
        };

        if transcript.messages.is_empty() {
            return Ok(vec![content.to_vec()]);
        }

        let mut chunks = Vec::new();
        let mut current_messages: Vec<GeminiMessage> = Vec::new();
        let mut current_size = br#"{"messages":[]}"#.len();

        for msg in transcript.messages {
            let msg_size = match serde_json::to_vec(&msg) {
                Ok(bytes) => bytes.len() + 1,
                Err(_) => {
                    continue;
                }
            };

            if current_size + msg_size > max_size && !current_messages.is_empty() {
                let chunk = GeminiTranscript {
                    messages: current_messages,
                };
                let chunk_data = serde_json::to_vec(&chunk)
                    .map_err(|err| anyhow!("failed to marshal chunk: {err}"))?;
                chunks.push(chunk_data);
                current_messages = Vec::new();
                current_size = br#"{"messages":[]}"#.len();
            }

            current_messages.push(msg);
            current_size += msg_size;
        }

        if !current_messages.is_empty() {
            let chunk = GeminiTranscript {
                messages: current_messages,
            };
            let chunk_data = serde_json::to_vec(&chunk)
                .map_err(|err| anyhow!("failed to marshal final chunk: {err}"))?;
            chunks.push(chunk_data);
        }

        if chunks.is_empty() {
            return Err(anyhow!(
                "failed to create any chunks: all messages failed to marshal"
            ));
        }

        Ok(chunks)
    }

    fn reassemble_transcript(&self, chunks: &[Vec<u8>]) -> Result<Vec<u8>> {
        let mut all_messages: Vec<GeminiMessage> = Vec::new();

        for chunk in chunks {
            let transcript: GeminiTranscript = serde_json::from_slice(chunk)
                .map_err(|err| anyhow!("failed to unmarshal chunk: {err}"))?;
            all_messages.extend(transcript.messages);
        }

        let merged = GeminiTranscript {
            messages: all_messages,
        };
        serde_json::to_vec(&merged)
            .map_err(|err| anyhow!("failed to marshal reassembled transcript: {err}"))
    }

    fn get_session_dir(&self, repo_path: &str) -> Result<String> {
        if let Ok(override_path) = std::env::var("BITLOOPS_TEST_GEMINI_PROJECT_DIR")
            && !override_path.is_empty()
        {
            return Ok(override_path);
        }

        let home_dir = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .ok_or_else(|| anyhow!("failed to get home directory"))?;
        let project_dir = Self::get_project_hash(repo_path);
        Ok(Path::new(&home_dir)
            .join(".gemini")
            .join("tmp")
            .join(project_dir)
            .join("chats")
            .to_string_lossy()
            .to_string())
    }

    fn resolve_session_file(&self, session_dir: &str, agent_session_id: &str) -> String {
        let short_id = if agent_session_id.len() > 8 {
            &agent_session_id[..8]
        } else {
            agent_session_id
        };

        let mut matches = Vec::new();
        if let Ok(entries) = std::fs::read_dir(session_dir) {
            for entry in entries.flatten() {
                let file_name = entry.file_name().to_string_lossy().to_string();
                if file_name.starts_with("session-")
                    && file_name.ends_with(&format!("-{short_id}.json"))
                {
                    matches.push(entry.path().to_string_lossy().to_string());
                }
            }
        }

        if !matches.is_empty() {
            matches.sort();
            return matches.last().cloned().unwrap_or_default();
        }

        let ts = Self::current_utc_session_timestamp();
        Path::new(session_dir)
            .join(format!("session-{ts}-{short_id}.json"))
            .to_string_lossy()
            .to_string()
    }

    fn read_session(&self, input: &HookInput) -> Result<Option<AgentSession>> {
        if input.session_ref.is_empty() {
            return Err(anyhow!("session reference (transcript path) is required"));
        }

        let data = std::fs::read(&input.session_ref)
            .map_err(|err| anyhow!("failed to read transcript: {err}"))?;

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
        if !session.agent_name.is_empty() && session.agent_name != self.name() {
            return Err(anyhow!(
                "session belongs to agent \"{}\", not \"{}\"",
                session.agent_name,
                self.name()
            ));
        }

        if session.session_ref.is_empty() {
            return Err(anyhow!("session reference (transcript path) is required"));
        }

        if session.native_data.is_empty() {
            return Err(anyhow!("session has no native data to write"));
        }

        std::fs::write(&session.session_ref, &session.native_data)
            .map_err(|err| anyhow!("failed to write transcript: {err}"))?;
        Ok(())
    }

    fn format_resume_command(&self, session_id: &str) -> String {
        format!("gemini --resume {session_id}")
    }
}

impl HookSupport for GeminiCliAgent {
    fn install_hooks(&self, local_dev: bool, force: bool) -> Result<usize> {
        let settings_path = self.settings_path()?;

        let mut raw_settings: Map<String, Value> = match std::fs::read(&settings_path) {
            Ok(data) => serde_json::from_slice(&data)
                .map_err(|err| anyhow!("failed to parse existing settings.json: {err}"))?,
            Err(_) => Map::new(),
        };

        let mut raw_hooks: Map<String, Value> = if let Some(hooks) = raw_settings.get("hooks") {
            serde_json::from_value(hooks.clone())
                .map_err(|err| anyhow!("failed to parse hooks in settings.json: {err}"))?
        } else {
            Map::new()
        };

        let mut hooks_config: GeminiHooksConfig =
            if let Some(hooks_config_value) = raw_settings.get("hooksConfig") {
                serde_json::from_value(hooks_config_value.clone())
                    .map_err(|err| anyhow!("failed to parse hooksConfig in settings.json: {err}"))?
            } else {
                GeminiHooksConfig::default()
            };
        hooks_config.enabled = true;

        let cmd_prefix = if local_dev {
            "cargo run -- hooks gemini "
        } else {
            "bitloops hooks gemini "
        };

        let mut session_start = Self::parse_gemini_hook_type(&raw_hooks, "SessionStart");
        let mut session_end = Self::parse_gemini_hook_type(&raw_hooks, "SessionEnd");
        let mut before_agent = Self::parse_gemini_hook_type(&raw_hooks, "BeforeAgent");
        let mut after_agent = Self::parse_gemini_hook_type(&raw_hooks, "AfterAgent");
        let mut before_model = Self::parse_gemini_hook_type(&raw_hooks, "BeforeModel");
        let mut after_model = Self::parse_gemini_hook_type(&raw_hooks, "AfterModel");
        let mut before_tool_selection =
            Self::parse_gemini_hook_type(&raw_hooks, "BeforeToolSelection");
        let mut before_tool = Self::parse_gemini_hook_type(&raw_hooks, "BeforeTool");
        let mut after_tool = Self::parse_gemini_hook_type(&raw_hooks, "AfterTool");
        let mut pre_compress = Self::parse_gemini_hook_type(&raw_hooks, "PreCompress");
        let mut notification = Self::parse_gemini_hook_type(&raw_hooks, "Notification");

        if !force {
            let existing_cmd = Self::get_first_bitloops_hook_command(&session_start);
            let expected_cmd = format!("{cmd_prefix}session-start");
            if existing_cmd == expected_cmd {
                return Ok(0);
            }
        }

        session_start = Self::remove_bitloops_hooks(session_start);
        session_end = Self::remove_bitloops_hooks(session_end);
        before_agent = Self::remove_bitloops_hooks(before_agent);
        after_agent = Self::remove_bitloops_hooks(after_agent);
        before_model = Self::remove_bitloops_hooks(before_model);
        after_model = Self::remove_bitloops_hooks(after_model);
        before_tool_selection = Self::remove_bitloops_hooks(before_tool_selection);
        before_tool = Self::remove_bitloops_hooks(before_tool);
        after_tool = Self::remove_bitloops_hooks(after_tool);
        pre_compress = Self::remove_bitloops_hooks(pre_compress);
        notification = Self::remove_bitloops_hooks(notification);

        session_start = Self::add_gemini_hook(
            session_start,
            "",
            "bitloops-session-start",
            format!("{cmd_prefix}session-start"),
        );
        session_end = Self::add_gemini_hook(
            session_end,
            "exit",
            "bitloops-session-end-exit",
            format!("{cmd_prefix}session-end"),
        );
        session_end = Self::add_gemini_hook(
            session_end,
            "logout",
            "bitloops-session-end-logout",
            format!("{cmd_prefix}session-end"),
        );
        before_agent = Self::add_gemini_hook(
            before_agent,
            "",
            "bitloops-before-agent",
            format!("{cmd_prefix}before-agent"),
        );
        after_agent = Self::add_gemini_hook(
            after_agent,
            "",
            "bitloops-after-agent",
            format!("{cmd_prefix}after-agent"),
        );
        before_model = Self::add_gemini_hook(
            before_model,
            "",
            "bitloops-before-model",
            format!("{cmd_prefix}before-model"),
        );
        after_model = Self::add_gemini_hook(
            after_model,
            "",
            "bitloops-after-model",
            format!("{cmd_prefix}after-model"),
        );
        before_tool_selection = Self::add_gemini_hook(
            before_tool_selection,
            "",
            "bitloops-before-tool-selection",
            format!("{cmd_prefix}before-tool-selection"),
        );
        before_tool = Self::add_gemini_hook(
            before_tool,
            "*",
            "bitloops-before-tool",
            format!("{cmd_prefix}before-tool"),
        );
        after_tool = Self::add_gemini_hook(
            after_tool,
            "*",
            "bitloops-after-tool",
            format!("{cmd_prefix}after-tool"),
        );
        pre_compress = Self::add_gemini_hook(
            pre_compress,
            "",
            "bitloops-pre-compress",
            format!("{cmd_prefix}pre-compress"),
        );
        notification = Self::add_gemini_hook(
            notification,
            "",
            "bitloops-notification",
            format!("{cmd_prefix}notification"),
        );

        Self::marshal_gemini_hook_type(&mut raw_hooks, "SessionStart", &session_start);
        Self::marshal_gemini_hook_type(&mut raw_hooks, "SessionEnd", &session_end);
        Self::marshal_gemini_hook_type(&mut raw_hooks, "BeforeAgent", &before_agent);
        Self::marshal_gemini_hook_type(&mut raw_hooks, "AfterAgent", &after_agent);
        Self::marshal_gemini_hook_type(&mut raw_hooks, "BeforeModel", &before_model);
        Self::marshal_gemini_hook_type(&mut raw_hooks, "AfterModel", &after_model);
        Self::marshal_gemini_hook_type(
            &mut raw_hooks,
            "BeforeToolSelection",
            &before_tool_selection,
        );
        Self::marshal_gemini_hook_type(&mut raw_hooks, "BeforeTool", &before_tool);
        Self::marshal_gemini_hook_type(&mut raw_hooks, "AfterTool", &after_tool);
        Self::marshal_gemini_hook_type(&mut raw_hooks, "PreCompress", &pre_compress);
        Self::marshal_gemini_hook_type(&mut raw_hooks, "Notification", &notification);

        let hooks_config_json = serde_json::to_value(hooks_config)
            .map_err(|err| anyhow!("failed to marshal hooksConfig: {err}"))?;
        raw_settings.insert("hooksConfig".to_string(), hooks_config_json);
        raw_settings.insert("hooks".to_string(), Value::Object(raw_hooks));

        if let Some(parent) = settings_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| anyhow!("failed to create .gemini directory: {err}"))?;
        }
        let output = serde_json::to_vec_pretty(&raw_settings)
            .map_err(|err| anyhow!("failed to marshal settings: {err}"))?;
        std::fs::write(&settings_path, output)
            .map_err(|err| anyhow!("failed to write settings.json: {err}"))?;

        Ok(12)
    }

    fn uninstall_hooks(&self) -> Result<()> {
        let settings_path = self.settings_path()?;
        let data = match std::fs::read(&settings_path) {
            Ok(data) => data,
            Err(_) => return Ok(()),
        };

        let mut raw_settings: Map<String, Value> = serde_json::from_slice(&data)
            .map_err(|err| anyhow!("failed to parse settings.json: {err}"))?;

        let mut raw_hooks: Map<String, Value> = if let Some(hooks) = raw_settings.get("hooks") {
            serde_json::from_value(hooks.clone())
                .map_err(|err| anyhow!("failed to parse hooks: {err}"))?
        } else {
            Map::new()
        };

        let mut session_start = Self::parse_gemini_hook_type(&raw_hooks, "SessionStart");
        let mut session_end = Self::parse_gemini_hook_type(&raw_hooks, "SessionEnd");
        let mut before_agent = Self::parse_gemini_hook_type(&raw_hooks, "BeforeAgent");
        let mut after_agent = Self::parse_gemini_hook_type(&raw_hooks, "AfterAgent");
        let mut before_model = Self::parse_gemini_hook_type(&raw_hooks, "BeforeModel");
        let mut after_model = Self::parse_gemini_hook_type(&raw_hooks, "AfterModel");
        let mut before_tool_selection =
            Self::parse_gemini_hook_type(&raw_hooks, "BeforeToolSelection");
        let mut before_tool = Self::parse_gemini_hook_type(&raw_hooks, "BeforeTool");
        let mut after_tool = Self::parse_gemini_hook_type(&raw_hooks, "AfterTool");
        let mut pre_compress = Self::parse_gemini_hook_type(&raw_hooks, "PreCompress");
        let mut notification = Self::parse_gemini_hook_type(&raw_hooks, "Notification");

        session_start = Self::remove_bitloops_hooks(session_start);
        session_end = Self::remove_bitloops_hooks(session_end);
        before_agent = Self::remove_bitloops_hooks(before_agent);
        after_agent = Self::remove_bitloops_hooks(after_agent);
        before_model = Self::remove_bitloops_hooks(before_model);
        after_model = Self::remove_bitloops_hooks(after_model);
        before_tool_selection = Self::remove_bitloops_hooks(before_tool_selection);
        before_tool = Self::remove_bitloops_hooks(before_tool);
        after_tool = Self::remove_bitloops_hooks(after_tool);
        pre_compress = Self::remove_bitloops_hooks(pre_compress);
        notification = Self::remove_bitloops_hooks(notification);

        Self::marshal_gemini_hook_type(&mut raw_hooks, "SessionStart", &session_start);
        Self::marshal_gemini_hook_type(&mut raw_hooks, "SessionEnd", &session_end);
        Self::marshal_gemini_hook_type(&mut raw_hooks, "BeforeAgent", &before_agent);
        Self::marshal_gemini_hook_type(&mut raw_hooks, "AfterAgent", &after_agent);
        Self::marshal_gemini_hook_type(&mut raw_hooks, "BeforeModel", &before_model);
        Self::marshal_gemini_hook_type(&mut raw_hooks, "AfterModel", &after_model);
        Self::marshal_gemini_hook_type(
            &mut raw_hooks,
            "BeforeToolSelection",
            &before_tool_selection,
        );
        Self::marshal_gemini_hook_type(&mut raw_hooks, "BeforeTool", &before_tool);
        Self::marshal_gemini_hook_type(&mut raw_hooks, "AfterTool", &after_tool);
        Self::marshal_gemini_hook_type(&mut raw_hooks, "PreCompress", &pre_compress);
        Self::marshal_gemini_hook_type(&mut raw_hooks, "Notification", &notification);

        if raw_hooks.is_empty() {
            raw_settings.remove("hooks");
        } else {
            raw_settings.insert("hooks".to_string(), Value::Object(raw_hooks));
        }

        let output = serde_json::to_vec_pretty(&raw_settings)
            .map_err(|err| anyhow!("failed to marshal settings: {err}"))?;
        std::fs::write(&settings_path, output)
            .map_err(|err| anyhow!("failed to write settings.json: {err}"))?;

        Ok(())
    }

    fn are_hooks_installed(&self) -> bool {
        let settings_path = match self.settings_path() {
            Ok(path) => path,
            Err(_) => return false,
        };

        let data = match std::fs::read(settings_path) {
            Ok(data) => data,
            Err(_) => return false,
        };

        let settings: GeminiSettings = match serde_json::from_slice(&data) {
            Ok(settings) => settings,
            Err(_) => return false,
        };

        Self::has_bitloops_hook(&settings.hooks.session_start)
            || Self::has_bitloops_hook(&settings.hooks.session_end)
            || Self::has_bitloops_hook(&settings.hooks.before_agent)
            || Self::has_bitloops_hook(&settings.hooks.after_agent)
            || Self::has_bitloops_hook(&settings.hooks.before_model)
            || Self::has_bitloops_hook(&settings.hooks.after_model)
            || Self::has_bitloops_hook(&settings.hooks.before_tool_selection)
            || Self::has_bitloops_hook(&settings.hooks.before_tool)
            || Self::has_bitloops_hook(&settings.hooks.after_tool)
            || Self::has_bitloops_hook(&settings.hooks.pre_compress)
            || Self::has_bitloops_hook(&settings.hooks.notification)
    }
}

impl TranscriptPositionProvider for GeminiCliAgent {
    fn get_transcript_position(&self, path: &str) -> Result<usize> {
        Self::get_transcript_position_impl(path)
    }
}

impl TranscriptAnalyzer for GeminiCliAgent {
    fn get_transcript_position(&self, path: &str) -> Result<usize> {
        Self::get_transcript_position_impl(path)
    }

    fn extract_modified_files_from_offset(
        &self,
        path: &str,
        start_offset: usize,
    ) -> Result<(Vec<String>, usize)> {
        Self::extract_modified_files_from_offset_impl(self, path, start_offset)
    }

    fn extract_prompts(&self, session_ref: &str, from_offset: usize) -> Result<Vec<String>> {
        Self::extract_prompts_impl(self, session_ref, from_offset)
    }

    fn extract_summary(&self, session_ref: &str) -> Result<String> {
        Self::extract_summary_impl(self, session_ref)
    }
}

impl TokenCalculator for GeminiCliAgent {
    fn calculate_token_usage(&self, session_ref: &str, from_offset: usize) -> Result<TokenUsage> {
        Self::calculate_token_usage_impl(self, session_ref, from_offset)
    }
}

impl GeminiCliAgent {
    pub fn get_project_hash(project_root: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(project_root.as_bytes());
        let digest = hasher.finalize();
        digest.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    pub fn get_transcript_position(&self, path: &str) -> Result<usize> {
        Self::get_transcript_position_impl(path)
    }

    fn get_transcript_position_impl(path: &str) -> Result<usize> {
        if path.is_empty() {
            return Ok(0);
        }

        let data = match std::fs::read(path) {
            Ok(data) => data,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(err) => return Err(anyhow!("failed to read transcript: {err}")),
        };
        if data.is_empty() {
            return Ok(0);
        }

        let transcript = parse_transcript(&data)?;
        Ok(transcript.messages.len())
    }

    pub fn extract_modified_files_from_offset(
        &self,
        path: &str,
        start_offset: usize,
    ) -> Result<(Vec<String>, usize)> {
        Self::extract_modified_files_from_offset_impl(self, path, start_offset)
    }

    fn extract_modified_files_from_offset_impl(
        _agent: &Self,
        path: &str,
        start_offset: usize,
    ) -> Result<(Vec<String>, usize)> {
        if path.is_empty() {
            return Ok((Vec::new(), 0));
        }

        let data = match std::fs::read(path) {
            Ok(data) => data,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok((Vec::new(), 0)),
            Err(err) => return Err(anyhow!("failed to read transcript: {err}")),
        };
        if data.is_empty() {
            return Ok((Vec::new(), 0));
        }

        let transcript = parse_transcript(&data)?;
        let total_messages = transcript.messages.len();
        let mut files = Vec::new();
        let mut seen = HashSet::new();

        for msg in transcript.messages.iter().skip(start_offset) {
            if msg.r#type != "gemini" {
                continue;
            }

            for tool_call in &msg.tool_calls {
                if !super::transcript::FILE_MODIFICATION_TOOLS
                    .iter()
                    .any(|tool| *tool == tool_call.name)
                {
                    continue;
                }

                let file = tool_call
                    .args
                    .get("file_path")
                    .and_then(Value::as_str)
                    .or_else(|| tool_call.args.get("path").and_then(Value::as_str))
                    .or_else(|| tool_call.args.get("filename").and_then(Value::as_str))
                    .unwrap_or_default();
                if file.is_empty() {
                    continue;
                }

                if seen.insert(file.to_string()) {
                    files.push(file.to_string());
                }
            }
        }

        Ok((files, total_messages))
    }

    pub fn read_and_parse_hook_input<T: for<'de> Deserialize<'de>>(raw: &str) -> Result<T> {
        if raw.trim().is_empty() {
            return Err(anyhow!("empty hook input"));
        }
        serde_json::from_str(raw).map_err(|err| anyhow!("failed to parse hook input: {err}"))
    }

    pub fn extract_prompts(&self, session_ref: &str, from_offset: usize) -> Result<Vec<String>> {
        Self::extract_prompts_impl(self, session_ref, from_offset)
    }

    fn extract_prompts_impl(
        _agent: &Self,
        session_ref: &str,
        from_offset: usize,
    ) -> Result<Vec<String>> {
        let data = std::fs::read(session_ref)
            .map_err(|err| anyhow!("failed to read transcript: {err}"))?;
        let transcript = parse_transcript(&data)?;

        let mut prompts = Vec::new();
        for (idx, msg) in transcript.messages.iter().enumerate() {
            if idx < from_offset {
                continue;
            }
            if msg.r#type == "user" && !msg.content.is_empty() {
                prompts.push(msg.content.clone());
            }
        }
        Ok(prompts)
    }

    pub fn extract_summary(&self, session_ref: &str) -> Result<String> {
        Self::extract_summary_impl(self, session_ref)
    }

    fn extract_summary_impl(_agent: &Self, session_ref: &str) -> Result<String> {
        let data = std::fs::read(session_ref)
            .map_err(|err| anyhow!("failed to read transcript: {err}"))?;
        extract_last_assistant_message(&data)
    }

    pub fn calculate_token_usage(
        &self,
        session_ref: &str,
        from_offset: usize,
    ) -> Result<TokenUsage> {
        Self::calculate_token_usage_impl(self, session_ref, from_offset)
    }

    fn calculate_token_usage_impl(
        _agent: &Self,
        session_ref: &str,
        from_offset: usize,
    ) -> Result<TokenUsage> {
        calculate_token_usage_from_file(session_ref, from_offset)
    }

    fn settings_path(&self) -> Result<PathBuf> {
        let repo_root = crate::engine::paths::repo_root().or_else(|_| {
            std::env::current_dir().map_err(|err| anyhow!("failed to get current directory: {err}"))
        })?;
        Ok(repo_root.join(".gemini").join(GEMINI_SETTINGS_FILE_NAME))
    }

    fn is_bitloops_hook(command: &str) -> bool {
        BITLOOPS_HOOK_PREFIXES
            .iter()
            .any(|prefix| command.starts_with(prefix))
    }

    fn parse_gemini_hook_type(
        raw_hooks: &Map<String, Value>,
        hook_type: &str,
    ) -> Vec<GeminiHookMatcher> {
        raw_hooks
            .get(hook_type)
            .and_then(|value| serde_json::from_value::<Vec<GeminiHookMatcher>>(value.clone()).ok())
            .unwrap_or_default()
    }

    fn marshal_gemini_hook_type(
        raw_hooks: &mut Map<String, Value>,
        hook_type: &str,
        matchers: &[GeminiHookMatcher],
    ) {
        if matchers.is_empty() {
            raw_hooks.remove(hook_type);
            return;
        }

        if let Ok(value) = serde_json::to_value(matchers) {
            raw_hooks.insert(hook_type.to_string(), value);
        }
    }

    fn remove_bitloops_hooks(matchers: Vec<GeminiHookMatcher>) -> Vec<GeminiHookMatcher> {
        let mut result = Vec::new();
        for mut matcher in matchers {
            matcher
                .hooks
                .retain(|hook| !Self::is_bitloops_hook(&hook.command));
            if !matcher.hooks.is_empty() {
                result.push(matcher);
            }
        }
        result
    }

    fn add_gemini_hook(
        mut matchers: Vec<GeminiHookMatcher>,
        matcher_name: &str,
        hook_name: &str,
        command: String,
    ) -> Vec<GeminiHookMatcher> {
        let entry = GeminiHookEntry {
            name: hook_name.to_string(),
            kind: "command".to_string(),
            command,
        };

        if let Some(existing) = matchers.iter_mut().find(|m| m.matcher == matcher_name) {
            existing.hooks.push(entry);
            return matchers;
        }

        matchers.push(GeminiHookMatcher {
            matcher: matcher_name.to_string(),
            hooks: vec![entry],
        });
        matchers
    }

    fn has_bitloops_hook(matchers: &[GeminiHookMatcher]) -> bool {
        for matcher in matchers {
            for hook in &matcher.hooks {
                if Self::is_bitloops_hook(&hook.command) {
                    return true;
                }
            }
        }
        false
    }

    fn get_first_bitloops_hook_command(matchers: &[GeminiHookMatcher]) -> String {
        for matcher in matchers {
            for hook in &matcher.hooks {
                if Self::is_bitloops_hook(&hook.command) {
                    return hook.command.clone();
                }
            }
        }
        String::new()
    }

    fn current_utc_session_timestamp() -> String {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let (year, month, day, hour, minute, _) = Self::unix_to_ymdhms(secs);
        format!("{year:04}-{month:02}-{day:02}T{hour:02}-{minute:02}")
    }

    fn unix_to_ymdhms(secs: u64) -> (u64, u64, u64, u64, u64, u64) {
        let second = secs % 60;
        let minute = (secs / 60) % 60;
        let hour = (secs / 3600) % 24;

        let mut days = secs / 86_400;
        let mut year = 1970u64;
        loop {
            let year_days = if Self::is_leap(year) { 366 } else { 365 };
            if days < year_days {
                break;
            }
            days -= year_days;
            year += 1;
        }

        let month_lengths = [
            31u64,
            if Self::is_leap(year) { 29 } else { 28 },
            31,
            30,
            31,
            30,
            31,
            31,
            30,
            31,
            30,
            31,
        ];
        let mut month = 1u64;
        for len in month_lengths {
            if days < len {
                break;
            }
            days -= len;
            month += 1;
        }
        let day = days + 1;

        (year, month, day, hour, minute, second)
    }

    fn is_leap(year: u64) -> bool {
        (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
    }
}

#[cfg(test)]
#[path = "agent_tests.rs"]
mod tests;

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::Arc;

use crate::host::transcript::{
    parse::extract_user_content,
    types::{
        AssistantMessage, CONTENT_TYPE_TEXT, CONTENT_TYPE_TOOL_USE, Line, TYPE_ASSISTANT,
        TYPE_USER, ToolInput,
    },
};

pub const DEFAULT_MODEL: &str = "sonnet";
const SKILL_CONTENT_PREFIX: &str = "Base directory for this skill:";
const SUMMARIZATION_PROMPT_TEMPLATE: &str = r#"Analyze this development session transcript and generate a structured summary.

<transcript>
%s
</transcript>

Return a JSON object with this exact structure:
{
  "intent": "What the user was trying to accomplish (1-2 sentences)",
  "outcome": "What was actually achieved (1-2 sentences)",
  "learnings": {
    "repo": ["Codebase-specific patterns, conventions, or gotchas discovered"],
    "code": [{"path": "file/path.rs", "line": 42, "end_line": 56, "finding": "What was learned"}],
    "workflow": ["General development practices or tool usage insights"]
  },
  "friction": ["Problems, blockers, or annoyances encountered"],
  "open_items": ["Tech debt, unfinished work, or things to revisit later"]
}

Guidelines:
- Be concise but specific
- Include line numbers for code learnings when the transcript references specific lines
- Friction should capture both blockers and minor annoyances
- Open items are things intentionally deferred, not failures
- Empty arrays are fine if a category doesn't apply
- Return ONLY the JSON object, no markdown formatting or explanation"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AgentType {
    ClaudeCode,
    Cursor,
    Gemini,
    OpenCode,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryType {
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub entry_type: EntryType,
    pub content: String,
    pub tool_name: String,
    pub tool_detail: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Input {
    pub transcript: Vec<Entry>,
    pub files_touched: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodeLearning {
    pub path: String,
    #[serde(default)]
    pub line: i32,
    #[serde(default)]
    pub end_line: i32,
    pub finding: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Learnings {
    pub repo: Vec<String>,
    pub code: Vec<CodeLearning>,
    pub workflow: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Summary {
    pub intent: String,
    pub outcome: String,
    pub learnings: Learnings,
    pub friction: Vec<String>,
    pub open_items: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct CommandInvocation {
    pub program: String,
    pub args: Vec<String>,
    pub dir: String,
    pub env: Vec<String>,
    pub stdin: String,
}

pub type CommandRunner = Arc<dyn Fn(CommandInvocation) -> Result<String> + Send + Sync>;

pub trait Generator {
    fn generate(&self, input: Input) -> Result<Summary>;
}

#[derive(Clone, Default)]
pub struct ClaudeGenerator {
    pub claude_path: String,
    pub model: String,
    pub command_runner: Option<CommandRunner>,
}

#[derive(Debug, Deserialize)]
struct ClaudeCliResponse {
    result: String,
}

#[derive(Debug, Deserialize)]
struct ClaudeLine {
    #[serde(rename = "type")]
    r#type: String,
    #[serde(default)]
    uuid: String,
    message: Value,
}

#[derive(Debug, Deserialize)]
struct GeminiTranscript {
    #[serde(default)]
    messages: Vec<GeminiMessage>,
}

#[derive(Debug, Deserialize)]
struct GeminiMessage {
    #[serde(rename = "type", default)]
    message_type: String,
    #[serde(default)]
    content: String,
    #[serde(rename = "toolCalls", default)]
    tool_calls: Vec<GeminiToolCall>,
}

#[derive(Debug, Deserialize)]
struct GeminiToolCall {
    #[serde(default)]
    name: String,
    #[serde(default)]
    args: HashMap<String, Value>,
}

#[derive(Debug, Deserialize)]
struct OpenCodeMessage {
    #[serde(default)]
    role: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    parts: Vec<OpenCodePart>,
}

#[derive(Debug, Deserialize)]
struct OpenCodePart {
    #[serde(rename = "type", default)]
    part_type: String,
    #[serde(default)]
    tool: String,
    state: Option<OpenCodeState>,
}

#[derive(Debug, Deserialize)]
struct OpenCodeState {
    #[serde(default)]
    input: HashMap<String, Value>,
}

impl Generator for ClaudeGenerator {
    fn generate(&self, input: Input) -> Result<Summary> {
        let transcript_text = format_condensed_transcript(input);
        let prompt = build_summarization_prompt(&transcript_text);

        let claude_path = if self.claude_path.is_empty() {
            "claude".to_string()
        } else {
            self.claude_path.clone()
        };

        let model = if self.model.is_empty() {
            DEFAULT_MODEL.to_string()
        } else {
            self.model.clone()
        };

        let invocation = CommandInvocation {
            program: claude_path,
            args: vec![
                "--print".to_string(),
                "--output-format".to_string(),
                "json".to_string(),
                "--model".to_string(),
                model,
                "--setting-sources".to_string(),
                String::new(),
            ],
            dir: std::env::temp_dir().to_string_lossy().to_string(),
            env: strip_git_env(
                &std::env::vars()
                    .map(|(key, value)| format!("{key}={value}"))
                    .collect::<Vec<_>>(),
            ),
            stdin: prompt,
        };

        let output = match &self.command_runner {
            Some(runner) => runner(invocation)?,
            None => run_command_invocation(invocation)?,
        };

        let cli_response: ClaudeCliResponse =
            serde_json::from_str(&output).context("parse claude CLI response")?;

        let result_json = extract_json_from_markdown(&cli_response.result);
        let summary: Summary = serde_json::from_str(&result_json)
            .with_context(|| format!("parse summary JSON: response: {result_json}"))?;

        Ok(summary)
    }
}

impl ClaudeGenerator {
    pub fn generate(&self, input: Input) -> Result<Summary> {
        <Self as Generator>::generate(self, input)
    }
}

fn run_command_invocation(invocation: CommandInvocation) -> Result<String> {
    let mut cmd = Command::new(&invocation.program);
    cmd.args(&invocation.args)
        .current_dir(&invocation.dir)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    for entry in invocation.env {
        if let Some((key, value)) = entry.split_once('=') {
            cmd.env(key, value);
        }
    }

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(anyhow!("claude CLI not found: {err}"));
        }
        Err(err) => return Err(anyhow!("failed to run claude CLI: {err}")),
    };

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(invocation.stdin.as_bytes())
            .context("failed to write prompt to claude CLI stdin")?;
    }

    let output = child
        .wait_with_output()
        .context("failed waiting for claude CLI")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if let Some(code) = output.status.code() {
            return Err(anyhow!("claude CLI failed (exit {code}): {stderr}"));
        }
        return Err(anyhow!("claude CLI failed: {stderr}"));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub fn strip_git_env(env: &[String]) -> Vec<String> {
    env.iter()
        .filter(|entry| !entry.starts_with("GIT_"))
        .cloned()
        .collect()
}

pub fn build_summarization_prompt(transcript_text: &str) -> String {
    SUMMARIZATION_PROMPT_TEMPLATE.replace("%s", transcript_text)
}

pub fn extract_json_from_markdown(input: &str) -> String {
    let trimmed = input.trim();

    if let Some(rest) = trimmed.strip_prefix("```json") {
        if let Some(idx) = rest.rfind("```") {
            return rest[..idx].trim().to_string();
        }
        return rest.trim().to_string();
    }

    if let Some(rest) = trimmed.strip_prefix("```") {
        if let Some(idx) = rest.rfind("```") {
            return rest[..idx].trim().to_string();
        }
        return rest.trim().to_string();
    }

    trimmed.to_string()
}

fn extract_tool_detail(tool_name: &str, input: &ToolInput) -> String {
    match tool_name {
        "Skill" => return input.skill.clone(),
        "Read" => {
            if !input.file_path.is_empty() {
                return input.file_path.clone();
            }
            return input.notebook_path.clone();
        }
        "WebFetch" => return input.url.clone(),
        _ => {}
    }

    if !input.description.is_empty() {
        return input.description.clone();
    }
    if !input.command.is_empty() {
        return input.command.clone();
    }
    if !input.file_path.is_empty() {
        return input.file_path.clone();
    }
    if !input.notebook_path.is_empty() {
        return input.notebook_path.clone();
    }
    input.pattern.clone()
}

fn extract_generic_tool_detail(input: &HashMap<String, Value>) -> String {
    for key in ["description", "command", "file_path", "path", "pattern"] {
        if let Some(Value::String(value)) = input.get(key)
            && !value.is_empty()
        {
            return value.clone();
        }
    }
    String::new()
}

fn empty_tool_input() -> ToolInput {
    ToolInput {
        file_path: String::new(),
        notebook_path: String::new(),
        description: String::new(),
        command: String::new(),
        pattern: String::new(),
        skill: String::new(),
        url: String::new(),
        prompt: String::new(),
    }
}

fn extract_user_entry(line: &Line) -> Option<Entry> {
    let raw = serde_json::to_vec(&line.message).ok()?;
    let content = extract_user_content(&raw);
    if content.is_empty() || content.starts_with(SKILL_CONTENT_PREFIX) {
        return None;
    }

    Some(Entry {
        entry_type: EntryType::User,
        content,
        tool_name: String::new(),
        tool_detail: String::new(),
    })
}

fn extract_assistant_entries(line: &Line) -> Vec<Entry> {
    let msg = match serde_json::from_value::<AssistantMessage>(line.message.clone()) {
        Ok(msg) => msg,
        Err(_) => return Vec::new(),
    };

    let mut entries = Vec::new();
    for block in msg.content {
        match block.r#type.as_str() {
            CONTENT_TYPE_TEXT => {
                if !block.text.is_empty() {
                    entries.push(Entry {
                        entry_type: EntryType::Assistant,
                        content: block.text,
                        tool_name: String::new(),
                        tool_detail: String::new(),
                    });
                }
            }
            CONTENT_TYPE_TOOL_USE => {
                let input = serde_json::from_value::<ToolInput>(block.input)
                    .unwrap_or_else(|_| empty_tool_input());
                entries.push(Entry {
                    entry_type: EntryType::Tool,
                    content: String::new(),
                    tool_name: block.name.clone(),
                    tool_detail: extract_tool_detail(&block.name, &input),
                });
            }
            _ => {}
        }
    }

    entries
}

fn parse_claude_lines(content: &[u8]) -> Vec<Line> {
    let mut lines = Vec::new();
    for raw_line in content.split(|byte| *byte == b'\n') {
        if raw_line.is_empty() {
            continue;
        }
        if let Ok(line) = serde_json::from_slice::<ClaudeLine>(raw_line) {
            lines.push(Line {
                r#type: line.r#type,
                uuid: line.uuid,
                message: line.message,
            });
        }
    }
    lines
}

pub fn build_condensed_transcript(lines: &[Line]) -> Vec<Entry> {
    let mut entries = Vec::new();

    for line in lines {
        match line.r#type.as_str() {
            TYPE_USER => {
                if let Some(entry) = extract_user_entry(line) {
                    entries.push(entry);
                }
            }
            TYPE_ASSISTANT => entries.extend(extract_assistant_entries(line)),
            _ => {}
        }
    }

    entries
}

fn build_condensed_transcript_from_gemini(content: &[u8]) -> Result<Vec<Entry>> {
    let transcript: GeminiTranscript =
        serde_json::from_slice(content).context("failed to parse Gemini transcript")?;
    let mut entries = Vec::new();

    for message in transcript.messages {
        match message.message_type.as_str() {
            "user" => {
                if !message.content.is_empty() {
                    entries.push(Entry {
                        entry_type: EntryType::User,
                        content: message.content,
                        tool_name: String::new(),
                        tool_detail: String::new(),
                    });
                }
            }
            "gemini" => {
                if !message.content.is_empty() {
                    entries.push(Entry {
                        entry_type: EntryType::Assistant,
                        content: message.content,
                        tool_name: String::new(),
                        tool_detail: String::new(),
                    });
                }
                for tool_call in message.tool_calls {
                    entries.push(Entry {
                        entry_type: EntryType::Tool,
                        content: String::new(),
                        tool_name: tool_call.name,
                        tool_detail: extract_generic_tool_detail(&tool_call.args),
                    });
                }
            }
            _ => {}
        }
    }

    Ok(entries)
}

fn build_condensed_transcript_from_opencode(content: &[u8]) -> Vec<Entry> {
    let mut entries = Vec::new();

    for raw_line in content.split(|byte| *byte == b'\n') {
        if raw_line.is_empty() {
            continue;
        }

        let message = match serde_json::from_slice::<OpenCodeMessage>(raw_line) {
            Ok(message) => message,
            Err(_) => continue,
        };

        match message.role.as_str() {
            "user" => {
                if !message.content.is_empty() {
                    entries.push(Entry {
                        entry_type: EntryType::User,
                        content: message.content,
                        tool_name: String::new(),
                        tool_detail: String::new(),
                    });
                }
            }
            "assistant" => {
                if !message.content.is_empty() {
                    entries.push(Entry {
                        entry_type: EntryType::Assistant,
                        content: message.content,
                        tool_name: String::new(),
                        tool_detail: String::new(),
                    });
                }

                for part in message.parts {
                    if part.part_type == "tool"
                        && let Some(state) = part.state
                    {
                        entries.push(Entry {
                            entry_type: EntryType::Tool,
                            content: String::new(),
                            tool_name: part.tool,
                            tool_detail: extract_generic_tool_detail(&state.input),
                        });
                    }
                }
            }
            _ => {}
        }
    }

    entries
}

fn build_condensed_transcript_from_cursor(content: &[u8]) -> Vec<Entry> {
    let mut entries = Vec::new();

    for raw_line in content.split(|byte| *byte == b'\n') {
        if raw_line.is_empty() {
            continue;
        }

        let Ok(value) = serde_json::from_slice::<Value>(raw_line) else {
            continue;
        };
        let role = value
            .get("role")
            .or_else(|| value.get("type"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let content = value
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        if content.is_empty() {
            continue;
        }

        match role {
            "user" => entries.push(Entry {
                entry_type: EntryType::User,
                content: crate::utils::text::strip_ide_context_tags(&content),
                tool_name: String::new(),
                tool_detail: String::new(),
            }),
            "assistant" => entries.push(Entry {
                entry_type: EntryType::Assistant,
                content,
                tool_name: String::new(),
                tool_detail: String::new(),
            }),
            _ => {}
        }
    }

    entries
}

pub fn build_condensed_transcript_from_bytes(
    content: &[u8],
    agent_type: AgentType,
) -> Result<Vec<Entry>> {
    match agent_type {
        AgentType::Cursor => Ok(build_condensed_transcript_from_cursor(content)),
        AgentType::Gemini => build_condensed_transcript_from_gemini(content),
        AgentType::OpenCode => Ok(build_condensed_transcript_from_opencode(content)),
        AgentType::ClaudeCode | AgentType::Unknown => {
            let lines = parse_claude_lines(content);
            Ok(build_condensed_transcript(&lines))
        }
    }
}

pub fn format_condensed_transcript(input: Input) -> String {
    if input.transcript.is_empty() && input.files_touched.is_empty() {
        return String::new();
    }

    let mut out = String::new();

    for (idx, entry) in input.transcript.iter().enumerate() {
        if idx > 0 {
            out.push('\n');
        }

        match entry.entry_type {
            EntryType::User => {
                out.push_str("[User] ");
                out.push_str(&entry.content);
                out.push('\n');
            }
            EntryType::Assistant => {
                out.push_str("[Assistant] ");
                out.push_str(&entry.content);
                out.push('\n');
            }
            EntryType::Tool => {
                out.push_str("[Tool] ");
                out.push_str(&entry.tool_name);
                if !entry.tool_detail.is_empty() {
                    out.push_str(": ");
                    out.push_str(&entry.tool_detail);
                }
                out.push('\n');
            }
        }
    }

    if !input.files_touched.is_empty() {
        if !input.transcript.is_empty() {
            out.push('\n');
        }
        out.push_str("[Files Modified]\n");
        for file in input.files_touched {
            out.push_str("- ");
            out.push_str(&file);
            out.push('\n');
        }
    }

    out
}

/// Slices a full session transcript down to the portion that belongs to a single checkpoint.
/// Claude/OpenCode transcripts are JSONL; `start_offset` is a line count.
/// Gemini transcripts are a single JSON blob; `start_offset` is a message index.
pub fn scope_transcript_for_checkpoint(
    full_transcript: &[u8],
    start_offset: usize,
    agent_type: AgentType,
) -> Vec<u8> {
    match agent_type {
        AgentType::Gemini => crate::adapters::agents::gemini::transcript::slice_from_message(
            full_transcript,
            start_offset,
        )
        .unwrap_or_else(|| full_transcript.to_vec()),
        AgentType::ClaudeCode | AgentType::Cursor | AgentType::OpenCode | AgentType::Unknown => {
            crate::host::transcript::parse::slice_from_line(full_transcript, start_offset)
        }
    }
}

pub fn generate_from_transcript(
    transcript_bytes: &[u8],
    files_touched: &[String],
    agent_type: AgentType,
    generator: Option<&dyn Generator>,
) -> Result<Summary> {
    if transcript_bytes.is_empty() {
        return Err(anyhow!("empty transcript"));
    }

    let condensed = build_condensed_transcript_from_bytes(transcript_bytes, agent_type)
        .context("failed to parse transcript")?;
    if condensed.is_empty() {
        return Err(anyhow!("transcript has no content to summarize"));
    }

    let input = Input {
        transcript: condensed,
        files_touched: files_touched.to_vec(),
    };

    let default_generator;
    let selected_generator: &dyn Generator = match generator {
        Some(generator) => generator,
        None => {
            default_generator = ClaudeGenerator::default();
            &default_generator
        }
    };

    selected_generator
        .generate(input)
        .context("failed to generate summary")
}

#[cfg(test)]
#[path = "summarize_tests.rs"]
mod tests;

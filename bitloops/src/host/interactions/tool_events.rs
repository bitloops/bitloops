use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use serde::Deserialize;
use serde_json::Value;

use crate::host::checkpoints::transcript::parse::parse_from_bytes;
use crate::host::checkpoints::transcript::types::{
    AssistantMessage, CONTENT_TYPE_TOOL_USE, TYPE_ASSISTANT, TYPE_USER,
};
use crate::host::interactions::types::{InteractionEvent, InteractionEventType};

pub(crate) const INTERACTION_SOURCE_LIVE_HOOK: &str = "live_hook";
pub(crate) const INTERACTION_SOURCE_TRANSCRIPT_DERIVATION: &str = "transcript_derivation";

const MAX_SUMMARY_CHARS: usize = 4_000;

pub(crate) struct DerivedToolEventContext<'a> {
    pub(crate) repo_id: &'a str,
    pub(crate) session_id: &'a str,
    pub(crate) turn_id: &'a str,
    pub(crate) branch: &'a str,
    pub(crate) actor_id: &'a str,
    pub(crate) actor_name: &'a str,
    pub(crate) actor_email: &'a str,
    pub(crate) actor_source: &'a str,
    pub(crate) event_time: &'a str,
    pub(crate) agent_type: &'a str,
    pub(crate) model: &'a str,
    pub(crate) transcript_path: &'a str,
}

#[derive(Debug, Clone, Default)]
struct DerivedToolInput {
    summary: String,
    command: String,
    command_binary: String,
    command_argv: Vec<String>,
}

#[derive(Debug, Clone, Default)]
struct PendingTool {
    tool_name: String,
    is_subagent_task: bool,
}

#[derive(Debug, Deserialize, Default)]
struct ToolResultMessage {
    #[serde(default)]
    content: Value,
}

#[derive(Debug, Deserialize, Default)]
struct ToolResultBlock {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    tool_use_id: String,
    #[serde(default)]
    content: Value,
}

pub(crate) fn derive_tool_events_from_transcript_fragment(
    ctx: &DerivedToolEventContext<'_>,
    transcript_fragment: &str,
) -> Result<Vec<InteractionEvent>> {
    if transcript_fragment.trim().is_empty() || ctx.turn_id.trim().is_empty() {
        return Ok(Vec::new());
    }

    let lines = parse_from_bytes(transcript_fragment.as_bytes())?;
    let mut sequence_number = 1_i64;
    let mut tool_use_block_number = 1_i64;
    let mut pending_tools = HashMap::<String, PendingTool>::new();
    let mut events = Vec::new();

    for line in lines {
        match line.r#type.as_str() {
            TYPE_ASSISTANT => {
                let Ok(message) = serde_json::from_value::<AssistantMessage>(line.message) else {
                    continue;
                };
                for block in message.content {
                    if block.r#type != CONTENT_TYPE_TOOL_USE {
                        continue;
                    }

                    let tool_name = block.name.trim().to_string();
                    if tool_name.is_empty() {
                        continue;
                    }

                    let is_subagent_task = tool_name.eq_ignore_ascii_case("task");
                    // Fallback correlation ids must be unique per observed tool_use block,
                    // not per emitted event. Task blocks are skipped as events, but they
                    // still consume a position in the transcript.
                    let fallback_tool_use_block_number = tool_use_block_number;
                    tool_use_block_number += 1;
                    let tool_use_id = if block.id.trim().is_empty() {
                        format!("{}:tool:{fallback_tool_use_block_number:04}", ctx.turn_id)
                    } else {
                        block.id.trim().to_string()
                    };
                    let input = derive_tool_input(&tool_name, &block.input);
                    pending_tools.insert(
                        tool_use_id.clone(),
                        PendingTool {
                            tool_name: tool_name.clone(),
                            is_subagent_task,
                        },
                    );

                    if is_subagent_task {
                        continue;
                    }

                    let event = InteractionEvent {
                        event_id: transcript_derived_event_id(
                            ctx.turn_id,
                            "tool-invocation",
                            sequence_number,
                            &tool_use_id,
                        ),
                        session_id: ctx.session_id.to_string(),
                        turn_id: Some(ctx.turn_id.to_string()),
                        repo_id: ctx.repo_id.to_string(),
                        branch: ctx.branch.to_string(),
                        actor_id: ctx.actor_id.to_string(),
                        actor_name: ctx.actor_name.to_string(),
                        actor_email: ctx.actor_email.to_string(),
                        actor_source: ctx.actor_source.to_string(),
                        event_type: InteractionEventType::ToolInvocationObserved,
                        event_time: ctx.event_time.to_string(),
                        source: INTERACTION_SOURCE_TRANSCRIPT_DERIVATION.to_string(),
                        sequence_number,
                        agent_type: ctx.agent_type.to_string(),
                        model: ctx.model.to_string(),
                        tool_use_id: tool_use_id.clone(),
                        tool_kind: tool_name.clone(),
                        task_description: input.summary.clone(),
                        subagent_id: String::new(),
                        payload: serde_json::json!({
                            "source": INTERACTION_SOURCE_TRANSCRIPT_DERIVATION,
                            "sequence_number": sequence_number,
                            "tool_name": tool_name,
                            "tool_input": block.input,
                            "input_summary": input.summary,
                            "command": input.command,
                            "command_binary": input.command_binary,
                            "command_argv": input.command_argv,
                            "transcript_path": ctx.transcript_path,
                        }),
                    };
                    events.push(event);
                    sequence_number += 1;
                }
            }
            TYPE_USER => {
                let Ok(message) = serde_json::from_value::<ToolResultMessage>(line.message) else {
                    continue;
                };
                let Ok(blocks) = serde_json::from_value::<Vec<ToolResultBlock>>(message.content)
                else {
                    continue;
                };
                for block in blocks {
                    if block.kind != "tool_result" {
                        continue;
                    }
                    let tool_use_id = block.tool_use_id.trim();
                    if tool_use_id.is_empty() {
                        continue;
                    }
                    let pending = pending_tools.get(tool_use_id);
                    if pending.is_some_and(|tool| tool.is_subagent_task) {
                        continue;
                    }

                    let output_summary = summarise_tool_result(&block.content);
                    if pending.is_none() && looks_like_subagent_result(&output_summary) {
                        continue;
                    }

                    let tool_name = pending
                        .map(|tool| tool.tool_name.clone())
                        .unwrap_or_default();
                    let event = InteractionEvent {
                        event_id: transcript_derived_event_id(
                            ctx.turn_id,
                            "tool-result",
                            sequence_number,
                            tool_use_id,
                        ),
                        session_id: ctx.session_id.to_string(),
                        turn_id: Some(ctx.turn_id.to_string()),
                        repo_id: ctx.repo_id.to_string(),
                        branch: ctx.branch.to_string(),
                        actor_id: ctx.actor_id.to_string(),
                        actor_name: ctx.actor_name.to_string(),
                        actor_email: ctx.actor_email.to_string(),
                        actor_source: ctx.actor_source.to_string(),
                        event_type: InteractionEventType::ToolResultObserved,
                        event_time: ctx.event_time.to_string(),
                        source: INTERACTION_SOURCE_TRANSCRIPT_DERIVATION.to_string(),
                        sequence_number,
                        agent_type: ctx.agent_type.to_string(),
                        model: ctx.model.to_string(),
                        tool_use_id: tool_use_id.to_string(),
                        tool_kind: tool_name.clone(),
                        task_description: output_summary.clone(),
                        subagent_id: String::new(),
                        payload: serde_json::json!({
                            "source": INTERACTION_SOURCE_TRANSCRIPT_DERIVATION,
                            "sequence_number": sequence_number,
                            "tool_name": tool_name,
                            "output_summary": output_summary,
                            "transcript_path": ctx.transcript_path,
                        }),
                    };
                    events.push(event);
                    sequence_number += 1;
                }
            }
            _ => {}
        }
    }

    Ok(events)
}

pub(crate) fn transcript_derived_turn_end_sequence(events: &[InteractionEvent]) -> i64 {
    events
        .iter()
        .map(|event| event.sequence_number)
        .max()
        .unwrap_or_default()
        + 1
}

fn derive_tool_input(tool_name: &str, input: &Value) -> DerivedToolInput {
    let command = value_string_field(input, &["command"]);
    let command_argv = parse_shell_words(&command);
    let command_binary = command_argv
        .first()
        .map(|value| command_binary_name(value))
        .unwrap_or_default();

    let summary = match tool_name.trim().to_ascii_lowercase().as_str() {
        "read" => first_non_empty_value(
            input,
            &["file_path", "path", "notebook_path", "url", "pattern"],
        ),
        "write" | "edit" | "multiedit" => {
            first_non_empty_value(input, &["file_path", "path", "description"])
        }
        "bash" => first_non_empty_value(input, &["command", "description"]),
        "grep" | "glob" => first_non_empty_value(input, &["pattern", "path", "description"]),
        "webfetch" => first_non_empty_value(input, &["url", "description"]),
        _ => first_non_empty_value(
            input,
            &[
                "description",
                "command",
                "file_path",
                "path",
                "pattern",
                "url",
                "prompt",
                "notebook_path",
            ],
        ),
    };

    let summary = if summary.is_empty() {
        truncate_summary(normalise_text(input.to_string()))
    } else {
        truncate_summary(summary)
    };

    DerivedToolInput {
        summary,
        command,
        command_binary,
        command_argv,
    }
}

fn first_non_empty_value(value: &Value, keys: &[&str]) -> String {
    for key in keys {
        let candidate = value_string_field(value, &[*key]);
        if !candidate.is_empty() {
            return candidate;
        }
    }
    String::new()
}

fn value_string_field(value: &Value, keys: &[&str]) -> String {
    let Some(map) = value.as_object() else {
        return String::new();
    };
    for key in keys {
        let Some(candidate) = map.get(*key) else {
            continue;
        };
        match candidate {
            Value::String(text) if !text.trim().is_empty() => return text.trim().to_string(),
            Value::Number(number) => return number.to_string(),
            Value::Bool(flag) => return flag.to_string(),
            _ => {}
        }
    }
    String::new()
}

fn summarise_tool_result(content: &Value) -> String {
    match content {
        Value::String(text) => truncate_summary(normalise_text(text)),
        Value::Array(items) => {
            let mut text_parts = Vec::new();
            for item in items {
                if let Some(text) = item
                    .as_object()
                    .and_then(|value| value.get("text"))
                    .and_then(Value::as_str)
                {
                    if !text.trim().is_empty() {
                        text_parts.push(text.trim().to_string());
                    }
                } else if let Some(text) = item.as_str()
                    && !text.trim().is_empty()
                {
                    text_parts.push(text.trim().to_string());
                }
            }
            if text_parts.is_empty() {
                truncate_summary(normalise_text(content.to_string()))
            } else {
                truncate_summary(normalise_text(text_parts.join("\n")))
            }
        }
        Value::Null => String::new(),
        _ => truncate_summary(normalise_text(content.to_string())),
    }
}

fn looks_like_subagent_result(output_summary: &str) -> bool {
    output_summary.contains("agentId:")
}

fn transcript_derived_event_id(
    turn_id: &str,
    kind: &str,
    sequence_number: i64,
    correlation: &str,
) -> String {
    let correlation = correlation
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == ':' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    format!("{turn_id}:{kind}:{sequence_number:04}:{correlation}")
}

fn command_binary_name(value: &str) -> String {
    Path::new(value)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(value)
        .to_string()
}

fn truncate_summary(value: String) -> String {
    if value.chars().count() <= MAX_SUMMARY_CHARS {
        return value;
    }
    value.chars().take(MAX_SUMMARY_CHARS).collect()
}

fn normalise_text(value: impl AsRef<str>) -> String {
    value
        .as_ref()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_shell_words(command: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut chars = command.chars().peekable();
    let mut quote = None::<char>;

    while let Some(ch) = chars.next() {
        match quote {
            Some(active) if ch == active => {
                quote = None;
            }
            Some(_) => current.push(ch),
            None => match ch {
                '\'' | '"' => quote = Some(ch),
                '\\' => {
                    if let Some(next) = chars.next() {
                        current.push(next);
                    }
                }
                ch if ch.is_whitespace() => {
                    if !current.is_empty() {
                        words.push(std::mem::take(&mut current));
                    }
                }
                _ => current.push(ch),
            },
        }
    }

    if !current.is_empty() {
        words.push(current);
    }

    words
}

#[cfg(test)]
mod tests {
    use super::{
        DerivedToolEventContext, INTERACTION_SOURCE_TRANSCRIPT_DERIVATION,
        derive_tool_events_from_transcript_fragment, parse_shell_words,
        transcript_derived_turn_end_sequence,
    };
    use crate::host::interactions::types::InteractionEventType;

    fn context<'a>() -> DerivedToolEventContext<'a> {
        DerivedToolEventContext {
            repo_id: "repo-1",
            session_id: "session-1",
            turn_id: "turn-1",
            branch: "main",
            actor_id: "actor-1",
            actor_name: "Alice",
            actor_email: "alice@example.com",
            actor_source: "workos",
            event_time: "2026-04-21T10:00:00Z",
            agent_type: "codex",
            model: "gpt-5.4",
            transcript_path: "/tmp/transcript.jsonl",
        }
    }

    #[test]
    fn derives_tool_invocation_and_result_events_from_transcript_fragment() {
        let fragment = concat!(
            "{\"type\":\"assistant\",\"uuid\":\"a1\",\"message\":{\"content\":[",
            "{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"Read\",\"input\":{\"file_path\":\"src/lib.rs\"}},",
            "{\"type\":\"tool_use\",\"id\":\"toolu_2\",\"name\":\"Bash\",\"input\":{\"command\":\"rg interaction_events src\"}}",
            "]}}\n",
            "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"content\":[",
            "{\"type\":\"tool_result\",\"tool_use_id\":\"toolu_2\",\"content\":\"found matches\"}",
            "]}}\n"
        );

        let events = derive_tool_events_from_transcript_fragment(&context(), fragment)
            .expect("derive transcript tool events");
        assert_eq!(events.len(), 3);
        assert_eq!(
            events[0].event_type,
            InteractionEventType::ToolInvocationObserved
        );
        assert_eq!(events[0].tool_use_id, "toolu_1");
        assert_eq!(events[0].tool_kind, "Read");
        assert_eq!(events[0].task_description, "src/lib.rs");
        assert_eq!(events[0].source, INTERACTION_SOURCE_TRANSCRIPT_DERIVATION);
        assert_eq!(
            events[1].event_type,
            InteractionEventType::ToolInvocationObserved
        );
        assert_eq!(events[1].tool_use_id, "toolu_2");
        assert_eq!(events[1].tool_kind, "Bash");
        assert_eq!(events[1].task_description, "rg interaction_events src");
        assert_eq!(events[1].payload["command_binary"].as_str(), Some("rg"));
        assert_eq!(
            events[1].payload["command_argv"]
                .as_array()
                .expect("argv array")
                .len(),
            3
        );
        assert_eq!(
            events[2].event_type,
            InteractionEventType::ToolResultObserved
        );
        assert_eq!(events[2].tool_use_id, "toolu_2");
        assert_eq!(events[2].tool_kind, "Bash");
        assert_eq!(events[2].task_description, "found matches");
        assert_eq!(events[2].sequence_number, 3);
        assert_eq!(transcript_derived_turn_end_sequence(&events), 4);
    }

    #[test]
    fn ignores_subagent_task_tool_usage_when_deriving_ordinary_tool_events() {
        let fragment = concat!(
            "{\"type\":\"assistant\",\"uuid\":\"a1\",\"message\":{\"content\":[",
            "{\"type\":\"tool_use\",\"id\":\"toolu_task\",\"name\":\"Task\",\"input\":{\"prompt\":\"delegate\"}}",
            "]}}\n",
            "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"content\":[",
            "{\"type\":\"tool_result\",\"tool_use_id\":\"toolu_task\",\"content\":\"agentId: sub123\"}",
            "]}}\n"
        );

        let events = derive_tool_events_from_transcript_fragment(&context(), fragment)
            .expect("derive transcript tool events");
        assert!(events.is_empty());
    }

    #[test]
    fn idless_tool_uses_after_subagent_tasks_receive_unique_fallback_ids() {
        let fragment = concat!(
            "{\"type\":\"assistant\",\"uuid\":\"a1\",\"message\":{\"content\":[",
            "{\"type\":\"tool_use\",\"name\":\"Task\",\"input\":{\"prompt\":\"delegate\"}},",
            "{\"type\":\"tool_use\",\"name\":\"Edit\",\"input\":{\"file_path\":\"src/lib.rs\"}}",
            "]}}\n",
            "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":{\"content\":[",
            "{\"type\":\"tool_result\",\"tool_use_id\":\"turn-1:tool:0002\",\"content\":\"updated file\"}",
            "]}}\n"
        );

        let events = derive_tool_events_from_transcript_fragment(&context(), fragment)
            .expect("derive transcript tool events");
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0].event_type,
            InteractionEventType::ToolInvocationObserved
        );
        assert_eq!(events[0].tool_use_id, "turn-1:tool:0002");
        assert_eq!(events[0].tool_kind, "Edit");
        assert_eq!(
            events[1].event_type,
            InteractionEventType::ToolResultObserved
        );
        assert_eq!(events[1].tool_use_id, "turn-1:tool:0002");
        assert_eq!(events[1].tool_kind, "Edit");
    }

    #[test]
    fn shell_word_parser_handles_basic_quoting() {
        let argv = parse_shell_words(r#"rg "tool events" src/host"#);
        assert_eq!(argv, vec!["rg", "tool events", "src/host"]);
    }
}

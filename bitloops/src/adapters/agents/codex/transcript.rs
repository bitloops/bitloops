use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::Result;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::adapters::agents::TranscriptToolEventDeriver;
use crate::host::interactions::tool_events::TranscriptToolEventObservation;

use super::agent::CodexAgent;

#[derive(Debug, Deserialize, Default)]
struct CodexEventMessage {
    #[serde(rename = "type", default)]
    record_type: String,
    #[serde(default)]
    payload: CodexEventPayload,
}

#[derive(Debug, Deserialize, Default)]
struct CodexEventPayload {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    call_id: String,
    #[serde(default)]
    arguments: String,
    #[serde(default)]
    output: String,
    #[serde(default)]
    command: Value,
    #[serde(default)]
    aggregated_output: String,
    #[serde(default)]
    stdout: String,
    #[serde(default)]
    stderr: String,
    #[serde(default)]
    formatted_output: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    exit_code: i64,
}

impl TranscriptToolEventDeriver for CodexAgent {
    fn derive_transcript_tool_event_observations(
        &self,
        _turn_id: &str,
        transcript_fragment: &str,
    ) -> Result<Vec<TranscriptToolEventObservation>> {
        derive_exec_command_observations(transcript_fragment)
    }
}

pub fn derive_exec_command_observations(
    transcript_fragment: &str,
) -> Result<Vec<TranscriptToolEventObservation>> {
    let mut pending_exec_commands = HashMap::<String, Value>::new();
    let mut handled_exec_commands = HashSet::<String>::new();
    let mut observations = Vec::new();

    for raw_line in transcript_fragment.lines() {
        if raw_line.trim().is_empty() {
            continue;
        }

        let Ok(record) = serde_json::from_str::<CodexEventMessage>(raw_line) else {
            continue;
        };
        match record.record_type.as_str() {
            "response_item"
                if record.payload.kind == "function_call"
                    && record.payload.name == "exec_command" =>
            {
                let tool_use_id = record.payload.call_id.trim();
                if tool_use_id.is_empty() {
                    continue;
                }

                let command = exec_command_string_from_arguments(&record.payload.arguments);
                if command.is_empty() {
                    continue;
                }

                let tool_input = json!({ "command": command });
                pending_exec_commands.insert(tool_use_id.to_string(), tool_input.clone());
                observations.push(TranscriptToolEventObservation::Invocation {
                    tool_use_id: tool_use_id.to_string(),
                    tool_name: "Bash".to_string(),
                    tool_input,
                });
            }
            "response_item" if record.payload.kind == "function_call_output" => {
                let tool_use_id = record.payload.call_id.trim();
                let Some(_tool_input) = pending_exec_commands.get(tool_use_id) else {
                    continue;
                };
                observations.push(TranscriptToolEventObservation::Result {
                    tool_use_id: tool_use_id.to_string(),
                    tool_name: "Bash".to_string(),
                    tool_output: Value::String(exec_command_output_from_function_call_output(
                        &record.payload.output,
                    )),
                });
                let _ = pending_exec_commands.remove(tool_use_id);
                handled_exec_commands.insert(tool_use_id.to_string());
            }
            "event_msg" if record.payload.kind == "exec_command_end" => {
                let tool_use_id = record.payload.call_id.trim();
                if tool_use_id.is_empty()
                    || pending_exec_commands.contains_key(tool_use_id)
                    || handled_exec_commands.contains(tool_use_id)
                {
                    continue;
                }

                let command = exec_command_string(&record.payload.command);
                if command.is_empty() {
                    continue;
                }

                observations.push(TranscriptToolEventObservation::Invocation {
                    tool_use_id: tool_use_id.to_string(),
                    tool_name: "Bash".to_string(),
                    tool_input: json!({ "command": command }),
                });
                observations.push(TranscriptToolEventObservation::Result {
                    tool_use_id: tool_use_id.to_string(),
                    tool_name: "Bash".to_string(),
                    tool_output: Value::String(exec_command_output_summary(&record.payload)),
                });
                handled_exec_commands.insert(tool_use_id.to_string());
            }
            _ => {}
        }
    }

    Ok(observations)
}

fn exec_command_string_from_arguments(arguments: &str) -> String {
    let Ok(arguments) = serde_json::from_str::<Value>(arguments) else {
        return String::new();
    };
    first_non_empty_string(&arguments, &["cmd", "command"])
}

fn exec_command_output_from_function_call_output(output: &str) -> String {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if let Some((_, content)) = trimmed.rsplit_once("\nOutput:\n") {
        return content.trim().to_string();
    }
    if let Some((_, content)) = trimmed.rsplit_once("Output:\n") {
        return content.trim().to_string();
    }
    trimmed.to_string()
}

fn exec_command_string(command: &Value) -> String {
    match command {
        Value::String(text) => text.trim().to_string(),
        Value::Array(items) => {
            let argv = items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>();
            if argv.is_empty() {
                return String::new();
            }

            if argv.len() >= 3
                && matches!(
                    command_binary_name(&argv[0]).as_str(),
                    "sh" | "bash" | "zsh" | "fish"
                )
                && matches!(argv[1].as_str(), "-c" | "-lc")
            {
                return argv[2].clone();
            }

            argv.join(" ")
        }
        _ => String::new(),
    }
}

fn exec_command_output_summary(payload: &CodexEventPayload) -> String {
    for candidate in [
        payload.aggregated_output.as_str(),
        payload.formatted_output.as_str(),
        payload.stderr.as_str(),
        payload.stdout.as_str(),
    ] {
        if !candidate.trim().is_empty() {
            return candidate.trim().to_string();
        }
    }

    if !payload.status.trim().is_empty() && payload.exit_code != 0 {
        return format!(
            "status: {}, exit_code: {}",
            payload.status.trim(),
            payload.exit_code
        );
    }
    if !payload.status.trim().is_empty() {
        return payload.status.trim().to_string();
    }
    if payload.exit_code != 0 {
        return format!("exit_code: {}", payload.exit_code);
    }

    String::new()
}

fn first_non_empty_string(value: &Value, keys: &[&str]) -> String {
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

fn command_binary_name(value: &str) -> String {
    Path::new(value)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(value)
        .to_string()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::derive_exec_command_observations;
    use crate::host::interactions::tool_events::TranscriptToolEventObservation;

    #[test]
    fn derives_codex_exec_command_observations_from_event_msg_transcript_fragment() {
        let fragment = concat!(
            "{\"timestamp\":\"2026-04-22T13:05:44.610Z\",\"type\":\"event_msg\",\"payload\":",
            "{\"type\":\"exec_command_end\",\"call_id\":\"call_u5mNwNUlD5uFD65canwRQbto\",",
            "\"command\":[\"/bin/zsh\",\"-lc\",\"git log -1 --date=iso-strict --format='%H%n%cd%n%cn%n%s'\"],",
            "\"aggregated_output\":\"06fe1f9aa7f98eb98e973df6d916703552ab7ce0\\n2026-04-17T23:49:07+03:00\\nVasilis Danias\\nUpdate bitloops-platform-embeddings.\\n\",",
            "\"exit_code\":0,\"status\":\"completed\"}}\n"
        );

        let observations = derive_exec_command_observations(fragment).expect("derive observations");

        assert_eq!(
            observations,
            vec![
                TranscriptToolEventObservation::Invocation {
                    tool_use_id: "call_u5mNwNUlD5uFD65canwRQbto".to_string(),
                    tool_name: "Bash".to_string(),
                    tool_input: json!({"command":"git log -1 --date=iso-strict --format='%H%n%cd%n%cn%n%s'"}),
                },
                TranscriptToolEventObservation::Result {
                    tool_use_id: "call_u5mNwNUlD5uFD65canwRQbto".to_string(),
                    tool_name: "Bash".to_string(),
                    tool_output: json!(
                        "06fe1f9aa7f98eb98e973df6d916703552ab7ce0\n2026-04-17T23:49:07+03:00\nVasilis Danias\nUpdate bitloops-platform-embeddings."
                    ),
                },
            ]
        );
    }

    #[test]
    fn derives_codex_exec_command_observations_from_response_item_transcript_fragment() {
        let fragment = concat!(
            "{\"timestamp\":\"2026-04-22T14:41:58.589Z\",\"type\":\"response_item\",\"payload\":",
            "{\"type\":\"function_call\",\"name\":\"exec_command\",",
            "\"arguments\":\"{\\\"cmd\\\":\\\"git log -1 --date=iso-strict --format='%H%n%cd%n%cn%n%s'\\\",\\\"shell\\\":\\\"bash\\\",\\\"login\\\":false}\",",
            "\"call_id\":\"call_nigk0HhtHOw611keZb2CvHU5\"}}\n",
            "{\"timestamp\":\"2026-04-22T14:41:58.678Z\",\"type\":\"response_item\",\"payload\":",
            "{\"type\":\"function_call_output\",\"call_id\":\"call_nigk0HhtHOw611keZb2CvHU5\",",
            "\"output\":\"Chunk ID: b48ae5\\nWall time: 0.0000 seconds\\nProcess exited with code 0\\nOriginal token count: 62\\nOutput:\\n06fe1f9aa7f98eb98e973df6d916703552ab7ce0\\n2026-04-17T23:49:07+03:00\\nVasilis Danias\\nUpdate bitloops-platform-embeddings.\\n\"}}\n"
        );

        let observations = derive_exec_command_observations(fragment).expect("derive observations");

        assert_eq!(
            observations,
            vec![
                TranscriptToolEventObservation::Invocation {
                    tool_use_id: "call_nigk0HhtHOw611keZb2CvHU5".to_string(),
                    tool_name: "Bash".to_string(),
                    tool_input: json!({"command":"git log -1 --date=iso-strict --format='%H%n%cd%n%cn%n%s'"}),
                },
                TranscriptToolEventObservation::Result {
                    tool_use_id: "call_nigk0HhtHOw611keZb2CvHU5".to_string(),
                    tool_name: "Bash".to_string(),
                    tool_output: json!(
                        "06fe1f9aa7f98eb98e973df6d916703552ab7ce0\n2026-04-17T23:49:07+03:00\nVasilis Danias\nUpdate bitloops-platform-embeddings."
                    ),
                },
            ]
        );
    }

    #[test]
    fn avoids_duplicate_codex_exec_command_observations_when_both_forms_exist() {
        let fragment = concat!(
            "{\"timestamp\":\"2026-04-22T14:41:58.589Z\",\"type\":\"response_item\",\"payload\":",
            "{\"type\":\"function_call\",\"name\":\"exec_command\",",
            "\"arguments\":\"{\\\"cmd\\\":\\\"git status --short\\\"}\",",
            "\"call_id\":\"call_status\"}}\n",
            "{\"timestamp\":\"2026-04-22T14:41:58.678Z\",\"type\":\"response_item\",\"payload\":",
            "{\"type\":\"function_call_output\",\"call_id\":\"call_status\",",
            "\"output\":\"Output:\\n M src/lib.rs\\n\"}}\n",
            "{\"timestamp\":\"2026-04-22T14:41:58.800Z\",\"type\":\"event_msg\",\"payload\":",
            "{\"type\":\"exec_command_end\",\"call_id\":\"call_status\",",
            "\"command\":[\"/bin/zsh\",\"-lc\",\"git status --short\"],",
            "\"aggregated_output\":\" M src/lib.rs\\n\",\"exit_code\":0,\"status\":\"completed\"}}\n"
        );

        let observations = derive_exec_command_observations(fragment).expect("derive observations");

        assert_eq!(
            observations,
            vec![
                TranscriptToolEventObservation::Invocation {
                    tool_use_id: "call_status".to_string(),
                    tool_name: "Bash".to_string(),
                    tool_input: json!({"command":"git status --short"}),
                },
                TranscriptToolEventObservation::Result {
                    tool_use_id: "call_status".to_string(),
                    tool_name: "Bash".to_string(),
                    tool_output: json!("M src/lib.rs"),
                },
            ]
        );
    }
}

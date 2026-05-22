use anyhow::{Result, anyhow, ensure};

use crate::host::interactions::types::{
    InteractionEvent, InteractionEventFilter, InteractionEventType,
};
use crate::host::runtime_store::RepoSqliteRuntimeStore;

use super::distillation::{GuidanceDistillationInput, GuidanceToolEvidence};

pub(crate) struct HistoryGuidanceInputSelector<'a> {
    pub(crate) repo_id: &'a str,
    pub(crate) checkpoint_id: Option<&'a str>,
    pub(crate) session_id: &'a str,
    pub(crate) turn_id: Option<&'a str>,
}

pub(crate) fn hydrate_history_guidance_input(
    repo_root: &std::path::Path,
    selector: HistoryGuidanceInputSelector<'_>,
) -> Result<GuidanceDistillationInput> {
    let spool = RepoSqliteRuntimeStore::open(repo_root)?.interaction_spool()?;
    ensure!(
        spool.repo_id() == selector.repo_id,
        "interaction spool repo_id mismatch for context guidance history distillation: expected {} got {}",
        selector.repo_id,
        spool.repo_id()
    );
    let turn = spool
        .list_turns_for_session(selector.session_id, 500)?
        .into_iter()
        .find(|turn| {
            selector
                .turn_id
                .is_none_or(|expected| turn.turn_id == expected)
                && selector
                    .checkpoint_id
                    .is_none_or(|expected| turn.checkpoint_id.as_deref() == Some(expected))
        })
        .ok_or_else(|| {
            anyhow!(
                "interaction turn not found for context guidance history distillation: session_id={} turn_id={}",
                selector.session_id,
                selector.turn_id.unwrap_or("<none>")
            )
        })?;
    let mut events = Vec::new();
    for event_type in [
        InteractionEventType::ToolInvocationObserved,
        InteractionEventType::ToolResultObserved,
    ] {
        events.extend(spool.list_events(
            &InteractionEventFilter {
                session_id: Some(selector.session_id.to_string()),
                turn_id: Some(turn.turn_id.clone()),
                event_type: Some(event_type),
                since: None,
            },
            100,
        )?);
    }
    events.sort_by(|left, right| {
        left.event_time
            .cmp(&right.event_time)
            .then_with(|| left.event_id.cmp(&right.event_id))
    });
    Ok(GuidanceDistillationInput {
        checkpoint_id: turn
            .checkpoint_id
            .clone()
            .or_else(|| selector.checkpoint_id.map(str::to_string)),
        session_id: turn.session_id,
        turn_id: Some(turn.turn_id),
        event_time: turn
            .ended_at
            .clone()
            .or_else(|| Some(turn.started_at.clone())),
        agent_type: non_empty_string(turn.agent_type),
        model: non_empty_string(turn.model),
        prompt: non_empty_string(turn.prompt),
        transcript_fragment: non_empty_string(turn.transcript_fragment),
        files_modified: turn.files_modified,
        tool_events: events.iter().map(tool_evidence_from_event).collect(),
    })
}

fn tool_evidence_from_event(event: &InteractionEvent) -> GuidanceToolEvidence {
    GuidanceToolEvidence {
        event_type: Some(event.event_type.as_str().to_string()),
        tool_kind: non_empty_string(event.tool_kind.clone()),
        input_summary: payload_string(&event.payload, "input_summary")
            .or_else(|| non_empty_string(event.task_description.clone())),
        output_summary: payload_string(&event.payload, "output_summary"),
        command: payload_string(&event.payload, "command"),
        file_path: event_file_path(event),
        evidence_text: event_evidence_text(event),
    }
}

fn event_file_path(event: &InteractionEvent) -> Option<String> {
    payload_string_at(&event.payload, &["tool_input", "file_path"])
        .or_else(|| payload_string_at(&event.payload, &["tool_response", "filePath"]))
        .or_else(|| payload_string_at(&event.payload, &["tool_response", "file", "filePath"]))
        .or_else(|| payload_string(&event.payload, "input_summary"))
        .filter(|value| value.contains('/'))
}

fn event_evidence_text(event: &InteractionEvent) -> Option<String> {
    let tool = event.tool_kind.to_ascii_lowercase();
    match tool.as_str() {
        "write" | "edit" | "multiedit" => write_edit_evidence_text(&event.payload),
        "bash" => bash_evidence_text(&event.payload),
        "askuserquestion" => payload_string(&event.payload, "output_summary")
            .or_else(|| payload_string_at(&event.payload, &["tool_response", "answers"])),
        _ => payload_string(&event.payload, "output_summary"),
    }
}

fn write_edit_evidence_text(payload: &serde_json::Value) -> Option<String> {
    let path = payload_string_at(payload, &["tool_input", "file_path"])
        .or_else(|| payload_string_at(payload, &["tool_response", "filePath"]));
    let content = payload_string_at(payload, &["tool_input", "content"])
        .or_else(|| payload_string_at(payload, &["tool_response", "content"]));
    let patch = payload
        .pointer("/tool_response/structuredPatch")
        .and_then(|value| serde_json::to_string(value).ok())
        .filter(|value| value != "[]");

    match (path, content, patch) {
        (Some(path), Some(content), Some(patch)) => Some(format!(
            "path: {path}\ncontent:\n{content}\nstructuredPatch:\n{patch}"
        )),
        (Some(path), Some(content), None) => Some(format!("path: {path}\ncontent:\n{content}")),
        (Some(path), None, Some(patch)) => Some(format!("path: {path}\nstructuredPatch:\n{patch}")),
        (None, Some(content), Some(patch)) => {
            Some(format!("content:\n{content}\nstructuredPatch:\n{patch}"))
        }
        (None, None, Some(patch)) => Some(format!("structuredPatch:\n{patch}")),
        (None, Some(content), None) => Some(content),
        (Some(path), None, None) => Some(format!("path: {path}")),
        (None, None, None) => None,
    }
}

fn bash_evidence_text(payload: &serde_json::Value) -> Option<String> {
    let command = payload_string(payload, "command")
        .or_else(|| payload_string_at(payload, &["tool_input", "command"]));
    let output = payload_string(payload, "output_summary")
        .or_else(|| payload_string_at(payload, &["tool_response", "stdout"]))
        .or_else(|| payload_string_at(payload, &["tool_response", "stderr"]));
    match (command, output) {
        (Some(command), Some(output)) => Some(format!("command: {command}\noutput:\n{output}")),
        (Some(command), None) => Some(format!("command: {command}")),
        (None, Some(output)) => Some(output),
        (None, None) => None,
    }
}

fn payload_string(payload: &serde_json::Value, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn payload_string_at(payload: &serde_json::Value, path: &[&str]) -> Option<String> {
    let mut current = payload;
    for segment in path {
        current = current.get(*segment)?;
    }
    match current {
        serde_json::Value::String(value) => non_empty_string(value.clone()),
        value if value.is_object() || value.is_array() => {
            serde_json::to_string(value).ok().and_then(non_empty_string)
        }
        _ => None,
    }
}

fn non_empty_string(value: String) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn tool_evidence_extracts_lowercase_write_payload() {
        let event = InteractionEvent {
            event_type: InteractionEventType::ToolInvocationObserved,
            tool_kind: "write".to_string(),
            payload: json!({
                "tool_input": {
                    "file_path": "src/api/response.rs",
                    "content": "pub struct HttpResponse;"
                }
            }),
            ..InteractionEvent::default()
        };

        let evidence = tool_evidence_from_event(&event);

        assert_eq!(evidence.file_path.as_deref(), Some("src/api/response.rs"));
        assert!(
            evidence
                .evidence_text
                .as_deref()
                .is_some_and(|text| text.contains("pub struct HttpResponse;"))
        );
    }

    #[test]
    fn tool_evidence_extracts_lowercase_bash_output() {
        let event = InteractionEvent {
            event_type: InteractionEventType::ToolResultObserved,
            tool_kind: "bash".to_string(),
            payload: json!({
                "tool_input": {
                    "command": "cargo nextest run context_guidance"
                },
                "tool_response": {
                    "stderr": "test result: ok. 4 passed"
                }
            }),
            ..InteractionEvent::default()
        };

        let evidence = tool_evidence_from_event(&event);

        assert!(
            evidence
                .evidence_text
                .as_deref()
                .is_some_and(|text| text.contains("cargo nextest run context_guidance"))
        );
        assert!(
            evidence
                .evidence_text
                .as_deref()
                .is_some_and(|text| text.contains("4 passed"))
        );
    }
}

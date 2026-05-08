use anyhow::{Result, anyhow, ensure};

use crate::host::interactions::store::InteractionSpool;
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
    let events = spool.list_events(
        &InteractionEventFilter {
            session_id: Some(selector.session_id.to_string()),
            turn_id: Some(turn.turn_id.clone()),
            event_type: Some(InteractionEventType::ToolInvocationObserved),
            since: None,
        },
        50,
    )?;
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
        tool_kind: non_empty_string(event.tool_kind.clone()),
        input_summary: payload_string(&event.payload, "input_summary")
            .or_else(|| non_empty_string(event.task_description.clone())),
        output_summary: payload_string(&event.payload, "output_summary"),
        command: payload_string(&event.payload, "command"),
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

fn non_empty_string(value: String) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

use anyhow::{Result, anyhow};

use super::adapter::LifecycleAgentAdapter;
use super::handlers_session::{handle_lifecycle_session_start, handle_lifecycle_turn_start};
use super::handlers_tail::{
    handle_lifecycle_compaction, handle_lifecycle_session_end, handle_lifecycle_subagent_end,
    handle_lifecycle_subagent_start, handle_lifecycle_todo_checkpoint,
    handle_lifecycle_tool_invocation, handle_lifecycle_tool_result,
};
use super::turn_end::handle_lifecycle_turn_end;
use super::types::{LifecycleEvent, LifecycleEventType};

pub fn dispatch_lifecycle_event(
    agent: Option<&dyn LifecycleAgentAdapter>,
    event: Option<&LifecycleEvent>,
) -> Result<()> {
    let Some(agent) = agent else {
        return Err(anyhow!("agent is required"));
    };

    let Some(event) = event else {
        return Err(anyhow!("event is required"));
    };

    match event.event_type.as_ref() {
        Some(LifecycleEventType::SessionStart) => handle_lifecycle_session_start(agent, event),
        Some(LifecycleEventType::TurnStart) => handle_lifecycle_turn_start(agent, event),
        Some(LifecycleEventType::TurnEnd) => handle_lifecycle_turn_end(agent, event),
        Some(LifecycleEventType::Compaction) => handle_lifecycle_compaction(agent, event),
        Some(LifecycleEventType::SessionEnd) => handle_lifecycle_session_end(agent, event),
        Some(LifecycleEventType::ToolInvocationObserved) => {
            handle_lifecycle_tool_invocation(agent, event)
        }
        Some(LifecycleEventType::ToolResultObserved) => handle_lifecycle_tool_result(agent, event),
        Some(LifecycleEventType::SubagentStart) => handle_lifecycle_subagent_start(agent, event),
        Some(LifecycleEventType::SubagentEnd) => handle_lifecycle_subagent_end(agent, event),
        Some(LifecycleEventType::TodoCheckpoint) => handle_lifecycle_todo_checkpoint(agent, event),
        Some(LifecycleEventType::Unknown(_)) | None => Err(anyhow!("unknown lifecycle event type")),
    }
}

use std::path::Path;

use anyhow::{Result, anyhow};

use super::adapter::LifecycleAgentAdapter;
use super::handlers_session::{
    handle_lifecycle_session_start_for_repo, handle_lifecycle_turn_start_for_repo,
};
use super::handlers_tail::{
    handle_lifecycle_compaction_for_repo, handle_lifecycle_session_end_for_repo,
    handle_lifecycle_subagent_end_for_repo, handle_lifecycle_subagent_start_for_repo,
    handle_lifecycle_todo_checkpoint_for_repo, handle_lifecycle_tool_invocation_for_repo,
    handle_lifecycle_tool_result_for_repo,
};
use super::turn_end::handle_lifecycle_turn_end_for_repo;
use super::types::{LifecycleEvent, LifecycleEventType};

pub fn dispatch_lifecycle_event(
    agent: Option<&dyn LifecycleAgentAdapter>,
    event: Option<&LifecycleEvent>,
) -> Result<()> {
    let repo_root = crate::utils::paths::repo_root()?;
    dispatch_lifecycle_event_for_repo(&repo_root, agent, event)
}

pub fn dispatch_lifecycle_event_for_repo(
    repo_root: &Path,
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
        Some(LifecycleEventType::SessionStart) => {
            handle_lifecycle_session_start_for_repo(repo_root, agent, event)
        }
        Some(LifecycleEventType::TurnStart) => {
            handle_lifecycle_turn_start_for_repo(repo_root, agent, event)
        }
        Some(LifecycleEventType::TurnEnd) => {
            handle_lifecycle_turn_end_for_repo(repo_root, agent, event)
        }
        Some(LifecycleEventType::Compaction) => {
            handle_lifecycle_compaction_for_repo(repo_root, agent, event)
        }
        Some(LifecycleEventType::SessionEnd) => {
            handle_lifecycle_session_end_for_repo(repo_root, agent, event)
        }
        Some(LifecycleEventType::ToolInvocationObserved) => {
            handle_lifecycle_tool_invocation_for_repo(repo_root, agent, event)
        }
        Some(LifecycleEventType::ToolResultObserved) => {
            handle_lifecycle_tool_result_for_repo(repo_root, agent, event)
        }
        Some(LifecycleEventType::SubagentStart) => {
            handle_lifecycle_subagent_start_for_repo(repo_root, agent, event)
        }
        Some(LifecycleEventType::SubagentEnd) => {
            handle_lifecycle_subagent_end_for_repo(repo_root, agent, event)
        }
        Some(LifecycleEventType::TodoCheckpoint) => {
            handle_lifecycle_todo_checkpoint_for_repo(repo_root, agent, event)
        }
        Some(LifecycleEventType::Unknown(_)) | None => Err(anyhow!("unknown lifecycle event type")),
    }
}

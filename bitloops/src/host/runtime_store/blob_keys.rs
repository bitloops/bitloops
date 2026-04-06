use crate::utils::paths;
use crate::utils::paths::TRANSCRIPT_FILE_NAME;

use super::types::{RuntimeMetadataBlobType, TaskCheckpointArtefact};
pub(crate) fn session_snapshot_blob_key(
    repo_id: &str,
    session_id: &str,
    snapshot_id: &str,
    blob_type: RuntimeMetadataBlobType,
) -> String {
    format!(
        "runtime/{repo_id}/session-metadata/{session_id}/{snapshot_id}/{}",
        blob_type.default_file_name()
    )
}

pub(crate) fn task_artefact_blob_key(repo_id: &str, artefact: &TaskCheckpointArtefact) -> String {
    let file_name = match artefact.kind {
        RuntimeMetadataBlobType::TaskCheckpoint => paths::CHECKPOINT_FILE_NAME.to_string(),
        RuntimeMetadataBlobType::Prompt => paths::PROMPT_FILE_NAME.to_string(),
        RuntimeMetadataBlobType::SubagentTranscript => {
            if artefact.agent_id.trim().is_empty() {
                "agent.jsonl".to_string()
            } else {
                format!("agent-{}.jsonl", artefact.agent_id)
            }
        }
        RuntimeMetadataBlobType::IncrementalCheckpoint => {
            if let Some(sequence) = artefact.incremental_sequence {
                format!("{sequence:03}-{}.json", artefact.tool_use_id)
            } else {
                "incremental-checkpoint.json".to_string()
            }
        }
        RuntimeMetadataBlobType::Transcript => TRANSCRIPT_FILE_NAME.to_string(),
        RuntimeMetadataBlobType::Prompts => paths::PROMPT_FILE_NAME.to_string(),
        RuntimeMetadataBlobType::Summary => paths::SUMMARY_FILE_NAME.to_string(),
        RuntimeMetadataBlobType::Context => paths::CONTEXT_FILE_NAME.to_string(),
    };

    format!(
        "runtime/{repo_id}/task-checkpoint-artefacts/{}/{}/{}/{}",
        artefact.session_id, artefact.tool_use_id, artefact.artefact_id, file_name
    )
}

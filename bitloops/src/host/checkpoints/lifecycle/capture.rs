use anyhow::{Result, anyhow};
use std::path::Path;

use crate::adapters::agents::TranscriptPositionProvider;
use crate::host::checkpoints::session::create_session_backend_or_local;

/// Captures pre-prompt state (including transcript position from the agent) for consumption at turn end.
///
/// **Orchestration stub:** currently saves transcript_offset 0 without calling the agent.
/// Implement by calling `agent.get_transcript_position(session_ref)` and persisting that offset.
pub fn capture_pre_prompt_state(
    agent: &dyn TranscriptPositionProvider,
    session_id: &str,
    session_ref: &str,
    repo_root: &Path,
) -> Result<()> {
    use crate::host::checkpoints::session::state::PrePromptState as SessionPrePromptState;
    use std::time::{SystemTime, UNIX_EPOCH};

    if session_id.is_empty() {
        return Err(anyhow!(
            "session_id is required for capture_pre_prompt_state"
        ));
    }

    let transcript_offset = agent.get_transcript_position(session_ref)?;
    let backend = create_session_backend_or_local(repo_root);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let state = SessionPrePromptState {
        session_id: session_id.to_string(),
        timestamp: format!("{}", timestamp),
        transcript_path: session_ref.to_string(),
        transcript_offset: transcript_offset as i64,
        ..SessionPrePromptState::default()
    };
    backend.save_pre_prompt(&state)
}

use anyhow::{Result, anyhow};
use std::path::Path;

use crate::adapters::agents::TranscriptPositionProvider;
use crate::host::checkpoints::session::create_session_backend_or_local;

/// Captures pre-prompt state for consumption at turn end, including the
/// transcript position returned by `agent.get_transcript_position(session_ref)`.
pub fn capture_pre_prompt_state(
    agent: &dyn TranscriptPositionProvider,
    session_id: &str,
    session_ref: &str,
    repo_root: &Path,
) -> Result<()> {
    use crate::host::checkpoints::session::state::PrePromptState as SessionPrePromptState;

    use super::time_and_ids::now_rfc3339;

    if session_id.is_empty() {
        return Err(anyhow!(
            "session_id is required for capture_pre_prompt_state"
        ));
    }

    let transcript_offset = agent.get_transcript_position(session_ref)?;
    let backend = create_session_backend_or_local(repo_root);
    let state = SessionPrePromptState {
        session_id: session_id.to_string(),
        timestamp: now_rfc3339(),
        transcript_path: session_ref.to_string(),
        transcript_offset: transcript_offset as i64,
        ..SessionPrePromptState::default()
    };
    backend.save_pre_prompt(&state)
}

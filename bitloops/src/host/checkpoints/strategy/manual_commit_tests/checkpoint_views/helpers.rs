use super::*;
use crate::host::checkpoints::session::state::PendingCheckpointState;

pub(crate) fn write_session_transcript(
    repo_root: &Path,
    session_id: &str,
    transcript_jsonl: &str,
) -> PathBuf {
    let transcript_path = repo_root.join(format!("{session_id}-transcript.jsonl"));
    fs::write(&transcript_path, transcript_jsonl).unwrap();
    transcript_path
}

pub(crate) fn idle_state(
    session_id: &str,
    base_commit: &str,
    files_touched: Vec<String>,
    step_count: u32,
) -> SessionState {
    SessionState {
        session_id: session_id.to_string(),
        phase: crate::host::checkpoints::session::phase::SessionPhase::Idle,
        base_commit: base_commit.to_string(),
        pending: PendingCheckpointState {
            files_touched,
            step_count,
            ..Default::default()
        },
        agent_type: "claude-code".to_string(),
        ..Default::default()
    }
}

pub(crate) fn condense_with_transcript(
    strategy: &ManualCommitStrategy,
    state: &mut SessionState,
    checkpoint_id: &str,
    new_head: &str,
    transcript_jsonl: &str,
) {
    let transcript_path =
        write_session_transcript(&strategy.repo_root, &state.session_id, transcript_jsonl);
    state.transcript_path = transcript_path.to_string_lossy().to_string();
    strategy
        .condense_session(state, checkpoint_id, new_head)
        .unwrap();
}

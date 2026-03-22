fn write_session_transcript(repo_root: &Path, session_id: &str, transcript_jsonl: &str) -> PathBuf {
    let meta_dir = repo_root.join(paths::session_metadata_dir_from_session_id(session_id));
    fs::create_dir_all(&meta_dir).unwrap();
    let transcript_path = meta_dir.join(paths::TRANSCRIPT_FILE_NAME);
    fs::write(&transcript_path, transcript_jsonl).unwrap();
    transcript_path
}

fn idle_state(
    session_id: &str,
    base_commit: &str,
    files_touched: Vec<String>,
    step_count: u32,
) -> SessionState {
    SessionState {
        session_id: session_id.to_string(),
        phase: crate::host::checkpoints::session::phase::SessionPhase::Idle,
        base_commit: base_commit.to_string(),
        files_touched,
        step_count,
        agent_type: "claude-code".to_string(),
        ..Default::default()
    }
}

fn condense_with_transcript(
    strategy: &ManualCommitStrategy,
    state: &mut SessionState,
    checkpoint_id: &str,
    new_head: &str,
    transcript_jsonl: &str,
) {
    let transcript_path = write_session_transcript(&strategy.repo_root, &state.session_id, transcript_jsonl);
    state.transcript_path = transcript_path.to_string_lossy().to_string();
    strategy
        .condense_session(state, checkpoint_id, new_head)
        .unwrap();
}

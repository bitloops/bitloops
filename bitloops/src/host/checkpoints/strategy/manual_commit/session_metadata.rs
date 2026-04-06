use super::*;

use crate::host::checkpoints::transcript::metadata::{
    SessionMetadataBundle, build_session_metadata_bundle, extract_user_prompts_from_jsonl,
};
use crate::host::runtime_store::{RepoSqliteRuntimeStore, SessionMetadataSnapshot};

// ── Session metadata helpers ──────────────────────────────────────────────────

pub(crate) fn write_session_metadata(
    repo_root: &Path,
    session_id: &str,
    transcript_path: &str,
) -> Result<Vec<String>> {
    if transcript_path.is_empty() {
        return Ok(vec![]);
    }

    let meta = fs::symlink_metadata(transcript_path)
        .with_context(|| format!("stat transcript path: {transcript_path}"))?;
    if meta.file_type().is_symlink() {
        anyhow::bail!("refusing symlink transcript path: {transcript_path}");
    }

    let transcript = match read_transcript_with_retry(transcript_path) {
        Some(transcript) => transcript,
        None => return Ok(vec![]),
    };

    let prompts = extract_user_prompts_from_jsonl(&transcript);
    let last_prompt = prompts.last().cloned().unwrap_or_default();
    let bundle = build_session_metadata_bundle(
        session_id,
        &generate_commit_message(&last_prompt),
        transcript.as_bytes(),
    )?;

    let runtime_store = RepoSqliteRuntimeStore::open(repo_root)
        .context("opening runtime store for session metadata snapshot")?;
    let mut snapshot = SessionMetadataSnapshot::new(session_id.to_string(), bundle.clone());
    snapshot.transcript_path = transcript_path.to_string();
    runtime_store
        .save_session_metadata_snapshot(&snapshot)
        .context("saving session metadata snapshot to runtime store")?;

    Ok(bundle
        .logical_entries(session_id)
        .into_iter()
        .map(|(path, _)| path)
        .collect())
}

/// Retries transcript reads briefly to handle asynchronous transcript flushing.
pub(crate) fn read_transcript_with_retry(transcript_path: &str) -> Option<String> {
    use std::thread::sleep;
    use std::time::{Duration, Instant};

    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Ok(transcript) = fs::read_to_string(transcript_path) {
            return Some(transcript);
        }
        if Instant::now() >= deadline {
            return None;
        }
        sleep(Duration::from_millis(50));
    }
}

pub(crate) fn read_session_metadata_bundle(
    repo_root: &Path,
    session_id: &str,
) -> Option<SessionMetadataBundle> {
    RepoSqliteRuntimeStore::open(repo_root)
        .ok()?
        .load_latest_session_metadata_snapshot(session_id)
        .ok()?
        .map(|snapshot| snapshot.bundle)
}

pub(crate) fn generate_commit_message(prompt: &str) -> String {
    commit_message::generate_commit_message(prompt)
}

#[cfg(test)]
mod session_metadata_inline_tests {
    use crate::host::checkpoints::transcript::metadata::{
        extract_summary_from_jsonl, extract_user_prompts_from_jsonl,
    };

    #[test]
    fn extract_user_prompts_supports_copilot_user_message_payloads() {
        let jsonl = r#"{"type":"user.message","data":{"content":"Create hello.txt"}}
{"type":"user.message","data":{"content":"","transformedContent":"Refactor parser"}}
"#;
        assert_eq!(
            extract_user_prompts_from_jsonl(jsonl),
            vec!["Create hello.txt", "Refactor parser"]
        );
    }

    #[test]
    fn extract_summary_supports_assistant_message_payloads() {
        let jsonl = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"First"}]}}
{"type":"assistant","message":{"content":[{"type":"text","text":"Final summary"}]}}"#;
        assert_eq!(extract_summary_from_jsonl(jsonl), "Final summary");
    }
}

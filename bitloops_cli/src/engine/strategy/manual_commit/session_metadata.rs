// ── Session metadata helpers ──────────────────────────────────────────────────

/// Writes a snapshot of the session transcript to `.bitloops/metadata/<session_id>/`.
///
/// Files written:
/// - `full.jsonl` — copy of the live transcript
/// - `prompt.txt` — user prompts extracted from the JSONL
///
/// Returns the list of repo-relative paths written (for inclusion in the shadow branch tree).
///
fn write_session_metadata(
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

    // Claude writes transcript entries asynchronously; retry briefly before giving up.
    let transcript = match read_transcript_with_retry(transcript_path) {
        Some(t) => t,
        None => return Ok(vec![]), // transcript not available yet — skip silently
    };

    let rel_base = paths::session_metadata_dir_from_session_id(session_id);
    let meta_dir = repo_root.join(&rel_base);
    fs::create_dir_all(&meta_dir).context("creating session metadata directory")?;

    // Write transcript snapshot.
    fs::write(meta_dir.join(paths::TRANSCRIPT_FILE_NAME), &transcript)
        .context("writing session full.jsonl")?;

    let prompt_path = meta_dir.join(paths::PROMPT_FILE_NAME);
    let summary_path = meta_dir.join(paths::SUMMARY_FILE_NAME);
    let context_path = meta_dir.join(paths::CONTEXT_FILE_NAME);

    // Preserve lifecycle-authored metadata if already present.
    // Lifecycle writes prompt/summary/context before strategy SaveStep.
    if !prompt_path.exists() || !summary_path.exists() || !context_path.exists() {
        let prompts = extract_user_prompts_from_jsonl(&transcript);
        let prompt_txt = prompts.join("\n\n---\n\n");
        let summary_txt = extract_summary_from_jsonl(&transcript);

        if !prompt_path.exists() {
            fs::write(&prompt_path, &prompt_txt).context("writing session prompt.txt")?;
        }
        if !summary_path.exists() {
            fs::write(&summary_path, &summary_txt).context("writing session summary.txt")?;
        }
        if !context_path.exists() {
            let last_prompt = prompts.last().cloned().unwrap_or_default();
            let context_md = build_context_md(
                session_id,
                &generate_commit_message(&last_prompt),
                &prompts,
                &summary_txt,
            );
            fs::write(&context_path, context_md).context("writing session context.md")?;
        }
    }

    Ok(vec![
        format!("{rel_base}/{}", paths::TRANSCRIPT_FILE_NAME),
        format!("{rel_base}/{}", paths::PROMPT_FILE_NAME),
        format!("{rel_base}/{}", paths::SUMMARY_FILE_NAME),
        format!("{rel_base}/{}", paths::CONTEXT_FILE_NAME),
    ])
}

/// Retries transcript reads briefly to handle asynchronous transcript flushing.
fn read_transcript_with_retry(transcript_path: &str) -> Option<String> {
    use std::thread::sleep;
    use std::time::{Duration, Instant};

    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Ok(t) = fs::read_to_string(transcript_path) {
            return Some(t);
        }
        if Instant::now() >= deadline {
            return None;
        }
        sleep(Duration::from_millis(50));
    }
}

/// Falls back to reading the transcript from the metadata directory on disk.
fn read_transcript_from_disk(repo_root: &Path, session_id: &str) -> Option<String> {
    let path = repo_root
        .join(paths::session_metadata_dir_from_session_id(session_id))
        .join(paths::TRANSCRIPT_FILE_NAME);
    fs::read_to_string(path).ok().filter(|s| !s.is_empty())
}

/// Extracts user-role prompt text from a Claude Code JSONL transcript.
///
/// Each line of a Claude Code transcript is a JSON object. We look for lines
/// where `role == "user"` and extract `content` as text.
///
fn extract_user_prompts_from_jsonl(jsonl: &str) -> Vec<String> {
    let mut prompts = Vec::new();
    for line in jsonl.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(val) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if !is_user_role(transcript_line_role(&val)) {
            continue;
        }
        let Some(content) = transcript_line_content(&val) else {
            continue;
        };
        let text = content_to_text(content);
        if !text.trim().is_empty() {
            prompts.push(text);
        }
    }
    prompts
}

fn is_user_role(role: Option<&str>) -> bool {
    matches!(role, Some("user") | Some("human"))
}

/// Extracts the last assistant text block as a session summary.
fn extract_summary_from_jsonl(jsonl: &str) -> String {
    let mut last_summary = String::new();
    for line in jsonl.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(val) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if transcript_line_role(&val) != Some("assistant") {
            continue;
        }
        let Some(content) = transcript_line_content(&val) else {
            continue;
        };
        let text = content_to_text(content);
        if !text.trim().is_empty() {
            last_summary = text;
        }
    }
    last_summary
}

fn transcript_line_role(val: &serde_json::Value) -> Option<&str> {
    val.get("message")
        .and_then(|m| m.get("role"))
        .and_then(|r| r.as_str())
        .or_else(|| val.get("role").and_then(|r| r.as_str()))
        .or_else(|| val.get("type").and_then(|r| r.as_str()))
}

fn transcript_line_content(val: &serde_json::Value) -> Option<&serde_json::Value> {
    val.get("message")
        .and_then(|m| m.get("content"))
        .or_else(|| val.get("content"))
}

fn content_to_text(content: &serde_json::Value) -> String {
    match content {
        serde_json::Value::String(s) => s.trim().to_string(),
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|item| {
                if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                    item.get("text").and_then(|t| t.as_str()).map(str::to_owned)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
            .trim()
            .to_string(),
        _ => String::new(),
    }
}

/// Builds a context markdown file mirroring the lifecycle output structure.
fn build_context_md(
    session_id: &str,
    commit_message: &str,
    prompts: &[String],
    summary: &str,
) -> String {
    let mut out = String::new();
    out.push_str("# Session Context\n\n");
    out.push_str(&format!("Session ID: {session_id}\n"));
    out.push_str(&format!("Commit Message: {commit_message}\n\n"));

    if !prompts.is_empty() {
        out.push_str("## Prompts\n\n");
        for (idx, prompt) in prompts.iter().enumerate() {
            out.push_str(&format!("### Prompt {}\n\n{}\n\n", idx + 1, prompt));
        }
    }

    if !summary.trim().is_empty() {
        out.push_str("## Summary\n\n");
        out.push_str(summary);
        out.push('\n');
    }

    out
}

/// Generates a commit message from a user prompt.
fn generate_commit_message(prompt: &str) -> String {
    commit_message::generate_commit_message(prompt)
}

// ── Trailer helpers ───────────────────────────────────────────────────────────

/// Extracts the checkpoint ID from a commit message.
/// Returns `None` if no valid `Bitloops-Checkpoint: <12hex>` trailer is found.
pub fn parse_checkpoint_id(message: &str) -> Option<String> {
    let prefix = format!("{CHECKPOINT_TRAILER_KEY}: ");
    for line in message.lines() {
        if let Some(id) = line.trim().strip_prefix(&prefix) {
            let id = id.trim();
            if is_valid_checkpoint_id(id) {
                return Some(id.to_string());
            }
        }
    }
    None
}

/// Returns the `Bitloops-Checkpoint: <id>` trailer from the HEAD commit, if present.
#[allow(dead_code)]
fn get_checkpoint_id_from_head(repo_root: &Path) -> Result<Option<String>> {
    let body = run_git(repo_root, &["cat-file", "commit", "HEAD"])?;
    Ok(parse_checkpoint_id(&body))
}

/// Returns `true` if the message has any non-comment, non-trailer lines.
fn has_user_content(message: &str) -> bool {
    let trailer_prefix = format!("{CHECKPOINT_TRAILER_KEY}:");
    for line in message.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if t.starts_with('#') {
            continue;
        }
        if t.starts_with(trailer_prefix.as_str()) {
            continue;
        }
        return true;
    }
    false
}

/// Removes the `Bitloops-Checkpoint:` trailer line from a message.
fn strip_checkpoint_trailer(message: &str) -> String {
    let trailer_prefix = format!("{CHECKPOINT_TRAILER_KEY}:");
    message
        .lines()
        .filter(|l| !l.trim().starts_with(trailer_prefix.as_str()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Appends `\n\nBitloops-Checkpoint: <id>\n` to the message.
fn add_checkpoint_trailer(message: &str, id: &str) -> String {
    let trailer = format!("{CHECKPOINT_TRAILER_KEY}: {id}");
    let trimmed = message.trim_end_matches('\n');
    format!("{trimmed}\n\n{trailer}\n")
}

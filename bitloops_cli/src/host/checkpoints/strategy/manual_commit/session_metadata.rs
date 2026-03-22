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
    matches!(role, Some("user") | Some("human") | Some("user.message"))
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
        if !matches!(
            transcript_line_role(&val),
            Some("assistant") | Some("assistant.message")
        ) {
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
    let message_content = val.get("message").and_then(|m| m.get("content"));
    if message_content.is_some() {
        return message_content;
    }

    let data_content = val.get("data").and_then(|d| d.get("content"));
    if let Some(content) = data_content {
        let text = content_to_text(content);
        if !text.is_empty() {
            return Some(content);
        }
    }

    val.get("data")
        .and_then(|d| d.get("transformedContent"))
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

#[cfg(test)]
mod session_metadata_inline_tests {
    use super::{extract_summary_from_jsonl, extract_user_prompts_from_jsonl};

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
    fn extract_summary_supports_copilot_assistant_messages() {
        let jsonl = r#"{"type":"assistant.message","data":{"content":"Created hello.txt"}}
"#;
        assert_eq!(extract_summary_from_jsonl(jsonl), "Created hello.txt");
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


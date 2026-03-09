/// Format a Unix timestamp as a sortable string.
/// Returns the decimal epoch seconds so that lexicographic sort == chronological sort.
fn chrono_format(unix: i64) -> String {
    if unix == 0 {
        return String::new();
    }
    format!("{unix}")
}

pub fn run_explain_commit(
    commit_ref: &str,
    no_pager: bool,
    verbose: bool,
    full: bool,
    search_all: bool,
) -> Result<String> {
    let repo_root = paths::repo_root()?;
    run_explain_commit_in(&repo_root, commit_ref, no_pager, verbose, full, search_all)
}

pub(crate) fn run_explain_commit_in(
    repo_root: &std::path::Path,
    commit_ref: &str,
    no_pager: bool,
    verbose: bool,
    full: bool,
    search_all: bool,
) -> Result<String> {
    let sha = run_git(repo_root, &["rev-parse", "--verify", commit_ref])
        .map_err(|_| anyhow!("commit not found: {commit_ref}"))?;
    let sha = sha.trim().to_string();

    let msg = run_git(repo_root, &["show", "-s", "--format=%B", &sha])?;
    let cp_id = parse_checkpoint_id(&msg);

    let Some(cp_id) = cp_id else {
        let short_sha = &sha[..sha.len().min(7)];
        return Ok(format!(
            "No associated Bitloops checkpoint\n\nCommit {short_sha} does not have a {CHECKPOINT_TRAILER_KEY} trailer.\nThis commit was not created during a Bitloops session, or the trailer was removed.\n"
        ));
    };

    let opts = ExplainExecutionOptions {
        no_pager,
        verbose,
        full,
        raw_transcript: false,
        generate: false,
        force: false,
        search_all,
    };
    run_explain_checkpoint_in(repo_root, cp_id.as_str(), &opts)
}

pub fn output_explain_content(content: &str, no_pager: bool) -> String {
    use std::io::IsTerminal;
    if no_pager || !std::io::stdout().is_terminal() {
        return content.to_string();
    }
    let line_count = content.bytes().filter(|&b| b == b'\n').count();
    let height = terminal_size::terminal_size()
        .map(|(_, h)| h.0 as usize)
        .unwrap_or(24);
    if line_count <= height.saturating_sub(2) {
        return content.to_string();
    }
    let pager = std::env::var("PAGER").unwrap_or_else(|_| "less".to_string());
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = match Command::new(&pager).stdin(Stdio::piped()).spawn() {
        Ok(c) => c,
        Err(_) => return content.to_string(),
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(content.as_bytes());
    }
    let _ = child.wait();
    // Content has been written to pager; caller should not print it again.
    String::new()
}

fn is_default_branch(branch_name: &str) -> bool {
    matches!(branch_name, "main" | "master")
}

fn truncate_description(input: &str, max_len: usize) -> String {
    let len = input.chars().count();
    if len <= max_len {
        return input.to_string();
    }
    if max_len < 3 {
        return input.chars().take(max_len).collect();
    }
    let mut out: String = input.chars().take(max_len - 3).collect();
    out.push_str("...");
    out
}

fn extract_user_prompt(value: &Value) -> Option<String> {
    if let Some(content) = value.get("content").and_then(Value::as_str) {
        return Some(crate::engine::textutil::strip_ide_context_tags(content));
    }

    let content = value.get("message")?.get("content")?;
    if let Some(text) = content.as_str() {
        return Some(crate::engine::textutil::strip_ide_context_tags(text));
    }

    let items = content.as_array()?;
    let joined = items
        .iter()
        .filter_map(Value::as_object)
        .filter(|obj| obj.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|obj| obj.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n\n");

    if joined.is_empty() {
        None
    } else {
        Some(crate::engine::textutil::strip_ide_context_tags(&joined))
    }
}

fn extract_assistant_response(value: &Value) -> Option<String> {
    if let Some(content) = value.get("content").and_then(Value::as_str) {
        return Some(content.to_string());
    }

    let content = value.get("message")?.get("content")?.as_array()?;
    let joined = content
        .iter()
        .filter_map(Value::as_object)
        .filter(|obj| obj.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|obj| obj.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n\n");
    if joined.is_empty() {
        None
    } else {
        Some(joined)
    }
}

fn commit_trailer_matches(commit: &CommitNode, checkpoint_id: &str) -> bool {
    commit.trailers.iter().any(|(key, value)| {
        key.eq_ignore_ascii_case(CHECKPOINT_TRAILER_KEY) && value == checkpoint_id
    })
}

fn to_associated_commit(commit: &CommitNode) -> AssociatedCommit {
    let short_sha = if commit.sha.chars().count() > 7 {
        commit.sha.chars().take(7).collect()
    } else {
        commit.sha.clone()
    };
    AssociatedCommit {
        sha: commit.sha.clone(),
        short_sha,
        message: commit.message.clone(),
        author: commit.author.clone(),
        date: commit.timestamp.to_string(),
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

#[derive(Debug, Default)]
struct FileChanges {
    modified: Vec<String>,
    new_files: Vec<String>,
    deleted: Vec<String>,
}

fn detect_transcript_modified_files(
    transcript_path: &str,
    session_id: &str,
    transcript_offset: i64,
    repo_root: Option<&Path>,
) -> Vec<String> {
    if transcript_path.is_empty() {
        return vec![];
    }

    let start_line = if transcript_offset <= 0 {
        0
    } else {
        transcript_offset as usize
    };
    let subagents_dir = subagents_dir_for_session(transcript_path, session_id);
    let modified = match claude_transcript::extract_all_modified_files(
        transcript_path,
        start_line,
        &subagents_dir,
    ) {
        Ok(files) => files,
        Err(err) => {
            eprintln!("[bitloops] Warning: failed to extract modified files: {err}");
            vec![]
        }
    };

    let Some(root) = repo_root else {
        return modified;
    };
    filter_and_normalize_paths(&modified, &root.to_string_lossy())
}

/// Change detection from `git status --porcelain`.
fn detect_file_changes(
    repo_root: Option<&Path>,
    previously_untracked: Option<&[String]>,
) -> FileChanges {
    use std::collections::{BTreeSet, HashSet};

    let Some(root) = repo_root else {
        return FileChanges::default();
    };
    let Some(output) = git_status_porcelain(root) else {
        return FileChanges::default();
    };

    let pre: HashSet<String> = previously_untracked
        .unwrap_or(&[])
        .iter()
        .cloned()
        .collect();
    let mut modified = BTreeSet::new();
    let mut new_files = BTreeSet::new();
    let mut deleted = BTreeSet::new();

    for line in output.lines() {
        if line.len() < 3 {
            continue;
        }
        let status = &line[..2];
        let mut path = line[3..].trim().to_string();
        if let Some(idx) = path.rfind(" -> ") {
            path = path[idx + 4..].to_string();
        }
        if path.is_empty() || path.ends_with('/') || paths::is_infrastructure_path(&path) {
            continue;
        }

        if status == "??" {
            if previously_untracked.is_none() || !pre.contains(&path) {
                new_files.insert(path);
            }
            continue;
        }

        let x = status.as_bytes().first().copied().unwrap_or(b' ');
        let y = status.as_bytes().get(1).copied().unwrap_or(b' ');
        if x == b'D' || y == b'D' {
            deleted.insert(path);
            continue;
        }
        if x != b' ' || y != b' ' {
            modified.insert(path);
        }
    }

    FileChanges {
        modified: modified.into_iter().collect(),
        new_files: new_files.into_iter().collect(),
        deleted: deleted.into_iter().collect(),
    }
}

fn merge_unique(mut base: Vec<String>, extra: Vec<String>) -> Vec<String> {
    if extra.is_empty() {
        return base;
    }
    let mut seen: HashSet<String> = base.iter().cloned().collect();
    for path in extra {
        if seen.insert(path.clone()) {
            base.push(path);
        }
    }
    base
}

fn filter_to_uncommitted_files(repo_root: Option<&Path>, files: Vec<String>) -> Vec<String> {
    if files.is_empty() {
        return files;
    }

    let Some(root) = repo_root else {
        return files;
    };

    let head_probe = Command::new("git")
        .args(["rev-parse", "--verify", "HEAD"])
        .current_dir(root)
        .output();
    let Ok(head_probe) = head_probe else {
        return files;
    };
    if !head_probe.status.success() {
        return files;
    }

    let mut filtered = Vec::with_capacity(files.len());
    for rel_path in files {
        let head_spec = format!("HEAD:{rel_path}");
        let head_has_file = Command::new("git")
            .args(["cat-file", "-e", &head_spec])
            .current_dir(root)
            .output();
        let Ok(head_has_file) = head_has_file else {
            filtered.push(rel_path);
            continue;
        };
        if !head_has_file.status.success() {
            // File does not exist in HEAD, so it must still be uncommitted.
            filtered.push(rel_path);
            continue;
        }

        let working_content = std::fs::read(root.join(&rel_path));
        let Ok(working_content) = working_content else {
            filtered.push(rel_path);
            continue;
        };

        let head_content = Command::new("git")
            .args(["show", &head_spec])
            .current_dir(root)
            .output();
        let Ok(head_content) = head_content else {
            filtered.push(rel_path);
            continue;
        };
        if !head_content.status.success() {
            filtered.push(rel_path);
            continue;
        }

        if working_content != head_content.stdout {
            filtered.push(rel_path);
        }
    }

    filtered
}

fn filter_and_normalize_paths(files: &[String], base_path: &str) -> Vec<String> {
    let mut result = Vec::new();
    for file in files {
        let rel = paths::to_relative_path(file, base_path);
        if rel.is_empty() || rel.starts_with("..") {
            continue;
        }
        if paths::is_infrastructure_path(&rel) {
            continue;
        }
        result.push(rel);
    }
    result
}

// Test-only parser stubs: the live dispatch uses inline serde_json::from_str.
#[cfg(test)]
fn parse_task_hook_input(stdin: &str) -> Result<TaskHookInput> {
    if stdin.is_empty() {
        bail!("empty input");
    }
    serde_json::from_str(stdin).context("failed to parse JSON")
}

#[cfg(test)]
fn parse_post_task_hook_input(stdin: &str) -> Result<PostTaskInput> {
    if stdin.is_empty() {
        bail!("empty input");
    }
    serde_json::from_str(stdin).context("failed to parse JSON")
}

#[cfg(test)]
fn parse_subagent_checkpoint_hook_input(stdin: &str) -> Result<SubagentCheckpointHookInput> {
    if stdin.is_empty() {
        bail!("empty input");
    }
    serde_json::from_str(stdin).context("failed to parse JSON")
}

fn log_pre_task_hook_context(w: &mut dyn Write, input: &TaskHookInput) {
    let _ = writeln!(w, "[bitloops] PreToolUse[Task] hook invoked");
    let _ = writeln!(w, "  Session ID: {}", input.session_id);
    let _ = writeln!(w, "  Tool Use ID: {}", input.tool_use_id);
    let _ = writeln!(w, "  Transcript: {}", input.transcript_path);
}

fn log_post_task_hook_context(
    w: &mut dyn Write,
    input: &PostTaskInput,
    subagent_transcript_path: &str,
) {
    let _ = writeln!(w, "[bitloops] PostToolUse[Task] hook invoked");
    let _ = writeln!(w, "  Session ID: {}", input.session_id);
    let _ = writeln!(w, "  Tool Use ID: {}", input.tool_use_id);
    if input.tool_response.agent_id.is_empty() {
        let _ = writeln!(w, "  Agent ID: (none)");
    } else {
        let _ = writeln!(w, "  Agent ID: {}", input.tool_response.agent_id);
    }
    let _ = writeln!(w, "  Transcript: {}", input.transcript_path);
    if subagent_transcript_path.is_empty() {
        let _ = writeln!(w, "  Subagent Transcript: (none)");
    } else {
        let _ = writeln!(w, "  Subagent Transcript: {}", subagent_transcript_path);
    }
}

fn todos_json_from_tool_input(tool_input: Option<&Value>) -> Option<Vec<u8>> {
    let todos = tool_input?.get("todos")?;
    serde_json::to_vec(todos).ok()
}

// Test-only: (in-progress extraction).
// Live code uses extract_last_completed_todo_from_tool_input instead for PostTodo hooks.
#[cfg(test)]
fn extract_todo_content_from_tool_input(tool_input: Option<&Value>) -> String {
    let Some(todos_json) = todos_json_from_tool_input(tool_input) else {
        return String::new();
    };
    crate::engine::strategy::messages::extract_in_progress_todo(&todos_json)
}

fn count_todos_from_tool_input(tool_input: Option<&Value>) -> usize {
    let Some(todos_json) = todos_json_from_tool_input(tool_input) else {
        return 0;
    };
    crate::engine::strategy::messages::count_todos(&todos_json)
}

fn extract_last_completed_todo_from_tool_input(tool_input: Option<&Value>) -> String {
    let Some(todos_json) = todos_json_from_tool_input(tool_input) else {
        return String::new();
    };
    crate::engine::strategy::messages::extract_last_completed_todo(&todos_json)
}

fn parse_subagent_type_and_description(tool_input: Option<&Value>) -> (String, String) {
    let Some(input) = tool_input else {
        return (String::new(), String::new());
    };
    let subagent_type = input
        .get("subagent_type")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let task_description = input
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    (subagent_type, task_description)
}

fn resolve_subagent_transcript_path(
    transcript_path: &str,
    session_id: &str,
    agent_id: &str,
) -> String {
    if transcript_path.is_empty() || session_id.is_empty() || agent_id.is_empty() {
        return String::new();
    }
    let base = Path::new(transcript_path)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let candidate = base
        .join(session_id)
        .join("subagents")
        .join(format!("{agent_id}.jsonl"));
    if candidate.exists() {
        candidate.to_string_lossy().into_owned()
    } else {
        String::new()
    }
}

fn next_incremental_sequence(
    repo_root: Option<&Path>,
    session_id: &str,
    task_tool_use_id: &str,
) -> u32 {
    let Some(root) = repo_root else {
        return 1;
    };
    let checkpoints_dir = root
        .join(paths::session_metadata_dir_from_session_id(session_id))
        .join("tasks")
        .join(task_tool_use_id)
        .join("checkpoints");
    let Ok(entries) = fs::read_dir(checkpoints_dir) else {
        return 1;
    };
    let count = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|ext| ext.to_str()) == Some("json"))
        .count();
    (count as u32) + 1
}

fn truncate_prompt_for_storage(prompt: &str) -> String {
    strings::truncate_runes(&strings::collapse_whitespace(prompt), 100, "...")
}

fn generate_commit_message(prompt: &str) -> String {
    commit_message::generate_commit_message(prompt)
}

/// Returns current time formatted as RFC 3339 (e.g. `2024-01-15T10:30:00Z`).
fn now_rfc3339() -> String {
    // Use std::time to avoid adding a chrono dependency for now.
    // Format: seconds since epoch converted to a simple ISO 8601 timestamp.
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Convert Unix timestamp to RFC 3339.
    let (y, mo, d, h, mi, s) = unix_to_ymdhms(secs);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// Minimal Unix → calendar conversion (no leap seconds, no timezone).
fn unix_to_ymdhms(secs: u64) -> (u64, u64, u64, u64, u64, u64) {
    let s = secs % 60;
    let mins = secs / 60;
    let mi = mins % 60;
    let hours = mins / 60;
    let h = hours % 24;
    let days = hours / 24;

    // Days since 1970-01-01
    let mut year = 1970u64;
    let mut remaining = days;
    loop {
        let days_in_year = if is_leap(year) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        year += 1;
    }

    let months = [31u64, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut mo = 1u64;
    for &dm in &months {
        let dm = if mo == 2 && is_leap(year) { 29 } else { dm };
        if remaining < dm {
            break;
        }
        remaining -= dm;
        mo += 1;
    }
    let d = remaining + 1;
    (year, mo, d, h, mi, s)
}

fn is_leap(y: u64) -> bool {
    (y.is_multiple_of(4) && !y.is_multiple_of(100)) || y.is_multiple_of(400)
}

/// Best-effort list of untracked files from `git status --porcelain`.
fn detect_untracked_files(repo_root: Option<&Path>) -> Vec<String> {
    let Some(root) = repo_root else {
        return vec![];
    };
    let Some(output) = git_status_porcelain(root) else {
        return vec![];
    };

    output
        .lines()
        .filter_map(|line| {
            if !line.starts_with("?? ") || line.len() < 4 {
                return None;
            }
            let path = line[3..].trim();
            if path.is_empty() || paths::is_infrastructure_path(path) {
                None
            } else {
                Some(path.to_string())
            }
        })
        .collect()
}

fn git_status_porcelain(repo_root: &Path) -> Option<String> {
    let output = Command::new("git")
        // Include all untracked files (not just directory placeholders)
        // so change detection captures paths like `src/auth.rs` instead of `src/`.
        .args(["status", "--porcelain", "--untracked-files=all"])
        .current_dir(repo_root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).to_string())
}

fn subagents_dir_for_session(transcript_path: &str, session_id: &str) -> String {
    if transcript_path.is_empty() || session_id.is_empty() {
        return String::new();
    }
    Path::new(transcript_path)
        .parent()
        .map(|base| base.join(session_id).join("subagents"))
        .unwrap_or_default()
        .to_string_lossy()
        .to_string()
}

/// Best-effort token usage calculation for stop hooks, including subagent usage.
fn calculate_stop_token_usage(
    transcript_path: &str,
    session_id: &str,
    transcript_offset: i64,
) -> Option<crate::engine::agent::TokenUsage> {
    if transcript_path.is_empty() {
        return None;
    }
    let start_line = if transcript_offset <= 0 {
        0
    } else {
        transcript_offset as usize
    };
    let subagents_dir = subagents_dir_for_session(transcript_path, session_id);
    claude_transcript::calculate_total_token_usage(transcript_path, start_line, &subagents_dir).ok()
}

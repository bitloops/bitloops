use super::*;

// ── Git sequence detection ────────────────────────────────────────────────────

/// Returns `true` if git is currently in a rebase, cherry-pick, or revert operation.
///
pub(crate) fn is_git_sequence_operation(repo_root: &Path) -> bool {
    let git_dir = match run_git(repo_root, &["rev-parse", "--git-dir"]) {
        Ok(d) => {
            let p = Path::new(d.trim());
            if p.is_absolute() {
                p.to_path_buf()
            } else {
                repo_root.join(p)
            }
        }
        Err(_) => return false,
    };

    git_dir.join("rebase-merge").exists()
        || git_dir.join("rebase-apply").exists()
        || git_dir.join("CHERRY_PICK_HEAD").exists()
        || git_dir.join("REVERT_HEAD").exists()
}

#[cfg(test)]
pub(crate) fn has_overlapping_files(staged_files: &[String], files_touched: &[String]) -> bool {
    let touched: std::collections::HashSet<&str> =
        files_touched.iter().map(String::as_str).collect();
    staged_files.iter().any(|f| touched.contains(f.as_str()))
}

pub(crate) fn subtract_files_by_name(
    files_touched: &[String],
    committed_files: &std::collections::HashSet<String>,
) -> Vec<String> {
    files_touched
        .iter()
        .filter(|f| !committed_files.contains(f.as_str()))
        .cloned()
        .collect()
}

pub(crate) fn files_changed_in_commit(
    repo_root: &Path,
    commit_hash: &str,
) -> Result<std::collections::HashSet<String>> {
    // Initial commits do not have a parent; fall back to listing names directly from `show`.
    let has_parent = run_git(repo_root, &["rev-parse", &format!("{commit_hash}^")]).is_ok();
    let output = if has_parent {
        run_git(
            repo_root,
            &[
                "diff-tree",
                "--no-commit-id",
                "--name-only",
                "-r",
                commit_hash,
            ],
        )?
    } else {
        run_git(
            repo_root,
            &["show", "--name-only", "--pretty=format:", commit_hash],
        )?
    };

    Ok(output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect())
}

pub(crate) fn calculate_session_initial_attribution(
    repo_root: &Path,
    state: &SessionState,
    session_tree_hash: Option<&str>,
    head_commit: &str,
    files_touched: &[String],
) -> Option<serde_json::Value> {
    if files_touched.is_empty() {
        return None;
    }

    let head_tree = load_tree_snapshot(repo_root, head_commit)?;
    let session_tree = session_tree_hash
        .and_then(|tree_hash| load_tree_snapshot_from_tree_hash(repo_root, tree_hash))
        .unwrap_or_else(|| head_tree.clone());

    let attribution_base = if state.attribution_base_commit.is_empty() {
        state.base_commit.as_str()
    } else {
        state.attribution_base_commit.as_str()
    };
    let base_tree = if attribution_base.is_empty() {
        None
    } else {
        load_tree_snapshot(repo_root, attribution_base)
    };

    let attribution = calculate_attribution_with_accumulated(
        base_tree.as_ref(),
        Some(&session_tree),
        Some(&head_tree),
        files_touched,
        &state
            .prompt_attributions
            .iter()
            .map(to_strategy_prompt_attribution)
            .collect::<Vec<_>>(),
    )?;

    Some(serde_json::json!({
        "calculated_at": now_rfc3339(),
        "agent_lines": attribution.agent_lines,
        "human_added": attribution.human_added,
        "human_modified": attribution.human_modified,
        "human_removed": attribution.human_removed,
        "total_committed": attribution.total_committed,
        "agent_percentage": attribution.agent_percentage,
    }))
}

pub(crate) fn to_strategy_prompt_attribution(
    pa: &SessionPromptAttribution,
) -> StrategyPromptAttribution {
    StrategyPromptAttribution {
        checkpoint_number: pa.checkpoint_number,
        user_lines_added: pa.user_lines_added,
        user_lines_removed: pa.user_lines_removed,
        agent_lines_added: pa.agent_lines_added,
        agent_lines_removed: pa.agent_lines_removed,
        user_added_per_file: pa.user_added_per_file.clone(),
    }
}

pub(crate) fn load_tree_snapshot(repo_root: &Path, commit: &str) -> Option<TreeSnapshot> {
    let tree_ref = format!("{commit}^{{tree}}");
    load_tree_snapshot_from_treeish(repo_root, &tree_ref)
}

pub(crate) fn load_tree_snapshot_from_tree_hash(
    repo_root: &Path,
    tree_hash: &str,
) -> Option<TreeSnapshot> {
    if tree_hash.trim().is_empty() {
        return None;
    }
    load_tree_snapshot_from_treeish(repo_root, tree_hash)
}

pub(crate) fn load_tree_snapshot_from_treeish(
    repo_root: &Path,
    treeish: &str,
) -> Option<TreeSnapshot> {
    let listed = run_git(repo_root, &["ls-tree", "-r", "--name-only", treeish]).ok()?;

    let mut files: Vec<(String, String)> = Vec::new();
    for file_path in listed
        .lines()
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        let content = match git_show_file_bytes(repo_root, treeish, file_path) {
            Ok(bytes) => {
                if bytes.contains(&0) {
                    String::new()
                } else {
                    String::from_utf8_lossy(&bytes).to_string()
                }
            }
            Err(_) => continue,
        };
        files.push((file_path.to_string(), content));
    }

    Some(TreeSnapshot::from_files(files))
}

#[cfg(test)]
pub(crate) fn resolve_commit(repo_root: &Path, rev: &str) -> Option<String> {
    run_git(
        repo_root,
        &["rev-parse", "--verify", &format!("{rev}^{{commit}}")],
    )
    .ok()
}

pub(crate) fn file_hash_in_tree(repo_root: &Path, rev: &str, file_path: &str) -> Option<String> {
    run_git(repo_root, &["rev-parse", &format!("{rev}:{file_path}")]).ok()
}

#[cfg(test)]
pub(crate) fn read_blob_content(repo_root: &Path, blob_hash: &str) -> Option<String> {
    run_git(repo_root, &["cat-file", "-p", blob_hash]).ok()
}

#[cfg(test)]
pub(crate) fn staged_index_hashes(
    repo_root: &Path,
) -> Option<std::collections::HashMap<String, String>> {
    let out = run_git(repo_root, &["ls-files", "--stage"]).ok()?;
    let mut hashes = std::collections::HashMap::new();
    for line in out.lines() {
        // Format: "<mode> <hash> <stage>\t<path>"
        let (left, path) = match line.split_once('\t') {
            Some(v) => v,
            None => continue,
        };
        let mut parts = left.split_whitespace();
        let _mode = parts.next();
        let hash = match parts.next() {
            Some(v) => v,
            None => continue,
        };
        hashes.insert(path.to_string(), hash.to_string());
    }
    Some(hashes)
}

#[cfg(test)]
pub(crate) fn files_overlap_with_content(
    repo_root: &Path,
    shadow_branch_name: &str,
    head_commit: &str,
    files_touched: &[String],
) -> bool {
    if run_git(
        repo_root,
        &["cat-file", "-e", &format!("{head_commit}^{{tree}}")],
    )
    .is_err()
    {
        return !files_touched.is_empty();
    }
    let shadow_commit = match resolve_commit(repo_root, shadow_branch_name) {
        Some(c) => c,
        None => return !files_touched.is_empty(),
    };

    let parent_commit = run_git(repo_root, &["rev-parse", &format!("{head_commit}^")]).ok();

    for file_path in files_touched {
        let head_hash = file_hash_in_tree(repo_root, head_commit, file_path);
        if head_hash.is_none() {
            if let Some(parent) = parent_commit.as_deref()
                && file_hash_in_tree(repo_root, parent, file_path).is_some()
            {
                return true;
            }
            continue;
        }

        let is_modified = parent_commit
            .as_deref()
            .and_then(|p| file_hash_in_tree(repo_root, p, file_path))
            .is_some();
        if is_modified {
            return true;
        }

        let shadow_hash = match file_hash_in_tree(repo_root, &shadow_commit, file_path) {
            Some(h) => h,
            None => continue,
        };

        if head_hash.as_deref() == Some(shadow_hash.as_str()) {
            return true;
        }
    }

    false
}

#[cfg(test)]
pub(crate) fn staged_files_overlap_with_content(
    repo_root: &Path,
    shadow_branch_name: &str,
    staged_files: &[String],
    files_touched: &[String],
) -> bool {
    let touched: std::collections::HashSet<&str> =
        files_touched.iter().map(String::as_str).collect();
    let head_commit = match run_git(repo_root, &["rev-parse", "HEAD"]) {
        Ok(v) => v,
        Err(_) => return has_overlapping_files(staged_files, files_touched),
    };
    let shadow_commit = match resolve_commit(repo_root, shadow_branch_name) {
        Some(c) => c,
        None => return has_overlapping_files(staged_files, files_touched),
    };
    let index_hashes = match staged_index_hashes(repo_root) {
        Some(v) => v,
        None => return has_overlapping_files(staged_files, files_touched),
    };

    for staged_path in staged_files {
        if !touched.contains(staged_path.as_str()) {
            continue;
        }

        // Modified (or deleted) files always count as overlap.
        if file_hash_in_tree(repo_root, &head_commit, staged_path).is_some() {
            return true;
        }

        let staged_hash = match index_hashes.get(staged_path) {
            Some(h) => h,
            None => continue,
        };
        let shadow_hash = match file_hash_in_tree(repo_root, &shadow_commit, staged_path) {
            Some(h) => h,
            None => continue,
        };
        if staged_hash == &shadow_hash {
            return true;
        }

        let staged_content = match read_blob_content(repo_root, staged_hash) {
            Some(v) => v,
            None => continue,
        };
        let shadow_content = match read_blob_content(repo_root, &shadow_hash) {
            Some(v) => v,
            None => continue,
        };
        if has_significant_content_overlap(&staged_content, &shadow_content) {
            return true;
        }
    }

    false
}

pub(crate) fn files_with_remaining_agent_changes_from_tree(
    repo_root: &Path,
    session_tree_hash: Option<&str>,
    head_commit: &str,
    files_touched: &[String],
    committed_files: &std::collections::HashSet<String>,
) -> Vec<String> {
    if run_git(
        repo_root,
        &["cat-file", "-e", &format!("{head_commit}^{{tree}}")],
    )
    .is_err()
    {
        return subtract_files_by_name(files_touched, committed_files);
    }
    let Some(session_tree_hash) = session_tree_hash.filter(|value| !value.trim().is_empty()) else {
        return subtract_files_by_name(files_touched, committed_files);
    };

    let mut remaining = Vec::new();
    for file_path in files_touched {
        if !committed_files.contains(file_path) {
            remaining.push(file_path.clone());
            continue;
        }

        let session_hash = match file_hash_in_tree(repo_root, session_tree_hash, file_path) {
            Some(h) => h,
            None => continue,
        };

        match file_hash_in_tree(repo_root, head_commit, file_path) {
            Some(commit_hash) => {
                if commit_hash != session_hash {
                    remaining.push(file_path.clone());
                }
            }
            None => remaining.push(file_path.clone()),
        }
    }

    remaining
}

#[cfg(test)]
pub(crate) fn files_with_remaining_agent_changes(
    repo_root: &Path,
    shadow_branch_name: &str,
    head_commit: &str,
    files_touched: &[String],
    committed_files: &std::collections::HashSet<String>,
) -> Vec<String> {
    if run_git(
        repo_root,
        &["cat-file", "-e", &format!("{head_commit}^{{tree}}")],
    )
    .is_err()
    {
        return subtract_files_by_name(files_touched, committed_files);
    }
    let shadow_commit = match resolve_commit(repo_root, shadow_branch_name) {
        Some(c) => c,
        None => return subtract_files_by_name(files_touched, committed_files),
    };

    let mut remaining = Vec::new();
    for file_path in files_touched {
        if !committed_files.contains(file_path) {
            remaining.push(file_path.clone());
            continue;
        }

        let shadow_hash = match file_hash_in_tree(repo_root, &shadow_commit, file_path) {
            Some(h) => h,
            None => continue,
        };

        match file_hash_in_tree(repo_root, head_commit, file_path) {
            Some(commit_hash) => {
                if commit_hash != shadow_hash {
                    remaining.push(file_path.clone());
                }
            }
            None => remaining.push(file_path.clone()),
        }
    }

    remaining
}

#[cfg(test)]
pub(crate) fn has_significant_content_overlap(staged_content: &str, shadow_content: &str) -> bool {
    let shadow_lines = extract_significant_lines(shadow_content);
    let staged_lines = extract_significant_lines(staged_content);
    if shadow_lines.is_empty() || staged_lines.is_empty() {
        return false;
    }

    let required_matches = if shadow_lines.len() < 2 || staged_lines.len() < 2 {
        1
    } else {
        2
    };

    let mut matches = 0usize;
    for line in staged_lines {
        if shadow_lines.contains(&line) {
            matches += 1;
            if matches >= required_matches {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
pub(crate) fn extract_significant_lines(content: &str) -> std::collections::HashSet<String> {
    let mut lines = std::collections::HashSet::new();
    for line in content.lines() {
        let trimmed = trim_line(line);
        if trimmed.len() >= 10 {
            lines.insert(trimmed);
        }
    }
    lines
}

#[cfg(test)]
pub(crate) fn trim_line(line: &str) -> String {
    line.trim_matches(|c| c == ' ' || c == '\t').to_string()
}

// ── Timestamp helper ──────────────────────────────────────────────────────────

pub(crate) fn now_rfc3339() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    let nanos = now.subsec_nanos();
    let (y, mo, d, h, mi, s) = unix_to_ymdhms(secs);
    if nanos == 0 {
        return format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z");
    }
    let mut frac = format!("{nanos:09}");
    while frac.ends_with('0') {
        frac.pop();
    }
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{frac}Z")
}

pub(crate) fn unix_to_ymdhms(secs: u64) -> (u64, u64, u64, u64, u64, u64) {
    let s = secs % 60;
    let mins = secs / 60;
    let mi = mins % 60;
    let hours = mins / 60;
    let h = hours % 24;
    let days = hours / 24;
    let mut year = 1970u64;
    let mut remaining = days;
    loop {
        let diy = if is_leap(year) { 366 } else { 365 };
        if remaining < diy {
            break;
        }
        remaining -= diy;
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
    (year, mo, remaining + 1, h, mi, s)
}

pub(crate) fn is_leap(y: u64) -> bool {
    (y.is_multiple_of(4) && !y.is_multiple_of(100)) || y.is_multiple_of(400)
}

pub(crate) fn is_zero_i64(value: &i64) -> bool {
    *value == 0
}

pub(crate) fn is_false(value: &bool) -> bool {
    !*value
}

pub(crate) fn non_negative_i32_to_u64(value: i32) -> u64 {
    if value <= 0 { 0 } else { value as u64 }
}

pub(crate) fn accumulate_token_usage(
    existing: Option<TokenUsage>,
    incoming: &TokenUsage,
) -> TokenUsage {
    let mut combined = existing.unwrap_or_default();
    combined.input_tokens += incoming.input_tokens;
    combined.cache_creation_tokens += incoming.cache_creation_tokens;
    combined.cache_read_tokens += incoming.cache_read_tokens;
    combined.output_tokens += incoming.output_tokens;
    combined.api_call_count += incoming.api_call_count;

    combined.subagent_tokens = match (
        combined.subagent_tokens.take(),
        incoming.subagent_tokens.as_deref(),
    ) {
        (None, None) => None,
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(Box::new(accumulate_token_usage(None, right))),
        (Some(left), Some(right)) => Some(Box::new(accumulate_token_usage(Some(*left), right))),
    };

    combined
}

pub(crate) fn token_usage_metadata_from_runtime(usage: &TokenUsage) -> TokenUsageMetadata {
    TokenUsageMetadata {
        input_tokens: non_negative_i32_to_u64(usage.input_tokens),
        cache_creation_tokens: non_negative_i32_to_u64(usage.cache_creation_tokens),
        cache_read_tokens: non_negative_i32_to_u64(usage.cache_read_tokens),
        output_tokens: non_negative_i32_to_u64(usage.output_tokens),
        api_call_count: non_negative_i32_to_u64(usage.api_call_count),
        subagent_tokens: usage
            .subagent_tokens
            .as_ref()
            .map(|nested| Box::new(token_usage_metadata_from_runtime(nested))),
    }
}

pub(crate) fn calculate_token_usage_from_transcript(
    transcript: &str,
    checkpoint_transcript_start: i64,
) -> Option<TokenUsageMetadata> {
    if transcript.is_empty() {
        return None;
    }

    let start_line = checkpoint_transcript_start.max(0) as usize;
    let scoped = transcript
        .lines()
        .skip(start_line)
        .collect::<Vec<_>>()
        .join("\n");
    if scoped.trim().is_empty() {
        return None;
    }

    let parsed = claude_transcript::parse_transcript(scoped.as_bytes()).ok()?;
    let usage = claude_transcript::calculate_token_usage(&parsed);
    Some(token_usage_metadata_from_runtime(&usage))
}

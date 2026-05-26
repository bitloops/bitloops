use super::*;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CommitHunkLine {
    pub(crate) line_number: i32,
    pub(crate) content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommitFileDelta {
    pub(crate) delta_id: String,
    pub(crate) commit_sha: String,
    pub(crate) path_before: Option<String>,
    pub(crate) path_after: Option<String>,
    pub(crate) change_kind: String,
    pub(crate) old_blob_sha: Option<String>,
    pub(crate) new_blob_sha: Option<String>,
    pub(crate) is_binary: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommitHunk {
    pub(crate) hunk_id: String,
    pub(crate) delta_id: String,
    pub(crate) commit_sha: String,
    pub(crate) path_before: Option<String>,
    pub(crate) path_after: Option<String>,
    pub(crate) hunk_index: i32,
    pub(crate) old_start: i32,
    pub(crate) old_line_count: i32,
    pub(crate) new_start: i32,
    pub(crate) new_line_count: i32,
    pub(crate) added_lines: Vec<CommitHunkLine>,
    pub(crate) deleted_lines: Vec<CommitHunkLine>,
    pub(crate) patch: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ParsedCommitHunks {
    pub(crate) file_deltas: Vec<CommitFileDelta>,
    pub(crate) hunks: Vec<CommitHunk>,
}

pub(crate) fn git_show_hunk_diff(repo_root: &Path, commit_sha: &str) -> Result<String> {
    run_git(
        repo_root,
        &[
            "show",
            "--format=",
            "--no-ext-diff",
            "--unified=0",
            "--find-renames",
            "--find-copies",
            "--full-index",
            "--root",
            commit_sha,
        ],
    )
    .with_context(|| format!("reading hunk diff for commit {commit_sha}"))
}

pub(crate) fn parse_commit_hunks_from_git_show(
    repo_id: &str,
    commit_sha: &str,
    raw_diff: &str,
) -> Result<ParsedCommitHunks> {
    let mut parsed = ParsedCommitHunks::default();
    let mut section = Vec::<&str>::new();
    for line in raw_diff.lines() {
        if line.starts_with("diff --git ") && !section.is_empty() {
            parse_file_diff_section(repo_id, commit_sha, &section, &mut parsed)?;
            section.clear();
        }
        if !section.is_empty() || line.starts_with("diff --git ") {
            section.push(line);
        }
    }
    if !section.is_empty() {
        parse_file_diff_section(repo_id, commit_sha, &section, &mut parsed)?;
    }
    Ok(parsed)
}

pub(crate) fn filter_commit_hunks_by_exclusions(
    parsed: &mut ParsedCommitHunks,
    matcher: &RepoExclusionMatcher,
) {
    let mut retained_delta_ids = HashSet::new();
    parsed.file_deltas.retain(|delta| {
        let keep = !delta_is_excluded(delta, matcher);
        if keep {
            retained_delta_ids.insert(delta.delta_id.clone());
        }
        keep
    });
    parsed
        .hunks
        .retain(|hunk| retained_delta_ids.contains(&hunk.delta_id));
}

fn parse_file_diff_section(
    repo_id: &str,
    commit_sha: &str,
    lines: &[&str],
    parsed: &mut ParsedCommitHunks,
) -> Result<()> {
    let Some(diff_header) = lines.first().copied() else {
        return Ok(());
    };
    let (mut path_before, mut path_after) = parse_diff_git_header_paths(diff_header);
    let mut change_kind: Option<String> = None;
    let mut old_blob_sha = None;
    let mut new_blob_sha = None;
    let mut is_binary = false;

    for line in lines {
        if line.starts_with("new file mode ") {
            change_kind = Some("added".to_string());
        } else if line.starts_with("deleted file mode ") {
            change_kind = Some("deleted".to_string());
        } else if let Some(raw) = line.strip_prefix("rename from ") {
            path_before = normalise_diff_path(raw, None);
            change_kind = Some("renamed".to_string());
        } else if let Some(raw) = line.strip_prefix("rename to ") {
            path_after = normalise_diff_path(raw, None);
            change_kind = Some("renamed".to_string());
        } else if let Some(raw) = line.strip_prefix("copy from ") {
            path_before = normalise_diff_path(raw, None);
            change_kind = Some("copied".to_string());
        } else if let Some(raw) = line.strip_prefix("copy to ") {
            path_after = normalise_diff_path(raw, None);
            change_kind = Some("copied".to_string());
        } else if let Some(raw) = line.strip_prefix("index ") {
            let (old_blob, new_blob) = parse_index_blob_pair(raw);
            old_blob_sha = old_blob;
            new_blob_sha = new_blob;
        } else if let Some(raw) = line.strip_prefix("--- ") {
            path_before = normalise_diff_path(raw, Some("a/"));
        } else if let Some(raw) = line.strip_prefix("+++ ") {
            path_after = normalise_diff_path(raw, Some("b/"));
        } else if line.starts_with("Binary files ") {
            is_binary = true;
        }
    }

    let change_kind = change_kind.unwrap_or_else(|| infer_change_kind(&path_before, &path_after));
    let delta_id = deterministic_uuid(&format!(
        "{repo_id}|{commit_sha}|{}|{}|{change_kind}|{}|{}",
        path_before.as_deref().unwrap_or(""),
        path_after.as_deref().unwrap_or(""),
        old_blob_sha.as_deref().unwrap_or(""),
        new_blob_sha.as_deref().unwrap_or(""),
    ));
    let delta = CommitFileDelta {
        delta_id: delta_id.clone(),
        commit_sha: commit_sha.to_string(),
        path_before: path_before.clone(),
        path_after: path_after.clone(),
        change_kind,
        old_blob_sha,
        new_blob_sha,
        is_binary,
    };

    let mut hunk_index = 0i32;
    let mut idx = 0usize;
    while idx < lines.len() {
        let line = lines[idx];
        if !line.starts_with("@@ ") {
            idx += 1;
            continue;
        }

        hunk_index += 1;
        let header = parse_hunk_header(line)
            .with_context(|| format!("parsing hunk header `{line}` for commit {commit_sha}"))?;
        let mut old_line = header.old_start;
        let mut new_line = header.new_start;
        let mut patch_lines = vec![line.to_string()];
        let mut added_lines = Vec::new();
        let mut deleted_lines = Vec::new();
        idx += 1;
        while idx < lines.len() && !lines[idx].starts_with("@@ ") {
            let body_line = lines[idx];
            if body_line.starts_with("diff --git ") {
                break;
            }
            patch_lines.push(body_line.to_string());
            if let Some(content) = body_line.strip_prefix('+')
                && !body_line.starts_with("+++ ")
            {
                added_lines.push(CommitHunkLine {
                    line_number: new_line,
                    content: content.to_string(),
                });
                new_line += 1;
            } else if let Some(content) = body_line.strip_prefix('-')
                && !body_line.starts_with("--- ")
            {
                deleted_lines.push(CommitHunkLine {
                    line_number: old_line,
                    content: content.to_string(),
                });
                old_line += 1;
            } else if body_line.starts_with(' ') {
                old_line += 1;
                new_line += 1;
            }
            idx += 1;
        }

        if added_lines.is_empty() && deleted_lines.is_empty() {
            continue;
        }

        let hunk_id = deterministic_uuid(&format!(
            "{repo_id}|{commit_sha}|{delta_id}|{hunk_index}|{}|{}|{}|{}",
            header.old_start, header.old_line_count, header.new_start, header.new_line_count,
        ));
        parsed.hunks.push(CommitHunk {
            hunk_id,
            delta_id: delta_id.clone(),
            commit_sha: commit_sha.to_string(),
            path_before: path_before.clone(),
            path_after: path_after.clone(),
            hunk_index,
            old_start: header.old_start,
            old_line_count: header.old_line_count,
            new_start: header.new_start,
            new_line_count: header.new_line_count,
            added_lines,
            deleted_lines,
            patch: patch_lines.join("\n"),
        });
    }

    parsed.file_deltas.push(delta);
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct HunkHeader {
    old_start: i32,
    old_line_count: i32,
    new_start: i32,
    new_line_count: i32,
}

fn parse_hunk_header(line: &str) -> Result<HunkHeader> {
    let header = line
        .strip_prefix("@@ ")
        .and_then(|rest| rest.split_once(" @@").map(|(header, _)| header))
        .ok_or_else(|| anyhow!("invalid hunk header"))?;
    let mut parts = header.split_whitespace();
    let old_part = parts
        .next()
        .ok_or_else(|| anyhow!("missing old hunk range"))?;
    let new_part = parts
        .next()
        .ok_or_else(|| anyhow!("missing new hunk range"))?;
    let (old_start, old_line_count) = parse_hunk_range(old_part, '-')?;
    let (new_start, new_line_count) = parse_hunk_range(new_part, '+')?;
    Ok(HunkHeader {
        old_start,
        old_line_count,
        new_start,
        new_line_count,
    })
}

fn parse_hunk_range(raw: &str, prefix: char) -> Result<(i32, i32)> {
    let range = raw
        .strip_prefix(prefix)
        .ok_or_else(|| anyhow!("invalid hunk range `{raw}`"))?;
    let (start, count) = range.split_once(',').unwrap_or((range, "1"));
    Ok((start.parse()?, count.parse()?))
}

fn parse_diff_git_header_paths(line: &str) -> (Option<String>, Option<String>) {
    let Some(rest) = line.strip_prefix("diff --git ") else {
        return (None, None);
    };
    let mut parts = rest.split_whitespace();
    (
        parts
            .next()
            .and_then(|raw| normalise_diff_path(raw, Some("a/"))),
        parts
            .next()
            .and_then(|raw| normalise_diff_path(raw, Some("b/"))),
    )
}

fn parse_index_blob_pair(raw: &str) -> (Option<String>, Option<String>) {
    let pair = raw.split_whitespace().next().unwrap_or_default();
    let (old_raw, new_raw) = pair.split_once("..").unwrap_or((pair, ""));
    (normalise_blob_sha(old_raw), normalise_blob_sha(new_raw))
}

fn normalise_blob_sha(raw: &str) -> Option<String> {
    let value = raw.trim();
    if value.is_empty() || value.chars().all(|ch| ch == '0') {
        None
    } else {
        Some(value.to_string())
    }
}

fn normalise_diff_path(raw: &str, prefix: Option<&str>) -> Option<String> {
    let mut value = raw.trim().trim_matches('"').replace('\\', "/");
    if value == "/dev/null" || value.is_empty() {
        return None;
    }
    if let Some(prefix) = prefix
        && let Some(stripped) = value.strip_prefix(prefix)
    {
        value = stripped.to_string();
    }
    Some(normalize_repo_path(&value))
}

fn infer_change_kind(path_before: &Option<String>, path_after: &Option<String>) -> String {
    match (path_before, path_after) {
        (None, Some(_)) => "added",
        (Some(_), None) => "deleted",
        (Some(before), Some(after)) if before != after => "renamed",
        _ => "modified",
    }
    .to_string()
}

fn delta_is_excluded(delta: &CommitFileDelta, matcher: &RepoExclusionMatcher) -> bool {
    let before_excluded = delta
        .path_before
        .as_deref()
        .map(|path| matcher.excludes_repo_relative_path(path))
        .unwrap_or(true);
    let after_excluded = delta
        .path_after
        .as_deref()
        .map(|path| matcher.excludes_repo_relative_path(path))
        .unwrap_or(true);
    before_excluded && after_excluded
}

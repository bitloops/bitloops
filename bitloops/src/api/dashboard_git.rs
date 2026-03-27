use super::{
    ApiPage, DashboardCommitNode, DashboardUser, GIT_FIELD_SEPARATOR, GIT_RECORD_SEPARATOR,
};
use crate::host::checkpoints::strategy::manual_commit::{
    CommittedInfo, read_commit_checkpoint_mappings, read_committed_info, run_git,
};
#[cfg(test)]
use crate::utils::paths;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;

fn canonical_user_key(name: &str, email: &str) -> String {
    let email_normalized = email.trim().to_ascii_lowercase();
    if !email_normalized.is_empty() {
        return email_normalized;
    }

    let name_normalized = name.trim().to_ascii_lowercase();
    if name_normalized.is_empty() {
        return String::new();
    }
    format!("name:{name_normalized}")
}

pub(super) fn dashboard_user(name: &str, email: &str) -> DashboardUser {
    DashboardUser {
        key: canonical_user_key(name, email),
        name: name.trim().to_string(),
        email: email.trim().to_ascii_lowercase(),
    }
}

pub(super) fn canonical_agent_key(agent: &str) -> String {
    let trimmed = agent.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let mut key = String::with_capacity(trimmed.len());
    let mut last_was_dash = false;

    for ch in trimmed.chars() {
        if ch.is_ascii_alphanumeric() {
            key.push(ch.to_ascii_lowercase());
            last_was_dash = false;
        } else if !key.is_empty() && !last_was_dash {
            key.push('-');
            last_was_dash = true;
        }
    }

    while key.ends_with('-') {
        key.pop();
    }

    key
}

pub(super) fn user_matches_filter(user: &DashboardUser, user_filter: Option<&str>) -> bool {
    let Some(filter) = user_filter else {
        return true;
    };

    let normalized = filter.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return true;
    }

    user.key == normalized || user.name.to_ascii_lowercase() == normalized
}

#[cfg(test)]
fn normalize_branch_name(branch: &str) -> &str {
    let trimmed = branch.trim().trim_start_matches('*').trim();
    if let Some(short) = trimmed.strip_prefix("refs/heads/") {
        return short;
    }
    if let Some(short) = trimmed.strip_prefix("refs/remotes/") {
        return short;
    }
    trimmed
}

#[cfg(test)]
pub(super) fn branch_is_excluded(branch: &str) -> bool {
    let normalized = normalize_branch_name(branch);
    let without_origin = normalized.strip_prefix("origin/").unwrap_or(normalized);

    without_origin == paths::METADATA_BRANCH_NAME || without_origin.starts_with("bitloops/")
}

pub(super) fn build_branch_commit_log_args(
    branch_ref: &str,
    from_unix: Option<i64>,
    to_unix: Option<i64>,
    max_count: usize,
) -> Vec<String> {
    let mut args = vec![
        "log".to_string(),
        branch_ref.to_string(),
        "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%ct%x1f%s%x1e".to_string(),
        "--max-count".to_string(),
        max_count.max(1).to_string(),
        "--no-color".to_string(),
    ];

    if let Some(from) = from_unix {
        args.push(format!("--since=@{from}"));
    }
    if let Some(to) = to_unix {
        args.push(format!("--until=@{to}"));
    }
    args
}

pub(super) fn parse_branch_commit_log(raw: &str) -> Vec<DashboardCommitNode> {
    let mut nodes = Vec::new();

    for record in raw.split(GIT_RECORD_SEPARATOR) {
        let record = record.trim();
        if record.is_empty() {
            continue;
        }

        let mut parts = record.split(GIT_FIELD_SEPARATOR);
        let Some(sha) = parts.next().map(str::trim) else {
            continue;
        };
        let Some(parents_raw) = parts.next() else {
            continue;
        };
        let Some(author_name) = parts.next().map(str::trim) else {
            continue;
        };
        let Some(author_email) = parts.next().map(str::trim) else {
            continue;
        };
        let Some(timestamp_raw) = parts.next().map(str::trim) else {
            continue;
        };
        let Some(message) = parts.next().map(str::trim) else {
            continue;
        };

        if sha.is_empty() {
            continue;
        }

        let timestamp = timestamp_raw.parse::<i64>().unwrap_or(0);

        nodes.push(DashboardCommitNode {
            sha: sha.to_string(),
            parents: parents_raw
                .split_whitespace()
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect(),
            author_name: author_name.to_string(),
            author_email: author_email.to_string(),
            timestamp,
            message: message.to_string(),
            checkpoint_id: String::new(),
        });
    }

    nodes
}

pub(super) fn parse_numstat_output(raw: &str) -> HashMap<String, (u64, u64)> {
    let mut stats: HashMap<String, (u64, u64)> = HashMap::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.splitn(3, '\t').collect();
        if parts.len() != 3 {
            continue;
        }
        let adds = if parts[0] == "-" {
            0u64
        } else {
            parts[0].parse::<u64>().unwrap_or(0)
        };
        let dels = if parts[1] == "-" {
            0u64
        } else {
            parts[1].parse::<u64>().unwrap_or(0)
        };
        let path = parts[2].to_string();
        let entry = stats.entry(path).or_insert((0, 0));
        entry.0 += adds;
        entry.1 += dels;
    }
    stats
}

pub(super) fn read_commit_numstat(
    repo_root: &Path,
    sha: &str,
) -> Result<HashMap<String, (u64, u64)>> {
    let raw = run_git(
        repo_root,
        &[
            "show",
            "--numstat",
            "--format=",
            "--no-color",
            "--find-renames",
            "--find-copies",
            sha,
        ],
    )?;
    Ok(parse_numstat_output(&raw))
}

pub(super) fn walk_branch_commits_with_checkpoints(
    repo_root: &Path,
    branch_ref: &str,
    from_unix: Option<i64>,
    to_unix: Option<i64>,
    max_count: usize,
) -> Result<Vec<DashboardCommitNode>> {
    let args = build_branch_commit_log_args(branch_ref, from_unix, to_unix, max_count);
    let args_ref: Vec<&str> = args.iter().map(String::as_str).collect();
    let raw = run_git(repo_root, &args_ref)?;
    let mut commits = parse_branch_commit_log(&raw);
    attach_checkpoint_ids_from_db(repo_root, &mut commits)?;
    Ok(commits)
}

fn attach_checkpoint_ids_from_db(
    repo_root: &Path,
    commits: &mut [DashboardCommitNode],
) -> Result<()> {
    let mappings = read_commit_checkpoint_mappings(repo_root)
        .context("reading commit_checkpoints mappings for dashboard commit walk")?;
    if mappings.is_empty() {
        return Ok(());
    }

    for commit in commits {
        if let Some(checkpoint_id) = mappings.get(&commit.sha) {
            commit.checkpoint_id = checkpoint_id.clone();
        }
    }
    Ok(())
}

pub(super) fn paginate<T: Clone>(items: &[T], page: ApiPage) -> Vec<T> {
    let page = page.normalized();
    let start = page.offset.min(items.len());
    let end = start.saturating_add(page.limit).min(items.len());
    items[start..end].to_vec()
}

pub(super) fn read_checkpoint_info_for_filtering(
    repo_root: &Path,
    checkpoint_id: &str,
) -> Result<Option<CommittedInfo>> {
    read_committed_info(repo_root, checkpoint_id)
}

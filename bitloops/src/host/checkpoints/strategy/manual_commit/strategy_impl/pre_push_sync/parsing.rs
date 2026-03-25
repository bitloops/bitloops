use super::*;

pub(super) fn collect_pre_push_ref_updates(stdin_lines: &[String]) -> Vec<types::PrePushRefUpdate> {
    let mut updates = Vec::new();
    for line in stdin_lines {
        let Some(update) = parse_pre_push_update_line(line) else {
            continue;
        };
        if is_zero_git_oid(&update.local_sha) {
            continue;
        }
        updates.push(update);
    }
    updates
}

pub(super) fn parse_pre_push_update_line(line: &str) -> Option<types::PrePushRefUpdate> {
    let mut parts = line.split_whitespace();
    let local_ref = parts.next()?.trim().to_string();
    let local_sha = parts.next()?.trim().to_string();
    let remote_ref = parts.next()?.trim().to_string();
    let remote_sha = parts.next()?.trim().to_string();
    if parts.next().is_some() {
        return None;
    }

    if local_ref.is_empty()
        || local_sha.is_empty()
        || remote_ref.is_empty()
        || remote_sha.is_empty()
    {
        return None;
    }

    if local_ref == "(delete)" {
        return None;
    }

    let remote_branch = remote_ref.strip_prefix("refs/heads/")?.trim().to_string();
    if remote_branch.is_empty() {
        return None;
    }

    let local_branch = local_ref
        .strip_prefix("refs/heads/")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    Some(types::PrePushRefUpdate {
        local_ref,
        local_sha,
        remote_ref,
        remote_sha,
        local_branch,
        remote_branch,
    })
}

pub(super) fn parse_sha_lines(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in raw.lines() {
        let sha = line.trim();
        if sha.is_empty() {
            continue;
        }
        if out.iter().any(|existing| existing == sha) {
            continue;
        }
        out.push(sha.to_string());
    }
    out
}

pub(super) fn is_zero_git_oid(value: &str) -> bool {
    value.trim() == constants::ZERO_GIT_OID
}

use super::*;

// Git remote URL parsing and repository identity resolution.

pub fn resolve_repo_identity(repo_root: &Path) -> Result<RepoIdentity> {
    let fallback_name = repo_root
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("repo")
        .to_string();

    let remote = run_git(repo_root, &["config", "--get", "remote.origin.url"]).unwrap_or_default();
    let remote = remote.trim();

    let (provider, organization, name) = if remote.is_empty() {
        ("local".to_string(), "local".to_string(), fallback_name)
    } else if let Some((org, name)) = parse_remote_owner_name(remote) {
        let provider = if remote.contains("github") {
            "github"
        } else if remote.contains("gitlab") {
            "gitlab"
        } else {
            "git"
        };
        (provider.to_string(), org, name)
    } else {
        ("git".to_string(), "local".to_string(), fallback_name)
    };

    let identity = format!("{}://{}/{}", provider, organization, name);
    let repo_id = deterministic_uuid(&identity);

    Ok(RepoIdentity {
        provider,
        organization,
        name,
        identity,
        repo_id,
    })
}

pub fn resolve_repo_id(repo_root: &Path) -> Result<String> {
    Ok(resolve_repo_identity(repo_root)?.repo_id)
}

pub(super) fn parse_remote_owner_name(remote: &str) -> Option<(String, String)> {
    let trimmed = remote.trim().trim_end_matches('/');

    if let Some(rest) = trimmed.strip_prefix("git@") {
        let (_, path) = rest.split_once(':')?;
        return parse_owner_name_path(path);
    }

    if let Some(pos) = trimmed.find("://") {
        let rest = &trimmed[pos + 3..];
        let (_, path) = rest.split_once('/')?;
        return parse_owner_name_path(path);
    }

    if let Some(path) = trimmed.strip_prefix("ssh://") {
        let (_, path) = path.split_once('/')?;
        return parse_owner_name_path(path);
    }

    None
}

pub(super) fn parse_owner_name_path(path: &str) -> Option<(String, String)> {
    let clean = path.trim().trim_end_matches(".git");
    let mut parts = clean
        .split('/')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();
    if parts.len() < 2 {
        return None;
    }
    let name = parts.pop()?.to_string();
    let org = parts.pop()?.to_string();
    Some((org, name))
}

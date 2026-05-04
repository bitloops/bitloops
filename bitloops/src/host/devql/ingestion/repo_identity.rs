use super::*;

// Git remote URL parsing and repository identity resolution.

pub fn resolve_repo_identity(repo_root: &Path) -> Result<RepoIdentity> {
    let fallback_name = repo_root
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("repo")
        .to_string();

    let remote = origin_remote_url(repo_root)?;
    let remote = remote.as_deref().unwrap_or_default().trim();

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

fn origin_remote_url(repo_root: &Path) -> Result<Option<String>> {
    let output = crate::host::checkpoints::strategy::manual_commit::new_git_command()
        .args(["config", "--get", "remote.origin.url"])
        .current_dir(repo_root)
        .stdin(std::process::Stdio::null())
        .output()
        .with_context(|| {
            format!(
                "running git config --get remote.origin.url in {}",
                repo_root.display()
            )
        })?;

    if output.status.success() {
        let remote = String::from_utf8_lossy(&output.stdout).trim().to_string();
        return Ok((!remote.is_empty()).then_some(remote));
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if is_missing_origin_remote(output.status.code(), stderr.as_str()) {
        return Ok(None);
    }

    let detail = if stderr.is_empty() {
        "no stderr".to_string()
    } else {
        stderr
    };
    bail!(
        "git config --get remote.origin.url failed in {} ({}): {}",
        repo_root.display(),
        output.status,
        detail
    )
}

fn is_missing_origin_remote(status_code: Option<i32>, stderr: &str) -> bool {
    if status_code == Some(1) && stderr.trim().is_empty() {
        return true;
    }

    let stderr = stderr.to_ascii_lowercase();
    stderr.contains("not a git repository") || stderr.contains("not in a git directory")
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::git_fixtures::{git_ok, init_test_repo};

    #[test]
    fn resolve_repo_identity_uses_origin_remote_when_available() {
        let repo = tempfile::tempdir().expect("temp repo");
        init_test_repo(repo.path(), "main", "Bitloops Test", "bitloops@example.com");
        git_ok(
            repo.path(),
            &[
                "remote",
                "add",
                "origin",
                "git@github.com:bitloops/bitloops.git",
            ],
        );

        let identity = resolve_repo_identity(repo.path()).expect("resolve repo identity");

        assert_eq!(identity.provider, "github");
        assert_eq!(identity.organization, "bitloops");
        assert_eq!(identity.name, "bitloops");
        assert_eq!(identity.identity, "github://bitloops/bitloops");
        assert_eq!(
            identity.repo_id,
            deterministic_uuid("github://bitloops/bitloops")
        );
    }

    #[test]
    fn resolve_repo_identity_keeps_local_fallback_for_missing_origin() {
        let repo = tempfile::tempdir().expect("temp repo");
        init_test_repo(repo.path(), "main", "Bitloops Test", "bitloops@example.com");
        let name = repo
            .path()
            .file_name()
            .and_then(|value| value.to_str())
            .expect("temp repo name");
        let expected_identity = format!("local://local/{name}");

        let identity = resolve_repo_identity(repo.path()).expect("resolve repo identity");

        assert_eq!(identity.provider, "local");
        assert_eq!(identity.organization, "local");
        assert_eq!(identity.name, name);
        assert_eq!(identity.identity, expected_identity);
    }

    #[test]
    fn resolve_repo_identity_keeps_local_fallback_for_non_git_roots() {
        let dir = tempfile::tempdir().expect("temp dir");
        let name = dir
            .path()
            .file_name()
            .and_then(|value| value.to_str())
            .expect("temp dir name");

        let identity = resolve_repo_identity(dir.path()).expect("resolve repo identity");

        assert_eq!(identity.provider, "local");
        assert_eq!(identity.organization, "local");
        assert_eq!(identity.name, name);
    }

    #[test]
    fn remote_config_failure_classifier_only_accepts_missing_remote_or_non_git() {
        assert!(is_missing_origin_remote(Some(1), ""));
        assert!(is_missing_origin_remote(
            Some(128),
            "fatal: not a git repository (or any of the parent directories): .git"
        ));
        assert!(is_missing_origin_remote(
            Some(128),
            "fatal: not in a git directory"
        ));
        assert!(!is_missing_origin_remote(
            Some(255),
            "fatal: unable to read config file .git/config: Resource temporarily unavailable"
        ));
    }
}

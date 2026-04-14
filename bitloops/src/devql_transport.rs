use anyhow::{Context, Result, anyhow, bail};
use axum::http::HeaderMap;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::RequestBuilder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::config::{
    BITLOOPS_CONFIG_RELATIVE_PATH, discover_repo_policy_optional,
    resolve_bound_daemon_config_path_for_repo, resolve_daemon_config_path_for_repo,
};
use crate::host::checkpoints::strategy::manual_commit::run_git;
use crate::host::devql::{RepoIdentity, resolve_repo_identity};

pub(crate) const HEADER_SCOPE_REPO_ID: &str = "x-bitloops-cli-repo-id";
pub(crate) const HEADER_SCOPE_REPO_NAME: &str = "x-bitloops-cli-repo-name";
pub(crate) const HEADER_SCOPE_REPO_PROVIDER: &str = "x-bitloops-cli-repo-provider";
pub(crate) const HEADER_SCOPE_REPO_ORGANISATION: &str = "x-bitloops-cli-repo-organisation";
pub(crate) const HEADER_SCOPE_REPO_IDENTITY: &str = "x-bitloops-cli-repo-identity";
pub(crate) const HEADER_SCOPE_REPO_ROOT: &str = "x-bitloops-cli-repo-root";
pub(crate) const HEADER_SCOPE_BRANCH: &str = "x-bitloops-cli-branch";
pub(crate) const HEADER_SCOPE_PROJECT_PATH: &str = "x-bitloops-cli-project-path";
pub(crate) const HEADER_SCOPE_GIT_DIR_RELATIVE_PATH: &str = "x-bitloops-cli-git-dir-relative-path";
pub(crate) const HEADER_SCOPE_CONFIG_FINGERPRINT: &str = "x-bitloops-cli-config-fingerprint";
pub(crate) const HEADER_DAEMON_BINDING: &str = "x-bitloops-daemon-binding";

#[derive(Debug, Clone)]
pub(crate) struct SlimCliRepoScope {
    pub(crate) repo: RepoIdentity,
    pub(crate) repo_root: PathBuf,
    pub(crate) branch_name: String,
    pub(crate) project_path: Option<String>,
    pub(crate) git_dir_relative_path: String,
    pub(crate) config_fingerprint: String,
}

#[derive(Debug)]
enum DevqlScopeDiscoveryError {
    RepoRootUnavailable { stderr: Option<String> },
}

impl std::fmt::Display for DevqlScopeDiscoveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RepoRootUnavailable {
                stderr: Some(stderr),
            } => write!(
                f,
                "failed to resolve git repository root for DevQL scope: {stderr}"
            ),
            Self::RepoRootUnavailable { stderr: None } => {
                write!(f, "failed to resolve git repository root for DevQL scope")
            }
        }
    }
}

impl std::error::Error for DevqlScopeDiscoveryError {}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RepoPathRegistry {
    pub(crate) version: u8,
    pub(crate) entries: Vec<RepoPathRegistryEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RepoPathRegistryEntry {
    pub(crate) repo_id: String,
    pub(crate) provider: String,
    pub(crate) organisation: String,
    pub(crate) name: String,
    pub(crate) identity: String,
    pub(crate) repo_root: PathBuf,
    pub(crate) last_branch: Option<String>,
    pub(crate) git_dir_relative_path: Option<String>,
    pub(crate) updated_at_unix: u64,
}

pub(crate) fn discover_slim_cli_repo_scope(cwd: Option<&Path>) -> Result<SlimCliRepoScope> {
    let cwd = match cwd {
        Some(path) => path.to_path_buf(),
        None => env::current_dir().context("resolving current directory for DevQL scope")?,
    };
    let cwd = cwd.canonicalize().unwrap_or_else(|_| cwd.clone());
    let repo_root = resolve_repo_root_from_cwd(&cwd)?;
    let repo = resolve_repo_identity(&repo_root)?;
    let branch_name = resolve_active_branch_name(&repo_root)?;
    let project_path = resolve_project_path(&repo_root, &cwd)?;
    let git_dir_relative_path = relative_path(&cwd, &repo_root.join(".git"))
        .to_string_lossy()
        .replace('\\', "/");
    let config_fingerprint = discover_repo_policy_optional(&cwd)?.fingerprint;

    Ok(SlimCliRepoScope {
        repo,
        repo_root,
        branch_name,
        project_path,
        git_dir_relative_path,
        config_fingerprint,
    })
}

pub(crate) fn is_repo_root_discovery_error(err: &anyhow::Error) -> bool {
    matches!(
        err.downcast_ref::<DevqlScopeDiscoveryError>(),
        Some(DevqlScopeDiscoveryError::RepoRootUnavailable { .. })
    )
}

pub(crate) fn attach_slim_cli_scope_headers(
    request: RequestBuilder,
    scope: &SlimCliRepoScope,
) -> RequestBuilder {
    let request = request
        .header(HEADER_SCOPE_REPO_ID, scope.repo.repo_id.as_str())
        .header(
            HEADER_SCOPE_REPO_NAME,
            encode_scope_header_value(scope.repo.name.as_str()),
        )
        .header(HEADER_SCOPE_REPO_PROVIDER, scope.repo.provider.as_str())
        .header(
            HEADER_SCOPE_REPO_ORGANISATION,
            encode_scope_header_value(scope.repo.organization.as_str()),
        )
        .header(
            HEADER_SCOPE_REPO_IDENTITY,
            encode_scope_header_value(scope.repo.identity.as_str()),
        )
        .header(
            HEADER_SCOPE_REPO_ROOT,
            encode_scope_header_value(&scope.repo_root.to_string_lossy()),
        )
        .header(
            HEADER_SCOPE_BRANCH,
            encode_scope_header_value(scope.branch_name.as_str()),
        )
        .header(
            HEADER_SCOPE_CONFIG_FINGERPRINT,
            scope.config_fingerprint.as_str(),
        )
        .header(
            HEADER_SCOPE_GIT_DIR_RELATIVE_PATH,
            encode_scope_header_value(scope.git_dir_relative_path.as_str()),
        );

    match scope.project_path.as_deref() {
        Some(project_path) => request.header(
            HEADER_SCOPE_PROJECT_PATH,
            encode_scope_header_value(project_path),
        ),
        None => request,
    }
}

pub(crate) fn daemon_binding_identifier_for_config_path(config_path: &Path) -> String {
    let config_path = config_path
        .canonicalize()
        .unwrap_or_else(|_| config_path.to_path_buf());
    let mut hasher = Sha256::new();
    hasher.update(config_path.to_string_lossy().as_bytes());
    hex::encode(hasher.finalize())
}

pub(crate) fn repo_daemon_config_path_for_binding(repo_root: &Path) -> PathBuf {
    resolve_bound_daemon_config_path_for_repo(repo_root)
        .or_else(|_| resolve_daemon_config_path_for_repo(repo_root))
        .unwrap_or_else(|_| repo_root.join(BITLOOPS_CONFIG_RELATIVE_PATH))
}

pub(crate) fn repo_daemon_binding_identifier(repo_root: &Path) -> String {
    daemon_binding_identifier_for_config_path(&repo_daemon_config_path_for_binding(repo_root))
}

pub(crate) fn attach_repo_daemon_binding_headers(
    request: RequestBuilder,
    repo_root: &Path,
) -> Result<RequestBuilder> {
    let binding = repo_daemon_binding_identifier(repo_root);
    Ok(request
        .header(
            HEADER_SCOPE_REPO_ROOT,
            encode_scope_header_value(&repo_root.to_string_lossy()),
        )
        .header(HEADER_DAEMON_BINDING, binding))
}

pub(crate) fn parse_repo_root_header(headers: &HeaderMap) -> Result<Option<PathBuf>> {
    decode_scope_header_value(headers, HEADER_SCOPE_REPO_ROOT).map(|value| value.map(PathBuf::from))
}

pub(crate) fn parse_daemon_binding_header(headers: &HeaderMap) -> Result<Option<String>> {
    header_value(headers, HEADER_DAEMON_BINDING)
}

pub(crate) fn parse_slim_cli_scope_headers(
    headers: &HeaderMap,
) -> Result<Option<SlimCliRepoScope>> {
    let Some(repo_root) = decode_scope_header_value(headers, HEADER_SCOPE_REPO_ROOT)? else {
        return Ok(None);
    };
    let repo_root = PathBuf::from(repo_root);
    let repo = RepoIdentity {
        repo_id: required_header_value(headers, HEADER_SCOPE_REPO_ID)?,
        name: required_decoded_scope_header_value(headers, HEADER_SCOPE_REPO_NAME)?,
        provider: required_header_value(headers, HEADER_SCOPE_REPO_PROVIDER)?,
        organization: required_decoded_scope_header_value(headers, HEADER_SCOPE_REPO_ORGANISATION)?,
        identity: required_decoded_scope_header_value(headers, HEADER_SCOPE_REPO_IDENTITY)?,
    };

    Ok(Some(SlimCliRepoScope {
        repo,
        repo_root,
        branch_name: required_decoded_scope_header_value(headers, HEADER_SCOPE_BRANCH)?,
        project_path: decode_scope_header_value(headers, HEADER_SCOPE_PROJECT_PATH)?,
        git_dir_relative_path: required_decoded_scope_header_value(
            headers,
            HEADER_SCOPE_GIT_DIR_RELATIVE_PATH,
        )?,
        config_fingerprint: required_header_value(headers, HEADER_SCOPE_CONFIG_FINGERPRINT)?,
    }))
}

pub(crate) fn encode_scope_header_value(input: &str) -> String {
    URL_SAFE_NO_PAD.encode(input.as_bytes())
}

pub(crate) fn load_repo_path_registry(path: &Path) -> Result<RepoPathRegistry> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RepoPathRegistry {
                version: 1,
                entries: Vec::new(),
            });
        }
        Err(err) => return Err(err).with_context(|| format!("reading {}", path.display())),
    };
    let mut registry: RepoPathRegistry =
        serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))?;
    if registry.version == 0 {
        registry.version = 1;
    }
    Ok(registry)
}

pub(crate) fn persist_repo_path_registry(path: &Path, registry: &RepoPathRegistry) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .context("resolving repo path registry parent directory")?;
    fs::create_dir_all(parent)
        .with_context(|| format!("creating repo path registry directory {}", parent.display()))?;
    let mut bytes = serde_json::to_vec_pretty(registry)
        .with_context(|| format!("serialising {}", path.display()))?;
    bytes.push(b'\n');
    fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))
}

pub(crate) fn upsert_repo_path_registry_scope(path: &Path, scope: &SlimCliRepoScope) -> Result<()> {
    let mut registry = load_repo_path_registry(path)?;
    let updated_at_unix = unix_timestamp_now();
    let new_entry = RepoPathRegistryEntry {
        repo_id: scope.repo.repo_id.clone(),
        provider: scope.repo.provider.clone(),
        organisation: scope.repo.organization.clone(),
        name: scope.repo.name.clone(),
        identity: scope.repo.identity.clone(),
        repo_root: scope.repo_root.clone(),
        last_branch: Some(scope.branch_name.clone()),
        git_dir_relative_path: Some(scope.git_dir_relative_path.clone()),
        updated_at_unix,
    };

    if let Some(existing) = registry
        .entries
        .iter_mut()
        .find(|entry| entry.repo_id == new_entry.repo_id)
    {
        *existing = new_entry;
    } else {
        registry.entries.push(new_entry);
    }
    registry
        .entries
        .sort_by(|left, right| left.name.cmp(&right.name));
    persist_repo_path_registry(path, &registry)
}

pub(crate) fn index_repo_path_registry(
    registry: &RepoPathRegistry,
) -> BTreeMap<String, RepoPathRegistryEntry> {
    registry
        .entries
        .iter()
        .cloned()
        .map(|entry| (entry.repo_id.clone(), entry))
        .collect()
}

fn resolve_repo_root_from_cwd(cwd: &Path) -> Result<PathBuf> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(cwd)
        .output()
        .context("resolving git repository root for DevQL scope")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(DevqlScopeDiscoveryError::RepoRootUnavailable {
            stderr: (!stderr.is_empty()).then_some(stderr),
        }
        .into());
    }
    let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if root.is_empty() {
        bail!("git returned an empty repository root while resolving DevQL scope");
    }
    Ok(PathBuf::from(root))
}

fn resolve_active_branch_name(repo_root: &Path) -> Result<String> {
    if let Ok(branch) = run_git(repo_root, &["symbolic-ref", "--quiet", "--short", "HEAD"]) {
        let branch = branch.trim();
        if !branch.is_empty() {
            return Ok(branch.to_string());
        }
    }

    let branch = run_git(repo_root, &["rev-parse", "--abbrev-ref", "HEAD"])
        .context("reading active git branch for DevQL scope")?;
    let branch = branch.trim();
    if branch.is_empty() || branch == "HEAD" {
        bail!("not on a branch (detached HEAD)");
    }
    Ok(branch.to_string())
}

fn resolve_project_path(repo_root: &Path, cwd: &Path) -> Result<Option<String>> {
    let relative = cwd.strip_prefix(repo_root).with_context(|| {
        format!(
            "current directory {} is not inside repository {}",
            cwd.display(),
            repo_root.display()
        )
    })?;
    if relative.as_os_str().is_empty() {
        return Ok(None);
    }
    Ok(Some(relative.to_string_lossy().replace('\\', "/")))
}

fn header_value(headers: &HeaderMap, name: &str) -> Result<Option<String>> {
    let Some(value) = headers.get(name) else {
        return Ok(None);
    };
    let value = value
        .to_str()
        .with_context(|| format!("decoding HTTP header `{name}` as UTF-8"))?
        .trim()
        .to_string();
    if value.is_empty() {
        return Ok(None);
    }
    Ok(Some(value))
}

fn decode_scope_header_value(headers: &HeaderMap, name: &str) -> Result<Option<String>> {
    let Some(value) = header_value(headers, name)? else {
        return Ok(None);
    };
    let bytes = URL_SAFE_NO_PAD
        .decode(value.as_bytes())
        .with_context(|| format!("decoding Bitloops DevQL scope header `{name}` from base64url"))?;
    let decoded = String::from_utf8(bytes)
        .with_context(|| format!("decoding Bitloops DevQL scope header `{name}` as UTF-8"))?;
    if decoded.is_empty() {
        return Ok(None);
    }
    Ok(Some(decoded))
}

fn required_header_value(headers: &HeaderMap, name: &str) -> Result<String> {
    header_value(headers, name)?
        .ok_or_else(|| anyhow!("missing Bitloops DevQL scope header `{name}`"))
}

fn required_decoded_scope_header_value(headers: &HeaderMap, name: &str) -> Result<String> {
    decode_scope_header_value(headers, name)?
        .ok_or_else(|| anyhow!("missing Bitloops DevQL scope header `{name}`"))
}

fn relative_path(from: &Path, to: &Path) -> PathBuf {
    let from_components = normalised_components(from);
    let to_components = normalised_components(to);
    let common_prefix_len = from_components
        .iter()
        .zip(to_components.iter())
        .take_while(|(left, right)| left == right)
        .count();

    let mut output = PathBuf::new();
    for _ in common_prefix_len..from_components.len() {
        output.push("..");
    }
    for component in &to_components[common_prefix_len..] {
        output.push(component);
    }

    if output.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        output
    }
}

fn normalised_components(path: &Path) -> Vec<PathBuf> {
    path.components()
        .filter_map(|component| match component {
            Component::Prefix(prefix) => Some(PathBuf::from(prefix.as_os_str())),
            Component::RootDir => Some(PathBuf::from(std::path::MAIN_SEPARATOR.to_string())),
            Component::Normal(value) => Some(PathBuf::from(value)),
            Component::CurDir | Component::ParentDir => None,
        })
        .collect()
}

fn unix_timestamp_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::git_fixtures::{git_ok, init_test_repo};
    use axum::http::HeaderValue;
    use reqwest::Client;
    use tempfile::TempDir;

    fn canonical_path(path: &Path) -> PathBuf {
        path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
    }

    fn seed_repo() -> TempDir {
        let repo = TempDir::new().expect("temp dir");
        init_test_repo(repo.path(), "main", "Alice", "alice@example.com");
        fs::write(repo.path().join("src.txt"), "hello\n").expect("write repo file");
        git_ok(repo.path(), &["add", "src.txt"]);
        git_ok(repo.path(), &["commit", "-m", "seed repo"]);
        repo
    }

    #[test]
    fn discover_slim_cli_repo_scope_from_repo_root() {
        let repo = seed_repo();

        let scope = discover_slim_cli_repo_scope(Some(repo.path())).expect("discover slim scope");

        assert_eq!(scope.repo_root, canonical_path(repo.path()));
        assert_eq!(scope.branch_name, "main");
        assert_eq!(scope.project_path, None);
        assert_eq!(scope.git_dir_relative_path, ".git");
        assert!(!scope.repo.repo_id.trim().is_empty());
    }

    #[test]
    fn discover_slim_cli_repo_scope_from_nested_directory_sets_project_path() {
        let repo = seed_repo();
        let nested = repo.path().join("packages").join("api");
        fs::create_dir_all(&nested).expect("create nested directory");

        let scope = discover_slim_cli_repo_scope(Some(&nested)).expect("discover slim scope");

        assert_eq!(scope.repo_root, canonical_path(repo.path()));
        assert_eq!(scope.branch_name, "main");
        assert_eq!(scope.project_path.as_deref(), Some("packages/api"));
        assert_eq!(scope.git_dir_relative_path, "../../.git");
    }

    #[test]
    fn discover_slim_cli_repo_scope_handles_git_worktree_files() {
        let repo = seed_repo();
        let worktree_root = repo.path().join("worktrees").join("feature");
        fs::create_dir_all(worktree_root.parent().expect("worktree parent"))
            .expect("create worktree parent");
        let worktree_root_str = worktree_root.to_string_lossy().to_string();
        git_ok(
            repo.path(),
            &[
                "worktree",
                "add",
                "-b",
                "feature/worktree",
                &worktree_root_str,
            ],
        );
        let nested = worktree_root.join("nested");
        fs::create_dir_all(&nested).expect("create nested worktree directory");

        let scope = discover_slim_cli_repo_scope(Some(&nested)).expect("discover slim scope");

        assert_eq!(scope.repo_root, canonical_path(&worktree_root));
        assert_eq!(scope.branch_name, "feature/worktree");
        assert_eq!(scope.project_path.as_deref(), Some("nested"));
        assert_eq!(scope.git_dir_relative_path, "../.git");
    }

    #[test]
    fn discover_slim_cli_repo_scope_rejects_detached_head() {
        let repo = seed_repo();
        git_ok(repo.path(), &["checkout", "--detach"]);

        let err = discover_slim_cli_repo_scope(Some(repo.path())).expect_err("detached HEAD");
        assert!(format!("{err:#}").contains("detached HEAD"));
    }

    #[test]
    fn discover_slim_cli_repo_scope_handles_unborn_head_branches() {
        let repo = TempDir::new().expect("temp dir");
        init_test_repo(repo.path(), "main", "Alice", "alice@example.com");

        let scope = discover_slim_cli_repo_scope(Some(repo.path())).expect("discover slim scope");

        assert_eq!(scope.branch_name, "main");
        assert_eq!(scope.project_path, None);
    }

    #[test]
    fn discover_slim_cli_repo_scope_rejects_non_git_directories() {
        let dir = TempDir::new().expect("temp dir");

        let err = discover_slim_cli_repo_scope(Some(dir.path())).expect_err("non-git directory");
        assert!(format!("{err:#}").contains("failed to resolve git repository root"));
    }

    #[test]
    fn repo_path_registry_upsert_replaces_existing_repo_entry() {
        let repo = seed_repo();
        let registry_dir = TempDir::new().expect("temp dir");
        let registry_path = registry_dir.path().join("repo-path-registry.json");
        let repo_identity = resolve_repo_identity(repo.path()).expect("resolve repo identity");
        let initial_scope = SlimCliRepoScope {
            repo: repo_identity.clone(),
            repo_root: repo.path().to_path_buf(),
            branch_name: "main".to_string(),
            project_path: Some("packages/api".to_string()),
            git_dir_relative_path: "../../.git".to_string(),
            config_fingerprint: "fingerprint-a".to_string(),
        };
        upsert_repo_path_registry_scope(&registry_path, &initial_scope)
            .expect("write initial registry scope");

        let updated_scope = SlimCliRepoScope {
            repo: repo_identity,
            repo_root: repo.path().to_path_buf(),
            branch_name: "feature/refactor".to_string(),
            project_path: None,
            git_dir_relative_path: ".git".to_string(),
            config_fingerprint: "fingerprint-b".to_string(),
        };
        upsert_repo_path_registry_scope(&registry_path, &updated_scope)
            .expect("write updated registry scope");

        let registry = load_repo_path_registry(&registry_path).expect("load registry");
        assert_eq!(registry.entries.len(), 1);
        assert_eq!(
            registry.entries[0].last_branch.as_deref(),
            Some("feature/refactor")
        );
        assert_eq!(
            registry.entries[0].git_dir_relative_path.as_deref(),
            Some(".git")
        );
    }

    #[test]
    fn slim_cli_scope_headers_round_trip_unicode_scope_values() {
        let scope = SlimCliRepoScope {
            repo: RepoIdentity {
                repo_id: "repo-id".to_string(),
                name: "cafe-dashboard".to_string(),
                provider: "local".to_string(),
                organization: "local".to_string(),
                identity: "local://local/cafe-dashboard".to_string(),
            },
            repo_root: PathBuf::from("/tmp/Jack’s MacBook Pro – 2/local-dashboard"),
            branch_name: "main".to_string(),
            project_path: Some("packages/café".to_string()),
            git_dir_relative_path: ".git".to_string(),
            config_fingerprint: "fingerprint-a".to_string(),
        };

        let request =
            attach_slim_cli_scope_headers(Client::new().post("http://127.0.0.1/devql"), &scope)
                .build()
                .expect("build request with slim headers");

        let parsed = parse_slim_cli_scope_headers(request.headers())
            .expect("unicode scope headers should round-trip successfully")
            .expect("scope headers should be present");

        assert_eq!(parsed.repo_root, scope.repo_root);
        assert_eq!(parsed.project_path, scope.project_path);
        assert_eq!(parsed.branch_name, scope.branch_name);
        assert_eq!(parsed.git_dir_relative_path, scope.git_dir_relative_path);
        assert_eq!(parsed.config_fingerprint, scope.config_fingerprint);
        assert_eq!(parsed.repo.repo_id, scope.repo.repo_id);
        assert_eq!(parsed.repo.name, scope.repo.name);
        assert_eq!(parsed.repo.provider, scope.repo.provider);
        assert_eq!(parsed.repo.organization, scope.repo.organization);
        assert_eq!(parsed.repo.identity, scope.repo.identity);
    }

    #[test]
    fn parsing_scope_headers_rejects_invalid_base64url_payload() {
        let mut headers = HeaderMap::new();
        headers.insert(
            HEADER_SCOPE_REPO_ROOT,
            HeaderValue::from_static("not-base64!"),
        );

        let err = parse_slim_cli_scope_headers(&headers).expect_err("invalid base64url payload");

        assert!(
            format!("{err:#}").contains(
                "decoding Bitloops DevQL scope header `x-bitloops-cli-repo-root` from base64url"
            ),
            "unexpected error: {err:#}"
        );
    }
}

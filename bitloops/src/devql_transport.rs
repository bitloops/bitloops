use anyhow::{Context, Result, anyhow, bail};
use axum::http::HeaderMap;
use reqwest::RequestBuilder;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};

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

#[derive(Debug, Clone)]
pub(crate) struct SlimCliRepoScope {
    pub(crate) repo: RepoIdentity,
    pub(crate) repo_root: PathBuf,
    pub(crate) branch_name: String,
    pub(crate) project_path: Option<String>,
    pub(crate) git_dir_relative_path: String,
}

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
    let cwd = cwd
        .canonicalize()
        .unwrap_or_else(|_| cwd.clone());
    let repo_root = resolve_repo_root_from_cwd(&cwd)?;
    let repo = resolve_repo_identity(&repo_root)?;
    let branch_name = resolve_active_branch_name(&repo_root)?;
    let project_path = resolve_project_path(&repo_root, &cwd)?;
    let git_dir_relative_path = relative_path(&cwd, &repo_root.join(".git"))
        .to_string_lossy()
        .replace('\\', "/");

    Ok(SlimCliRepoScope {
        repo,
        repo_root,
        branch_name,
        project_path,
        git_dir_relative_path,
    })
}

pub(crate) fn attach_slim_cli_scope_headers(
    request: RequestBuilder,
    scope: &SlimCliRepoScope,
) -> RequestBuilder {
    let request = request
        .header(HEADER_SCOPE_REPO_ID, scope.repo.repo_id.as_str())
        .header(HEADER_SCOPE_REPO_NAME, scope.repo.name.as_str())
        .header(HEADER_SCOPE_REPO_PROVIDER, scope.repo.provider.as_str())
        .header(
            HEADER_SCOPE_REPO_ORGANISATION,
            scope.repo.organization.as_str(),
        )
        .header(HEADER_SCOPE_REPO_IDENTITY, scope.repo.identity.as_str())
        .header(
            HEADER_SCOPE_REPO_ROOT,
            scope.repo_root.to_string_lossy().to_string(),
        )
        .header(HEADER_SCOPE_BRANCH, scope.branch_name.as_str())
        .header(
            HEADER_SCOPE_GIT_DIR_RELATIVE_PATH,
            scope.git_dir_relative_path.as_str(),
        );

    match scope.project_path.as_deref() {
        Some(project_path) => request.header(HEADER_SCOPE_PROJECT_PATH, project_path),
        None => request,
    }
}

pub(crate) fn parse_slim_cli_scope_headers(headers: &HeaderMap) -> Result<Option<SlimCliRepoScope>> {
    let Some(repo_root) = header_value(headers, HEADER_SCOPE_REPO_ROOT)? else {
        return Ok(None);
    };
    let repo_root = PathBuf::from(repo_root);
    let repo = RepoIdentity {
        repo_id: required_header_value(headers, HEADER_SCOPE_REPO_ID)?,
        name: required_header_value(headers, HEADER_SCOPE_REPO_NAME)?,
        provider: required_header_value(headers, HEADER_SCOPE_REPO_PROVIDER)?,
        organization: required_header_value(headers, HEADER_SCOPE_REPO_ORGANISATION)?,
        identity: required_header_value(headers, HEADER_SCOPE_REPO_IDENTITY)?,
    };

    Ok(Some(SlimCliRepoScope {
        repo,
        repo_root,
        branch_name: required_header_value(headers, HEADER_SCOPE_BRANCH)?,
        project_path: header_value(headers, HEADER_SCOPE_PROJECT_PATH)?,
        git_dir_relative_path: required_header_value(headers, HEADER_SCOPE_GIT_DIR_RELATIVE_PATH)?,
    }))
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
    let mut registry: RepoPathRegistry = serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing {}", path.display()))?;
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
    registry.entries.sort_by(|left, right| left.name.cmp(&right.name));
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
        if stderr.is_empty() {
            bail!("failed to resolve git repository root for DevQL scope");
        }
        bail!("failed to resolve git repository root for DevQL scope: {stderr}");
    }
    let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if root.is_empty() {
        bail!("git returned an empty repository root while resolving DevQL scope");
    }
    Ok(PathBuf::from(root))
}

fn resolve_active_branch_name(repo_root: &Path) -> Result<String> {
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

fn required_header_value(headers: &HeaderMap, name: &str) -> Result<String> {
    header_value(headers, name)?
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
            &["worktree", "add", "-b", "feature/worktree", &worktree_root_str],
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
        };
        upsert_repo_path_registry_scope(&registry_path, &initial_scope)
            .expect("write initial registry scope");

        let updated_scope = SlimCliRepoScope {
            repo: repo_identity,
            repo_root: repo.path().to_path_buf(),
            branch_name: "feature/refactor".to_string(),
            project_path: None,
            git_dir_relative_path: ".git".to_string(),
        };
        upsert_repo_path_registry_scope(&registry_path, &updated_scope)
            .expect("write updated registry scope");

        let registry = load_repo_path_registry(&registry_path).expect("load registry");
        assert_eq!(registry.entries.len(), 1);
        assert_eq!(registry.entries[0].last_branch.as_deref(), Some("feature/refactor"));
        assert_eq!(registry.entries[0].git_dir_relative_path.as_deref(), Some(".git"));
    }
}

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result as AnyhowResult;
use async_graphql::ID;

use crate::api::DashboardState;
use crate::graphql::bad_user_input_error;

use super::errors::internal_config_error;
use super::identity::{canonicalize_lossy, target_id};
use super::types::{ConfigTarget, ConfigTargetKind};

const MAX_CONFIG_SCAN_DIRS: usize = 20_000;
const SKIPPED_SCAN_DIRS: [&str; 9] = [
    ".git",
    "target",
    "node_modules",
    "dist",
    "build",
    ".next",
    ".turbo",
    ".cache",
    "coverage",
];

pub(super) async fn resolve_target(
    state: &DashboardState,
    target_id: &ID,
) -> async_graphql::Result<ConfigTarget> {
    let targets = discover_config_targets(state)
        .await
        .map_err(internal_config_error)?;
    targets
        .into_iter()
        .find(|target| target.id == *target_id)
        .ok_or_else(|| bad_user_input_error(format!("unknown config target `{target_id:?}`")))
}

pub(super) async fn discover_config_targets(
    state: &DashboardState,
) -> AnyhowResult<Vec<ConfigTarget>> {
    let mut targets = BTreeMap::<String, ConfigTarget>::new();
    let daemon_path = canonicalize_lossy(&state.config_path);
    let daemon = ConfigTarget {
        id: target_id(ConfigTargetKind::Daemon.as_str(), &daemon_path),
        kind: ConfigTargetKind::Daemon,
        label: "Daemon config".to_string(),
        group: "Daemon".to_string(),
        path: daemon_path.clone(),
        repo_root: None,
        exists: daemon_path.is_file(),
    };
    targets.insert(daemon.path.display().to_string(), daemon);

    for root in known_repo_roots(state).await {
        scan_repo_config_targets(&root, &mut targets)?;
    }

    let mut values = targets.into_values().collect::<Vec<_>>();
    values.sort_by(|left, right| {
        target_sort_key(left)
            .cmp(&target_sort_key(right))
            .then_with(|| left.path.cmp(&right.path))
    });
    Ok(values)
}

async fn known_repo_roots(state: &DashboardState) -> BTreeSet<PathBuf> {
    let mut roots = BTreeSet::new();
    roots.insert(canonicalize_lossy(&state.repo_root));

    let context = crate::graphql::DevqlGraphqlContext::for_global_request(
        state.config_root.clone(),
        state.repo_root.clone(),
        state.repo_registry_path().map(Path::to_path_buf),
        state.db.clone(),
    );

    match context.list_known_repositories().await {
        Ok(repositories) => {
            for repository in repositories {
                if let Some(repo_root) = repository.repo_root() {
                    roots.insert(canonicalize_lossy(repo_root));
                }
            }
        }
        Err(err) => {
            log::debug!("config target discovery could not load known repositories: {err:#}");
        }
    }

    roots
}

fn scan_repo_config_targets(
    repo_root: &Path,
    targets: &mut BTreeMap<String, ConfigTarget>,
) -> AnyhowResult<()> {
    let mut queue = VecDeque::from([canonicalize_lossy(repo_root)]);
    let mut visited = BTreeSet::new();
    let mut scanned = 0usize;

    while let Some(directory) = queue.pop_front() {
        if !visited.insert(directory.clone()) {
            continue;
        }
        scanned += 1;
        if scanned > MAX_CONFIG_SCAN_DIRS {
            log::warn!(
                "config target discovery stopped after scanning {MAX_CONFIG_SCAN_DIRS} directories under {}",
                repo_root.display()
            );
            break;
        }

        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(err) => {
                log::debug!(
                    "config target discovery skipped unreadable directory {}: {err}",
                    directory.display()
                );
                continue;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) => continue,
            };
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if file_type.is_dir() {
                if !file_type.is_symlink() && !SKIPPED_SCAN_DIRS.contains(&name.as_ref()) {
                    queue.push_back(path);
                }
                continue;
            }

            if !file_type.is_file() {
                continue;
            }

            let Some(kind) = ConfigTargetKind::from_path(&path) else {
                continue;
            };
            if kind == ConfigTargetKind::Daemon {
                continue;
            }
            let canonical_path = canonicalize_lossy(&path);
            let root =
                canonicalize_lossy(canonical_path.parent().unwrap_or_else(|| Path::new("/")));
            let label = match kind {
                ConfigTargetKind::RepoShared => ".bitloops.toml".to_string(),
                ConfigTargetKind::RepoLocal => ".bitloops.local.toml".to_string(),
                ConfigTargetKind::Daemon => unreachable!("daemon targets are handled separately"),
            };
            let display_root = root
                .strip_prefix(repo_root)
                .ok()
                .filter(|relative| !relative.as_os_str().is_empty())
                .map(|relative| relative.display().to_string())
                .unwrap_or_else(|| repo_root.display().to_string());
            let target = ConfigTarget {
                id: target_id(kind.as_str(), &canonical_path),
                kind,
                label,
                group: display_root,
                path: canonical_path,
                repo_root: Some(canonicalize_lossy(repo_root)),
                exists: true,
            };
            targets.insert(target.path.display().to_string(), target);
        }
    }

    Ok(())
}

fn target_sort_key(target: &ConfigTarget) -> (u8, String, String) {
    let rank = match target.kind {
        ConfigTargetKind::Daemon => 0,
        ConfigTargetKind::RepoShared => 1,
        ConfigTargetKind::RepoLocal => 2,
    };
    (rank, target.group.clone(), target.label.clone())
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::path::Path;

    use tempfile::TempDir;

    use crate::config::{REPO_POLICY_FILE_NAME, REPO_POLICY_LOCAL_FILE_NAME};

    use super::*;

    fn write(path: &Path, text: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(path, text).expect("write file");
    }

    #[test]
    fn scan_repo_config_targets_finds_root_and_nested_policy_files() {
        let temp = TempDir::new().expect("temp dir");
        write(
            &temp.path().join(REPO_POLICY_FILE_NAME),
            "[capture]\nenabled = true\n",
        );
        write(
            &temp
                .path()
                .join("packages")
                .join("app")
                .join(REPO_POLICY_LOCAL_FILE_NAME),
            "[capture]\nstrategy = \"manual-commit\"\n",
        );
        write(
            &temp
                .path()
                .join("target")
                .join("ignored")
                .join(REPO_POLICY_FILE_NAME),
            "[capture]\nenabled = false\n",
        );

        let mut targets = BTreeMap::new();
        scan_repo_config_targets(temp.path(), &mut targets).expect("scan targets");

        let temp_root = canonicalize_lossy(temp.path());
        let paths = targets
            .values()
            .map(|target| {
                target
                    .path
                    .strip_prefix(&temp_root)
                    .expect("target under temp")
                    .display()
                    .to_string()
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(
            paths,
            BTreeSet::from([
                REPO_POLICY_FILE_NAME.to_string(),
                format!("packages/app/{REPO_POLICY_LOCAL_FILE_NAME}"),
            ])
        );
    }
}

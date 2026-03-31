#![cfg_attr(not(test), allow(dead_code))]

use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use super::content_identity::compute_blob_oid;
use super::types::{
    ClassifiedPath, DesiredFileState, DesiredManifest, EffectiveSource, PathAction, StoredManifest,
};
use super::workspace_state::{StagedChange, WorkspaceState};

pub(crate) fn build_desired_manifest<F>(
    workspace: &WorkspaceState,
    repo_root: &Path,
    language_filter: F,
) -> Result<DesiredManifest>
where
    F: Fn(&str) -> Option<String>,
{
    let dirty_paths: HashSet<&str> = workspace.dirty_files.iter().map(String::as_str).collect();
    let untracked_paths: HashSet<&str> =
        workspace.untracked_files.iter().map(String::as_str).collect();
    let mut manifest = DesiredManifest::new();

    for path in collect_candidate_paths(workspace) {
        let Some(language) = language_filter(&path) else {
            continue;
        };

        let Some(state) = build_desired_file_state(
            &path,
            &language,
            workspace,
            repo_root,
            &dirty_paths,
            &untracked_paths,
        )?
        else {
            continue;
        };

        manifest.insert(path, state);
    }

    Ok(manifest)
}

pub(crate) fn classify_paths(
    desired: &DesiredManifest,
    stored: &StoredManifest,
    parser_version: &str,
    extractor_version: &str,
    repair: bool,
) -> Vec<ClassifiedPath> {
    let all_paths: BTreeSet<&str> = desired
        .keys()
        .chain(stored.keys())
        .map(String::as_str)
        .collect();

    all_paths
        .into_iter()
        .filter_map(|path| {
            let desired_state = desired.get(path);
            let stored_state = stored.get(path);

            let action = match (desired_state, stored_state) {
                (Some(_), None) => PathAction::Added,
                (None, Some(_)) => PathAction::Removed,
                (Some(_), Some(_)) if repair => PathAction::Changed,
                (Some(desired), Some(stored))
                    if desired.effective_content_id == stored.effective_content_id
                        && stored.parser_version == parser_version
                        && stored.extractor_version == extractor_version =>
                {
                    PathAction::Unchanged
                }
                (Some(_), Some(_)) => PathAction::Changed,
                (None, None) => return None,
            };

            Some(ClassifiedPath {
                path: path.to_string(),
                action,
                desired: desired_state.cloned(),
            })
        })
        .collect()
}

fn collect_candidate_paths(workspace: &WorkspaceState) -> BTreeSet<String> {
    workspace
        .head_tree
        .keys()
        .cloned()
        .chain(workspace.staged_changes.keys().cloned())
        .chain(workspace.dirty_files.iter().cloned())
        .chain(workspace.untracked_files.iter().cloned())
        .collect()
}

fn build_desired_file_state(
    path: &str,
    language: &str,
    workspace: &WorkspaceState,
    repo_root: &Path,
    dirty_paths: &HashSet<&str>,
    untracked_paths: &HashSet<&str>,
) -> Result<Option<DesiredFileState>> {
    let head_content_id = workspace.head_tree.get(path).cloned();
    let exists_in_head = head_content_id.is_some();

    let staged_change = workspace.staged_changes.get(path);
    let (index_content_id, exists_in_index, index_deleted) = match staged_change {
        Some(StagedChange::Added(content_id)) | Some(StagedChange::Modified(content_id)) => {
            (Some(content_id.clone()), true, false)
        }
        Some(StagedChange::Deleted) => (None, false, true),
        None => (head_content_id.clone(), exists_in_head, false),
    };

    let path_is_dirty = dirty_paths.contains(path);
    let path_is_untracked = untracked_paths.contains(path);
    let (worktree_content_id, exists_in_worktree) = read_worktree_content_id(
        repo_root,
        path,
        path_is_dirty || path_is_untracked,
        index_content_id.as_deref(),
    )?;
    let worktree_deleted = path_is_dirty && !exists_in_worktree;

    if worktree_deleted || (index_deleted && !exists_in_worktree) {
        return Ok(None);
    }

    let (effective_content_id, effective_source) = if exists_in_worktree {
        let worktree_content_id = worktree_content_id
            .clone()
            .expect("worktree content id must exist when worktree file exists");

        if !exists_in_index || index_content_id.as_ref() != Some(&worktree_content_id) {
            (worktree_content_id, EffectiveSource::Worktree)
        } else {
            let index_content_id = index_content_id
                .clone()
                .expect("index content id must exist when index entry exists");

            if !exists_in_head || head_content_id.as_ref() != Some(&index_content_id) {
                (index_content_id, EffectiveSource::Index)
            } else {
                (index_content_id, EffectiveSource::Head)
            }
        }
    } else if exists_in_index {
        let index_content_id = index_content_id
            .clone()
            .expect("index content id must exist when index entry exists");

        if !exists_in_head || head_content_id.as_ref() != Some(&index_content_id) {
            (index_content_id, EffectiveSource::Index)
        } else {
            (index_content_id, EffectiveSource::Head)
        }
    } else if exists_in_head {
        (
            head_content_id
                .clone()
                .expect("head content id must exist when HEAD entry exists"),
            EffectiveSource::Head,
        )
    } else {
        return Ok(None);
    };

    Ok(Some(DesiredFileState {
        path: path.to_string(),
        language: language.to_string(),
        head_content_id,
        index_content_id,
        worktree_content_id,
        effective_content_id,
        effective_source,
        exists_in_head,
        exists_in_index,
        exists_in_worktree,
    }))
}

fn read_worktree_content_id(
    repo_root: &Path,
    path: &str,
    needs_disk_read: bool,
    index_content_id: Option<&str>,
) -> Result<(Option<String>, bool)> {
    let full_path = repo_root.join(path);
    let metadata = match fs::symlink_metadata(&full_path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok((None, false)),
        Err(err) => {
            return Err(anyhow::Error::from(err))
                .with_context(|| format!("reading metadata for `{path}`"));
        }
    };

    if !metadata.file_type().is_file() {
        return Ok((None, false));
    }

    if !needs_disk_read
        && let Some(index_content_id) = index_content_id
    {
        return Ok((Some(index_content_id.to_string()), true));
    }

    let content = fs::read(&full_path).with_context(|| format!("reading `{path}` from worktree"))?;
    Ok((Some(compute_blob_oid(&content)), true))
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::fs;

    use super::{build_desired_file_state, classify_paths};
    use crate::host::devql::sync::content_identity::compute_blob_oid;
    use crate::host::devql::sync::types::{
        ClassifiedPath, DesiredFileState, DesiredManifest, EffectiveSource, PathAction,
        StoredFileState, StoredManifest,
    };
    use crate::host::devql::sync::workspace_state::WorkspaceState;
    use tempfile::tempdir;

    #[test]
    fn classify_added_path() {
        let desired = make_manifest(vec![("src/a.rs", "abc123")]);
        let stored = StoredManifest::new();

        let classified = classify_paths(&desired, &stored, "1.0", "1.0", false);

        assert_classified_action(&classified, PathAction::Added);
    }

    #[test]
    fn classify_unchanged_path() {
        let desired = make_manifest(vec![("src/a.rs", "abc123")]);
        let stored = make_stored(vec![("src/a.rs", "abc123", "1.0", "1.0")]);

        let classified = classify_paths(&desired, &stored, "1.0", "1.0", false);

        assert_classified_action(&classified, PathAction::Unchanged);
    }

    #[test]
    fn classify_changed_content() {
        let desired = make_manifest(vec![("src/a.rs", "new_hash")]);
        let stored = make_stored(vec![("src/a.rs", "old_hash", "1.0", "1.0")]);

        let classified = classify_paths(&desired, &stored, "1.0", "1.0", false);

        assert_classified_action(&classified, PathAction::Changed);
    }

    #[test]
    fn classify_changed_parser_version() {
        let desired = make_manifest(vec![("src/a.rs", "abc123")]);
        let stored = make_stored(vec![("src/a.rs", "abc123", "0.9", "1.0")]);

        let classified = classify_paths(&desired, &stored, "1.0", "1.0", false);

        assert_classified_action(&classified, PathAction::Changed);
    }

    #[test]
    fn classify_removed_path() {
        let desired = DesiredManifest::new();
        let stored = make_stored(vec![("src/a.rs", "abc123", "1.0", "1.0")]);

        let classified = classify_paths(&desired, &stored, "1.0", "1.0", false);

        assert_classified_action(&classified, PathAction::Removed);
    }

    #[test]
    fn repair_mode_forces_changed() {
        let desired = make_manifest(vec![("src/a.rs", "abc123")]);
        let stored = make_stored(vec![("src/a.rs", "abc123", "1.0", "1.0")]);

        let classified = classify_paths(&desired, &stored, "1.0", "1.0", true);

        assert_classified_action(&classified, PathAction::Changed);
    }

    #[test]
    fn desired_file_state_uses_head_for_unchanged_tracked_file() {
        let repo = tempdir().expect("temp dir");
        let path = "src/a.rs";
        let content = b"fn same() {}\n";
        let content_id = compute_blob_oid(content);
        fs::create_dir_all(repo.path().join("src")).expect("create src dir");
        fs::write(repo.path().join(path), content).expect("write worktree file");

        let state = build_desired_file_state(
            path,
            "rust",
            &workspace_state_with_head(path, &content_id),
            repo.path(),
            &HashSet::new(),
            &HashSet::new(),
        )
        .expect("build desired file state")
        .expect("desired state should exist");

        assert_eq!(state.effective_source, EffectiveSource::Head);
        assert_eq!(state.effective_content_id, content_id);
        assert_eq!(state.index_content_id.as_deref(), Some(content_id.as_str()));
    }

    #[test]
    fn desired_file_state_uses_index_for_staged_change_with_clean_worktree() {
        let repo = tempdir().expect("temp dir");
        let path = "src/a.rs";
        let head_content = b"fn old() {}\n";
        let worktree_content = b"fn staged() {}\n";
        let head_content_id = compute_blob_oid(head_content);
        let index_content_id = compute_blob_oid(worktree_content);
        fs::create_dir_all(repo.path().join("src")).expect("create src dir");
        fs::write(repo.path().join(path), worktree_content).expect("write worktree file");

        let mut workspace = workspace_state_with_head(path, &head_content_id);
        workspace
            .staged_changes
            .insert(path.to_string(), crate::host::devql::sync::workspace_state::StagedChange::Modified(index_content_id.clone()));

        let state = build_desired_file_state(
            path,
            "rust",
            &workspace,
            repo.path(),
            &HashSet::new(),
            &HashSet::new(),
        )
        .expect("build desired file state")
        .expect("desired state should exist");

        assert_eq!(state.effective_source, EffectiveSource::Index);
        assert_eq!(state.effective_content_id, index_content_id);
    }

    #[test]
    fn desired_file_state_uses_worktree_for_dirty_unstaged_change() {
        let repo = tempdir().expect("temp dir");
        let path = "src/a.rs";
        let head_content = b"fn stable() {}\n";
        let dirty_content = b"fn dirty() {}\n";
        let head_content_id = compute_blob_oid(head_content);
        let dirty_content_id = compute_blob_oid(dirty_content);
        fs::create_dir_all(repo.path().join("src")).expect("create src dir");
        fs::write(repo.path().join(path), dirty_content).expect("write dirty worktree file");

        let state = build_desired_file_state(
            path,
            "rust",
            &workspace_state_with_head(path, &head_content_id),
            repo.path(),
            &HashSet::from([path]),
            &HashSet::new(),
        )
        .expect("build desired file state")
        .expect("desired state should exist");

        assert_eq!(state.effective_source, EffectiveSource::Worktree);
        assert_eq!(state.effective_content_id, dirty_content_id);
        assert_eq!(
            state.worktree_content_id.as_deref(),
            Some(dirty_content_id.as_str())
        );
    }

    #[test]
    fn desired_file_state_uses_worktree_for_untracked_file() {
        let repo = tempdir().expect("temp dir");
        let path = "src/new.rs";
        let content = b"fn created() {}\n";
        let content_id = compute_blob_oid(content);
        fs::create_dir_all(repo.path().join("src")).expect("create src dir");
        fs::write(repo.path().join(path), content).expect("write untracked worktree file");

        let state = build_desired_file_state(
            path,
            "rust",
            &empty_workspace_state(),
            repo.path(),
            &HashSet::new(),
            &HashSet::from([path]),
        )
        .expect("build desired file state")
        .expect("desired state should exist");

        assert_eq!(state.effective_source, EffectiveSource::Worktree);
        assert_eq!(state.effective_content_id, content_id);
        assert!(!state.exists_in_index);
        assert!(!state.exists_in_head);
    }

    #[test]
    fn desired_file_state_uses_index_for_staged_new_file() {
        let repo = tempdir().expect("temp dir");
        let path = "src/a.rs";
        let content = b"fn staged_new() {}\n";
        let content_id = compute_blob_oid(content);
        fs::create_dir_all(repo.path().join("src")).expect("create src dir");
        fs::write(repo.path().join(path), content).expect("write staged worktree file");

        let mut workspace = empty_workspace_state();
        workspace.staged_changes.insert(
            path.to_string(),
            crate::host::devql::sync::workspace_state::StagedChange::Added(content_id.clone()),
        );

        let state = build_desired_file_state(
            path,
            "rust",
            &workspace,
            repo.path(),
            &HashSet::new(),
            &HashSet::new(),
        )
        .expect("build desired file state")
        .expect("desired state should exist");

        assert_eq!(state.effective_source, EffectiveSource::Index);
        assert_eq!(state.effective_content_id, content_id);
        assert_eq!(state.index_content_id.as_deref(), Some(content_id.as_str()));
        assert!(state.exists_in_worktree);
    }

    #[test]
    fn desired_file_state_uses_index_when_worktree_matches_index_and_index_differs_from_head() {
        let repo = tempdir().expect("temp dir");
        let path = "src/a.rs";
        let head_content = b"fn old() {}\n";
        let staged_content = b"fn new() {}\n";
        let head_content_id = compute_blob_oid(head_content);
        let index_content_id = compute_blob_oid(staged_content);
        fs::create_dir_all(repo.path().join("src")).expect("create src dir");
        fs::write(repo.path().join(path), staged_content).expect("write worktree file");

        let mut workspace = workspace_state_with_head(path, &head_content_id);
        workspace.staged_changes.insert(
            path.to_string(),
            crate::host::devql::sync::workspace_state::StagedChange::Modified(index_content_id.clone()),
        );

        let state = build_desired_file_state(
            path,
            "rust",
            &workspace,
            repo.path(),
            &HashSet::new(),
            &HashSet::new(),
        )
        .expect("build desired file state")
        .expect("desired state should exist");

        assert_eq!(state.effective_source, EffectiveSource::Index);
        assert_eq!(state.effective_content_id, index_content_id);
    }

    fn assert_classified_action(classified: &[ClassifiedPath], expected: PathAction) {
        assert_eq!(classified.len(), 1);
        assert_eq!(classified[0].path, "src/a.rs");
        assert_eq!(classified[0].action, expected);
    }

    fn make_manifest(entries: Vec<(&str, &str)>) -> DesiredManifest {
        entries
            .into_iter()
            .map(|(path, content_id)| {
                (
                    path.to_string(),
                    DesiredFileState {
                        path: path.to_string(),
                        language: "rust".to_string(),
                        head_content_id: Some(content_id.to_string()),
                        index_content_id: Some(content_id.to_string()),
                        worktree_content_id: Some(content_id.to_string()),
                        effective_content_id: content_id.to_string(),
                        effective_source: EffectiveSource::Head,
                        exists_in_head: true,
                        exists_in_index: true,
                        exists_in_worktree: true,
                    },
                )
            })
            .collect()
    }

    fn make_stored(entries: Vec<(&str, &str, &str, &str)>) -> StoredManifest {
        entries
            .into_iter()
            .map(|(path, content_id, parser_version, extractor_version)| {
                (
                    path.to_string(),
                    StoredFileState {
                        path: path.to_string(),
                        language: "rust".to_string(),
                        effective_content_id: content_id.to_string(),
                        parser_version: parser_version.to_string(),
                        extractor_version: extractor_version.to_string(),
                    },
                )
            })
            .collect()
    }

    fn workspace_state_with_head(path: &str, content_id: &str) -> WorkspaceState {
        let mut head_tree = HashMap::new();
        head_tree.insert(path.to_string(), content_id.to_string());
        WorkspaceState {
            head_commit_sha: Some("head".to_string()),
            head_tree_sha: Some("tree".to_string()),
            active_branch: Some("main".to_string()),
            head_tree,
            staged_changes: HashMap::new(),
            dirty_files: Vec::new(),
            untracked_files: Vec::new(),
        }
    }

    fn empty_workspace_state() -> WorkspaceState {
        WorkspaceState {
            head_commit_sha: None,
            head_tree_sha: None,
            active_branch: Some("main".to_string()),
            head_tree: HashMap::new(),
            staged_changes: HashMap::new(),
            dirty_files: Vec::new(),
            untracked_files: Vec::new(),
        }
    }
}

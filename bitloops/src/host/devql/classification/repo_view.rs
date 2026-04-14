use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::path_rules::parent_dir;

pub(super) trait RepoContentView {
    fn list_dir_entries(&self, dir: &str) -> Result<Vec<String>>;
    fn read_text(&self, path: &str) -> Result<Option<String>>;
}

pub(super) struct FsRepoContentView {
    repo_root: PathBuf,
}

impl FsRepoContentView {
    pub(super) fn new(repo_root: PathBuf) -> Self {
        Self { repo_root }
    }
}

impl RepoContentView for FsRepoContentView {
    fn list_dir_entries(&self, dir: &str) -> Result<Vec<String>> {
        let full = if dir.is_empty() {
            self.repo_root.clone()
        } else {
            self.repo_root.join(dir)
        };
        let entries = match fs::read_dir(&full) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => {
                return Err(anyhow::Error::from(err))
                    .with_context(|| format!("listing directory {}", full.display()));
            }
        };
        let mut out = entries
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect::<Vec<_>>();
        out.sort();
        out.dedup();
        Ok(out)
    }

    fn read_text(&self, path: &str) -> Result<Option<String>> {
        let full = self.repo_root.join(path);
        match fs::read_to_string(&full) {
            Ok(content) => Ok(Some(content)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) if err.kind() == std::io::ErrorKind::InvalidData => Ok(None),
            Err(err) => {
                Err(anyhow::Error::from(err)).with_context(|| format!("reading {}", full.display()))
            }
        }
    }
}

pub(super) struct RevisionRepoContentView {
    repo_root: PathBuf,
    revision: String,
    dir_entries: HashMap<String, Vec<String>>,
}

impl RevisionRepoContentView {
    pub(super) fn new(repo_root: PathBuf, revision: String, candidate_paths: &[String]) -> Self {
        let mut dir_entries = HashMap::<String, BTreeSet<String>>::new();
        for path in candidate_paths {
            let parent = parent_dir(path);
            let file_name = Path::new(path)
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_string();
            if !file_name.is_empty() {
                dir_entries.entry(parent).or_default().insert(file_name);
            }
        }
        Self {
            repo_root,
            revision,
            dir_entries: dir_entries
                .into_iter()
                .map(|(dir, entries)| (dir, entries.into_iter().collect()))
                .collect(),
        }
    }
}

impl RepoContentView for RevisionRepoContentView {
    fn list_dir_entries(&self, dir: &str) -> Result<Vec<String>> {
        Ok(self.dir_entries.get(dir).cloned().unwrap_or_default())
    }

    fn read_text(&self, path: &str) -> Result<Option<String>> {
        let spec = format!("{}:{}", self.revision, path);
        match crate::host::checkpoints::strategy::manual_commit::run_git(
            &self.repo_root,
            &["show", &spec],
        ) {
            Ok(content) => Ok(Some(content)),
            Err(_) => Ok(None),
        }
    }
}

use std::ffi::OsStr;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use walkdir::WalkDir;

use crate::capability_packs::test_harness::mapping::model::CandidateTestFile;
use crate::host::language_adapter::LanguageTestSupport;

pub(crate) fn discover_test_files(
    repo_dir: &Path,
    providers: &[Arc<dyn LanguageTestSupport>],
) -> Result<Vec<CandidateTestFile>> {
    let mut files = Vec::new();

    for entry in WalkDir::new(repo_dir)
        .into_iter()
        .filter_entry(|entry| !is_ignored_path(entry.path(), repo_dir))
        .filter_map(|item| item.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }

        let relative = entry
            .path()
            .strip_prefix(repo_dir)
            .with_context(|| format!("file {} is not under repo dir", entry.path().display()))?;
        let relative_path = normalize_rel_path(relative);

        let Some((language_id, priority)) = providers.iter().find_map(|provider| {
            provider
                .supports_path(entry.path(), &relative_path)
                .then_some((provider.language_id().to_string(), provider.priority()))
        }) else {
            continue;
        };

        files.push(CandidateTestFile {
            relative_path,
            language_id,
            priority,
        });
    }

    files.sort_by(|a, b| {
        a.priority
            .cmp(&b.priority)
            .then(a.relative_path.cmp(&b.relative_path))
    });
    Ok(files)
}

pub(crate) fn is_ignored_path(path: &Path, repo_dir: &Path) -> bool {
    let relative = path.strip_prefix(repo_dir).unwrap_or(path);
    relative.components().any(|component| match component {
        Component::Normal(value) => is_ignored_component(value),
        _ => false,
    })
}

fn is_ignored_component(component: &OsStr) -> bool {
    component == OsStr::new("node_modules")
        || component == OsStr::new("coverage")
        || component == OsStr::new("dist")
        || component == OsStr::new("target")
}

pub(crate) fn normalize_rel_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::is_ignored_path;
    use std::path::Path;

    #[test]
    fn absolute_ancestor_target_directory_does_not_hide_repo_files() {
        let repo_dir = Path::new("/tmp/target/qat-runs/run-123/bitloops");
        let file_path =
            Path::new("/tmp/target/qat-runs/run-123/bitloops/tests/user-service.test.ts");
        assert!(!is_ignored_path(file_path, repo_dir));
    }

    #[test]
    fn nested_target_directory_inside_repo_is_ignored() {
        let repo_dir = Path::new("/tmp/target/qat-runs/run-123/bitloops");
        let file_path =
            Path::new("/tmp/target/qat-runs/run-123/bitloops/target/generated/test-output.test.ts");
        assert!(is_ignored_path(file_path, repo_dir));
    }
}

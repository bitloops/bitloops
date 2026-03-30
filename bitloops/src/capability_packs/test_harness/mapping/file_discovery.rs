use std::path::Path;
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
        .filter_entry(|entry| !is_ignored_path(entry.path()))
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

pub(crate) fn is_ignored_path(path: &Path) -> bool {
    let normalized = path.to_string_lossy().replace('\\', "/");
    normalized.contains("/node_modules/")
        || normalized.contains("/coverage/")
        || normalized.contains("/dist/")
        || normalized.contains("/target/")
}

pub(crate) fn normalize_rel_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

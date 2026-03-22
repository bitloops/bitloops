use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};
use walkdir::WalkDir;

use crate::capability_packs::test_harness::mapping::model::CandidateTestFile;
use crate::capability_packs::test_harness::mapping::registry::LanguageProvider;

pub(crate) fn discover_test_files(
    repo_dir: &Path,
    providers: &[Box<dyn LanguageProvider>],
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

        let Some((provider_index, priority)) =
            providers.iter().enumerate().find_map(|(index, provider)| {
                provider
                    .supports_path(entry.path(), &relative_path)
                    .then_some((index, provider.priority()))
            })
        else {
            continue;
        };

        files.push(CandidateTestFile {
            relative_path,
            provider_index,
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

pub(crate) fn looks_like_inline_rust_test_source(
    absolute_path: &Path,
    relative_path: &str,
) -> bool {
    if !relative_path.ends_with(".rs") {
        return false;
    }
    if !(relative_path.starts_with("src/") || relative_path.contains("/src/")) {
        return false;
    }

    let Ok(source) = fs::read_to_string(absolute_path) else {
        return false;
    };

    rust_source_contains_test_markers(&source) || rust_source_contains_doctest_markers(&source)
}

pub(crate) fn rust_source_contains_test_markers(source: &str) -> bool {
    source.contains("#[cfg(test)]")
        || source.contains("#[test")
        || source.contains("::test")
        || source.contains("#[test_case")
        || source.contains("::test_case")
        || source.contains("#[rstest")
        || source.contains("::rstest")
        || source.contains("#[wasm_bindgen_test")
        || source.contains("::wasm_bindgen_test")
        || source.contains("#[quickcheck")
        || source.contains("::quickcheck")
        || source.contains("proptest!")
}

pub(crate) fn rust_source_contains_doctest_markers(source: &str) -> bool {
    let mut in_block_doc = false;

    for line in source.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("/// ```") || trimmed.starts_with("//! ```") {
            return true;
        }

        if trimmed.starts_with("/**") || trimmed.starts_with("/*!") {
            in_block_doc = true;
            if trimmed.contains("```") {
                return true;
            }
        } else if in_block_doc && trimmed.contains("```") {
            return true;
        }

        if in_block_doc && trimmed.contains("*/") {
            in_block_doc = false;
        }
    }

    false
}

pub(crate) fn read_source_file(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| format!("failed reading test file {}", path.display()))
}

pub(crate) fn normalize_join(base: &Path, relative: &Path) -> PathBuf {
    let joined = base.join(relative);
    let mut normalized = PathBuf::new();

    for component in joined.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(part) => normalized.push(part),
            Component::RootDir | Component::Prefix(_) => normalized.push(component.as_os_str()),
        }
    }

    normalized
}

pub(crate) fn normalize_rel_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

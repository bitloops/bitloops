use std::ffi::OsStr;
use std::path::{Component, Path};
use std::process::Command;
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

    for relative_path in candidate_file_paths(repo_dir)? {
        let absolute_path = repo_dir.join(&relative_path);
        if !absolute_path.is_file() {
            continue;
        }

        let Some((language_id, priority)) = providers.iter().find_map(|provider| {
            provider
                .supports_path(&absolute_path, &relative_path)
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

fn candidate_file_paths(repo_dir: &Path) -> Result<Vec<String>> {
    if let Some(paths) = git_candidate_file_paths(repo_dir)? {
        return Ok(paths);
    }

    walk_candidate_file_paths(repo_dir)
}

fn git_candidate_file_paths(repo_dir: &Path) -> Result<Option<Vec<String>>> {
    let output = match Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
        ])
        .output()
    {
        Ok(output) => output,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).context("listing Git-visible test discovery candidates"),
    };

    if !output.status.success() {
        return Ok(None);
    }

    let mut paths = output
        .stdout
        .split(|byte| *byte == b'\0')
        .filter(|path| !path.is_empty())
        .map(|path| String::from_utf8_lossy(path).replace('\\', "/"))
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    Ok(Some(paths))
}

fn walk_candidate_file_paths(repo_dir: &Path) -> Result<Vec<String>> {
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
        files.push(normalize_rel_path(relative));
    }

    files.sort();
    files.dedup();
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
    component == OsStr::new(".git")
        || component == OsStr::new(".mypy_cache")
        || component == OsStr::new(".pytest_cache")
        || component == OsStr::new(".tox")
        || component == OsStr::new(".venv")
        || component == OsStr::new("__pycache__")
        || component == OsStr::new("node_modules")
        || component == OsStr::new("coverage")
        || component == OsStr::new("dist")
        || component == OsStr::new("venv")
        || component == OsStr::new("target")
}

pub(crate) fn normalize_rel_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::{candidate_file_paths, is_ignored_path};
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    use tempfile::TempDir;

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

    #[test]
    fn fallback_walk_ignores_virtualenv_directory() {
        let repo = TempDir::new().expect("temp repo");
        fs::create_dir_all(
            repo.path()
                .join(".venv/lib/python3.12/site-packages/pkg/tests"),
        )
        .expect("create virtualenv test path");
        fs::create_dir_all(repo.path().join("tests")).expect("create tests dir");
        fs::write(
            repo.path()
                .join(".venv/lib/python3.12/site-packages/pkg/tests/test_hidden.py"),
            "def test_hidden(): pass\n",
        )
        .expect("write hidden test");
        fs::write(
            repo.path().join("tests/test_visible.py"),
            "def test_visible(): pass\n",
        )
        .expect("write visible test");

        let paths = candidate_file_paths(repo.path()).expect("discover candidate paths");

        assert!(paths.contains(&"tests/test_visible.py".to_string()));
        assert!(
            !paths
                .iter()
                .any(|path| path.contains("site-packages/pkg/tests/test_hidden.py")),
            "fallback discovery should skip virtualenv files"
        );
    }

    #[test]
    fn git_discovery_respects_gitignored_virtualenv_directory() {
        let repo = TempDir::new().expect("temp repo");
        let init = Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .arg("init")
            .output()
            .expect("run git init");
        assert!(
            init.status.success(),
            "git init failed: {}",
            String::from_utf8_lossy(&init.stderr)
        );

        fs::write(repo.path().join(".gitignore"), ".venv\n").expect("write gitignore");
        fs::create_dir_all(
            repo.path()
                .join(".venv/lib/python3.12/site-packages/pkg/tests"),
        )
        .expect("create virtualenv test path");
        fs::create_dir_all(repo.path().join("tests")).expect("create tests dir");
        fs::write(
            repo.path()
                .join(".venv/lib/python3.12/site-packages/pkg/tests/test_hidden.py"),
            "def test_hidden(): pass\n",
        )
        .expect("write hidden test");
        fs::write(
            repo.path().join("tests/test_visible.py"),
            "def test_visible(): pass\n",
        )
        .expect("write visible test");

        let paths = candidate_file_paths(repo.path()).expect("discover candidate paths");

        assert!(paths.contains(&".gitignore".to_string()));
        assert!(paths.contains(&"tests/test_visible.py".to_string()));
        assert!(
            !paths
                .iter()
                .any(|path| path.contains("site-packages/pkg/tests/test_hidden.py")),
            "Git discovery should honour .gitignore"
        );
    }
}

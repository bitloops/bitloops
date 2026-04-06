use super::*;
use std::process::Command;

pub(crate) fn collect_checkpoint_file_provenance_rows(
    repo_root: &Path,
    ctx: CheckpointProvenanceContext<'_>,
) -> Result<Vec<CheckpointFileProvenanceRow>> {
    let entries = git_commit_diff_entries(repo_root, ctx.commit_sha)?;
    let parent_commit = commit_first_parent(repo_root, ctx.commit_sha);
    let mut rows = Vec::new();

    for entry in entries {
        let before_path = entry.path_before.as_deref().map(normalize_repo_path);
        let after_path = entry.path_after.as_deref().map(normalize_repo_path);
        let copy_source_path = entry.copy_source_path.as_deref().map(normalize_repo_path);
        let before_path = before_path.filter(|path| !path.is_empty());
        let after_path = after_path.filter(|path| !path.is_empty());
        let copy_source_path = copy_source_path.filter(|path| !path.is_empty());

        let all_infra = [
            before_path.as_deref(),
            after_path.as_deref(),
            copy_source_path.as_deref(),
        ]
        .into_iter()
        .flatten()
        .all(crate::utils::paths::is_infrastructure_path);
        if all_infra {
            continue;
        }

        let blob_sha_before = before_path.as_deref().and_then(|path| {
            parent_commit
                .as_deref()
                .and_then(|parent| git_blob_sha(repo_root, parent, path))
        });
        let blob_sha_after = after_path
            .as_deref()
            .and_then(|path| git_blob_sha(repo_root, ctx.commit_sha, path));
        let copy_source_blob_sha = copy_source_path
            .as_deref()
            .and_then(|path| git_blob_sha(repo_root, ctx.commit_sha, path));

        let mut row = CheckpointFileProvenanceRow {
            relation_id: String::new(),
            repo_id: ctx.repo_id.to_string(),
            checkpoint_id: ctx.checkpoint_id.to_string(),
            session_id: ctx.session_id.to_string(),
            event_time: ctx.event_time.to_string(),
            agent: ctx.agent.to_string(),
            branch: ctx.branch.to_string(),
            strategy: ctx.strategy.to_string(),
            commit_sha: ctx.commit_sha.to_string(),
            change_kind: entry.change_kind,
            path_before: before_path,
            path_after: after_path,
            blob_sha_before,
            blob_sha_after,
            copy_source_path,
            copy_source_blob_sha,
        };
        row.relation_id = row.deterministic_id();
        rows.push(row);
    }

    Ok(rows)
}

pub(super) fn git_blob_content(repo_root: &Path, blob_sha: &str) -> Option<String> {
    run_git(repo_root, &["cat-file", "-p", blob_sha]).ok()
}

fn commit_first_parent(repo_root: &Path, commit_sha: &str) -> Option<String> {
    let raw = run_git(repo_root, &["rev-list", "--parents", "-n", "1", commit_sha]).ok()?;
    raw.split_whitespace().nth(1).map(str::to_string)
}

fn git_blob_sha(repo_root: &Path, revision: &str, path: &str) -> Option<String> {
    run_git(repo_root, &["rev-parse", &format!("{revision}:{path}")])
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GitCommitDiffEntry {
    change_kind: CheckpointFileChangeKind,
    path_before: Option<String>,
    path_after: Option<String>,
    copy_source_path: Option<String>,
}

fn git_commit_diff_entries(repo_root: &Path, commit_sha: &str) -> Result<Vec<GitCommitDiffEntry>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args([
            "diff-tree",
            "--root",
            "--no-commit-id",
            "--name-status",
            "--find-renames",
            "--find-copies-harder",
            "-r",
            "-z",
            commit_sha,
        ])
        .output()
        .with_context(|| {
            format!("running git diff-tree for checkpoint provenance at {commit_sha}")
        })?;
    if !output.status.success() {
        bail!(
            "git diff-tree failed for checkpoint provenance (commit_sha={}): {}",
            commit_sha,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let fields = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .map(|field| String::from_utf8_lossy(field).to_string())
        .collect::<Vec<_>>();

    let mut rows = Vec::new();
    let mut index = 0usize;
    while index < fields.len() {
        let status = fields[index].trim().to_string();
        index += 1;
        if status.is_empty() {
            continue;
        }

        let kind = parse_change_kind(&status);
        match kind {
            CheckpointFileChangeKind::Rename | CheckpointFileChangeKind::Copy => {
                if index + 1 >= fields.len() {
                    break;
                }
                let source = fields[index].trim().to_string();
                let after = fields[index + 1].trim().to_string();
                index += 2;
                rows.push(GitCommitDiffEntry {
                    change_kind: kind,
                    path_before: (kind == CheckpointFileChangeKind::Rename)
                        .then_some(source.clone()),
                    path_after: Some(after),
                    copy_source_path: (kind == CheckpointFileChangeKind::Copy).then_some(source),
                });
            }
            CheckpointFileChangeKind::Add => {
                if index >= fields.len() {
                    break;
                }
                let after = fields[index].trim().to_string();
                index += 1;
                rows.push(GitCommitDiffEntry {
                    change_kind: CheckpointFileChangeKind::Add,
                    path_before: None,
                    path_after: Some(after),
                    copy_source_path: None,
                });
            }
            CheckpointFileChangeKind::Delete => {
                if index >= fields.len() {
                    break;
                }
                let before = fields[index].trim().to_string();
                index += 1;
                rows.push(GitCommitDiffEntry {
                    change_kind: CheckpointFileChangeKind::Delete,
                    path_before: Some(before),
                    path_after: None,
                    copy_source_path: None,
                });
            }
            CheckpointFileChangeKind::Modify => {
                if index >= fields.len() {
                    break;
                }
                let path = fields[index].trim().to_string();
                index += 1;
                rows.push(GitCommitDiffEntry {
                    change_kind: CheckpointFileChangeKind::Modify,
                    path_before: Some(path.clone()),
                    path_after: Some(path),
                    copy_source_path: None,
                });
            }
        }
    }

    Ok(rows)
}

fn parse_change_kind(status: &str) -> CheckpointFileChangeKind {
    let leading = status.chars().next().unwrap_or('M');
    match leading {
        'A' => CheckpointFileChangeKind::Add,
        'D' => CheckpointFileChangeKind::Delete,
        'R' => CheckpointFileChangeKind::Rename,
        'C' => CheckpointFileChangeKind::Copy,
        _ => CheckpointFileChangeKind::Modify,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::git_fixtures::{git_ok, init_test_repo};
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn pure_copy_commit_is_reported_with_copy_origin() {
        let repo = TempDir::new().expect("temp dir");
        init_test_repo(
            repo.path(),
            "main",
            "Checkpoint Provenance",
            "provenance@example.com",
        );
        fs::create_dir_all(repo.path().join("src")).expect("create src dir");
        fs::write(repo.path().join("src/a.txt"), "one\n").expect("write source file");
        git_ok(repo.path(), &["add", "src/a.txt"]);
        git_ok(repo.path(), &["commit", "-m", "seed"]);

        fs::copy(repo.path().join("src/a.txt"), repo.path().join("src/b.txt"))
            .expect("copy source file");
        git_ok(repo.path(), &["add", "src/b.txt"]);
        git_ok(repo.path(), &["commit", "-m", "copy"]);

        let commit_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);
        let entries = git_commit_diff_entries(repo.path(), commit_sha.trim())
            .expect("collect git diff entries");

        assert_eq!(
            entries,
            vec![GitCommitDiffEntry {
                change_kind: CheckpointFileChangeKind::Copy,
                path_before: None,
                path_after: Some("src/b.txt".to_string()),
                copy_source_path: Some("src/a.txt".to_string()),
            }]
        );
    }

    #[test]
    fn modify_and_copy_commit_reports_modify_and_copy() {
        let repo = TempDir::new().expect("temp dir");
        init_test_repo(
            repo.path(),
            "main",
            "Checkpoint Provenance",
            "provenance@example.com",
        );
        fs::create_dir_all(repo.path().join("src")).expect("create src dir");
        fs::write(repo.path().join("src/a.txt"), "one\n").expect("write source file");
        git_ok(repo.path(), &["add", "src/a.txt"]);
        git_ok(repo.path(), &["commit", "-m", "seed"]);

        fs::write(repo.path().join("src/a.txt"), "one\ntwo\n").expect("rewrite source file");
        fs::copy(repo.path().join("src/a.txt"), repo.path().join("src/b.txt"))
            .expect("copy source file");
        git_ok(repo.path(), &["add", "src/a.txt", "src/b.txt"]);
        git_ok(repo.path(), &["commit", "-m", "modify and copy"]);

        let commit_sha = git_ok(repo.path(), &["rev-parse", "HEAD"]);
        let entries = git_commit_diff_entries(repo.path(), commit_sha.trim())
            .expect("collect git diff entries");

        assert_eq!(entries.len(), 2);
        assert!(entries.contains(&GitCommitDiffEntry {
            change_kind: CheckpointFileChangeKind::Modify,
            path_before: Some("src/a.txt".to_string()),
            path_after: Some("src/a.txt".to_string()),
            copy_source_path: None,
        }));
        assert!(entries.contains(&GitCommitDiffEntry {
            change_kind: CheckpointFileChangeKind::Copy,
            path_before: None,
            path_after: Some("src/b.txt".to_string()),
            copy_source_path: Some("src/a.txt".to_string()),
        }));
    }
}

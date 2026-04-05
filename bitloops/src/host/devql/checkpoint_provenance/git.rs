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
        let before_path = before_path.filter(|path| !path.is_empty());
        let after_path = after_path.filter(|path| !path.is_empty());

        let all_infra = [before_path.as_deref(), after_path.as_deref()]
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
            "--find-copies",
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
            CheckpointFileChangeKind::Rename => {
                if index + 1 >= fields.len() {
                    break;
                }
                let before = fields[index].trim().to_string();
                let after = fields[index + 1].trim().to_string();
                index += 2;
                rows.push(GitCommitDiffEntry {
                    change_kind: CheckpointFileChangeKind::Rename,
                    path_before: Some(before),
                    path_after: Some(after),
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
                });
            }
        }
    }

    Ok(rows)
}

fn parse_change_kind(status: &str) -> CheckpointFileChangeKind {
    let leading = status.chars().next().unwrap_or('M');
    match leading {
        'A' | 'C' => CheckpointFileChangeKind::Add,
        'D' => CheckpointFileChangeKind::Delete,
        'R' => CheckpointFileChangeKind::Rename,
        _ => CheckpointFileChangeKind::Modify,
    }
}

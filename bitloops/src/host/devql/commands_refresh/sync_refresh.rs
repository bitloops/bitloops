use anyhow::{Context, Result};

use crate::host::devql::{DevqlConfig, SyncMode};

use super::super::normalize_repo_path;
use super::filtering::filter_refresh_paths_for_sync;
use super::snapshot::snapshot_committed_current_rows_for_commit_for_config;
use super::stats::PostCommitArtefactRefreshStats;

pub async fn run_post_commit_artefact_refresh(
    cfg: &DevqlConfig,
    commit_sha: &str,
    changed_files: &[String],
) -> Result<PostCommitArtefactRefreshStats> {
    sync_changed_paths(
        cfg,
        changed_files,
        "post-commit",
        Some(crate::daemon::PostCommitSnapshotSpec {
            commit_sha: commit_sha.trim().to_string(),
            changed_paths: changed_files.to_vec(),
        }),
    )
    .await
}

pub async fn run_post_merge_artefact_refresh(
    cfg: &DevqlConfig,
    _commit_sha: &str,
    changed_files: &[String],
) -> Result<PostCommitArtefactRefreshStats> {
    sync_changed_paths(cfg, changed_files, "post-merge", None).await
}

async fn sync_changed_paths(
    cfg: &DevqlConfig,
    changed_files: &[String],
    source_hook: &str,
    post_commit_snapshot: Option<crate::daemon::PostCommitSnapshotSpec>,
) -> Result<PostCommitArtefactRefreshStats> {
    let paths = refresh_paths_for_sync(cfg, changed_files, source_hook)?;
    let post_commit_snapshot = post_commit_snapshot.map(|mut snapshot| {
        snapshot.changed_paths = paths.clone();
        snapshot
    });

    let stats = PostCommitArtefactRefreshStats {
        files_seen: paths.len(),
        ..PostCommitArtefactRefreshStats::default()
    };
    if paths.is_empty() {
        if let Some(snapshot) = post_commit_snapshot.as_ref() {
            snapshot_committed_current_rows_for_commit_for_config(cfg, snapshot)
                .await
                .with_context(|| {
                    format!("snapshotting committed current rows for {source_hook} refresh")
                })?;
        }
        return Ok(stats);
    }

    #[cfg(test)]
    {
        let summary = crate::host::devql::run_sync_with_summary(cfg, SyncMode::Paths(paths))
            .await
            .with_context(|| {
                format!("running DevQL sync inline for {source_hook} refresh in tests")
            })?;
        if let Some(snapshot) = post_commit_snapshot.as_ref() {
            snapshot_committed_current_rows_for_commit_for_config(cfg, snapshot)
                .await
                .with_context(|| {
                    format!("snapshotting committed current rows after {source_hook} sync in tests")
                })?;
        }
        Ok(PostCommitArtefactRefreshStats::inline_from_summary(
            stats.files_seen,
            &summary,
        ))
    }

    #[cfg(not(test))]
    {
        let source = match source_hook {
            "post-commit" => crate::daemon::DevqlTaskSource::PostCommit,
            "post-merge" => crate::daemon::DevqlTaskSource::PostMerge,
            _ => crate::daemon::DevqlTaskSource::ManualCli,
        };
        let queued = crate::daemon::enqueue_sync_for_config_with_snapshot(
            cfg,
            source,
            SyncMode::Paths(paths),
            post_commit_snapshot,
        )
        .with_context(|| format!("queueing DevQL sync for {source_hook} refresh"))?;
        Ok(PostCommitArtefactRefreshStats::queued(
            stats.files_seen,
            queued,
        ))
    }
}

pub(crate) fn refresh_paths_for_sync(
    cfg: &DevqlConfig,
    changed_files: &[String],
    source_hook: &str,
) -> Result<Vec<String>> {
    let mut paths = changed_files
        .iter()
        .map(|raw| normalize_repo_path(raw))
        .filter(|path| !path.is_empty())
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    filter_refresh_paths_for_sync(cfg, &paths, source_hook)
}

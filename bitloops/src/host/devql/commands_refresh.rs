use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedSyncTaskMetadata {
    pub task_id: String,
    pub merged: bool,
    pub queue_position: Option<u64>,
    pub tasks_ahead: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PostCommitArtefactRefreshStats {
    pub files_seen: usize,
    pub files_indexed: usize,
    pub files_deleted: usize,
    pub files_failed: usize,
    pub queued_task: Option<QueuedSyncTaskMetadata>,
}

impl PostCommitArtefactRefreshStats {
    pub(crate) fn completed_with_failures(&self) -> bool {
        self.queued_task.is_none() && self.files_failed > 0
    }

    #[cfg_attr(not(test), allow(dead_code))]
    fn inline_from_summary(files_seen: usize, summary: &SyncSummary) -> Self {
        Self {
            files_seen,
            files_indexed: summary.paths_added + summary.paths_changed,
            files_deleted: summary.paths_removed,
            files_failed: summary.parse_errors,
            queued_task: None,
        }
    }

    fn queued(files_seen: usize, queued: crate::daemon::DevqlTaskEnqueueResult) -> Self {
        Self {
            files_seen,
            files_indexed: 0,
            files_deleted: 0,
            files_failed: 0,
            queued_task: Some(QueuedSyncTaskMetadata {
                task_id: queued.task.task_id,
                merged: queued.merged,
                queue_position: queued.task.queue_position,
                tasks_ahead: queued.task.tasks_ahead,
            }),
        }
    }
}

pub async fn run_post_commit_artefact_refresh(
    cfg: &DevqlConfig,
    _commit_sha: &str,
    changed_files: &[String],
) -> Result<PostCommitArtefactRefreshStats> {
    sync_changed_paths(cfg, changed_files, "post-commit").await
}

pub async fn run_post_commit_checkpoint_projection_refresh(
    cfg: &DevqlConfig,
    commit_sha: &str,
    checkpoint_id: &str,
) -> Result<()> {
    let commit_sha = commit_sha.trim();
    let checkpoint_id = checkpoint_id.trim();
    if commit_sha.is_empty() || checkpoint_id.is_empty() {
        return Ok(());
    }

    let backends = resolve_store_backend_config_for_repo(&cfg.daemon_config_root)
        .context("resolving DevQL backend config for post-commit checkpoint projection refresh")?;
    let relational = RelationalStorage::connect(
        cfg,
        &backends.relational,
        "git post-commit checkpoint projection refresh",
    )
    .await?;

    refresh_checkpoint_projection_for_commit(cfg, &relational, commit_sha, checkpoint_id).await
}

pub async fn run_post_merge_artefact_refresh(
    cfg: &DevqlConfig,
    _commit_sha: &str,
    changed_files: &[String],
) -> Result<PostCommitArtefactRefreshStats> {
    sync_changed_paths(cfg, changed_files, "post-merge").await
}

async fn sync_changed_paths(
    cfg: &DevqlConfig,
    changed_files: &[String],
    source_hook: &str,
) -> Result<PostCommitArtefactRefreshStats> {
    let mut paths = changed_files
        .iter()
        .map(|raw| normalize_repo_path(raw))
        .filter(|path| !path.is_empty())
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();

    let stats = PostCommitArtefactRefreshStats {
        files_seen: paths.len(),
        ..PostCommitArtefactRefreshStats::default()
    };
    if paths.is_empty() {
        return Ok(stats);
    }

    #[cfg(test)]
    {
        let summary = crate::host::devql::run_sync_with_summary(cfg, SyncMode::Paths(paths))
            .await
            .with_context(|| {
                format!("running DevQL sync inline for {source_hook} refresh in tests")
            })?;
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
        let queued = crate::daemon::enqueue_sync_for_config(cfg, source, SyncMode::Paths(paths))
            .with_context(|| format!("queueing DevQL sync for {source_hook} refresh"))?;
        Ok(PostCommitArtefactRefreshStats::queued(
            stats.files_seen,
            queued,
        ))
    }
}

async fn refresh_checkpoint_projection_for_commit(
    cfg: &DevqlConfig,
    relational: &RelationalStorage,
    commit_sha: &str,
    checkpoint_id: &str,
) -> Result<()> {
    ensure_repository_row(cfg, relational).await?;

    let checkpoint = crate::host::checkpoints::strategy::manual_commit::read_committed_info(
        &cfg.repo_root,
        checkpoint_id,
    )?
    .ok_or_else(|| {
        anyhow::anyhow!("checkpoint not found for projection refresh: {checkpoint_id}")
    })?;
    let commit_info = checkpoint_commit_info_from_sha(&cfg.repo_root, commit_sha);

    let _projected_rows = upsert_checkpoint_file_snapshot_rows(
        cfg,
        relational,
        &checkpoint,
        commit_sha,
        commit_info.as_ref(),
    )
    .await?;

    Ok(())
}

pub async fn run_post_checkout_branch_seed(
    cfg: &DevqlConfig,
    _previous_head: &str,
    new_head: &str,
    is_branch_checkout: bool,
) -> Result<()> {
    if !is_branch_checkout || new_head.trim().is_empty() || is_zero_git_oid(new_head) {
        return Ok(());
    }

    #[cfg(test)]
    {
        crate::host::devql::run_sync_with_summary(cfg, SyncMode::Full)
            .await
            .context("running full DevQL sync inline for post-checkout branch seed in tests")?;
        Ok(())
    }

    #[cfg(not(test))]
    {
        crate::daemon::enqueue_sync_for_config(
            cfg,
            crate::daemon::DevqlTaskSource::PostCheckout,
            SyncMode::Full,
        )
        .context("queueing full DevQL sync for post-checkout branch seed")?;
        Ok(())
    }
}

fn is_zero_git_oid(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty() && trimmed.chars().all(|ch| ch == '0')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::{
        DevqlTaskKind, DevqlTaskProgress, DevqlTaskRecord, DevqlTaskSource, DevqlTaskSpec,
        DevqlTaskStatus, SyncTaskMode, SyncTaskSpec,
    };

    fn sample_queued_result() -> crate::daemon::DevqlTaskEnqueueResult {
        crate::daemon::DevqlTaskEnqueueResult {
            task: DevqlTaskRecord {
                task_id: "sync-task-123".to_string(),
                repo_id: "repo-1".to_string(),
                repo_name: "demo".to_string(),
                repo_provider: "local".to_string(),
                repo_organisation: "local".to_string(),
                repo_identity: "local/demo".to_string(),
                daemon_config_root: PathBuf::from("/tmp/repo"),
                repo_root: PathBuf::from("/tmp/repo"),
                kind: DevqlTaskKind::Sync,
                source: DevqlTaskSource::PostCommit,
                spec: DevqlTaskSpec::Sync(SyncTaskSpec {
                    mode: SyncTaskMode::Paths {
                        paths: vec!["src/lib.rs".to_string()],
                    },
                }),
                status: DevqlTaskStatus::Queued,
                submitted_at_unix: 1,
                started_at_unix: None,
                updated_at_unix: 1,
                completed_at_unix: None,
                queue_position: Some(3),
                tasks_ahead: Some(2),
                progress: DevqlTaskProgress::Sync(SyncProgressUpdate::default()),
                error: None,
                result: None,
            },
            merged: true,
        }
    }

    #[test]
    fn queued_refresh_stats_include_task_metadata() {
        let stats = PostCommitArtefactRefreshStats::queued(2, sample_queued_result());

        assert_eq!(stats.files_seen, 2);
        assert_eq!(stats.files_indexed, 0);
        assert_eq!(stats.files_deleted, 0);
        assert_eq!(stats.files_failed, 0);
        assert_eq!(
            stats.queued_task,
            Some(QueuedSyncTaskMetadata {
                task_id: "sync-task-123".to_string(),
                merged: true,
                queue_position: Some(3),
                tasks_ahead: Some(2),
            })
        );
        assert!(!stats.completed_with_failures());
    }

    #[test]
    fn inline_refresh_stats_report_completed_failures() {
        let stats = PostCommitArtefactRefreshStats {
            files_seen: 2,
            files_indexed: 1,
            files_deleted: 0,
            files_failed: 1,
            queued_task: None,
        };

        assert!(stats.completed_with_failures());
    }
}

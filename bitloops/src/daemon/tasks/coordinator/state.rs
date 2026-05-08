use std::collections::HashSet;

use anyhow::{Result, anyhow, bail};

use crate::graphql::Checkpoint;
use crate::host::devql::{DevqlConfig, IngestionCounters, SyncSummary};
use crate::host::runtime_store::PersistedDevqlTaskQueueState;

use super::super::super::types::{
    DevqlTaskControlResult, DevqlTaskKind, DevqlTaskProgress, DevqlTaskQueueStatus,
    DevqlTaskRecord, DevqlTaskResult, DevqlTaskSource, DevqlTaskStatus, EmbeddingsBootstrapPhase,
    EmbeddingsBootstrapProgress, EmbeddingsBootstrapResult, RepoTaskControlState,
    SummaryBootstrapPhase, SummaryBootstrapProgress, SummaryBootstrapResultRecord, SyncTaskMode,
    unix_timestamp_now,
};
use super::super::queue::{
    changed_tasks, default_progress_for_spec, failed_progress, ingest_progress_from_summary,
    project_status, prune_terminal_tasks, recompute_queue_positions, sync_progress_from_summary,
};
use super::DevqlTaskCoordinator;
use super::helpers::{progress_action, sync_spec_from_task_spec_mut};

impl DevqlTaskCoordinator {
    pub(crate) fn snapshot(&self, repo_id: Option<&str>) -> Result<DevqlTaskQueueStatus> {
        let state = self.load_state()?;
        let persisted = self.runtime_store.devql_task_state_exists()?;
        Ok(project_status(&state, repo_id, persisted))
    }

    pub(crate) fn task(&self, task_id: &str) -> Result<Option<DevqlTaskRecord>> {
        let state = self.load_state()?;
        Ok(state.tasks.into_iter().find(|task| task.task_id == task_id))
    }

    pub(crate) fn tasks(
        &self,
        repo_id: Option<&str>,
        kind: Option<super::super::super::types::DevqlTaskKind>,
        status: Option<DevqlTaskStatus>,
        limit: Option<usize>,
    ) -> Result<Vec<DevqlTaskRecord>> {
        let state = self.load_state()?;
        let mut tasks = state
            .tasks
            .into_iter()
            .filter(|task| repo_id.is_none_or(|repo_id| task.repo_id == repo_id))
            .filter(|task| kind.is_none_or(|kind| task.kind == kind))
            .filter(|task| status.is_none_or(|status| task.status == status))
            .collect::<Vec<_>>();
        tasks.sort_by(|left, right| {
            right
                .updated_at_unix
                .cmp(&left.updated_at_unix)
                .then_with(|| left.task_id.cmp(&right.task_id))
        });
        if let Some(limit) = limit {
            tasks.truncate(limit);
        }
        Ok(tasks)
    }

    pub(crate) fn pause_repo(
        &self,
        repo_id: &str,
        reason: Option<String>,
    ) -> Result<DevqlTaskControlResult> {
        let repo_id = repo_id.to_string();
        self.mutate_state(|state| {
            let control = state
                .repo_controls
                .entry(repo_id.clone())
                .or_insert_with(|| RepoTaskControlState {
                    repo_id: repo_id.clone(),
                    paused: false,
                    paused_reason: None,
                    updated_at_unix: 0,
                });
            control.paused = true;
            control.paused_reason = reason.clone();
            control.updated_at_unix = unix_timestamp_now();
            state.last_action = Some("paused".to_string());
            Ok(DevqlTaskControlResult {
                message: format!("paused DevQL task queue for {repo_id}"),
                control: control.clone(),
            })
        })
    }

    pub(crate) fn resume_repo(&self, repo_id: &str) -> Result<DevqlTaskControlResult> {
        let repo_id = repo_id.to_string();
        self.mutate_state(|state| {
            let control = state
                .repo_controls
                .entry(repo_id.clone())
                .or_insert_with(|| RepoTaskControlState {
                    repo_id: repo_id.clone(),
                    paused: false,
                    paused_reason: None,
                    updated_at_unix: 0,
                });
            control.paused = false;
            control.paused_reason = None;
            control.updated_at_unix = unix_timestamp_now();
            state.last_action = Some("resumed".to_string());
            Ok(DevqlTaskControlResult {
                message: format!("resumed DevQL task queue for {repo_id}"),
                control: control.clone(),
            })
        })
    }

    pub(crate) fn cancel_task(&self, task_id: &str) -> Result<DevqlTaskRecord> {
        let task_id = task_id.to_string();
        self.mutate_state(|state| {
            let Some(task) = state.tasks.iter_mut().find(|task| task.task_id == task_id) else {
                bail!("unknown task `{task_id}`");
            };
            match task.status {
                DevqlTaskStatus::Queued => {
                    let now = unix_timestamp_now();
                    task.status = DevqlTaskStatus::Cancelled;
                    task.updated_at_unix = now;
                    task.completed_at_unix = Some(now);
                    task.error = None;
                    task.result = None;
                    state.last_action = Some("cancelled".to_string());
                    Ok(task.clone())
                }
                DevqlTaskStatus::Running => {
                    bail!("task `{task_id}` is already running and cannot be cancelled")
                }
                _ => bail!("task `{task_id}` is not queued and cannot be cancelled"),
            }
        })
    }

    pub(super) fn update_sync_mode(&self, task_id: &str, mode: SyncTaskMode) -> Result<()> {
        let task_id = task_id.to_string();
        self.mutate_state(|state| {
            let Some(task) = state.tasks.iter_mut().find(|task| task.task_id == task_id) else {
                return Ok(());
            };
            if let Some(spec) = sync_spec_from_task_spec_mut(&mut task.spec) {
                spec.mode = mode;
            }
            task.updated_at_unix = unix_timestamp_now();
            state.last_action = Some("mode_updated".to_string());
            Ok(())
        })
        .map(|_: ()| ())
    }

    pub(super) fn recover_running_tasks(&self) -> Result<()> {
        self.mutate_state(|state| {
            for task in &mut state.tasks {
                if task.status == DevqlTaskStatus::Running {
                    task.status = DevqlTaskStatus::Queued;
                    task.progress = default_progress_for_spec(&task.spec);
                    task.error = None;
                    task.result = None;
                    task.started_at_unix = None;
                    task.completed_at_unix = None;
                    task.updated_at_unix = unix_timestamp_now();
                }
            }
            state.last_action = Some("recovered_running_tasks".to_string());
            Ok(())
        })
        .map(|_: ()| ())
    }

    pub(super) fn update_task_progress(
        &self,
        task_id: &str,
        update: DevqlTaskProgress,
    ) -> Result<()> {
        let task_id = task_id.to_string();
        self.mutate_state(|state| {
            let Some(task) = state.tasks.iter_mut().find(|task| task.task_id == task_id) else {
                return Ok(());
            };
            task.progress = update.clone();
            task.updated_at_unix = unix_timestamp_now();
            state.last_action = Some(progress_action(&update));
            Ok(())
        })
        .map(|_: ()| ())
    }

    pub(super) fn finish_sync_task_completed(
        &self,
        task_id: &str,
        summary: SyncSummary,
    ) -> Result<()> {
        let task_id = task_id.to_string();
        self.mutate_state(|state| {
            let Some(task) = state.tasks.iter_mut().find(|task| task.task_id == task_id) else {
                return Ok(());
            };
            let now = unix_timestamp_now();
            task.status = DevqlTaskStatus::Completed;
            task.updated_at_unix = now;
            task.completed_at_unix = Some(now);
            task.error = None;
            task.result = Some(DevqlTaskResult::Sync(Box::new(summary.clone())));
            task.progress = DevqlTaskProgress::Sync(sync_progress_from_summary(&summary));
            state.last_action = Some("completed".to_string());
            Ok(())
        })
        .map(|_: ()| ())
    }

    pub(super) fn finish_ingest_task_completed(
        &self,
        task_id: &str,
        summary: IngestionCounters,
    ) -> Result<()> {
        let task_id = task_id.to_string();
        self.mutate_state(|state| {
            let Some(task) = state.tasks.iter_mut().find(|task| task.task_id == task_id) else {
                return Ok(());
            };
            let now = unix_timestamp_now();
            task.status = DevqlTaskStatus::Completed;
            task.updated_at_unix = now;
            task.completed_at_unix = Some(now);
            task.error = None;
            task.result = Some(DevqlTaskResult::Ingest(summary.clone()));
            task.progress = DevqlTaskProgress::Ingest(ingest_progress_from_summary(&summary));
            state.last_action = Some("completed".to_string());
            Ok(())
        })
        .map(|_: ()| ())
    }

    pub(super) fn finish_embeddings_bootstrap_task_completed(
        &self,
        task_id: &str,
        result: EmbeddingsBootstrapResult,
    ) -> Result<()> {
        let task_id = task_id.to_string();
        self.mutate_state(|state| {
            let Some(task) = state.tasks.iter_mut().find(|task| task.task_id == task_id) else {
                return Ok(());
            };
            let now = unix_timestamp_now();
            task.status = DevqlTaskStatus::Completed;
            task.updated_at_unix = now;
            task.completed_at_unix = Some(now);
            task.error = None;
            task.result = Some(DevqlTaskResult::EmbeddingsBootstrap(result.clone()));
            task.progress = DevqlTaskProgress::EmbeddingsBootstrap(EmbeddingsBootstrapProgress {
                phase: EmbeddingsBootstrapPhase::Complete,
                asset_name: None,
                bytes_downloaded: 0,
                bytes_total: None,
                version: result.version.clone(),
                message: Some(result.message.clone()),
            });
            state.last_action = Some("completed".to_string());
            Ok(())
        })
        .map(|_: ()| ())
    }

    pub(super) fn finish_summary_bootstrap_task_completed(
        &self,
        task_id: &str,
        result: SummaryBootstrapResultRecord,
    ) -> Result<()> {
        let task_id = task_id.to_string();
        self.mutate_state(|state| {
            let Some(task) = state.tasks.iter_mut().find(|task| task.task_id == task_id) else {
                return Ok(());
            };
            let now = unix_timestamp_now();
            task.status = DevqlTaskStatus::Completed;
            task.updated_at_unix = now;
            task.completed_at_unix = Some(now);
            task.error = None;
            task.result = Some(DevqlTaskResult::SummaryBootstrap(result.clone()));
            task.progress = DevqlTaskProgress::SummaryBootstrap(SummaryBootstrapProgress {
                phase: SummaryBootstrapPhase::Complete,
                asset_name: None,
                bytes_downloaded: 0,
                bytes_total: None,
                version: None,
                message: Some(result.message.clone()),
            });
            state.last_action = Some("completed".to_string());
            Ok(())
        })
        .map(|_: ()| ())
    }

    pub(super) fn finish_task_failed(&self, task_id: &str, err: anyhow::Error) -> Result<()> {
        let task_id = task_id.to_string();
        let error = format!("{err:#}");
        let mut task_context: Option<(String, String, DevqlTaskKind, DevqlTaskSource)> = None;
        self.mutate_state(|state| {
            let Some(task) = state.tasks.iter_mut().find(|task| task.task_id == task_id) else {
                return Ok(());
            };
            let now = unix_timestamp_now();
            task.status = DevqlTaskStatus::Failed;
            task.updated_at_unix = now;
            task.completed_at_unix = Some(now);
            task.error = Some(error.clone());
            task.result = None;
            task.progress = failed_progress(&task.progress);
            task_context = Some((
                task.task_id.clone(),
                task.repo_id.clone(),
                task.kind,
                task.source,
            ));
            state.last_action = Some("failed".to_string());
            Ok(())
        })
        .map(|_: ()| ())?;
        if let Some((task_id, repo_id, kind, source)) = task_context {
            log::error!(
                "DevQL task failed: id={} repo={} kind={} source={} error={}",
                task_id,
                repo_id,
                kind,
                source,
                error
            );
        }
        Ok(())
    }

    pub(super) fn has_blocking_scope_exclusion_reconcile(&self, repo_id: &str) -> Result<bool> {
        Ok(self.load_state()?.tasks.into_iter().any(|task| {
            task.repo_id == repo_id
                && task.kind == DevqlTaskKind::Sync
                && task.source == DevqlTaskSource::RepoPolicyChange
                && matches!(
                    task.status,
                    DevqlTaskStatus::Queued | DevqlTaskStatus::Running
                )
        }))
    }

    pub(super) fn prune_excluded_path_sync_tasks_for_repo(&self, cfg: &DevqlConfig) -> Result<()> {
        let exclusion_matcher = crate::host::devql::load_repo_exclusion_matcher(&cfg.repo_root)?;
        self.mutate_state(|state| {
            let now = unix_timestamp_now();
            let mut changed = false;
            for task in state.tasks.iter_mut().filter(|task| {
                task.repo_id == cfg.repo.repo_id && task.status == DevqlTaskStatus::Queued
            }) {
                let Some(sync_spec) = sync_spec_from_task_spec_mut(&mut task.spec) else {
                    continue;
                };
                let SyncTaskMode::Paths { paths } = &mut sync_spec.mode else {
                    continue;
                };
                let previous_len = paths.len();
                paths.retain(|path| !exclusion_matcher.excludes_repo_relative_path(path));
                paths.sort();
                paths.dedup();
                if paths.is_empty() {
                    task.status = DevqlTaskStatus::Cancelled;
                    task.updated_at_unix = now;
                    task.completed_at_unix = Some(now);
                    task.error =
                        Some("task only targeted paths now excluded by repo policy".to_string());
                    task.result = None;
                    changed = true;
                    continue;
                }
                if paths.len() != previous_len {
                    task.updated_at_unix = now;
                    task.error = None;
                    changed = true;
                }
            }
            if changed {
                state.last_action = Some("prune_excluded_paths".to_string());
            }
            Ok(())
        })
        .map(|_: ()| ())
    }

    pub(super) fn load_state(&self) -> Result<PersistedDevqlTaskQueueState> {
        Ok(self
            .runtime_store
            .load_devql_task_queue_state()?
            .unwrap_or_else(PersistedDevqlTaskQueueState::default))
    }

    pub(super) fn mutate_state<T>(
        &self,
        mutate: impl FnOnce(&mut PersistedDevqlTaskQueueState) -> Result<T>,
    ) -> Result<T> {
        let guard = self
            .lock
            .lock()
            .map_err(|_| anyhow!("DevQL task coordinator lock poisoned"))?;
        let protected_task_ids = self.active_init_session_task_ids()?;
        let (result, tasks_to_publish) =
            self.runtime_store.mutate_devql_task_queue_state(|state| {
                let previous_tasks = state.tasks.clone();
                let result = mutate(state)?;
                let tasks_to_publish =
                    Self::save_state(state, &previous_tasks, &protected_task_ids)?;
                Ok((result, tasks_to_publish))
            })?;
        drop(guard);
        self.publish_tasks(tasks_to_publish);
        self.notify.notify_waiters();
        Ok(result)
    }

    fn save_state(
        state: &mut PersistedDevqlTaskQueueState,
        previous_tasks: &[DevqlTaskRecord],
        protected_task_ids: &HashSet<String>,
    ) -> Result<Vec<DevqlTaskRecord>> {
        state.version = 1;
        state.updated_at_unix = unix_timestamp_now();
        recompute_queue_positions(&mut state.tasks);
        prune_terminal_tasks(&mut state.tasks, protected_task_ids);
        Ok(changed_tasks(previous_tasks, &state.tasks))
    }

    fn active_init_session_task_ids(&self) -> Result<HashSet<String>> {
        let init_state = self
            .runtime_store
            .load_init_session_state()?
            .unwrap_or_default();
        let mut protected = HashSet::new();
        for session in init_state
            .sessions
            .into_iter()
            .filter(|session| session.terminal_status.is_none())
        {
            protected.extend(
                [
                    session.initial_sync_task_id,
                    session.ingest_task_id,
                    session.embeddings_bootstrap_task_id,
                    session.summary_bootstrap_task_id,
                    session.follow_up_sync_task_id,
                ]
                .into_iter()
                .flatten(),
            );
        }
        Ok(protected)
    }

    fn publish_tasks(&self, tasks: Vec<DevqlTaskRecord>) {
        let hub = self
            .subscription_hub
            .lock()
            .ok()
            .and_then(|slot| slot.clone());
        for task in tasks {
            if let Err(err) =
                crate::daemon::shared_init_runtime_coordinator().handle_task_update(task.clone())
            {
                log::warn!(
                    "failed to reconcile init runtime state for task {}: {err:#}",
                    task.task_id
                );
            }
            if let Some(hub) = hub.as_ref() {
                hub.publish_task(task);
            }
        }
    }

    pub(super) fn publish_checkpoint(&self, repo_name: String, checkpoint: Checkpoint) {
        let Some(hub) = self
            .subscription_hub
            .lock()
            .ok()
            .and_then(|slot| slot.clone())
        else {
            return;
        };
        hub.publish_checkpoint(repo_name, checkpoint);
    }
}

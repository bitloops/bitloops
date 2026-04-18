use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use anyhow::Result;
use tokio::time::{Duration, sleep};

use crate::daemon::tasks::queue::next_runnable_task_indexes;
use crate::daemon::types::{DevqlTaskKind, DevqlTaskRecord, DevqlTaskStatus};
use crate::graphql::SubscriptionHub;

use super::super::DevqlTaskCoordinator;

const WORKER_POLL_INTERVAL: Duration = Duration::from_secs(2);

struct WorkerStartedGuard {
    coordinator: Arc<DevqlTaskCoordinator>,
}

impl Drop for WorkerStartedGuard {
    fn drop(&mut self) {
        self.coordinator
            .worker_started
            .store(false, Ordering::SeqCst);
    }
}

impl DevqlTaskCoordinator {
    pub(crate) fn activate_worker(
        self: &Arc<Self>,
        config_root: &Path,
        repo_registry_path: Option<&Path>,
        hub: Option<Arc<SubscriptionHub>>,
    ) {
        if let Some(hub) = hub {
            self.register_subscription_hub(hub);
        }
        if self.worker_started.swap(true, Ordering::SeqCst) {
            return;
        }
        if let Err(err) = self.recover_running_tasks() {
            log::warn!("failed to recover queued DevQL tasks: {err:#}");
        }
        if let Err(err) = crate::host::devql::recover_running_producer_spool_jobs(config_root) {
            log::warn!("failed to recover DevQL producer spool jobs: {err:#}");
        }
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            self.worker_started.store(false, Ordering::SeqCst);
            log::error!("DevQL task worker activation requested without an active tokio runtime");
            return;
        };
        let coordinator = Arc::clone(self);
        let producer_spool_config_root = config_root.to_path_buf();
        let producer_spool_repo_registry_path = repo_registry_path.map(Path::to_path_buf);
        handle.spawn(async move {
            let _guard = WorkerStartedGuard {
                coordinator: Arc::clone(&coordinator),
            };
            coordinator
                .run_loop(
                    producer_spool_config_root,
                    producer_spool_repo_registry_path,
                )
                .await;
        });
    }

    pub(crate) fn register_subscription_hub(&self, hub: Arc<SubscriptionHub>) {
        if let Ok(mut slot) = self.subscription_hub.lock() {
            *slot = Some(hub);
        }
    }

    async fn run_loop(
        self: Arc<Self>,
        producer_spool_config_root: std::path::PathBuf,
        repo_registry_path: Option<std::path::PathBuf>,
    ) {
        loop {
            let mut made_progress = false;
            let reconcile_blocked = match self
                .ensure_scope_exclusion_reconciles(
                    &producer_spool_config_root,
                    repo_registry_path.as_deref(),
                )
                .await
            {
                Ok(blocked) => blocked,
                Err(err) => {
                    log::warn!("daemon DevQL exclusion reconcile error: {err:#}");
                    false
                }
            };

            if !reconcile_blocked {
                match self.schedule_pending_producer_spool_jobs(&producer_spool_config_root) {
                    Ok(progressed) => made_progress |= progressed,
                    Err(err) => log::warn!("daemon DevQL producer spool worker error: {err:#}"),
                }
            }
            match self.schedule_pending_tasks() {
                Ok(progressed) => made_progress |= progressed,
                Err(err) => log::warn!("daemon DevQL task worker error: {err:#}"),
            }

            if made_progress {
                continue;
            }
            tokio::select! {
                _ = self.notify.notified() => {},
                _ = sleep(WORKER_POLL_INTERVAL) => {},
            }
        }
    }

    fn schedule_pending_producer_spool_jobs(self: &Arc<Self>, config_root: &Path) -> Result<bool> {
        let jobs = crate::host::devql::claim_next_producer_spool_jobs(config_root)?;
        if jobs.is_empty() {
            return Ok(false);
        }

        for job in jobs {
            let coordinator = Arc::clone(self);
            tokio::spawn(async move {
                if let Err(err) = coordinator.run_producer_spool_job(job).await {
                    log::warn!("DevQL producer spool execution failed: {err:#}");
                }
            });
        }

        Ok(true)
    }

    fn schedule_pending_tasks(self: &Arc<Self>) -> Result<bool> {
        let tasks = self.mutate_state(|state| {
            let indexes = next_runnable_task_indexes(state);
            if indexes.is_empty() {
                return Ok(Vec::new());
            }

            let now = crate::daemon::types::unix_timestamp_now();
            let mut scheduled = Vec::with_capacity(indexes.len());
            for index in indexes {
                let mut task = state.tasks[index].clone();
                task.status = DevqlTaskStatus::Running;
                task.started_at_unix = Some(task.started_at_unix.unwrap_or(now));
                task.updated_at_unix = now;
                task.error = None;
                task.completed_at_unix = None;
                task.result = None;
                state.tasks[index] = task.clone();
                scheduled.push(task);
            }
            state.last_action = Some("running".to_string());
            Ok(scheduled)
        })?;

        if tasks.is_empty() {
            return Ok(false);
        }

        for task in tasks {
            let coordinator = Arc::clone(self);
            tokio::spawn(async move {
                if let Err(err) = coordinator.run_task(task).await {
                    log::warn!("DevQL task execution failed: {err:#}");
                }
            });
        }

        Ok(true)
    }

    async fn run_task(self: Arc<Self>, task: DevqlTaskRecord) -> Result<()> {
        match task.kind {
            DevqlTaskKind::Sync => self.run_sync_task(task).await,
            DevqlTaskKind::Ingest => self.run_ingest_task(task).await,
            DevqlTaskKind::EmbeddingsBootstrap => self.run_embeddings_bootstrap_task(task).await,
            DevqlTaskKind::SummaryBootstrap => self.run_summary_bootstrap_task(task).await,
        }
    }
}

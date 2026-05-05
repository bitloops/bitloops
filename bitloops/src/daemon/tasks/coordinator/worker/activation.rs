use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use anyhow::Result;
use tokio::time::{Duration, sleep};

use crate::daemon::tasks::queue::{
    next_runnable_task_indexes_blocking_repo_ids, post_commit_derivation_claim_guards,
};
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
            if self
                .run_worker_cycle(&producer_spool_config_root, repo_registry_path.as_deref())
                .await
                .unwrap_or_else(|err| {
                    log::warn!("daemon DevQL task worker cycle error: {err:#}");
                    false
                })
            {
                continue;
            }
            tokio::select! {
                _ = self.notify.notified() => {},
                _ = sleep(WORKER_POLL_INTERVAL) => {},
            }
        }
    }

    async fn run_worker_cycle(
        self: &Arc<Self>,
        producer_spool_config_root: &Path,
        repo_registry_path: Option<&Path>,
    ) -> Result<bool> {
        let mut made_progress = false;
        let reconcile_blocked = match self
            .ensure_scope_exclusion_reconciles(producer_spool_config_root, repo_registry_path)
            .await
        {
            Ok(blocked) => blocked,
            Err(err) => {
                log::warn!("daemon DevQL exclusion reconcile error: {err:#}");
                false
            }
        };

        if !reconcile_blocked {
            match self.schedule_pending_producer_spool_jobs(producer_spool_config_root) {
                Ok(progressed) => made_progress |= progressed,
                Err(err) => log::warn!("daemon DevQL producer spool worker error: {err:#}"),
            }
        }
        match self.schedule_pending_tasks(producer_spool_config_root) {
            Ok(progressed) => made_progress |= progressed,
            Err(err) => log::warn!("daemon DevQL task worker error: {err:#}"),
        }

        Ok(made_progress)
    }

    fn schedule_pending_producer_spool_jobs(self: &Arc<Self>, config_root: &Path) -> Result<bool> {
        let state = self.load_state()?;
        let running_task_repo_ids = state
            .tasks
            .iter()
            .filter(|task| task.status == DevqlTaskStatus::Running)
            .map(|task| task.repo_id.clone())
            .collect();
        let post_commit_derivation_guards = post_commit_derivation_claim_guards(&state);
        let jobs = crate::host::devql::claim_next_producer_spool_jobs_excluding(
            config_root,
            &running_task_repo_ids,
            &post_commit_derivation_guards,
        )?;
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

    fn schedule_pending_tasks(self: &Arc<Self>, config_root: &Path) -> Result<bool> {
        let producer_spool_running_repo_ids =
            crate::host::devql::running_producer_spool_repo_ids(config_root)?;
        let tasks = self.mutate_state(|state| {
            let indexes = next_runnable_task_indexes_blocking_repo_ids(
                state,
                &producer_spool_running_repo_ids,
            );
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};

    use tempfile::TempDir;
    use tokio::sync::Notify;

    use crate::daemon::tasks::DevqlTaskCoordinator;
    use crate::daemon::types::{
        DevqlTaskKind, DevqlTaskSource, DevqlTaskSpec, DevqlTaskStatus, SyncTaskMode, SyncTaskSpec,
    };
    use crate::host::devql::DevqlConfig;
    use crate::host::runtime_store::DaemonSqliteRuntimeStore;

    fn coordinator(temp: &TempDir) -> Arc<DevqlTaskCoordinator> {
        Arc::new(DevqlTaskCoordinator {
            runtime_store: DaemonSqliteRuntimeStore::open_at(
                temp.path().join("daemon-runtime.sqlite"),
            )
            .expect("open daemon runtime store"),
            lock: Mutex::new(()),
            notify: Notify::new(),
            worker_started: AtomicBool::new(false),
            subscription_hub: Mutex::new(None),
        })
    }

    fn bound_repo_config(
        config_root: &std::path::Path,
        repo_root: &std::path::Path,
    ) -> DevqlConfig {
        std::fs::create_dir_all(repo_root).expect("create repo root");
        crate::test_support::git_fixtures::init_test_repo(
            repo_root,
            "main",
            "Bitloops Test",
            "bitloops-test@example.com",
        );
        let config_path = crate::test_support::git_fixtures::write_test_daemon_config(config_root);
        crate::config::settings::write_repo_daemon_binding(
            &repo_root.join(crate::config::REPO_POLICY_LOCAL_FILE_NAME),
            &config_path,
        )
        .expect("write repo daemon binding");

        let repo = crate::host::devql::resolve_repo_identity(repo_root).expect("resolve repo");
        DevqlConfig::from_roots(config_root.to_path_buf(), repo_root.to_path_buf(), repo)
            .expect("build devql config")
    }

    #[tokio::test]
    async fn worker_cycle_schedules_visible_tasks_after_claiming_producer_spool_work() {
        let dir = TempDir::new().expect("temp dir");
        let config_root = dir.path().join("config");
        let visible_repo_root = dir.path().join("visible-repo");
        let producer_repo_root = dir.path().join("producer-repo");
        std::fs::create_dir_all(&config_root).expect("create config root");

        let visible_cfg = bound_repo_config(&config_root, &visible_repo_root);
        let producer_cfg = bound_repo_config(&config_root, &producer_repo_root);
        let coordinator = coordinator(&dir);
        coordinator
            .enqueue(
                &visible_cfg,
                DevqlTaskSource::Watcher,
                DevqlTaskSpec::Sync(SyncTaskSpec {
                    mode: SyncTaskMode::Paths {
                        paths: vec!["src/lib.rs".to_string()],
                    },
                    post_commit_snapshot: None,
                }),
            )
            .expect("enqueue visible sync task");
        crate::host::devql::enqueue_spooled_sync_task(
            &producer_cfg,
            DevqlTaskSource::Watcher,
            crate::host::devql::SyncMode::Paths(vec!["src/lib.rs".to_string()]),
        )
        .expect("enqueue producer spool task");

        let made_progress = coordinator
            .run_worker_cycle(&config_root, None)
            .await
            .expect("run worker cycle");

        assert!(made_progress, "worker cycle should claim or schedule work");
        let queued_visible_tasks = coordinator
            .tasks(
                Some(&visible_cfg.repo.repo_id),
                Some(DevqlTaskKind::Sync),
                Some(DevqlTaskStatus::Queued),
                None,
            )
            .expect("load queued visible tasks");
        assert!(
            queued_visible_tasks.is_empty(),
            "visible tasks for other repos should be scheduled in the same cycle as producer spool work"
        );
    }
}

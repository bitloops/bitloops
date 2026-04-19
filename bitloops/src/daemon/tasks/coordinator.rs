use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::Result;
use tokio::sync::Notify;
use uuid::Uuid;

use crate::graphql::SubscriptionHub;
#[cfg(test)]
use crate::utils::paths::default_global_runtime_db_path;
#[cfg(test)]
use tokio::sync::{mpsc, oneshot};

#[cfg(test)]
use super::super::types::{
    DevqlTaskKind, EmbeddingsBootstrapPhase, EmbeddingsBootstrapProgress,
    EmbeddingsBootstrapResult, SyncTaskMode,
};
use super::super::types::{
    DevqlTaskRecord, DevqlTaskSource, DevqlTaskSpec, DevqlTaskStatus, unix_timestamp_now,
};
use super::queue::{default_progress_for_spec, merge_existing_task};
use crate::host::devql::DevqlConfig;
use crate::host::runtime_store::DaemonSqliteRuntimeStore;

#[path = "coordinator/helpers.rs"]
mod helpers;
#[path = "coordinator/observers.rs"]
mod observers;
#[path = "coordinator/state.rs"]
mod state;
#[path = "coordinator/worker.rs"]
mod worker;

#[cfg(test)]
use helpers::receive_embeddings_bootstrap_outcome;

#[derive(Debug, Clone)]
pub struct DevqlTaskEnqueueResult {
    pub task: DevqlTaskRecord,
    pub merged: bool,
}

#[derive(Debug)]
pub struct DevqlTaskCoordinator {
    pub(super) runtime_store: DaemonSqliteRuntimeStore,
    pub(super) lock: Mutex<()>,
    pub(super) notify: Notify,
    pub(super) worker_started: AtomicBool,
    pub(super) subscription_hub: Mutex<Option<Arc<SubscriptionHub>>>,
}

impl DevqlTaskCoordinator {
    pub(crate) fn shared() -> Arc<Self> {
        static INSTANCE: OnceLock<Mutex<Arc<DevqlTaskCoordinator>>> = OnceLock::new();
        let slot = INSTANCE.get_or_init(|| {
            let runtime_store = DaemonSqliteRuntimeStore::open()
                .expect("opening daemon runtime store for DevQL tasks");
            Mutex::new(Self::new_shared_instance(runtime_store))
        });
        let coordinator = slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

        #[cfg(test)]
        let mut coordinator = coordinator;

        #[cfg(test)]
        {
            let runtime_db_path = default_global_runtime_db_path();
            if coordinator.runtime_store.db_path() != runtime_db_path.as_path() {
                let runtime_store = DaemonSqliteRuntimeStore::open_at(runtime_db_path)
                    .expect("opening daemon runtime store for DevQL tasks");
                *coordinator = Self::new_shared_instance(runtime_store);
            }
        }

        Arc::clone(&coordinator)
    }

    fn new_shared_instance(runtime_store: DaemonSqliteRuntimeStore) -> Arc<Self> {
        Arc::new(Self {
            runtime_store,
            lock: Mutex::new(()),
            notify: Notify::new(),
            worker_started: AtomicBool::new(false),
            subscription_hub: Mutex::new(None),
        })
    }

    pub(crate) fn enqueue(
        &self,
        cfg: &DevqlConfig,
        source: DevqlTaskSource,
        spec: DevqlTaskSpec,
    ) -> Result<DevqlTaskEnqueueResult> {
        self.enqueue_with_init_session(cfg, source, spec, None)
    }

    pub(crate) fn enqueue_with_init_session(
        &self,
        cfg: &DevqlConfig,
        source: DevqlTaskSource,
        spec: DevqlTaskSpec,
        init_session_id: Option<String>,
    ) -> Result<DevqlTaskEnqueueResult> {
        let kind = helpers::task_kind_from_spec(&spec);
        self.mutate_state(|state| {
            if let Some(task) =
                merge_existing_task(state, cfg, source, kind, &spec, init_session_id.as_deref())
            {
                return Ok(DevqlTaskEnqueueResult { task, merged: true });
            }

            let now = unix_timestamp_now();
            let task = DevqlTaskRecord {
                task_id: format!("{kind}-task-{}", Uuid::new_v4()),
                repo_id: cfg.repo.repo_id.clone(),
                repo_name: cfg.repo.name.clone(),
                repo_provider: cfg.repo.provider.clone(),
                repo_organisation: cfg.repo.organization.clone(),
                repo_identity: cfg.repo.identity.clone(),
                daemon_config_root: cfg.daemon_config_root.clone(),
                repo_root: cfg.repo_root.clone(),
                init_session_id: init_session_id.clone(),
                kind,
                source,
                spec: spec.clone(),
                status: DevqlTaskStatus::Queued,
                submitted_at_unix: now,
                started_at_unix: None,
                updated_at_unix: now,
                completed_at_unix: None,
                queue_position: None,
                tasks_ahead: None,
                progress: default_progress_for_spec(&spec),
                error: None,
                result: None,
            };
            state.tasks.push(task.clone());
            state.last_action = Some("enqueue".to_string());
            Ok(DevqlTaskEnqueueResult {
                task,
                merged: false,
            })
        })
    }
}

#[cfg(test)]
#[path = "coordinator_tests.rs"]
mod tests;

#[cfg(test)]
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{Context, Result};
use rusqlite::params;
use tokio::sync::Notify;

use crate::config::resolve_repo_runtime_db_path_for_config_root;
use crate::daemon::capability_events::queue::sql_i64;
use crate::daemon::memory::{MemoryMaintenance, PlatformMemoryMaintenance};
use crate::daemon::types::{CapabilityEventRunStatus, unix_timestamp_now};
use crate::graphql::SubscriptionHub;
use crate::host::runtime_store::DaemonSqliteRuntimeStore;

use super::types::CapabilityEventCoordinator;

impl CapabilityEventCoordinator {
    pub(crate) fn try_shared() -> Result<Arc<Self>> {
        let daemon_config =
            crate::daemon::resolve_daemon_config(None).context("resolving daemon config")?;
        let runtime_store = DaemonSqliteRuntimeStore::open_at(
            resolve_repo_runtime_db_path_for_config_root(&daemon_config.config_root),
        )
        .context("opening repo runtime workplane store for current-state consumers")?;
        static INSTANCE: OnceLock<Mutex<Arc<CapabilityEventCoordinator>>> = OnceLock::new();
        let slot =
            INSTANCE.get_or_init(|| Mutex::new(Self::new_shared_instance(runtime_store.clone())));
        let coordinator = slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

        #[cfg(test)]
        let mut coordinator = coordinator;

        #[cfg(test)]
        if coordinator.runtime_store.db_path() != runtime_store.db_path() {
            *coordinator = Self::new_shared_instance(runtime_store);
        }

        Ok(Arc::clone(&coordinator))
    }

    pub(crate) fn shared() -> Arc<Self> {
        Self::try_shared().expect("building current-state consumer coordinator")
    }

    pub(crate) fn new_shared_instance(runtime_store: DaemonSqliteRuntimeStore) -> Arc<Self> {
        Self::new_shared_instance_with_memory(runtime_store, Arc::new(PlatformMemoryMaintenance))
    }

    pub(crate) fn new_shared_instance_with_memory(
        runtime_store: DaemonSqliteRuntimeStore,
        memory_maintenance: Arc<dyn MemoryMaintenance>,
    ) -> Arc<Self> {
        Arc::new(Self {
            runtime_store,
            lock: Mutex::new(()),
            notify: Notify::new(),
            worker_started: AtomicBool::new(false),
            subscription_hub: Mutex::new(None),
            memory_maintenance,
        })
    }

    pub(crate) fn set_subscription_hub(&self, subscription_hub: Arc<SubscriptionHub>) {
        if let Ok(mut slot) = self.subscription_hub.lock() {
            *slot = Some(subscription_hub);
        }
    }

    pub(crate) fn recover_running_runs(&self) -> Result<()> {
        self.runtime_store.with_connection(|conn| {
            conn.execute(
                "UPDATE capability_workplane_cursor_runs SET status = ?1, started_at_unix = NULL, updated_at_unix = ?2 WHERE status = ?3",
                params![
                    CapabilityEventRunStatus::Queued.to_string(),
                    sql_i64(unix_timestamp_now())?,
                    CapabilityEventRunStatus::Running.to_string(),
                ],
            )
            .context("recovering in-flight current-state consumer runs")?;
            Ok(())
        })
    }
}

#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn test_shared_instance_at(db_path: PathBuf) -> Arc<CapabilityEventCoordinator> {
    CapabilityEventCoordinator::new_shared_instance(
        DaemonSqliteRuntimeStore::open_at(db_path)
            .expect("opening test daemon runtime store for current-state consumers"),
    )
}

use anyhow::Result;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use tokio::sync::{Mutex, Notify};
use tokio::time::{Duration, sleep};

use crate::config::resolve_repo_runtime_db_path_for_config_root;
use crate::graphql::SubscriptionHub;
use crate::host::runtime_store::DaemonSqliteRuntimeStore;

use rusqlite::params;

use super::worker_count::{
    EnrichmentWorkerBudgets, EnrichmentWorkerPool, configured_enrichment_worker_budgets_for_repo,
};
use super::workplane::{
    compact_and_prune_workplane_jobs, default_state, migrate_legacy_semantic_workplane_rows,
    prune_failed_semantic_inbox_items, recover_expired_semantic_inbox_leases,
    requeue_leased_semantic_inbox_items, sql_i64,
};
use super::{EnrichmentControlState, effective_worker_budgets};
use crate::daemon::types::unix_timestamp_now;
use crate::host::runtime_store::WorkplaneJobStatus;

#[derive(Debug)]
pub struct EnrichmentCoordinator {
    pub(crate) runtime_store: DaemonSqliteRuntimeStore,
    pub(crate) workplane_store: DaemonSqliteRuntimeStore,
    pub(crate) daemon_config_root: PathBuf,
    pub(crate) lock: Mutex<()>,
    pub(crate) notify: Notify,
    pub(crate) state_initialised: AtomicBool,
    pub(crate) maintenance_started: AtomicBool,
    pub(crate) started_worker_counts: std::sync::Mutex<EnrichmentWorkerBudgets>,
    pub(crate) subscription_hub: std::sync::Mutex<Option<Arc<SubscriptionHub>>>,
}

impl EnrichmentCoordinator {
    pub(crate) fn shared() -> Arc<Self> {
        static INSTANCE: OnceLock<Arc<EnrichmentCoordinator>> = OnceLock::new();
        let coordinator =
            Arc::clone(
                INSTANCE.get_or_init(|| {
                    let daemon_config = crate::daemon::resolve_daemon_config(None)
                        .expect("resolving daemon config");
                    Arc::new(Self {
                        runtime_store: DaemonSqliteRuntimeStore::open()
                            .expect("opening daemon runtime store for enrichment controls"),
                        workplane_store: DaemonSqliteRuntimeStore::open_at(
                            resolve_repo_runtime_db_path_for_config_root(
                                &daemon_config.config_root,
                            ),
                        )
                        .expect("opening repo runtime workplane store for enrichment queue"),
                        daemon_config_root: daemon_config.config_root.clone(),
                        lock: Mutex::new(()),
                        notify: Notify::new(),
                        state_initialised: AtomicBool::new(false),
                        maintenance_started: AtomicBool::new(false),
                        started_worker_counts: std::sync::Mutex::new(
                            EnrichmentWorkerBudgets::default(),
                        ),
                        subscription_hub: std::sync::Mutex::new(None),
                    })
                }),
            );
        coordinator.ensure_started();
        coordinator
    }

    pub(crate) fn set_subscription_hub(&self, subscription_hub: Arc<SubscriptionHub>) {
        if let Ok(mut slot) = self.subscription_hub.lock() {
            *slot = Some(subscription_hub);
        }
    }

    pub(crate) fn ensure_started(self: &Arc<Self>) {
        if !self.state_initialised.swap(true, Ordering::AcqRel) {
            self.ensure_state_file();
            let _ = migrate_legacy_semantic_workplane_rows(&self.workplane_store);
            self.requeue_running_jobs();
            if let Err(err) = recover_expired_semantic_inbox_leases(&self.workplane_store) {
                log::warn!(
                    "failed to recover expired semantic inbox leases during startup: {err:#}"
                );
            }
            if let Err(err) = prune_failed_semantic_inbox_items(&self.workplane_store) {
                log::warn!("failed to prune failed semantic inbox items during startup: {err:#}");
            }
            if let Err(err) = compact_and_prune_workplane_jobs(&self.workplane_store) {
                log::warn!("failed to compact enrichment workplane jobs during startup: {err:#}");
            }
        }
        self.ensure_maintenance_loop();
        self.ensure_worker_capacity();
    }

    fn ensure_maintenance_loop(self: &Arc<Self>) {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            log::error!("enrichment worker activation requested without an active tokio runtime");
            return;
        };
        if self.maintenance_started.swap(true, Ordering::AcqRel) {
            return;
        }
        let coordinator = Arc::clone(self);
        handle.spawn(async move {
            coordinator.maintenance_loop().await;
        });
    }

    async fn maintenance_loop(self: Arc<Self>) {
        loop {
            sleep(Duration::from_secs(60)).await;
            if let Err(err) = self.run_maintenance_pass().await {
                log::warn!("semantic inbox maintenance failed: {err:#}");
            }
        }
    }

    async fn run_maintenance_pass(&self) -> Result<()> {
        let _guard = self.lock.lock().await;
        recover_expired_semantic_inbox_leases(&self.workplane_store)?;
        prune_failed_semantic_inbox_items(&self.workplane_store)?;
        compact_and_prune_workplane_jobs(&self.workplane_store)
    }

    pub(crate) fn ensure_worker_capacity(self: &Arc<Self>) {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let budgets = effective_worker_budgets(&self.workplane_store, &self.daemon_config_root)
            .unwrap_or_else(|err| {
                log::warn!(
                    "failed to resolve effective enrichment worker budgets from `{}`: {err:#}",
                    self.daemon_config_root.display()
                );
                configured_enrichment_worker_budgets_for_repo(&self.daemon_config_root)
            });
        let Ok(mut started_worker_counts) = self.started_worker_counts.lock() else {
            log::warn!("failed to lock enrichment worker counts; skipping worker-capacity update");
            return;
        };
        for pool in [
            EnrichmentWorkerPool::SummaryRefresh,
            EnrichmentWorkerPool::Embeddings,
            EnrichmentWorkerPool::CloneRebuild,
        ] {
            let current_count = started_worker_counts.for_pool(pool);
            let desired_count = budgets.for_pool(pool);
            if desired_count <= current_count {
                continue;
            }
            let additional_workers = desired_count - current_count;
            if additional_workers > 0 {
                log::info!(
                    "starting {} additional enrichment workers for pool {} (total {})",
                    additional_workers,
                    pool.as_str(),
                    desired_count
                );
            }
            started_worker_counts.set_for_pool(pool, desired_count);
            for _ in 0..additional_workers {
                let coordinator = Arc::clone(self);
                handle.spawn(async move {
                    coordinator.run_loop(pool).await;
                });
            }
        }
    }

    fn ensure_state_file(&self) {
        match self.runtime_store.enrichment_state_exists() {
            Ok(true) => return,
            Ok(false) => {}
            Err(err) => {
                log::warn!(
                    "failed to check persisted enrichment queue state during startup: {err:#}"
                );
            }
        }
        let mut state = default_state();
        if let Err(err) = self.save_state(&mut state) {
            log::warn!("failed to initialise persisted enrichment queue state: {err:#}");
        }
    }

    pub(crate) fn requeue_running_jobs(&self) {
        let recovered_mailbox_items =
            match requeue_leased_semantic_inbox_items(&self.workplane_store) {
                Ok(recovered) => recovered,
                Err(err) => {
                    log::warn!(
                        "failed to recover leased semantic inbox items during startup: {err:#}"
                    );
                    0
                }
            };
        let recovered_clone_rebuild_jobs = match self.workplane_store.with_write_connection(|conn| {
            conn.execute(
                "UPDATE capability_workplane_jobs
                     SET status = ?1,
                         started_at_unix = NULL,
                         updated_at_unix = ?2,
                         lease_owner = NULL,
                         lease_expires_at_unix = NULL
                     WHERE status = ?3
                       AND mailbox_name = ?4",
                params![
                    WorkplaneJobStatus::Pending.as_str(),
                    sql_i64(unix_timestamp_now())?,
                    WorkplaneJobStatus::Running.as_str(),
                    crate::capability_packs::semantic_clones::types::SEMANTIC_CLONES_CLONE_REBUILD_MAILBOX,
                ],
            )
            .map_err(anyhow::Error::from)
        }) {
            Ok(recovered) => u64::try_from(recovered).unwrap_or_default(),
            Err(err) => {
                log::warn!(
                    "failed to recover stale clone rebuild enrichment jobs during startup: {err:#}"
                );
                0
            }
        };
        let recovered = recovered_mailbox_items.saturating_add(recovered_clone_rebuild_jobs);
        if recovered == 0 {
            return;
        }
        let mut state = match self.load_state() {
            Ok(state) => state,
            Err(err) => {
                log::warn!(
                    "failed to load enrichment queue state during startup recovery: {err:#}"
                );
                default_state()
            }
        };
        state.last_action = Some("requeue_running".to_string());
        if let Err(err) = self.save_state(&mut state) {
            log::warn!("failed to persist enrichment queue recovery state: {err:#}");
        }
        log::warn!("requeued {recovered} stale running enrichment jobs on daemon startup");
    }

    pub(crate) fn load_state(&self) -> Result<EnrichmentControlState> {
        Ok(self
            .runtime_store
            .load_enrichment_queue_state()?
            .unwrap_or_else(default_state))
    }

    pub(crate) fn save_state(&self, state: &mut EnrichmentControlState) -> Result<()> {
        state.version = 1;
        state.jobs.clear();
        state.active_branch_by_repo.clear();
        state.updated_at_unix = unix_timestamp_now();
        self.runtime_store.save_enrichment_queue_state(state)
    }
}

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::Notify;
use tokio::time::Duration;

use crate::daemon::memory::MemoryMaintenance;
use crate::daemon::types::CapabilityEventRunRecord;
use crate::graphql::SubscriptionHub;
use crate::host::capability_host::{SyncArtefactDiff, SyncFileDiff};
use crate::host::runtime_store::DaemonSqliteRuntimeStore;

pub(crate) const WORKER_POLL_INTERVAL: Duration = Duration::from_secs(2);
pub(crate) const MAX_RUN_ATTEMPTS: u32 = 5;

#[derive(Debug, Clone)]
pub struct CapabilityEventEnqueueResult {
    pub runs: Vec<CapabilityEventRunRecord>,
}

pub(crate) struct SyncGenerationInput<'a> {
    pub(crate) file_diff: SyncFileDiff,
    pub(crate) artefact_diff: SyncArtefactDiff,
    pub(crate) source_task_id: Option<&'a str>,
    pub(crate) init_session_id: Option<&'a str>,
}

#[derive(Debug)]
pub struct CapabilityEventCoordinator {
    pub(crate) runtime_store: DaemonSqliteRuntimeStore,
    pub(crate) lock: Mutex<()>,
    pub(crate) notify: Notify,
    pub(crate) worker_started: AtomicBool,
    pub(crate) subscription_hub: Mutex<Option<Arc<SubscriptionHub>>>,
    pub(crate) memory_maintenance: Arc<dyn MemoryMaintenance>,
}

pub(crate) struct WorkerStartedGuard {
    pub(crate) coordinator: Arc<CapabilityEventCoordinator>,
}

impl Drop for WorkerStartedGuard {
    fn drop(&mut self) {
        self.coordinator
            .worker_started
            .store(false, Ordering::SeqCst);
    }
}

#[derive(Debug, Clone)]
pub(crate) enum RunCompletion {
    NoopCompleted {
        run: CapabilityEventRunRecord,
    },
    Completed {
        run: CapabilityEventRunRecord,
        applied_to_generation_seq: u64,
    },
    RetryableFailure {
        run: CapabilityEventRunRecord,
        error: String,
    },
    Failed {
        run: CapabilityEventRunRecord,
        error: String,
    },
}

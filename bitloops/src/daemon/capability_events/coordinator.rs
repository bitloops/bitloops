#[path = "coordinator/completion.rs"]
mod completion;
#[path = "coordinator/ingestion.rs"]
mod ingestion;
#[path = "coordinator/instance.rs"]
mod instance;
#[path = "coordinator/queries.rs"]
mod queries;
#[path = "coordinator/types.rs"]
mod types;
#[path = "coordinator/worker.rs"]
mod worker;

#[cfg(test)]
#[path = "coordinator/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "coordinator_logging_tests.rs"]
mod logging_tests;

pub(crate) use self::types::SyncGenerationInput;
pub use self::types::{CapabilityEventCoordinator, CapabilityEventEnqueueResult};

#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use self::instance::test_shared_instance_at;

// Items re-exported here so `coordinator_logging_tests.rs` (declared above as a
// child of this facade) can keep using `use super::*;`.
#[cfg(test)]
pub(super) use self::types::{MAX_RUN_ATTEMPTS, RunCompletion};
#[cfg(test)]
pub(super) use self::worker::{
    IdleReclaimLogFields, build_idle_reclaim_log_fields, should_attempt_idle_reclaim,
    terminal_or_retry,
};
#[cfg(test)]
pub(super) use crate::daemon::types::{CapabilityEventRunRecord, CapabilityEventRunStatus};

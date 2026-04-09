#[allow(unused_imports)]
use super::*;

#[path = "commands_sync/diff_collector.rs"]
pub(crate) mod diff_collector;
#[path = "commands_sync/orchestrator.rs"]
mod orchestrator;
#[path = "commands_sync/progress.rs"]
mod progress;
#[path = "commands_sync/shared.rs"]
mod shared;
#[path = "commands_sync/sqlite_writer.rs"]
mod sqlite_writer;
#[path = "commands_sync/stats.rs"]
mod stats;
#[path = "commands_sync/summary.rs"]
mod summary;
#[path = "commands_sync/validation.rs"]
mod validation;

pub use self::orchestrator::{
    run_sync, run_sync_with_summary, run_sync_with_summary_and_observer,
    run_sync_with_summary_and_observer_and_diffs,
};
pub use self::progress::{SyncObserver, SyncProgressPhase, SyncProgressUpdate};
pub use self::summary::{SyncSummary, SyncValidationFileDrift, SyncValidationSummary};

pub(crate) use self::orchestrator::execute_sync;
pub(crate) use self::orchestrator::execute_sync_with_observer_and_stats_and_diffs;
#[cfg(test)]
pub(crate) use self::orchestrator::execute_sync_with_stats;
#[cfg(test)]
pub(crate) use self::validation::execute_sync_validation;

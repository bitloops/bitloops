//! Init-session runtime orchestration.
//!
//! This file is a slim facade over the [`init_runtime`](self) module tree. The
//! production logic lives in the sibling `init_runtime/` folder, broken down by
//! responsibility (coordinator, lanes, workplane, session statistics, progress
//! queries, and so on). The facade only re-exports the previously public surface
//! so that external callers continue to reach items via
//! `crate::daemon::init_runtime::*` without changes.
//!
//! The crate's `daemon.rs` loads this file via `#[path = "daemon/init_runtime.rs"]`,
//! which alters Rust's module path resolution for nested submodules. We therefore
//! pin each submodule with an explicit `#[path]` attribute pointing into the
//! sibling `daemon/init_runtime/` folder, mirroring the convention used by
//! `daemon.rs` for its other submodules.

#[path = "init_runtime/coordinator.rs"]
mod coordinator;
#[path = "init_runtime/lanes.rs"]
mod lanes;
#[path = "init_runtime/orchestration.rs"]
mod orchestration;
#[path = "init_runtime/progress.rs"]
mod progress;
#[path = "init_runtime/session_stats.rs"]
mod session_stats;
#[path = "init_runtime/stats.rs"]
mod stats;
#[path = "init_runtime/tasks.rs"]
mod tasks;
#[path = "init_runtime/types.rs"]
mod types;
#[path = "init_runtime/workplane.rs"]
mod workplane;

#[cfg(test)]
#[path = "init_runtime/tests.rs"]
mod tests;

pub use self::coordinator::InitRuntimeCoordinator;
pub(crate) use self::types::{
    InitRuntimeLaneProgressView, InitRuntimeLaneQueueView, InitRuntimeLaneView,
    InitRuntimeLaneWarningView, InitRuntimeSessionView, InitRuntimeSnapshot,
    InitRuntimeWorkplaneMailboxSnapshot, InitRuntimeWorkplanePoolSnapshot,
    InitRuntimeWorkplaneSnapshot, PersistedInitSessionState, PersistedSummaryBootstrapState,
};
pub use self::types::{InitSessionHandle, RuntimeEventRecord};

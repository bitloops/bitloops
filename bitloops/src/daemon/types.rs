//! Daemon-facing data types.
//!
//! This file is a slim facade for the `types/` submodules. Each submodule
//! groups a cohesive cluster of types or helpers; this facade re-exports
//! them while preserving their original visibility so that the public and
//! daemon-private API surface remains unchanged.

// NOTE: Each `mod` declaration carries an explicit `#[path]` attribute
// because the parent `daemon.rs` loads this file via `#[path = "daemon/types.rs"]`,
// which causes the compiler to look for child modules in the parent's directory
// rather than in `types/`. The `#[path]` attributes below restore the intended
// `types/` subdirectory layout.

#[path = "types/capability_events.rs"]
mod capability_events;
#[path = "types/constants.rs"]
mod constants;
#[path = "types/devql_task.rs"]
mod devql_task;
#[path = "types/embeddings_bootstrap.rs"]
mod embeddings_bootstrap;
#[path = "types/enrichment.rs"]
mod enrichment;
#[path = "types/health.rs"]
mod health;
#[path = "types/init_session.rs"]
mod init_session;
#[path = "types/paths.rs"]
mod paths;
#[path = "types/process_args.rs"]
mod process_args;
#[path = "types/resolved_config.rs"]
mod resolved_config;
#[path = "types/runtime_state.rs"]
mod runtime_state;
#[path = "types/service_metadata.rs"]
mod service_metadata;
#[path = "types/status_report.rs"]
mod status_report;
#[path = "types/summary_bootstrap.rs"]
mod summary_bootstrap;
#[path = "types/supervisor.rs"]
mod supervisor;

pub(crate) use constants::{
    ENRICHMENT_STATE_FILE_NAME, SUPERVISOR_RUNTIME_STATE_FILE_NAME, SYNC_STATE_FILE_NAME,
};
// Re-exports consumed by sibling daemon submodules via `use super::*;`. The
// `unused_imports` lint cannot see those transitive uses (and the constants
// gated on `#[cfg(test)]` are only used in test builds), so allow it here.
#[allow(unused_imports)]
pub(super) use constants::{
    FORCE_KILL_TIMEOUT, GLOBAL_SUPERVISOR_SERVICE_NAME, INTERNAL_DAEMON_COMMAND_NAME,
    INTERNAL_SUPERVISOR_COMMAND_NAME, READY_TIMEOUT, RUNTIME_STATE_FILE_NAME,
    SERVICE_STATE_FILE_NAME, STOP_RUNTIME_CLEAN_EXIT_GRACE, STOP_TIMEOUT,
    SUPERVISOR_SERVICE_STATE_FILE_NAME,
};

pub use capability_events::{
    CapabilityEventQueueState, CapabilityEventQueueStatus, CapabilityEventRunRecord,
    CapabilityEventRunStatus,
};
pub use devql_task::{
    DevqlTaskControlResult, DevqlTaskKind, DevqlTaskKindCounts, DevqlTaskProgress,
    DevqlTaskQueueState, DevqlTaskQueueStatus, DevqlTaskRecord, DevqlTaskResult, DevqlTaskSource,
    DevqlTaskSpec, DevqlTaskStatus, IngestTaskSpec, PostCommitSnapshotSpec, RepoTaskControlState,
    SyncTaskMode, SyncTaskSpec,
};
pub use embeddings_bootstrap::{
    EmbeddingsBootstrapGateEntry, EmbeddingsBootstrapGateStatus, EmbeddingsBootstrapMode,
    EmbeddingsBootstrapPhase, EmbeddingsBootstrapProgress, EmbeddingsBootstrapReadiness,
    EmbeddingsBootstrapResult, EmbeddingsBootstrapState, EmbeddingsBootstrapTaskSpec,
    InitEmbeddingsBootstrapRequest,
};
pub use enrichment::{
    BlockedMailboxStatus, EnrichmentQueueMode, EnrichmentQueueState, EnrichmentQueueStatus,
    EnrichmentWorkerPoolKind, EnrichmentWorkerPoolStatus, FailedEmbeddingJobSummary,
};
pub use health::DaemonHealthSummary;
pub use init_session::{
    InitSessionRecord, InitSessionState, InitSessionTaskTerminalSnapshot,
    InitSessionTerminalStatus, StartInitSessionSelections,
};
pub use process_args::{
    DaemonMode, DaemonProcessModeArg, InternalDaemonProcessArgs, InternalDaemonSupervisorArgs,
};
pub use resolved_config::ResolvedDaemonConfig;
pub use runtime_state::{DaemonRuntimeState, SupervisorRuntimeState};
pub use service_metadata::{DaemonServiceMetadata, ServiceManagerKind, SupervisorServiceMetadata};
pub use status_report::DaemonStatusReport;
pub use summary_bootstrap::{
    SummaryBootstrapAction, SummaryBootstrapPhase, SummaryBootstrapProgress,
    SummaryBootstrapRequest, SummaryBootstrapResultRecord, SummaryBootstrapRunRecord,
    SummaryBootstrapState, SummaryBootstrapStatus,
};

pub(super) use supervisor::{
    SupervisorAppState, SupervisorHealthResponse, SupervisorStartRequest, SupervisorStopRequest,
};

#[allow(unused_imports)]
pub(super) use paths::{
    global_daemon_dir, global_daemon_dir_fallback, supervisor_service_metadata_path,
    unix_timestamp_now,
};
pub use paths::{runtime_state_path, service_metadata_path};

use serde::Serialize;

use super::capability_events::CapabilityEventQueueStatus;
use super::devql_task::DevqlTaskQueueStatus;
use super::enrichment::EnrichmentQueueStatus;
use super::health::DaemonHealthSummary;
use super::runtime_state::DaemonRuntimeState;
use super::service_metadata::DaemonServiceMetadata;

#[derive(Debug, Clone, Serialize)]
pub struct DaemonStatusReport {
    pub runtime: Option<DaemonRuntimeState>,
    pub service: Option<DaemonServiceMetadata>,
    pub service_running: bool,
    pub health: Option<DaemonHealthSummary>,
    pub current_state_consumers: Option<CapabilityEventQueueStatus>,
    pub capability_events: Option<CapabilityEventQueueStatus>,
    pub enrichment: Option<EnrichmentQueueStatus>,
    pub devql_tasks: Option<DevqlTaskQueueStatus>,
}

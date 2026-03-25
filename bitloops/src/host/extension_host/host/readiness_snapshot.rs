use crate::host::extension_host::capability::CapabilityPackRegistrationObservation;
use crate::host::extension_host::language::LanguagePackRegistrationObservation;
use crate::host::extension_host::lifecycle::{
    ExtensionDiagnostic, ExtensionReadinessReport, ExtensionReadinessStatus,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreExtensionHostReadinessSnapshot {
    pub language_pack_ids: Vec<String>,
    pub capability_pack_ids: Vec<String>,
    pub language_observations: Vec<LanguagePackRegistrationObservation>,
    pub capability_observations: Vec<CapabilityPackRegistrationObservation>,
    pub diagnostics: Vec<ExtensionDiagnostic>,
    pub readiness_reports: Vec<ExtensionReadinessReport>,
}

impl CoreExtensionHostReadinessSnapshot {
    pub fn is_ready(&self) -> bool {
        !self.language_pack_ids.is_empty()
            && !self.capability_pack_ids.is_empty()
            && self
                .readiness_reports
                .iter()
                .all(|report| report.status == ExtensionReadinessStatus::Ready)
    }
}

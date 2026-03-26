use crate::host::extension_host::capability::CapabilityPackRegistrationObservation;
use crate::host::extension_host::language::LanguagePackRegistrationObservation;
use crate::host::extension_host::lifecycle::{
    ExtensionDiagnostic, ExtensionReadinessReport, ExtensionReadinessStatus,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreExtensionHostReadinessSnapshot {
    pub language_pack_ids: Vec<String>,
    pub language_adapter_pack_ids: Vec<String>,
    pub capability_pack_ids: Vec<String>,
    pub language_observations: Vec<LanguagePackRegistrationObservation>,
    pub capability_observations: Vec<CapabilityPackRegistrationObservation>,
    pub diagnostics: Vec<ExtensionDiagnostic>,
    pub language_adapter_readiness_reports: Vec<ExtensionReadinessReport>,
    pub readiness_reports: Vec<ExtensionReadinessReport>,
}

impl CoreExtensionHostReadinessSnapshot {
    pub fn with_language_adapter_readiness(
        mut self,
        language_adapter_pack_ids: Vec<String>,
        language_adapter_readiness_reports: Vec<ExtensionReadinessReport>,
    ) -> Self {
        self.language_adapter_pack_ids = language_adapter_pack_ids;
        self.language_adapter_readiness_reports = language_adapter_readiness_reports.clone();
        self.readiness_reports
            .extend(language_adapter_readiness_reports);
        self
    }

    pub fn is_ready(&self) -> bool {
        !self.language_pack_ids.is_empty()
            && !self.capability_pack_ids.is_empty()
            && (self.language_adapter_pack_ids.is_empty()
                || !self.language_adapter_readiness_reports.is_empty())
            && self
                .readiness_reports
                .iter()
                .all(|report| report.status == ExtensionReadinessStatus::Ready)
    }
}

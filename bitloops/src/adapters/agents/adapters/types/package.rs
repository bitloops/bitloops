use std::collections::HashSet;

use anyhow::{Result, bail};
use semver::Version;

use super::{
    AgentAdapterCompatibility, AgentAdapterRuntimeCompatibility, HOST_PACKAGE_METADATA_VERSION,
    normalise_key,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AgentAdapterPackageMetadataVersion(u16);

impl AgentAdapterPackageMetadataVersion {
    pub const fn current() -> Self {
        Self(HOST_PACKAGE_METADATA_VERSION)
    }

    pub const fn new(version: u16) -> Self {
        Self(version)
    }

    pub const fn value(self) -> u16 {
        self.0
    }

    pub(crate) fn validate(&self, scope: &str, id: &str) -> Result<()> {
        if self.0 != HOST_PACKAGE_METADATA_VERSION {
            bail!(
                "{scope} {id} has unsupported package metadata version {} (expected {})",
                self.0,
                HOST_PACKAGE_METADATA_VERSION
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentAdapterPackageTrustModel {
    FirstPartyLinked,
    HostVerifiedManifest,
}

impl AgentAdapterPackageTrustModel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FirstPartyLinked => "first-party-linked",
            Self::HostVerifiedManifest => "host-verified-manifest",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentAdapterPackageSource {
    FirstPartyLinked,
    Manifest,
}

impl AgentAdapterPackageSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FirstPartyLinked => "first-party-linked",
            Self::Manifest => "manifest",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentAdapterPackageResponsibility {
    HostResolution,
    HostValidation,
    HostLifecycleControl,
    HostAudit,
    PackageIdentity,
    PackageManifest,
    PackageVersioning,
    PackageEntrypoint,
    PackageTargetBehaviour,
    PackageCompatibilityClaims,
}

impl AgentAdapterPackageResponsibility {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::HostResolution => "host-resolution",
            Self::HostValidation => "host-validation",
            Self::HostLifecycleControl => "host-lifecycle-control",
            Self::HostAudit => "host-audit",
            Self::PackageIdentity => "package-identity",
            Self::PackageManifest => "package-manifest",
            Self::PackageVersioning => "package-versioning",
            Self::PackageEntrypoint => "package-entrypoint",
            Self::PackageTargetBehaviour => "package-target-behaviour",
            Self::PackageCompatibilityClaims => "package-compatibility-claims",
        }
    }
}

const FIRST_PARTY_HOST_RESPONSIBILITIES: &[AgentAdapterPackageResponsibility] = &[
    AgentAdapterPackageResponsibility::HostResolution,
    AgentAdapterPackageResponsibility::HostValidation,
    AgentAdapterPackageResponsibility::HostLifecycleControl,
    AgentAdapterPackageResponsibility::HostAudit,
];

const FIRST_PARTY_PACKAGE_RESPONSIBILITIES: &[AgentAdapterPackageResponsibility] = &[
    AgentAdapterPackageResponsibility::PackageIdentity,
    AgentAdapterPackageResponsibility::PackageManifest,
    AgentAdapterPackageResponsibility::PackageVersioning,
    AgentAdapterPackageResponsibility::PackageEntrypoint,
    AgentAdapterPackageResponsibility::PackageTargetBehaviour,
];

const HOST_VERIFIED_PACKAGE_RESPONSIBILITIES: &[AgentAdapterPackageResponsibility] = &[
    AgentAdapterPackageResponsibility::PackageIdentity,
    AgentAdapterPackageResponsibility::PackageManifest,
    AgentAdapterPackageResponsibility::PackageVersioning,
    AgentAdapterPackageResponsibility::PackageEntrypoint,
    AgentAdapterPackageResponsibility::PackageTargetBehaviour,
    AgentAdapterPackageResponsibility::PackageCompatibilityClaims,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentAdapterPackageBoundary {
    pub host_owned_responsibilities: &'static [AgentAdapterPackageResponsibility],
    pub package_owned_responsibilities: &'static [AgentAdapterPackageResponsibility],
}

impl AgentAdapterPackageBoundary {
    pub const fn first_party_linked() -> Self {
        Self {
            host_owned_responsibilities: FIRST_PARTY_HOST_RESPONSIBILITIES,
            package_owned_responsibilities: FIRST_PARTY_PACKAGE_RESPONSIBILITIES,
        }
    }

    pub const fn host_verified_manifest() -> Self {
        Self {
            host_owned_responsibilities: FIRST_PARTY_HOST_RESPONSIBILITIES,
            package_owned_responsibilities: HOST_VERIFIED_PACKAGE_RESPONSIBILITIES,
        }
    }

    pub(crate) fn validate(&self, scope: &str, id: &str) -> Result<()> {
        if self.host_owned_responsibilities.is_empty() {
            bail!("{scope} {id} must define host-owned responsibilities");
        }
        if self.package_owned_responsibilities.is_empty() {
            bail!("{scope} {id} must define package-owned responsibilities");
        }

        let mut host = HashSet::new();
        for responsibility in self.host_owned_responsibilities {
            if !host.insert(*responsibility) {
                bail!(
                    "{scope} {id} has duplicate host-owned responsibility: {}",
                    responsibility.as_str()
                );
            }
        }

        let mut package = HashSet::new();
        for responsibility in self.package_owned_responsibilities {
            if !package.insert(*responsibility) {
                bail!(
                    "{scope} {id} has duplicate package-owned responsibility: {}",
                    responsibility.as_str()
                );
            }
            if host.contains(responsibility) {
                bail!(
                    "{scope} {id} assigns responsibility '{}' to both host and package",
                    responsibility.as_str()
                );
            }
        }

        for required in [
            AgentAdapterPackageResponsibility::HostResolution,
            AgentAdapterPackageResponsibility::HostValidation,
            AgentAdapterPackageResponsibility::HostLifecycleControl,
            AgentAdapterPackageResponsibility::HostAudit,
        ] {
            if !host.contains(&required) {
                bail!(
                    "{scope} {id} is missing required host responsibility: {}",
                    required.as_str()
                );
            }
        }

        for required in [
            AgentAdapterPackageResponsibility::PackageIdentity,
            AgentAdapterPackageResponsibility::PackageManifest,
            AgentAdapterPackageResponsibility::PackageVersioning,
            AgentAdapterPackageResponsibility::PackageEntrypoint,
            AgentAdapterPackageResponsibility::PackageTargetBehaviour,
        ] {
            if !package.contains(&required) {
                bail!(
                    "{scope} {id} is missing required package responsibility: {}",
                    required.as_str()
                );
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentAdapterPackageLifecyclePhase {
    Discovered,
    Validated,
    Loaded,
    Activated,
    Retired,
}

impl AgentAdapterPackageLifecyclePhase {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Discovered => "discovered",
            Self::Validated => "validated",
            Self::Loaded => "loaded",
            Self::Activated => "activated",
            Self::Retired => "retired",
        }
    }
}

const DEFAULT_PACKAGE_LIFECYCLE: &[AgentAdapterPackageLifecyclePhase] = &[
    AgentAdapterPackageLifecyclePhase::Discovered,
    AgentAdapterPackageLifecyclePhase::Validated,
    AgentAdapterPackageLifecyclePhase::Loaded,
    AgentAdapterPackageLifecyclePhase::Activated,
    AgentAdapterPackageLifecyclePhase::Retired,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentAdapterPackageLifecycle {
    pub phases: &'static [AgentAdapterPackageLifecyclePhase],
    pub host_controls_activation: bool,
    pub host_controls_unload: bool,
}

impl AgentAdapterPackageLifecycle {
    pub const fn default() -> Self {
        Self {
            phases: DEFAULT_PACKAGE_LIFECYCLE,
            host_controls_activation: true,
            host_controls_unload: true,
        }
    }

    pub(crate) fn validate(&self, scope: &str, id: &str) -> Result<()> {
        if self.phases.is_empty() {
            bail!("{scope} {id} must define a package lifecycle");
        }
        if self.phases != DEFAULT_PACKAGE_LIFECYCLE {
            bail!(
                "{scope} {id} has unsupported lifecycle phases: expected {}",
                DEFAULT_PACKAGE_LIFECYCLE
                    .iter()
                    .map(|phase| phase.as_str())
                    .collect::<Vec<_>>()
                    .join(" -> ")
            );
        }
        if !self.host_controls_activation {
            bail!("{scope} {id} must keep activation host-controlled");
        }
        if !self.host_controls_unload {
            bail!("{scope} {id} must keep unload host-controlled");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentAdapterPackageCompatibility {
    pub contract: AgentAdapterCompatibility,
    pub runtime: AgentAdapterRuntimeCompatibility,
}

impl AgentAdapterPackageCompatibility {
    pub const fn phase1() -> Self {
        Self {
            contract: AgentAdapterCompatibility::phase1(),
            runtime: AgentAdapterRuntimeCompatibility::local_cli(),
        }
    }

    pub(crate) fn validate(&self, scope: &str, id: &str) -> Result<()> {
        self.contract.validate(id, scope)?;
        self.runtime.validate(id, scope)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentAdapterPackageDescriptor {
    pub id: &'static str,
    pub display_name: &'static str,
    pub version: &'static str,
    pub metadata_version: AgentAdapterPackageMetadataVersion,
    pub source: AgentAdapterPackageSource,
    pub trust_model: AgentAdapterPackageTrustModel,
    pub boundary: AgentAdapterPackageBoundary,
    pub lifecycle: AgentAdapterPackageLifecycle,
    pub compatibility: AgentAdapterPackageCompatibility,
}

impl AgentAdapterPackageDescriptor {
    pub const fn first_party_linked(id: &'static str, display_name: &'static str) -> Self {
        Self {
            id,
            display_name,
            version: "1.0.0",
            metadata_version: AgentAdapterPackageMetadataVersion::current(),
            source: AgentAdapterPackageSource::FirstPartyLinked,
            trust_model: AgentAdapterPackageTrustModel::FirstPartyLinked,
            boundary: AgentAdapterPackageBoundary::first_party_linked(),
            lifecycle: AgentAdapterPackageLifecycle::default(),
            compatibility: AgentAdapterPackageCompatibility::phase1(),
        }
    }

    pub const fn host_verified_manifest(
        id: &'static str,
        display_name: &'static str,
        version: &'static str,
    ) -> Self {
        Self {
            id,
            display_name,
            version,
            metadata_version: AgentAdapterPackageMetadataVersion::current(),
            source: AgentAdapterPackageSource::Manifest,
            trust_model: AgentAdapterPackageTrustModel::HostVerifiedManifest,
            boundary: AgentAdapterPackageBoundary::host_verified_manifest(),
            lifecycle: AgentAdapterPackageLifecycle::default(),
            compatibility: AgentAdapterPackageCompatibility::phase1(),
        }
    }

    pub(crate) fn validation_diagnostics(
        &self,
        scope: &str,
        id: &str,
    ) -> Vec<AgentAdapterPackageDiagnostic> {
        let mut diagnostics = Vec::new();
        let package_id = match normalise_key(self.id) {
            Ok(value) => value,
            Err(err) => {
                diagnostics.push(AgentAdapterPackageDiagnostic {
                    scope: scope.to_string(),
                    id: id.to_string(),
                    field: Some("id".to_string()),
                    code: "invalid_package_id".to_string(),
                    message: format!("{scope} {id} has invalid package id {}: {err}", self.id),
                });
                return diagnostics;
            }
        };
        let descriptor_id = match normalise_key(id) {
            Ok(value) => value,
            Err(err) => {
                diagnostics.push(AgentAdapterPackageDiagnostic {
                    scope: scope.to_string(),
                    id: id.to_string(),
                    field: Some("id".to_string()),
                    code: "invalid_adapter_id".to_string(),
                    message: format!("{scope} {id} is invalid: {err}"),
                });
                return diagnostics;
            }
        };
        if package_id != descriptor_id {
            diagnostics.push(AgentAdapterPackageDiagnostic {
                scope: scope.to_string(),
                id: id.to_string(),
                field: Some("id".to_string()),
                code: "package_id_mismatch".to_string(),
                message: format!(
                    "{scope} {id} is linked to package {} but expected {}",
                    self.id, id
                ),
            });
            return diagnostics;
        }
        if self.display_name.trim().is_empty() {
            diagnostics.push(AgentAdapterPackageDiagnostic {
                scope: scope.to_string(),
                id: id.to_string(),
                field: Some("display_name".to_string()),
                code: "missing_package_display_name".to_string(),
                message: format!("{scope} {id} must define a package display name"),
            });
        }
        if self.version.trim().is_empty() {
            diagnostics.push(AgentAdapterPackageDiagnostic {
                scope: scope.to_string(),
                id: id.to_string(),
                field: Some("version".to_string()),
                code: "missing_package_version".to_string(),
                message: format!("{scope} {id} must define a package version"),
            });
        } else if let Err(err) = Version::parse(self.version.trim()) {
            diagnostics.push(AgentAdapterPackageDiagnostic {
                scope: scope.to_string(),
                id: id.to_string(),
                field: Some("version".to_string()),
                code: "invalid_package_version".to_string(),
                message: format!(
                    "{scope} {id} has invalid package version {}: {err}",
                    self.version.trim()
                ),
            });
        }

        if let Err(err) = self.metadata_version.validate(scope, id) {
            diagnostics.push(AgentAdapterPackageDiagnostic {
                scope: scope.to_string(),
                id: id.to_string(),
                field: Some("metadata_version".to_string()),
                code: "unsupported_metadata_version".to_string(),
                message: err.to_string(),
            });
        }

        let expected_source = match self.trust_model {
            AgentAdapterPackageTrustModel::FirstPartyLinked => {
                AgentAdapterPackageSource::FirstPartyLinked
            }
            AgentAdapterPackageTrustModel::HostVerifiedManifest => {
                AgentAdapterPackageSource::Manifest
            }
        };
        if self.source != expected_source {
            diagnostics.push(AgentAdapterPackageDiagnostic {
                scope: scope.to_string(),
                id: id.to_string(),
                field: Some("source".to_string()),
                code: "package_source_mismatch".to_string(),
                message: format!(
                    "{scope} {id} declares package source {} but trust model {} expects {}",
                    self.source.as_str(),
                    self.trust_model.as_str(),
                    expected_source.as_str()
                ),
            });
        }
        if self.trust_model == AgentAdapterPackageTrustModel::FirstPartyLinked
            && self.source != AgentAdapterPackageSource::FirstPartyLinked
        {
            diagnostics.push(AgentAdapterPackageDiagnostic {
                scope: scope.to_string(),
                id: id.to_string(),
                field: Some("trust_model".to_string()),
                code: "package_trust_mismatch".to_string(),
                message: format!(
                    "{scope} {id} must use first-party-linked trust model for first-party packages"
                ),
            });
        }

        if let Err(err) = self.boundary.validate(scope, id) {
            diagnostics.push(AgentAdapterPackageDiagnostic {
                scope: scope.to_string(),
                id: id.to_string(),
                field: Some("boundary".to_string()),
                code: "invalid_package_boundary".to_string(),
                message: err.to_string(),
            });
        }
        if let Err(err) = self.lifecycle.validate(scope, id) {
            diagnostics.push(AgentAdapterPackageDiagnostic {
                scope: scope.to_string(),
                id: id.to_string(),
                field: Some("lifecycle".to_string()),
                code: "invalid_package_lifecycle".to_string(),
                message: err.to_string(),
            });
        }
        if let Err(err) = self.compatibility.validate(scope, id) {
            diagnostics.push(AgentAdapterPackageDiagnostic {
                scope: scope.to_string(),
                id: id.to_string(),
                field: Some("compatibility".to_string()),
                code: "invalid_package_compatibility".to_string(),
                message: err.to_string(),
            });
        }

        if matches!(
            self.trust_model,
            AgentAdapterPackageTrustModel::HostVerifiedManifest
        ) && !self
            .boundary
            .package_owned_responsibilities
            .contains(&AgentAdapterPackageResponsibility::PackageCompatibilityClaims)
        {
            diagnostics.push(AgentAdapterPackageDiagnostic {
                scope: scope.to_string(),
                id: id.to_string(),
                field: Some("boundary".to_string()),
                code: "missing_package_compatibility_claims".to_string(),
                message: format!(
                    "{scope} {id} must expose package compatibility claims for host-verified manifests"
                ),
            });
        }

        diagnostics
    }

    pub(crate) fn validate(&self, scope: &str, id: &str) -> Result<()> {
        let diagnostics = self.validation_diagnostics(scope, id);
        if let Some(first) = diagnostics.first() {
            bail!("{}", first.message);
        }
        Ok(())
    }

    pub(crate) fn discovery_report(&self, scope: &str, id: &str) -> AgentAdapterPackageDiscovery {
        let diagnostics = self.validation_diagnostics(scope, id);
        AgentAdapterPackageDiscovery {
            adapter_id: id.to_string(),
            package_id: self.id.to_string(),
            metadata_version: self.metadata_version,
            package_version: self.version.to_string(),
            source: self.source,
            trust_model: self.trust_model,
            status: if diagnostics.is_empty() {
                AgentAdapterPackageDiscoveryStatus::Ready
            } else {
                AgentAdapterPackageDiscoveryStatus::Invalid
            },
            diagnostics,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentAdapterPackageDiagnostic {
    pub scope: String,
    pub id: String,
    pub field: Option<String>,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentAdapterPackageDiscoveryStatus {
    Ready,
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentAdapterPackageDiscovery {
    pub adapter_id: String,
    pub package_id: String,
    pub metadata_version: AgentAdapterPackageMetadataVersion,
    pub package_version: String,
    pub source: AgentAdapterPackageSource,
    pub trust_model: AgentAdapterPackageTrustModel,
    pub status: AgentAdapterPackageDiscoveryStatus,
    pub diagnostics: Vec<AgentAdapterPackageDiagnostic>,
}

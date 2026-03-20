use std::fmt::{self, Display, Formatter};

pub const HOST_EXTENSION_CONTRACT_VERSION: u16 = 1;
pub const HOST_EXTENSION_RUNTIME_VERSION: u16 = 1;

const LOCAL_CLI_RUNTIME: &[ExtensionRuntime] = &[ExtensionRuntime::LocalCli];
const PHASE1_HOST_FEATURES: &[&str] = &[
    "language-packs",
    "capability-packs",
    "readiness",
    "diagnostics",
    "capability-migrations",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExtensionRuntime {
    LocalCli,
    RemoteRuntime,
}

impl ExtensionRuntime {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LocalCli => "local-cli",
            Self::RemoteRuntime => "remote-runtime",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostCompatibilityContext {
    pub contract_version: u16,
    pub runtime_version: u16,
    pub runtime: ExtensionRuntime,
    pub supported_features: &'static [&'static str],
}

impl HostCompatibilityContext {
    pub const fn local_cli_phase1() -> Self {
        Self {
            contract_version: HOST_EXTENSION_CONTRACT_VERSION,
            runtime_version: HOST_EXTENSION_RUNTIME_VERSION,
            runtime: ExtensionRuntime::LocalCli,
            supported_features: PHASE1_HOST_FEATURES,
        }
    }

    pub fn supports_feature(self, feature: &str) -> bool {
        self.supported_features
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(feature))
    }
}

impl Default for HostCompatibilityContext {
    fn default() -> Self {
        Self::local_cli_phase1()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtensionCompatibility {
    pub contract_version: u16,
    pub min_host_version: u16,
    pub max_host_version: u16,
    pub supported_runtimes: &'static [ExtensionRuntime],
    pub required_host_features: &'static [&'static str],
}

impl ExtensionCompatibility {
    pub const fn phase1_local_cli(required_host_features: &'static [&'static str]) -> Self {
        Self {
            contract_version: HOST_EXTENSION_CONTRACT_VERSION,
            min_host_version: HOST_EXTENSION_RUNTIME_VERSION,
            max_host_version: HOST_EXTENSION_RUNTIME_VERSION,
            supported_runtimes: LOCAL_CLI_RUNTIME,
            required_host_features,
        }
    }

    pub fn validate(
        self,
        family: &'static str,
        id: &str,
        host: HostCompatibilityContext,
    ) -> Result<(), ExtensionCompatibilityError> {
        if self.supported_runtimes.is_empty() {
            return Err(ExtensionCompatibilityError::EmptyRuntimeSupport {
                family,
                id: id.to_string(),
            });
        }
        if self.contract_version != host.contract_version {
            return Err(ExtensionCompatibilityError::ContractVersionMismatch {
                family,
                id: id.to_string(),
                declared_contract_version: self.contract_version,
                host_contract_version: host.contract_version,
            });
        }
        if host.runtime_version < self.min_host_version
            || host.runtime_version > self.max_host_version
        {
            return Err(ExtensionCompatibilityError::HostRuntimeOutOfRange {
                family,
                id: id.to_string(),
                host_runtime_version: host.runtime_version,
                min_supported_version: self.min_host_version,
                max_supported_version: self.max_host_version,
            });
        }
        if !self.supported_runtimes.contains(&host.runtime) {
            return Err(ExtensionCompatibilityError::RuntimeNotSupported {
                family,
                id: id.to_string(),
                host_runtime: host.runtime,
            });
        }
        for feature in self.required_host_features {
            if !host.supports_feature(feature) {
                return Err(ExtensionCompatibilityError::MissingHostFeature {
                    family,
                    id: id.to_string(),
                    feature: (*feature).to_string(),
                });
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtensionCompatibilityError {
    EmptyRuntimeSupport {
        family: &'static str,
        id: String,
    },
    ContractVersionMismatch {
        family: &'static str,
        id: String,
        declared_contract_version: u16,
        host_contract_version: u16,
    },
    HostRuntimeOutOfRange {
        family: &'static str,
        id: String,
        host_runtime_version: u16,
        min_supported_version: u16,
        max_supported_version: u16,
    },
    RuntimeNotSupported {
        family: &'static str,
        id: String,
        host_runtime: ExtensionRuntime,
    },
    MissingHostFeature {
        family: &'static str,
        id: String,
        feature: String,
    },
}

impl Display for ExtensionCompatibilityError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyRuntimeSupport { family, id } => {
                write!(
                    f,
                    "{family} `{id}` must declare at least one supported runtime"
                )
            }
            Self::ContractVersionMismatch {
                family,
                id,
                declared_contract_version,
                host_contract_version,
            } => {
                write!(
                    f,
                    "{family} `{id}` declares contract version {declared_contract_version}, but host expects {host_contract_version}"
                )
            }
            Self::HostRuntimeOutOfRange {
                family,
                id,
                host_runtime_version,
                min_supported_version,
                max_supported_version,
            } => {
                write!(
                    f,
                    "{family} `{id}` is incompatible with host runtime version {host_runtime_version} (supported range: {min_supported_version}-{max_supported_version})"
                )
            }
            Self::RuntimeNotSupported {
                family,
                id,
                host_runtime,
            } => {
                write!(
                    f,
                    "{family} `{id}` does not support host runtime `{}`",
                    host_runtime.as_str()
                )
            }
            Self::MissingHostFeature {
                family,
                id,
                feature,
            } => {
                write!(
                    f,
                    "{family} `{id}` requires unsupported host feature `{feature}`"
                )
            }
        }
    }
}

impl std::error::Error for ExtensionCompatibilityError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionReadinessStatus {
    Ready,
    NotReady,
}

impl ExtensionReadinessStatus {
    pub fn from_failures(has_failures: bool) -> Self {
        if has_failures {
            Self::NotReady
        } else {
            Self::Ready
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionLifecycleState {
    Discovered,
    Validated,
    Registered,
    Migrated,
    Ready,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionReadinessFailure {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionReadinessReport {
    pub family: String,
    pub id: String,
    pub registered: bool,
    pub ready: bool,
    pub status: ExtensionReadinessStatus,
    pub lifecycle_state: ExtensionLifecycleState,
    pub failures: Vec<ExtensionReadinessFailure>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionDiagnosticSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionDiagnosticKind {
    Registration,
    Compatibility,
    Readiness,
    Migration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionDiagnostic {
    pub family: String,
    pub extension_id: String,
    pub severity: ExtensionDiagnosticSeverity,
    pub kind: ExtensionDiagnosticKind,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityPackMigrationDescriptor {
    pub id: &'static str,
    pub order: u32,
    pub description: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityMigrationStep {
    pub pack_id: String,
    pub migration_id: String,
    pub order: u32,
    pub description: String,
}

impl CapabilityMigrationStep {
    pub fn from_descriptor(pack_id: &str, descriptor: &CapabilityPackMigrationDescriptor) -> Self {
        Self {
            pack_id: pack_id.to_string(),
            migration_id: descriptor.id.to_string(),
            order: descriptor.order,
            description: descriptor.description.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityMigrationExecution {
    pub pack_id: String,
    pub migration_id: String,
    pub order: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityMigrationFailure {
    pub pack_id: String,
    pub migration_id: String,
    pub order: u32,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityMigrationStatus {
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityMigrationRunReport {
    pub status: CapabilityMigrationStatus,
    pub executed: Vec<CapabilityMigrationExecution>,
    pub failure: Option<CapabilityMigrationFailure>,
}

pub fn orchestrate_capability_migrations<F>(
    mut steps: Vec<CapabilityMigrationStep>,
    mut executor: F,
) -> CapabilityMigrationRunReport
where
    F: FnMut(&CapabilityMigrationStep) -> Result<(), String>,
{
    steps.sort_by(|left, right| {
        left.order
            .cmp(&right.order)
            .then_with(|| left.pack_id.cmp(&right.pack_id))
            .then_with(|| left.migration_id.cmp(&right.migration_id))
    });

    let mut executed = Vec::new();
    for step in steps {
        match executor(&step) {
            Ok(()) => executed.push(CapabilityMigrationExecution {
                pack_id: step.pack_id,
                migration_id: step.migration_id,
                order: step.order,
            }),
            Err(reason) => {
                return CapabilityMigrationRunReport {
                    status: CapabilityMigrationStatus::Failed,
                    executed,
                    failure: Some(CapabilityMigrationFailure {
                        pack_id: step.pack_id,
                        migration_id: step.migration_id,
                        order: step.order,
                        reason,
                    }),
                };
            }
        }
    }

    CapabilityMigrationRunReport {
        status: CapabilityMigrationStatus::Completed,
        executed,
        failure: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compatibility_validation_rejects_missing_host_feature() {
        let compatibility = ExtensionCompatibility::phase1_local_cli(&["imaginary-feature"]);
        let error = compatibility
            .validate(
                "language pack",
                "example-pack",
                HostCompatibilityContext::local_cli_phase1(),
            )
            .expect_err("missing host feature should fail compatibility");
        assert!(matches!(
            error,
            ExtensionCompatibilityError::MissingHostFeature { .. }
        ));
    }

    #[test]
    fn compatibility_validation_accepts_phase1_local_cli_declaration() {
        let compatibility =
            ExtensionCompatibility::phase1_local_cli(&["language-packs", "readiness"]);
        compatibility
            .validate(
                "language pack",
                "example-pack",
                HostCompatibilityContext::local_cli_phase1(),
            )
            .expect("phase1 local cli declaration should pass");
    }

    #[test]
    fn capability_migration_orchestration_runs_in_order_and_fails_fast() {
        let steps = vec![
            CapabilityMigrationStep {
                pack_id: "cap-b".to_string(),
                migration_id: "002".to_string(),
                order: 2,
                description: "second".to_string(),
            },
            CapabilityMigrationStep {
                pack_id: "cap-a".to_string(),
                migration_id: "001".to_string(),
                order: 1,
                description: "first".to_string(),
            },
            CapabilityMigrationStep {
                pack_id: "cap-c".to_string(),
                migration_id: "003".to_string(),
                order: 3,
                description: "third".to_string(),
            },
        ];

        let report = orchestrate_capability_migrations(steps, |step| {
            if step.pack_id == "cap-b" {
                Err("boom".to_string())
            } else {
                Ok(())
            }
        });

        assert_eq!(report.status, CapabilityMigrationStatus::Failed);
        assert_eq!(
            report
                .executed
                .iter()
                .map(|execution| execution.pack_id.as_str())
                .collect::<Vec<_>>(),
            vec!["cap-a"]
        );
        let failure = report.failure.expect("failure should be present");
        assert_eq!(failure.pack_id, "cap-b");
        assert_eq!(failure.reason, "boom");
    }
}

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt::{self, Display, Formatter};

use super::capability::{
    CapabilityDescriptor, CapabilityIngesterContribution, CapabilityPackDescriptor,
    CapabilityPackRegistrationObservation, CapabilityPackRegistry, CapabilityPackRegistryError,
    CapabilityQueryExampleContribution, CapabilitySchemaModuleContribution,
    CapabilityStageContribution,
};
use super::language::{
    LanguagePackDescriptor, LanguagePackRegistrationObservation, LanguagePackRegistry,
    LanguagePackRegistryError, LanguageProfileDescriptor,
};
use super::lifecycle::{
    CapabilityMigrationExecution, CapabilityMigrationRunReport, CapabilityMigrationStep,
    ExtensionCompatibility, ExtensionCompatibilityError, ExtensionDiagnostic,
    ExtensionDiagnosticKind, ExtensionDiagnosticSeverity, ExtensionLifecycleState,
    ExtensionReadinessFailure, ExtensionReadinessReport, ExtensionReadinessStatus,
    HostCompatibilityContext, orchestrate_capability_migrations,
};

const LANGUAGE_PACK_FEATURES: &[&str] = &["language-packs", "readiness", "diagnostics"];
const CAPABILITY_PACK_FEATURES: &[&str] = &[
    "capability-packs",
    "readiness",
    "diagnostics",
    "capability-migrations",
];

const RUST_LANGUAGE_PACK: LanguagePackDescriptor = LanguagePackDescriptor {
    id: "rust-language-pack",
    version: "1.0.0",
    api_version: 1,
    display_name: "Rust Language Pack",
    aliases: &["rust-pack"],
    supported_languages: &["rust"],
    language_profiles: &[LanguageProfileDescriptor {
        id: "rust-default",
        display_name: "Rust Default",
        language_id: "rust",
        dialect: None,
        aliases: &["rust-profile"],
        file_extensions: &["rs"],
        supported_source_versions: &["^1.70"],
    }],
    compatibility: ExtensionCompatibility::phase1_local_cli(LANGUAGE_PACK_FEATURES),
};

const TS_JS_LANGUAGE_PACK: LanguagePackDescriptor = LanguagePackDescriptor {
    id: "ts-js-language-pack",
    version: "1.0.0",
    api_version: 1,
    display_name: "TypeScript/JavaScript Language Pack",
    aliases: &["typescript-pack", "javascript-pack"],
    supported_languages: &["typescript", "javascript", "tsx", "jsx"],
    language_profiles: &[
        LanguageProfileDescriptor {
            id: "typescript-standard",
            display_name: "TypeScript Standard",
            language_id: "typescript",
            dialect: Some("ts"),
            aliases: &["ts"],
            file_extensions: &["ts", "tsx", "mts", "cts"],
            supported_source_versions: &["^5.0"],
        },
        LanguageProfileDescriptor {
            id: "javascript-standard",
            display_name: "JavaScript Standard",
            language_id: "javascript",
            dialect: Some("js"),
            aliases: &["js"],
            file_extensions: &["js", "jsx", "mjs", "cjs"],
            supported_source_versions: &[],
        },
    ],
    compatibility: ExtensionCompatibility::phase1_local_cli(LANGUAGE_PACK_FEATURES),
};

const SEMANTIC_CLONES_CAPABILITY_DESCRIPTOR: CapabilityDescriptor = CapabilityDescriptor {
    id: "semantic-clones-capability-pack",
    display_name: "Semantic Clones Capability Pack",
    version: "1.0.0",
    api_version: 1,
    description: "Semantic clone detection and ranking capability",
    default_enabled: true,
    experimental: false,
    dependencies: &[],
    required_host_features: CAPABILITY_PACK_FEATURES,
};

const KNOWLEDGE_CAPABILITY_DESCRIPTOR: CapabilityDescriptor = CapabilityDescriptor {
    id: "knowledge-capability-pack",
    display_name: "Knowledge Capability Pack",
    version: "1.0.0",
    api_version: 1,
    description: "Knowledge retrieval and enrichment capability",
    default_enabled: true,
    experimental: false,
    dependencies: &[],
    required_host_features: CAPABILITY_PACK_FEATURES,
};

const TEST_HARNESS_CAPABILITY_DESCRIPTOR: CapabilityDescriptor = CapabilityDescriptor {
    id: "test-harness-capability-pack",
    display_name: "Test Harness Capability Pack",
    version: "1.0.0",
    api_version: 1,
    description: "Test harness ingestion and verification capability",
    default_enabled: true,
    experimental: false,
    dependencies: &[],
    required_host_features: CAPABILITY_PACK_FEATURES,
};

const SEMANTIC_CLONES_CAPABILITY_PACK: CapabilityPackDescriptor = CapabilityPackDescriptor {
    capability: SEMANTIC_CLONES_CAPABILITY_DESCRIPTOR,
    aliases: &["semantic-clones-pack"],
    stage_contributions: &[CapabilityStageContribution {
        id: "semantic-clones",
    }],
    ingester_contributions: &[CapabilityIngesterContribution {
        id: "semantic-clones-ingester",
    }],
    schema_module_contributions: &[CapabilitySchemaModuleContribution {
        id: "semantic-clones-schema",
    }],
    query_example_contributions: &[CapabilityQueryExampleContribution {
        id: "semantic-clones-basic",
        query: "repo(\"bitloops\")->semanticClones()->limit(10)",
    }],
    compatibility: ExtensionCompatibility::phase1_local_cli(CAPABILITY_PACK_FEATURES),
    migrations: &[],
};

const KNOWLEDGE_CAPABILITY_PACK: CapabilityPackDescriptor = CapabilityPackDescriptor {
    capability: KNOWLEDGE_CAPABILITY_DESCRIPTOR,
    aliases: &["knowledge-pack"],
    stage_contributions: &[CapabilityStageContribution { id: "knowledge" }],
    ingester_contributions: &[CapabilityIngesterContribution {
        id: "knowledge-ingester",
    }],
    schema_module_contributions: &[CapabilitySchemaModuleContribution {
        id: "knowledge-schema",
    }],
    query_example_contributions: &[CapabilityQueryExampleContribution {
        id: "knowledge-basic",
        query: "repo(\"bitloops\")->knowledge()->limit(10)",
    }],
    compatibility: ExtensionCompatibility::phase1_local_cli(CAPABILITY_PACK_FEATURES),
    migrations: &[],
};

const TEST_HARNESS_CAPABILITY_PACK: CapabilityPackDescriptor = CapabilityPackDescriptor {
    capability: TEST_HARNESS_CAPABILITY_DESCRIPTOR,
    aliases: &["test-harness-pack"],
    stage_contributions: &[CapabilityStageContribution { id: "test-harness" }],
    ingester_contributions: &[CapabilityIngesterContribution {
        id: "test-harness-ingester",
    }],
    schema_module_contributions: &[CapabilitySchemaModuleContribution {
        id: "test-harness-schema",
    }],
    query_example_contributions: &[CapabilityQueryExampleContribution {
        id: "test-harness-basic",
        query: "repo(\"bitloops\")->testHarness()->limit(10)",
    }],
    compatibility: ExtensionCompatibility::phase1_local_cli(CAPABILITY_PACK_FEATURES),
    migrations: &[],
};

#[derive(Debug)]
pub enum CoreExtensionHostError {
    Language(LanguagePackRegistryError),
    Capability(CapabilityPackRegistryError),
    Compatibility(ExtensionCompatibilityError),
    Migration(String),
}

impl Display for CoreExtensionHostError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Language(error) => write!(f, "language pack registration failed: {error}"),
            Self::Capability(error) => {
                write!(f, "capability pack registration failed: {error}")
            }
            Self::Compatibility(error) => {
                write!(f, "extension compatibility check failed: {error}")
            }
            Self::Migration(message) => write!(f, "capability migration failed: {message}"),
        }
    }
}

impl Error for CoreExtensionHostError {}

impl From<LanguagePackRegistryError> for CoreExtensionHostError {
    fn from(value: LanguagePackRegistryError) -> Self {
        Self::Language(value)
    }
}

impl From<CapabilityPackRegistryError> for CoreExtensionHostError {
    fn from(value: CapabilityPackRegistryError) -> Self {
        Self::Capability(value)
    }
}

impl From<ExtensionCompatibilityError> for CoreExtensionHostError {
    fn from(value: ExtensionCompatibilityError) -> Self {
        Self::Compatibility(value)
    }
}

#[derive(Debug, Clone)]
pub struct CoreExtensionHost {
    compatibility_context: HostCompatibilityContext,
    language_packs: LanguagePackRegistry,
    capability_packs: CapabilityPackRegistry,
    diagnostics: Vec<ExtensionDiagnostic>,
    migrated_capability_packs: HashSet<String>,
    applied_migrations: Vec<CapabilityMigrationExecution>,
}

impl Default for CoreExtensionHost {
    fn default() -> Self {
        Self {
            compatibility_context: HostCompatibilityContext::default(),
            language_packs: LanguagePackRegistry::new(),
            capability_packs: CapabilityPackRegistry::new(),
            diagnostics: Vec::new(),
            migrated_capability_packs: HashSet::new(),
            applied_migrations: Vec::new(),
        }
    }
}

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

impl CoreExtensionHost {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_builtins() -> Result<Self, CoreExtensionHostError> {
        let mut host = Self::new();
        host.bootstrap_builtins()?;
        Ok(host)
    }

    pub fn compatibility_context(&self) -> HostCompatibilityContext {
        self.compatibility_context
    }

    pub fn bootstrap_builtins(&mut self) -> Result<(), CoreExtensionHostError> {
        self.register_language_pack(RUST_LANGUAGE_PACK)?;
        self.register_language_pack(TS_JS_LANGUAGE_PACK)?;
        self.register_capability_pack(SEMANTIC_CLONES_CAPABILITY_PACK)?;
        self.register_capability_pack(KNOWLEDGE_CAPABILITY_PACK)?;
        self.register_capability_pack(TEST_HARNESS_CAPABILITY_PACK)?;
        Ok(())
    }

    pub fn register_language_pack(
        &mut self,
        descriptor: LanguagePackDescriptor,
    ) -> Result<(), CoreExtensionHostError> {
        let pack_id = descriptor.id.to_ascii_lowercase();
        if let Err(error) = descriptor.compatibility.validate(
            "language pack",
            descriptor.id,
            self.compatibility_context,
        ) {
            self.push_diagnostic(
                "language-pack",
                descriptor.id,
                ExtensionDiagnosticSeverity::Error,
                ExtensionDiagnosticKind::Compatibility,
                "compatibility_failed",
                error.to_string(),
            );
            return Err(error.into());
        }

        if let Err(error) = self.language_packs.register(descriptor) {
            self.push_diagnostic(
                "language-pack",
                &pack_id,
                ExtensionDiagnosticSeverity::Error,
                ExtensionDiagnosticKind::Registration,
                "registration_failed",
                error.to_string(),
            );
            return Err(error.into());
        }

        self.push_diagnostic(
            "language-pack",
            &pack_id,
            ExtensionDiagnosticSeverity::Info,
            ExtensionDiagnosticKind::Registration,
            "registered",
            "language pack registered".to_string(),
        );
        Ok(())
    }

    pub fn register_capability_pack(
        &mut self,
        descriptor: CapabilityPackDescriptor,
    ) -> Result<(), CoreExtensionHostError> {
        let pack_id = descriptor.id().to_ascii_lowercase();
        if let Err(error) = descriptor.compatibility.validate(
            "capability pack",
            descriptor.id(),
            self.compatibility_context,
        ) {
            self.push_diagnostic(
                "capability-pack",
                descriptor.id(),
                ExtensionDiagnosticSeverity::Error,
                ExtensionDiagnosticKind::Compatibility,
                "compatibility_failed",
                error.to_string(),
            );
            return Err(error.into());
        }

        if let Err(error) = self.capability_packs.register(descriptor) {
            self.push_diagnostic(
                "capability-pack",
                &pack_id,
                ExtensionDiagnosticSeverity::Error,
                ExtensionDiagnosticKind::Registration,
                "registration_failed",
                error.to_string(),
            );
            return Err(error.into());
        }

        self.push_diagnostic(
            "capability-pack",
            &pack_id,
            ExtensionDiagnosticSeverity::Info,
            ExtensionDiagnosticKind::Registration,
            "registered",
            "capability pack registered".to_string(),
        );
        Ok(())
    }

    pub fn run_capability_migrations<F>(&mut self, executor: F) -> CapabilityMigrationRunReport
    where
        F: FnMut(&CapabilityMigrationStep) -> Result<(), String>,
    {
        let mut steps = Vec::new();
        let mut steps_per_pack: HashMap<String, usize> = HashMap::new();
        let mut packs_without_migrations = Vec::new();

        for pack_id in self.capability_packs.registered_pack_ids() {
            let Some(descriptor) = self.capability_packs.resolve_pack(pack_id) else {
                continue;
            };
            if descriptor.migrations.is_empty() {
                packs_without_migrations.push(pack_id.to_string());
                continue;
            }
            for migration in descriptor.migrations {
                steps.push(CapabilityMigrationStep::from_descriptor(pack_id, migration));
                let entry = steps_per_pack.entry(pack_id.to_string()).or_insert(0);
                *entry += 1;
            }
        }

        let report = orchestrate_capability_migrations(steps, executor);

        for pack_id in packs_without_migrations {
            self.migrated_capability_packs.insert(pack_id);
        }

        let mut executed_per_pack: HashMap<String, usize> = HashMap::new();
        for execution in &report.executed {
            self.applied_migrations.push(execution.clone());
            let entry = executed_per_pack
                .entry(execution.pack_id.clone())
                .or_insert(0);
            *entry += 1;
            self.push_diagnostic(
                "capability-pack",
                &execution.pack_id,
                ExtensionDiagnosticSeverity::Info,
                ExtensionDiagnosticKind::Migration,
                "migration_applied",
                format!(
                    "applied migration `{}` at order {}",
                    execution.migration_id, execution.order
                ),
            );
        }

        for (pack_id, total_steps) in steps_per_pack {
            if executed_per_pack.get(&pack_id).copied().unwrap_or_default() == total_steps {
                self.migrated_capability_packs.insert(pack_id);
            }
        }

        if let Some(failure) = report.failure.as_ref() {
            self.push_diagnostic(
                "capability-pack",
                &failure.pack_id,
                ExtensionDiagnosticSeverity::Error,
                ExtensionDiagnosticKind::Migration,
                "migration_failed",
                format!(
                    "migration `{}` failed at order {}: {}",
                    failure.migration_id, failure.order, failure.reason
                ),
            );
        }

        report
    }

    pub fn language_packs(&self) -> &LanguagePackRegistry {
        &self.language_packs
    }

    pub fn language_packs_mut(&mut self) -> &mut LanguagePackRegistry {
        &mut self.language_packs
    }

    pub fn capability_packs(&self) -> &CapabilityPackRegistry {
        &self.capability_packs
    }

    pub fn capability_packs_mut(&mut self) -> &mut CapabilityPackRegistry {
        &mut self.capability_packs
    }

    pub fn diagnostics(&self) -> &[ExtensionDiagnostic] {
        &self.diagnostics
    }

    pub fn applied_migrations(&self) -> &[CapabilityMigrationExecution] {
        &self.applied_migrations
    }

    pub fn readiness_snapshot(&self) -> CoreExtensionHostReadinessSnapshot {
        let readiness_reports = self.readiness_reports();
        let mut diagnostics = self.diagnostics.clone();
        diagnostics.extend(self.readiness_diagnostics(&readiness_reports));

        CoreExtensionHostReadinessSnapshot {
            language_pack_ids: self
                .language_packs
                .registered_pack_ids()
                .into_iter()
                .map(str::to_string)
                .collect(),
            capability_pack_ids: self
                .capability_packs
                .registered_pack_ids()
                .into_iter()
                .map(str::to_string)
                .collect(),
            language_observations: self.language_packs.observations().to_vec(),
            capability_observations: self.capability_packs.observations().to_vec(),
            diagnostics,
            readiness_reports,
        }
    }

    fn readiness_reports(&self) -> Vec<ExtensionReadinessReport> {
        let mut reports = Vec::new();

        for pack_id in self.language_packs.registered_pack_ids() {
            reports.push(ExtensionReadinessReport {
                family: "language-pack".to_string(),
                id: pack_id.to_string(),
                registered: true,
                ready: true,
                status: ExtensionReadinessStatus::Ready,
                lifecycle_state: ExtensionLifecycleState::Ready,
                failures: Vec::new(),
            });
        }

        for pack_id in self.capability_packs.registered_pack_ids() {
            let Some(descriptor) = self.capability_packs.resolve_pack(pack_id) else {
                continue;
            };
            let migrations_pending = !descriptor.migrations.is_empty()
                && !self.migrated_capability_packs.contains(pack_id);
            let failures = if migrations_pending {
                vec![ExtensionReadinessFailure {
                    code: "migrations_pending".to_string(),
                    message: "capability pack has unapplied migrations".to_string(),
                }]
            } else {
                Vec::new()
            };
            let status = ExtensionReadinessStatus::from_failures(!failures.is_empty());
            reports.push(ExtensionReadinessReport {
                family: "capability-pack".to_string(),
                id: pack_id.to_string(),
                registered: true,
                ready: status == ExtensionReadinessStatus::Ready,
                status,
                lifecycle_state: if status == ExtensionReadinessStatus::Ready {
                    if descriptor.migrations.is_empty() {
                        ExtensionLifecycleState::Ready
                    } else {
                        ExtensionLifecycleState::Migrated
                    }
                } else {
                    ExtensionLifecycleState::Registered
                },
                failures,
            });
        }

        reports
    }

    fn push_diagnostic(
        &mut self,
        family: &str,
        extension_id: &str,
        severity: ExtensionDiagnosticSeverity,
        kind: ExtensionDiagnosticKind,
        code: &str,
        message: String,
    ) {
        self.diagnostics.push(ExtensionDiagnostic {
            family: family.to_string(),
            extension_id: extension_id.to_string(),
            severity,
            kind,
            code: code.to_string(),
            message,
        });
    }

    fn readiness_diagnostics(
        &self,
        readiness_reports: &[ExtensionReadinessReport],
    ) -> Vec<ExtensionDiagnostic> {
        let mut diagnostics = Vec::new();
        for report in readiness_reports {
            for failure in &report.failures {
                diagnostics.push(ExtensionDiagnostic {
                    family: report.family.clone(),
                    extension_id: report.id.clone(),
                    severity: ExtensionDiagnosticSeverity::Error,
                    kind: ExtensionDiagnosticKind::Readiness,
                    code: failure.code.clone(),
                    message: failure.message.clone(),
                });
            }
        }
        diagnostics
    }
}

#[cfg(test)]
mod tests {
    use super::super::lifecycle::{CapabilityMigrationStatus, CapabilityPackMigrationDescriptor};
    use super::*;

    const CAPABILITY_WITH_MIGRATIONS: CapabilityPackDescriptor = CapabilityPackDescriptor {
        capability: CapabilityDescriptor {
            id: "migrating-pack",
            display_name: "Migrating Pack",
            version: "1.0.0",
            api_version: 1,
            description: "Capability pack with host-managed migrations",
            default_enabled: true,
            experimental: false,
            dependencies: &[],
            required_host_features: CAPABILITY_PACK_FEATURES,
        },
        aliases: &["migrating"],
        stage_contributions: &[CapabilityStageContribution {
            id: "migrating-stage",
        }],
        ingester_contributions: &[CapabilityIngesterContribution {
            id: "migrating-ingester",
        }],
        schema_module_contributions: &[CapabilitySchemaModuleContribution {
            id: "migrating-schema",
        }],
        query_example_contributions: &[CapabilityQueryExampleContribution {
            id: "migrating-example",
            query: "repo(\"bitloops\")->migratingStage()",
        }],
        compatibility: ExtensionCompatibility::phase1_local_cli(CAPABILITY_PACK_FEATURES),
        migrations: &[
            CapabilityPackMigrationDescriptor {
                id: "001",
                order: 1,
                description: "create tables",
            },
            CapabilityPackMigrationDescriptor {
                id: "002",
                order: 2,
                description: "backfill rows",
            },
        ],
    };

    const INCOMPATIBLE_LANGUAGE_PACK: LanguagePackDescriptor = LanguagePackDescriptor {
        id: "incompatible-language-pack",
        version: "1.0.0",
        api_version: 1,
        display_name: "Incompatible Language Pack",
        aliases: &["incompatible-language"],
        supported_languages: &["incompatible"],
        language_profiles: &[LanguageProfileDescriptor {
            id: "incompatible-default",
            display_name: "Incompatible Default",
            language_id: "incompatible",
            dialect: None,
            aliases: &[],
            file_extensions: &["inc"],
            supported_source_versions: &[],
        }],
        compatibility: ExtensionCompatibility::phase1_local_cli(&["missing-feature"]),
    };

    const INCOMPATIBLE_CAPABILITY_PACK: CapabilityPackDescriptor = CapabilityPackDescriptor {
        capability: CapabilityDescriptor {
            id: "incompatible-capability-pack",
            display_name: "Incompatible Capability Pack",
            version: "1.0.0",
            api_version: 1,
            description: "Capability pack requiring unsupported host features",
            default_enabled: true,
            experimental: true,
            dependencies: &[],
            required_host_features: &["missing-feature"],
        },
        aliases: &["incompatible-capability"],
        stage_contributions: &[CapabilityStageContribution {
            id: "incompatible-stage",
        }],
        ingester_contributions: &[],
        schema_module_contributions: &[],
        query_example_contributions: &[],
        compatibility: ExtensionCompatibility::phase1_local_cli(&["missing-feature"]),
        migrations: &[],
    };

    #[test]
    fn core_extension_host_bootstraps_language_and_capability_builtins() {
        let host = CoreExtensionHost::with_builtins().expect("bootstrap builtins");

        assert!(
            host.language_packs().resolve_for_language("rust").is_some(),
            "rust language pack should be resolvable"
        );
        assert!(
            host.language_packs()
                .resolve_for_language("typescript")
                .is_some(),
            "typescript language pack should be resolvable"
        );
        assert_eq!(
            host.capability_packs()
                .resolve_stage_owner("semantic-clones"),
            Some("semantic-clones-capability-pack")
        );
        assert_eq!(
            host.capability_packs()
                .resolve_ingester_owner("test-harness-ingester"),
            Some("test-harness-capability-pack")
        );
        assert_eq!(
            host.capability_packs()
                .resolve_schema_module_owner("knowledge-schema"),
            Some("knowledge-capability-pack")
        );
        assert_eq!(
            host.capability_packs()
                .resolve_query_example_owner("semantic-clones-basic"),
            Some("semantic-clones-capability-pack")
        );

        let readiness = host.readiness_snapshot();
        assert!(readiness.is_ready(), "built-in host should report ready");
        assert!(
            readiness
                .language_pack_ids
                .iter()
                .any(|pack_id| pack_id == "rust-language-pack")
        );
        assert!(
            readiness
                .capability_pack_ids
                .iter()
                .any(|pack_id| pack_id == "semantic-clones-capability-pack")
        );
    }

    #[test]
    fn core_extension_host_reports_compatibility_failures_in_diagnostics() {
        let mut host = CoreExtensionHost::new();
        let error = host
            .register_language_pack(INCOMPATIBLE_LANGUAGE_PACK)
            .expect_err("incompatible pack should fail registration");
        assert!(matches!(error, CoreExtensionHostError::Compatibility(_)));
        assert!(
            host.diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code == "compatibility_failed")
        );

        let error = host
            .register_capability_pack(INCOMPATIBLE_CAPABILITY_PACK)
            .expect_err("incompatible capability should fail registration");
        assert!(matches!(error, CoreExtensionHostError::Compatibility(_)));
    }

    #[test]
    fn core_extension_host_distinguishes_registered_vs_ready_for_migrating_capability_packs() {
        let mut host = CoreExtensionHost::new();
        host.register_capability_pack(CAPABILITY_WITH_MIGRATIONS)
            .expect("register migrating capability pack");

        let before = host.readiness_snapshot();
        let before_report = before
            .readiness_reports
            .iter()
            .find(|report| report.id == "migrating-pack")
            .expect("migrating report before migrations");
        assert!(before_report.registered);
        assert!(!before_report.ready);
        assert_eq!(before_report.status, ExtensionReadinessStatus::NotReady);
        assert!(
            before_report
                .failures
                .iter()
                .any(|failure| failure.code == "migrations_pending")
        );
        assert!(
            before
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.kind == ExtensionDiagnosticKind::Readiness)
        );

        let report = host.run_capability_migrations(|_| Ok(()));
        assert_eq!(report.status, CapabilityMigrationStatus::Completed);

        let after = host.readiness_snapshot();
        let after_report = after
            .readiness_reports
            .iter()
            .find(|report| report.id == "migrating-pack")
            .expect("migrating report after migrations");
        assert!(after_report.registered);
        assert!(after_report.ready);
        assert_eq!(after_report.status, ExtensionReadinessStatus::Ready);
    }

    #[test]
    fn core_extension_host_capability_migrations_fail_fast_and_record_diagnostics() {
        let mut host = CoreExtensionHost::new();
        host.register_capability_pack(CAPABILITY_WITH_MIGRATIONS)
            .expect("register migrating capability pack");

        let report = host.run_capability_migrations(|step| {
            if step.migration_id == "002" {
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
                .map(|execution| execution.migration_id.as_str())
                .collect::<Vec<_>>(),
            vec!["001"]
        );
        assert!(
            host.diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code == "migration_failed")
        );
    }
}

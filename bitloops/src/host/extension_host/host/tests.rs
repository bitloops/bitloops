use crate::host::extension_host::capability::{
    CapabilityDescriptor, CapabilityIngesterContribution, CapabilityPackDescriptor,
    CapabilityQueryExampleContribution, CapabilitySchemaModuleContribution,
    CapabilityStageContribution,
};
use crate::host::extension_host::host::builtins::CAPABILITY_PACK_FEATURES;
use crate::host::extension_host::host::{CoreExtensionHost, CoreExtensionHostError};
use crate::host::extension_host::language::{LanguagePackDescriptor, LanguageProfileDescriptor};
use crate::host::extension_host::lifecycle::{
    CapabilityMigrationStatus, CapabilityPackMigrationDescriptor, ExtensionCompatibility,
    ExtensionDiagnosticKind, ExtensionReadinessStatus,
};

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
    assert!(
        host.language_packs()
            .resolve_for_language("python")
            .is_some(),
        "python language pack should be resolvable"
    );
    assert!(
        host.language_packs().resolve_for_language("csharp").is_some(),
        "csharp language pack should be resolvable"
    );
    assert!(
        host.language_packs().resolve_for_language("go").is_some(),
        "go language pack should be resolvable"
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
            .language_pack_ids
            .iter()
            .any(|pack_id| pack_id == "python-language-pack")
    );
    assert!(
        readiness
            .language_pack_ids
            .iter()
            .any(|pack_id| pack_id == "go-language-pack")
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

    let report = host.run_capability_migrations(|context| {
        if context.migration_id == "002" {
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

#[test]
fn core_extension_host_blocks_execution_eligibility_for_not_ready_capability_packs() {
    let mut host = CoreExtensionHost::new();
    host.register_capability_pack(CAPABILITY_WITH_MIGRATIONS)
        .expect("register migrating capability pack");

    let stage_error = host
        .resolve_stage_owner_for_execution("migrating-stage")
        .expect_err("stage should be blocked before migrations complete");
    assert!(matches!(
        stage_error,
        CoreExtensionHostError::CapabilityNotReady { .. }
    ));

    let ingester_error = host
        .resolve_ingester_owner_for_ingest("migrating-ingester")
        .expect_err("ingester should be blocked before migrations complete");
    assert!(matches!(
        ingester_error,
        CoreExtensionHostError::CapabilityNotReady { .. }
    ));

    let report = host.run_capability_migrations(|_| Ok(()));
    assert_eq!(report.status, CapabilityMigrationStatus::Completed);

    assert_eq!(
        host.resolve_stage_owner_for_execution("migrating-stage")
            .expect("stage should be eligible after migrations"),
        "migrating-pack"
    );
    assert_eq!(
        host.resolve_ingester_owner_for_ingest("migrating-ingester")
            .expect("ingester should be eligible after migrations"),
        "migrating-pack"
    );
}

#[test]
fn core_extension_host_exposes_migration_context_to_executor() {
    let mut host = CoreExtensionHost::new();
    host.register_capability_pack(CAPABILITY_WITH_MIGRATIONS)
        .expect("register migrating capability pack");

    let mut observed = Vec::new();
    let report = host.run_capability_migrations(|context| {
        observed.push((
            context.capability_pack_id.clone(),
            context.migration_id.clone(),
            context.order,
        ));
        Ok(())
    });

    assert_eq!(report.status, CapabilityMigrationStatus::Completed);
    assert_eq!(
        observed,
        vec![
            ("migrating-pack".to_string(), "001".to_string(), 1),
            ("migrating-pack".to_string(), "002".to_string(), 2),
        ]
    );
}

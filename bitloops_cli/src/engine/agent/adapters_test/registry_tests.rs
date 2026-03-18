use super::super::adapters::{
    AgentAdapterConfiguration, AgentAdapterPackageBoundary, AgentAdapterPackageCompatibility,
    AgentAdapterPackageDescriptor, AgentAdapterPackageDiscoveryStatus,
    AgentAdapterPackageLifecycle, AgentAdapterPackageLifecyclePhase,
    AgentAdapterPackageMetadataVersion, AgentAdapterPackageResponsibility,
    AgentAdapterPackageSource, AgentAdapterPackageTrustModel, AgentAdapterRegistry,
    AgentReadinessStatus,
};
use super::fixtures::{
    ALIAS_ALPHA, ALIAS_BETA, ALPHA_CALLBACKS, ALPHA_REQUIRED_SCHEMA, BETA_CALLBACKS, EMPTY_SCHEMA,
    LOCAL_RUNTIME, NO_ALIASES, PROFILE_ALPHA_ALIASES, PROFILE_BETA_ALIASES, REMOTE_ONLY_RUNTIME,
    SHARED_ALIAS, make_registration, make_registration_with_package, test_family, test_package,
    test_profile,
};

#[test]
#[allow(non_snake_case)]
fn TestAgentAdapterRegistryRejectsInvalidRegistrations() {
    let err = match AgentAdapterRegistry::new(vec![]) {
        Ok(_) => panic!("expected empty registration error"),
        Err(err) => err,
    };
    assert!(
        err.to_string()
            .contains("at least one adapter registration is required")
    );

    let duplicate_id = match AgentAdapterRegistry::new(vec![
        make_registration(
            "alpha",
            "Alpha",
            "alpha-type",
            NO_ALIASES,
            true,
            ALPHA_CALLBACKS,
            test_family("family-a", "test.family.a", LOCAL_RUNTIME),
            test_profile(
                "profile-alpha",
                "family-a",
                NO_ALIASES,
                EMPTY_SCHEMA,
                LOCAL_RUNTIME,
            ),
            LOCAL_RUNTIME,
        ),
        make_registration(
            "alpha",
            "Alpha Duplicate",
            "alpha-type-2",
            NO_ALIASES,
            false,
            BETA_CALLBACKS,
            test_family("family-b", "test.family.b", LOCAL_RUNTIME),
            test_profile(
                "profile-beta",
                "family-b",
                NO_ALIASES,
                EMPTY_SCHEMA,
                LOCAL_RUNTIME,
            ),
            LOCAL_RUNTIME,
        ),
    ]) {
        Ok(_) => panic!("expected duplicate id error"),
        Err(err) => err,
    };
    assert!(duplicate_id.to_string().contains("duplicate adapter id"));

    let duplicate_agent_type = match AgentAdapterRegistry::new(vec![
        make_registration(
            "alpha",
            "Alpha",
            "shared-type",
            NO_ALIASES,
            true,
            ALPHA_CALLBACKS,
            test_family("family-a", "test.family.a", LOCAL_RUNTIME),
            test_profile(
                "profile-alpha",
                "family-a",
                NO_ALIASES,
                EMPTY_SCHEMA,
                LOCAL_RUNTIME,
            ),
            LOCAL_RUNTIME,
        ),
        make_registration(
            "beta",
            "Beta",
            "shared-type",
            NO_ALIASES,
            false,
            BETA_CALLBACKS,
            test_family("family-a", "test.family.a", LOCAL_RUNTIME),
            test_profile(
                "profile-beta",
                "family-a",
                NO_ALIASES,
                EMPTY_SCHEMA,
                LOCAL_RUNTIME,
            ),
            LOCAL_RUNTIME,
        ),
    ]) {
        Ok(_) => panic!("expected duplicate type error"),
        Err(err) => err,
    };
    assert!(
        duplicate_agent_type
            .to_string()
            .contains("duplicate adapter agent type")
    );

    let alias_collision = match AgentAdapterRegistry::new(vec![
        make_registration(
            "alpha",
            "Alpha",
            "alpha-type",
            SHARED_ALIAS,
            true,
            ALPHA_CALLBACKS,
            test_family("family-a", "test.family.a", LOCAL_RUNTIME),
            test_profile(
                "profile-alpha",
                "family-a",
                NO_ALIASES,
                EMPTY_SCHEMA,
                LOCAL_RUNTIME,
            ),
            LOCAL_RUNTIME,
        ),
        make_registration(
            "beta",
            "Beta",
            "beta-type",
            SHARED_ALIAS,
            false,
            BETA_CALLBACKS,
            test_family("family-a", "test.family.a", LOCAL_RUNTIME),
            test_profile(
                "profile-beta",
                "family-a",
                NO_ALIASES,
                EMPTY_SCHEMA,
                LOCAL_RUNTIME,
            ),
            LOCAL_RUNTIME,
        ),
    ]) {
        Ok(_) => panic!("expected alias collision"),
        Err(err) => err,
    };
    assert!(alias_collision.to_string().contains("alias collision"));

    let multiple_defaults = match AgentAdapterRegistry::new(vec![
        make_registration(
            "alpha",
            "Alpha",
            "alpha-type",
            NO_ALIASES,
            true,
            ALPHA_CALLBACKS,
            test_family("family-a", "test.family.a", LOCAL_RUNTIME),
            test_profile(
                "profile-alpha",
                "family-a",
                NO_ALIASES,
                EMPTY_SCHEMA,
                LOCAL_RUNTIME,
            ),
            LOCAL_RUNTIME,
        ),
        make_registration(
            "beta",
            "Beta",
            "beta-type",
            NO_ALIASES,
            true,
            BETA_CALLBACKS,
            test_family("family-a", "test.family.a", LOCAL_RUNTIME),
            test_profile(
                "profile-beta",
                "family-a",
                NO_ALIASES,
                EMPTY_SCHEMA,
                LOCAL_RUNTIME,
            ),
            LOCAL_RUNTIME,
        ),
    ]) {
        Ok(_) => panic!("expected multiple defaults"),
        Err(err) => err,
    };
    assert!(
        multiple_defaults
            .to_string()
            .contains("multiple default adapters configured")
    );

    let runtime_mismatch = match AgentAdapterRegistry::new(vec![make_registration(
        "alpha",
        "Alpha",
        "alpha-type",
        NO_ALIASES,
        true,
        ALPHA_CALLBACKS,
        test_family("family-a", "test.family.a", LOCAL_RUNTIME),
        test_profile(
            "profile-alpha",
            "family-a",
            NO_ALIASES,
            EMPTY_SCHEMA,
            REMOTE_ONLY_RUNTIME,
        ),
        LOCAL_RUNTIME,
    )]) {
        Ok(_) => panic!("expected runtime mismatch"),
        Err(err) => err,
    };
    let runtime_text = runtime_mismatch.to_string();
    assert!(
        runtime_text.contains("must support at least one runtime")
            || runtime_text.contains("incompatible with host runtime"),
        "unexpected runtime mismatch text: {runtime_text}"
    );

    let package_mismatch = match AgentAdapterRegistry::new(vec![make_registration_with_package(
        "alpha",
        "Alpha",
        "alpha-type",
        NO_ALIASES,
        true,
        ALPHA_CALLBACKS,
        test_family("family-a", "test.family.a", LOCAL_RUNTIME),
        test_profile(
            "profile-alpha",
            "family-a",
            NO_ALIASES,
            EMPTY_SCHEMA,
            LOCAL_RUNTIME,
        ),
        LOCAL_RUNTIME,
        test_package("alpha-package", "Alpha"),
    )]) {
        Ok(_) => panic!("expected package mismatch"),
        Err(err) => err,
    };
    assert!(
        package_mismatch
            .to_string()
            .contains("is linked to package alpha-package but expected alpha")
    );

    let missing_compatibility_claims =
        match AgentAdapterRegistry::new(vec![make_registration_with_package(
            "alpha",
            "Alpha",
            "alpha-type",
            NO_ALIASES,
            true,
            ALPHA_CALLBACKS,
            test_family("family-a", "test.family.a", LOCAL_RUNTIME),
            test_profile(
                "profile-alpha",
                "family-a",
                NO_ALIASES,
                EMPTY_SCHEMA,
                LOCAL_RUNTIME,
            ),
            LOCAL_RUNTIME,
            AgentAdapterPackageDescriptor {
                id: "alpha",
                display_name: "Alpha",
                version: "1.0.0",
                metadata_version: AgentAdapterPackageMetadataVersion::current(),
                source: AgentAdapterPackageSource::Manifest,
                trust_model: AgentAdapterPackageTrustModel::HostVerifiedManifest,
                boundary: AgentAdapterPackageBoundary::first_party_linked(),
                lifecycle: AgentAdapterPackageLifecycle::default(),
                compatibility: AgentAdapterPackageCompatibility::phase1(),
            },
        )]) {
            Ok(_) => panic!("expected package trust validation failure"),
            Err(err) => err,
        };
    assert!(
        missing_compatibility_claims
            .to_string()
            .contains("must expose package compatibility claims")
    );

    let invalid_lifecycle = match AgentAdapterRegistry::new(vec![make_registration_with_package(
        "alpha",
        "Alpha",
        "alpha-type",
        NO_ALIASES,
        true,
        ALPHA_CALLBACKS,
        test_family("family-a", "test.family.a", LOCAL_RUNTIME),
        test_profile(
            "profile-alpha",
            "family-a",
            NO_ALIASES,
            EMPTY_SCHEMA,
            LOCAL_RUNTIME,
        ),
        LOCAL_RUNTIME,
        AgentAdapterPackageDescriptor {
            id: "alpha",
            display_name: "Alpha",
            version: "1.0.0",
            metadata_version: AgentAdapterPackageMetadataVersion::current(),
            source: AgentAdapterPackageSource::FirstPartyLinked,
            trust_model: AgentAdapterPackageTrustModel::FirstPartyLinked,
            boundary: AgentAdapterPackageBoundary {
                host_owned_responsibilities: &[
                    AgentAdapterPackageResponsibility::HostResolution,
                    AgentAdapterPackageResponsibility::HostValidation,
                    AgentAdapterPackageResponsibility::HostLifecycleControl,
                    AgentAdapterPackageResponsibility::HostAudit,
                ],
                package_owned_responsibilities: &[
                    AgentAdapterPackageResponsibility::PackageIdentity,
                    AgentAdapterPackageResponsibility::PackageManifest,
                    AgentAdapterPackageResponsibility::PackageVersioning,
                    AgentAdapterPackageResponsibility::PackageEntrypoint,
                    AgentAdapterPackageResponsibility::PackageTargetBehaviour,
                ],
            },
            lifecycle: AgentAdapterPackageLifecycle {
                phases: &[
                    AgentAdapterPackageLifecyclePhase::Discovered,
                    AgentAdapterPackageLifecyclePhase::Validated,
                ],
                host_controls_activation: true,
                host_controls_unload: true,
            },
            compatibility: AgentAdapterPackageCompatibility::phase1(),
        },
    )]) {
        Ok(_) => panic!("expected lifecycle validation failure"),
        Err(err) => err,
    };
    assert!(
        invalid_lifecycle
            .to_string()
            .contains("unsupported lifecycle phases")
    );
}

#[test]
#[allow(non_snake_case)]
fn TestAgentAdapterRegistryDiscoversPackageMetadataForLinkedAndManifestPackages() {
    let registry = AgentAdapterRegistry::new(vec![
        make_registration(
            "alpha",
            "Alpha",
            "alpha-type",
            ALIAS_ALPHA,
            true,
            ALPHA_CALLBACKS,
            test_family("family-a", "test.family.a", LOCAL_RUNTIME),
            test_profile(
                "profile-alpha",
                "family-a",
                PROFILE_ALPHA_ALIASES,
                EMPTY_SCHEMA,
                LOCAL_RUNTIME,
            ),
            LOCAL_RUNTIME,
        ),
        make_registration_with_package(
            "beta",
            "Beta",
            "beta-type",
            ALIAS_BETA,
            false,
            BETA_CALLBACKS,
            test_family("family-b", "test.family.b", LOCAL_RUNTIME),
            test_profile(
                "profile-beta",
                "family-b",
                PROFILE_BETA_ALIASES,
                EMPTY_SCHEMA,
                LOCAL_RUNTIME,
            ),
            LOCAL_RUNTIME,
            AgentAdapterPackageDescriptor {
                id: "beta",
                display_name: "Beta",
                version: "2.1.0",
                metadata_version: AgentAdapterPackageMetadataVersion::current(),
                source: AgentAdapterPackageSource::Manifest,
                trust_model: AgentAdapterPackageTrustModel::HostVerifiedManifest,
                boundary: AgentAdapterPackageBoundary::host_verified_manifest(),
                lifecycle: AgentAdapterPackageLifecycle::default(),
                compatibility: AgentAdapterPackageCompatibility::phase1(),
            },
        ),
    ])
    .expect("registry");

    let reports = registry.discover_packages();
    assert_eq!(reports.len(), 2);
    assert_eq!(reports[0].adapter_id, "alpha");
    assert_eq!(reports[0].package_id, "alpha");
    assert_eq!(
        reports[0].metadata_version,
        AgentAdapterPackageMetadataVersion::current()
    );
    assert_eq!(
        reports[0].source,
        AgentAdapterPackageSource::FirstPartyLinked
    );
    assert_eq!(reports[0].status, AgentAdapterPackageDiscoveryStatus::Ready);
    assert_eq!(reports[0].diagnostics.len(), 0);

    assert_eq!(reports[1].adapter_id, "beta");
    assert_eq!(reports[1].package_id, "beta");
    assert_eq!(
        reports[1].metadata_version,
        AgentAdapterPackageMetadataVersion::current()
    );
    assert_eq!(reports[1].source, AgentAdapterPackageSource::Manifest);
    assert_eq!(
        reports[1].trust_model,
        AgentAdapterPackageTrustModel::HostVerifiedManifest
    );
    assert_eq!(reports[1].status, AgentAdapterPackageDiscoveryStatus::Ready);
    assert_eq!(reports[1].diagnostics.len(), 0);

    assert_eq!(registry.validate_package_metadata().len(), 2);
    assert_eq!(registry.package_discovery_reports().len(), 2);
    assert_eq!(registry.package_validation_reports().len(), 2);
}

#[test]
#[allow(non_snake_case)]
fn TestAgentAdapterPackageValidationReportsInvalidMetadataDeterministically() {
    let package = AgentAdapterPackageDescriptor {
        id: "alpha",
        display_name: "Alpha",
        version: "not-a-version",
        metadata_version: AgentAdapterPackageMetadataVersion::new(2),
        source: AgentAdapterPackageSource::Manifest,
        trust_model: AgentAdapterPackageTrustModel::FirstPartyLinked,
        boundary: AgentAdapterPackageBoundary::first_party_linked(),
        lifecycle: AgentAdapterPackageLifecycle::default(),
        compatibility: AgentAdapterPackageCompatibility::phase1(),
    };

    let diagnostics = package.validation_diagnostics("package", "alpha");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "invalid_package_version"
            && diagnostic.field.as_deref() == Some("version")
    }));
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "unsupported_metadata_version"
            && diagnostic.field.as_deref() == Some("metadata_version")
    }));
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "package_source_mismatch"
            && diagnostic.field.as_deref() == Some("source")
    }));
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "package_trust_mismatch"
            && diagnostic.field.as_deref() == Some("trust_model")
    }));
    assert_eq!(
        package.discovery_report("package", "alpha").status,
        AgentAdapterPackageDiscoveryStatus::Invalid
    );
}

#[test]
#[allow(non_snake_case)]
fn TestAgentAdapterRegistryResolvesAliasesAndCollectsReadiness() {
    let registry = AgentAdapterRegistry::new(vec![
        make_registration(
            "alpha",
            "Alpha",
            "alpha-type",
            ALIAS_ALPHA,
            true,
            ALPHA_CALLBACKS,
            test_family("family-a", "test.family.a", LOCAL_RUNTIME),
            test_profile(
                "profile-alpha",
                "family-a",
                PROFILE_ALPHA_ALIASES,
                EMPTY_SCHEMA,
                LOCAL_RUNTIME,
            ),
            LOCAL_RUNTIME,
        ),
        make_registration(
            "beta",
            "Beta",
            "beta-type",
            ALIAS_BETA,
            false,
            BETA_CALLBACKS,
            test_family("family-a", "test.family.a", LOCAL_RUNTIME),
            test_profile(
                "profile-beta",
                "family-a",
                PROFILE_BETA_ALIASES,
                EMPTY_SCHEMA,
                LOCAL_RUNTIME,
            ),
            LOCAL_RUNTIME,
        ),
    ])
    .expect("valid adapter registry");

    assert_eq!(
        registry.available_agents(),
        vec!["alpha".to_string(), "beta".to_string()]
    );
    assert_eq!(registry.default_agent_name(), "alpha");
    assert_eq!(
        registry.normalise_agent_name("alpha-cli").expect("alias"),
        "alpha"
    );
    assert_eq!(
        registry.normalise_agent_name("BETA-CLI").expect("alias"),
        "beta"
    );
    assert_eq!(
        registry
            .resolve_profile("alpha-profile")
            .expect("profile alias")
            .id,
        "profile-alpha"
    );
    assert_eq!(
        registry
            .format_resume_command("alpha", "session-123")
            .expect("resume command"),
        "alpha --resume session-123"
    );

    let repo = tempfile::tempdir().expect("tempdir");
    assert_eq!(
        registry.detect_project_agents(repo.path()),
        vec!["alpha".to_string()]
    );
    assert_eq!(
        registry.installed_agents(repo.path()),
        vec!["alpha".to_string()]
    );

    let readiness = registry.collect_readiness(repo.path());
    assert_eq!(readiness.len(), 2);
    assert!(readiness[0].project_detected);
    assert!(readiness[0].hooks_installed);
    assert!(readiness[0].compatibility_ok);
    assert!(readiness[0].config_valid);
    assert_eq!(readiness[0].status, AgentReadinessStatus::Ready);
    assert_eq!(readiness[0].package_id, "alpha");
    assert_eq!(readiness[0].package_version, "1.0.0");
    assert_eq!(readiness[0].package_trust_model, "first-party-linked");
    assert_eq!(readiness[0].protocol_family, "family-a");
    assert_eq!(readiness[0].target_profile, "profile-alpha");

    assert!(!readiness[1].project_detected);
    assert!(!readiness[1].hooks_installed);
    assert!(readiness[1].compatibility_ok);
    assert!(readiness[1].config_valid);
    assert_eq!(readiness[1].status, AgentReadinessStatus::NotReady);

    assert_eq!(
        registry.all_protected_dirs(),
        vec![
            ".alpha".to_string(),
            ".beta".to_string(),
            ".shared".to_string(),
        ]
    );
}

#[test]
#[allow(non_snake_case)]
fn TestAgentAdapterRegistryComposedResolutionByFamilyAndProfile() {
    let registry = AgentAdapterRegistry::new(vec![
        make_registration(
            "alpha",
            "Alpha",
            "alpha-type",
            ALIAS_ALPHA,
            true,
            ALPHA_CALLBACKS,
            test_family("shared-family", "test.family.shared", LOCAL_RUNTIME),
            test_profile(
                "profile-alpha",
                "shared-family",
                PROFILE_ALPHA_ALIASES,
                EMPTY_SCHEMA,
                LOCAL_RUNTIME,
            ),
            LOCAL_RUNTIME,
        ),
        make_registration(
            "beta",
            "Beta",
            "beta-type",
            ALIAS_BETA,
            false,
            BETA_CALLBACKS,
            test_family("shared-family", "test.family.shared", LOCAL_RUNTIME),
            test_profile(
                "profile-beta",
                "shared-family",
                PROFILE_BETA_ALIASES,
                EMPTY_SCHEMA,
                LOCAL_RUNTIME,
            ),
            LOCAL_RUNTIME,
        ),
    ])
    .expect("registry");

    assert_eq!(registry.available_protocol_families().len(), 1);
    assert_eq!(registry.available_target_profiles().len(), 2);

    let alpha = registry
        .resolve_composed("shared-family", "alpha-profile")
        .expect("composed alpha");
    assert_eq!(alpha.descriptor().id, "alpha");

    let err = match registry.resolve_composed("other-family", "alpha-profile") {
        Ok(_) => panic!("expected invalid composition"),
        Err(err) => err,
    };
    assert!(
        err.to_string()
            .contains("does not belong to protocol family other-family")
    );
}

#[test]
#[allow(non_snake_case)]
fn TestAgentAdapterRegistryReadinessValidatesConfigSchema() {
    let registry = AgentAdapterRegistry::new(vec![make_registration(
        "alpha",
        "Alpha",
        "alpha-type",
        NO_ALIASES,
        true,
        ALPHA_CALLBACKS,
        test_family("family-a", "test.family.a", LOCAL_RUNTIME),
        test_profile(
            "profile-alpha",
            "family-a",
            NO_ALIASES,
            ALPHA_REQUIRED_SCHEMA,
            LOCAL_RUNTIME,
        ),
        LOCAL_RUNTIME,
    )])
    .expect("registry");

    let repo = tempfile::tempdir().expect("tempdir");

    let readiness_without_config =
        registry.collect_readiness_with_config(repo.path(), &AgentAdapterConfiguration::default());
    assert_eq!(readiness_without_config.len(), 1);
    assert!(!readiness_without_config[0].config_valid);
    assert_eq!(
        readiness_without_config[0].status,
        AgentReadinessStatus::NotReady
    );
    assert!(
        readiness_without_config[0]
            .failures
            .iter()
            .any(|failure| failure.message.contains("missing required config field"))
    );

    let config = AgentAdapterConfiguration::default().with_profile_value(
        "profile-alpha",
        "api_key",
        "secret-token",
    );
    let readiness_with_config = registry.collect_readiness_with_config(repo.path(), &config);
    assert!(readiness_with_config[0].config_valid);
    assert_eq!(readiness_with_config[0].status, AgentReadinessStatus::Ready);
}

#[test]
#[allow(non_snake_case)]
fn TestAgentAdapterRegistryResolveWithTraceIncludesCorrelationMetadata() {
    let registry = AgentAdapterRegistry::new(vec![make_registration(
        "alpha",
        "Alpha",
        "alpha-type",
        ALIAS_ALPHA,
        true,
        ALPHA_CALLBACKS,
        test_family("family-a", "test.family.a", LOCAL_RUNTIME),
        test_profile(
            "profile-alpha",
            "family-a",
            PROFILE_ALPHA_ALIASES,
            EMPTY_SCHEMA,
            LOCAL_RUNTIME,
        ),
        LOCAL_RUNTIME,
    )])
    .expect("registry");

    let resolution = registry
        .resolve_with_trace("alpha-cli", Some("corr-123"))
        .expect("resolution");

    assert_eq!(resolution.registration.descriptor().id, "alpha");
    assert_eq!(resolution.trace.correlation_id, "corr-123");
    assert_eq!(resolution.trace.package_id, "alpha");
    assert_eq!(resolution.trace.package_version, "1.0.0");
    assert_eq!(resolution.trace.package_trust_model, "first-party-linked");
    assert_eq!(resolution.trace.protocol_family, "family-a");
    assert_eq!(resolution.trace.target_profile, "profile-alpha");
    assert_eq!(resolution.trace.resolution_path, "legacy-target-compat");
    assert!(resolution.trace.diagnostics.len() >= 2);

    let observations = registry.registration_observability();
    assert_eq!(observations.len(), 1);
    assert_eq!(observations[0].adapter_id, "alpha");
    assert_eq!(observations[0].package_id, "alpha");
    assert_eq!(observations[0].package_version, "1.0.0");
    assert_eq!(observations[0].package_trust_model, "first-party-linked");
    assert_eq!(observations[0].protocol_family, "family-a");
    assert_eq!(observations[0].target_profile, "profile-alpha");
}

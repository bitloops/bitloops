use super::super::adapters::AgentAdapterRegistry;

#[test]
#[allow(non_snake_case)]
fn TestBuiltinAdapterRegistrySupportsCanonicalResolution() {
    let registry = AgentAdapterRegistry::builtin();

    assert_eq!(registry.default_agent_name(), "claude-code");
    assert_eq!(
        registry.normalise_agent_name("copilot").expect("alias"),
        "copilot"
    );
    assert_eq!(
        registry.normalise_agent_name("gemini").expect("alias"),
        "gemini"
    );
    assert_eq!(
        registry.normalise_agent_name("open-code").expect("alias"),
        "opencode"
    );

    let observations = registry.registration_observability();
    assert!(
        observations
            .iter()
            .all(|observation| observation.package_id == observation.adapter_id)
    );
    assert!(
        observations
            .iter()
            .all(|observation| observation.package_trust_model == "first-party-linked")
    );

    let package_reports = registry.discover_packages();
    assert_eq!(package_reports.len(), observations.len());
    assert!(
        package_reports
            .iter()
            .all(|report| report.metadata_version.value() == 1)
    );
    assert!(
        package_reports
            .iter()
            .all(|report| report.source.as_str() == "first-party-linked")
    );
    assert!(package_reports.iter().all(|report| matches!(
        report.status,
        super::super::adapters::AgentAdapterPackageDiscoveryStatus::Ready
    )));

    let families = registry.available_protocol_families();
    assert!(families.iter().any(|family| family == "json-event"));
    assert!(families.iter().any(|family| family == "jsonl-cli"));

    let profiles = registry.available_target_profiles();
    assert!(profiles.iter().any(|profile| profile == "claude-code"));
    assert!(profiles.iter().any(|profile| profile == "codex"));
    assert!(profiles.iter().any(|profile| profile == "cursor"));
    assert!(profiles.iter().any(|profile| profile == "opencode"));
    assert!(profiles.iter().any(|profile| profile == "copilot"));
    assert!(profiles.iter().any(|profile| profile == "gemini"));
}

use super::*;

#[test]
fn devql_extension_host_builds_capability_contexts_from_registered_owners() {
    let cfg = extension_runtime_cfg();

    let ingest_context = capability_ingest_context_for_ingester(
        &cfg,
        Some("abc123"),
        TEST_HARNESS_CAPABILITY_INGESTER_ID,
    )
    .expect("resolve test-harness ingester owner");
    assert_eq!(
        ingest_context.capability_pack_id,
        "test-harness-capability-pack"
    );
    assert_eq!(
        ingest_context.ingester_id,
        TEST_HARNESS_CAPABILITY_INGESTER_ID
    );
    assert_eq!(ingest_context.commit_sha.as_deref(), Some("abc123"));
}

#[test]
fn devql_language_adapter_lifecycle_summary_reports_builtins_and_readiness() {
    let cfg = extension_runtime_cfg();
    let lifecycle = collect_language_adapter_lifecycle(&cfg, "local-cli", false, false)
        .expect("collect language adapter lifecycle summary");

    let pack_ids = lifecycle
        .summary
        .packs
        .iter()
        .map(|pack| pack.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        pack_ids,
        vec![
            GO_LANGUAGE_PACK_ID,
            JAVA_LANGUAGE_PACK_ID,
            PYTHON_LANGUAGE_PACK_ID,
            RUST_LANGUAGE_PACK_ID,
            TS_JS_LANGUAGE_PACK_ID
        ]
    );
    assert!(
        lifecycle
            .readiness_reports
            .iter()
            .all(|report| report.ready),
        "built-in language adapters should report ready without pending migrations"
    );
}

#[test]
fn core_extension_host_registry_report_with_language_adapter_snapshot_includes_adapter_entries() {
    let cfg = extension_runtime_cfg();
    let lifecycle = collect_language_adapter_lifecycle(&cfg, "local-cli", false, false)
        .expect("collect language adapter lifecycle summary");
    let ext_host = crate::host::extension_host::CoreExtensionHost::with_builtins()
        .expect("bootstrap core extension host");
    let snapshot = ext_host
        .readiness_snapshot()
        .with_language_adapter_readiness(
            lifecycle
                .summary
                .packs
                .iter()
                .map(|pack| pack.id.clone())
                .collect(),
            lifecycle.readiness_reports,
        );
    let report = ext_host.registry_report_with_snapshot(snapshot);

    assert_eq!(
        report.language_adapter_pack_ids,
        vec![
            GO_LANGUAGE_PACK_ID.to_string(),
            JAVA_LANGUAGE_PACK_ID.to_string(),
            PYTHON_LANGUAGE_PACK_ID.to_string(),
            RUST_LANGUAGE_PACK_ID.to_string(),
            TS_JS_LANGUAGE_PACK_ID.to_string()
        ]
    );
    assert!(
        report
            .readiness
            .iter()
            .any(|entry| entry.family == "language-adapter-pack"),
        "language adapter readiness entries should be present in extension report"
    );
}

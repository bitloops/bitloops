use tempfile::TempDir;

use super::SqliteTestHarnessRepository;
use crate::capability_packs::test_harness::storage::TestHarnessRepository;
use crate::models::{TestArtefactCurrentRecord, TestDiscoveryRunRecord};
use crate::storage::init::init_database;

const REPO_ID: &str = "ruff-workspace";
const COMMIT_SHA: &str = "commit-workspace";
const SUITE_ARTEFACT_ID: &str = "test-artefact:suite:user-service";
const SCENARIO_ARTEFACT_ID: &str = "test-artefact:scenario:checks-email-domain";
const SUITE_ID: &str = "suite:user-service";
const SCENARIO_ID: &str = "scenario:checks-email-domain";

#[test]
fn load_test_scenarios_uses_test_domain_rows_only() {
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let db_path = temp_dir.path().join("workspace-tests.db");
    init_database(&db_path, false, "seed").expect("failed to initialize db");

    let mut repository = SqliteTestHarnessRepository::open_existing(&db_path).expect("open db");
    repository
        .replace_test_discovery(
            COMMIT_SHA,
            &test_artefacts(),
            &[],
            &test_discovery_run(),
            &[],
        )
        .expect("replace test discovery");

    let scenarios = repository
        .load_test_scenarios(COMMIT_SHA)
        .expect("load test scenarios");
    assert_eq!(scenarios.len(), 1);
    let scenario = &scenarios[0];
    assert_eq!(scenario.scenario_id, SCENARIO_ID);
    assert_eq!(scenario.path, "tests/user_service.rs");
    assert_eq!(scenario.suite_name, "UserService");
    assert_eq!(scenario.test_name, "checks_email_domain");
}

fn test_discovery_run() -> TestDiscoveryRunRecord {
    TestDiscoveryRunRecord {
        discovery_run_id: "discovery:user-service".to_string(),
        repo_id: REPO_ID.to_string(),
        commit_sha: COMMIT_SHA.to_string(),
        language: Some("rust".to_string()),
        started_at: "2026-03-24T00:00:00Z".to_string(),
        finished_at: Some("2026-03-24T00:00:01Z".to_string()),
        status: "complete".to_string(),
        enumeration_status: Some("static_only".to_string()),
        notes_json: None,
        stats_json: None,
    }
}

fn test_artefacts() -> Vec<TestArtefactCurrentRecord> {
    vec![
        TestArtefactCurrentRecord {
            artefact_id: SUITE_ARTEFACT_ID.to_string(),
            symbol_id: SUITE_ID.to_string(),
            repo_id: REPO_ID.to_string(),
            commit_sha: COMMIT_SHA.to_string(),
            blob_sha: "blob:test:user-service".to_string(),
            path: "tests/user_service.rs".to_string(),
            language: "rust".to_string(),
            canonical_kind: "test_suite".to_string(),
            language_kind: None,
            symbol_fqn: Some("tests/user_service.rs::UserService".to_string()),
            name: "UserService".to_string(),
            parent_artefact_id: None,
            parent_symbol_id: None,
            start_line: 1,
            end_line: 30,
            start_byte: Some(0),
            end_byte: Some(400),
            signature: None,
            modifiers: "[]".to_string(),
            docstring: None,
            content_hash: None,
            discovery_source: "hybrid_enumeration".to_string(),
            revision_kind: "commit".to_string(),
            revision_id: COMMIT_SHA.to_string(),
        },
        TestArtefactCurrentRecord {
            artefact_id: SCENARIO_ARTEFACT_ID.to_string(),
            symbol_id: SCENARIO_ID.to_string(),
            repo_id: REPO_ID.to_string(),
            commit_sha: COMMIT_SHA.to_string(),
            blob_sha: "blob:test:user-service".to_string(),
            path: "tests/user_service.rs".to_string(),
            language: "rust".to_string(),
            canonical_kind: "test_scenario".to_string(),
            language_kind: None,
            symbol_fqn: Some("tests/user_service.rs::checks_email_domain".to_string()),
            name: "checks_email_domain".to_string(),
            parent_artefact_id: Some(SUITE_ARTEFACT_ID.to_string()),
            parent_symbol_id: Some(SUITE_ID.to_string()),
            start_line: 8,
            end_line: 14,
            start_byte: Some(80),
            end_byte: Some(220),
            signature: Some("fn checks_email_domain()".to_string()),
            modifiers: "[]".to_string(),
            docstring: None,
            content_hash: None,
            discovery_source: "hybrid_enumeration".to_string(),
            revision_kind: "commit".to_string(),
            revision_id: COMMIT_SHA.to_string(),
        },
    ]
}

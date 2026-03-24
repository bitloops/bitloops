use std::path::Path;

use rusqlite::Connection;
use tempfile::TempDir;

use super::SqliteTestHarnessRepository;
use crate::capability_packs::test_harness::storage::TestHarnessRepository;
use crate::models::{
    CoverageCaptureRecord, CoverageFormat, CoverageHitRecord, TestArtefactCurrentRecord,
    TestArtefactEdgeCurrentRecord, TestDiscoveryDiagnosticRecord, TestDiscoveryRunRecord,
    TestRunRecord,
};
use crate::models::ScopeKind;
use crate::storage::init::init_database;

const REPO_ID: &str = "ruff-workspace";
const COMMIT_SHA: &str = "commit-workspace";
const FILE_USER: &str = "src/user/service.rs";
const PRODUCTION_SYMBOL_ID: &str = "sym:function:create_user";
const SUITE_ARTEFACT_ID: &str = "test-artefact:suite:user-service";
const SCENARIO_ARTEFACT_ID: &str = "test-artefact:scenario:checks-email-domain";
const SUITE_ID: &str = "suite:user-service";
const SCENARIO_ID: &str = "scenario:checks-email-domain";
const TEST_LINK_ID: &str = "link:checks-email-domain:create-user";
const RUN_ID: &str = "run:checks-email-domain";
const CAPTURE_ID: &str = "capture:checks-email-domain";

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

#[test]
fn replace_test_discovery_clears_stale_runs_coverage_and_classifications() {
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let db_path = temp_dir.path().join("workspace-cleanup.db");
    init_database(&db_path, false, "seed").expect("failed to initialize db");

    let mut repository = SqliteTestHarnessRepository::open_existing(&db_path).expect("open db");
    repository
        .replace_test_discovery(
            COMMIT_SHA,
            &stale_test_artefacts(),
            &stale_test_edges(),
            &stale_discovery_run(),
            &[stale_diagnostic()],
        )
        .expect("replace stale test discovery");
    repository
        .replace_test_runs(COMMIT_SHA, &[test_run_record()])
        .expect("replace stale test runs");
    repository
        .insert_coverage_capture(&coverage_capture_record())
        .expect("insert stale coverage capture");
    repository
        .insert_coverage_hits(&coverage_hits())
        .expect("insert stale coverage hits");
    assert_eq!(
        repository
            .rebuild_classifications_from_coverage(COMMIT_SHA)
            .expect("rebuild classifications"),
        1
    );

    assert_eq!(table_count(&db_path, "test_runs"), 1);
    assert_eq!(table_count(&db_path, "coverage_captures"), 1);
    assert_eq!(table_count(&db_path, "coverage_hits"), 2);
    assert_eq!(table_count(&db_path, "test_classifications"), 1);

    repository
        .replace_test_discovery(
            COMMIT_SHA,
            &test_artefacts(),
            &test_edges(),
            &test_discovery_run(),
            &[],
        )
        .expect("replace fresh test discovery");

    assert_eq!(table_count(&db_path, "test_runs"), 0);
    assert_eq!(table_count(&db_path, "coverage_captures"), 0);
    assert_eq!(table_count(&db_path, "coverage_hits"), 0);
    assert_eq!(table_count(&db_path, "test_classifications"), 0);
    assert_eq!(table_count(&db_path, "test_artefacts_current"), 2);
    assert_eq!(table_count(&db_path, "test_artefact_edges_current"), 1);

    let scenarios = repository
        .load_test_scenarios(COMMIT_SHA)
        .expect("load fresh test scenarios");
    assert_eq!(scenarios.len(), 1);
    assert_eq!(scenarios[0].scenario_id, SCENARIO_ID);
}

fn table_count(db_path: &Path, table: &str) -> i64 {
    let conn = Connection::open(db_path).expect("open sqlite connection");
    let query = format!("SELECT COUNT(*) FROM {table}");
    conn.query_row(&query, [], |row| row.get(0))
        .expect("count rows")
}

fn stale_test_artefacts() -> Vec<TestArtefactCurrentRecord> {
    vec![
        TestArtefactCurrentRecord {
            artefact_id: "test-artefact:suite:stale".to_string(),
            symbol_id: "suite:stale".to_string(),
            repo_id: REPO_ID.to_string(),
            commit_sha: COMMIT_SHA.to_string(),
            blob_sha: "blob:test:stale".to_string(),
            path: "tests/stale.rs".to_string(),
            language: "rust".to_string(),
            canonical_kind: "test_suite".to_string(),
            language_kind: None,
            symbol_fqn: Some("tests/stale.rs::StaleSuite".to_string()),
            name: "StaleSuite".to_string(),
            parent_artefact_id: None,
            parent_symbol_id: None,
            start_line: 1,
            end_line: 10,
            start_byte: Some(0),
            end_byte: Some(100),
            signature: None,
            modifiers: "[]".to_string(),
            docstring: None,
            content_hash: None,
            discovery_source: "static_analysis".to_string(),
            revision_kind: "commit".to_string(),
            revision_id: COMMIT_SHA.to_string(),
        },
        TestArtefactCurrentRecord {
            artefact_id: "test-artefact:scenario:stale".to_string(),
            symbol_id: "scenario:stale".to_string(),
            repo_id: REPO_ID.to_string(),
            commit_sha: COMMIT_SHA.to_string(),
            blob_sha: "blob:test:stale".to_string(),
            path: "tests/stale.rs".to_string(),
            language: "rust".to_string(),
            canonical_kind: "test_scenario".to_string(),
            language_kind: None,
            symbol_fqn: Some("tests/stale.rs::stale_test".to_string()),
            name: "stale_test".to_string(),
            parent_artefact_id: Some("test-artefact:suite:stale".to_string()),
            parent_symbol_id: Some("suite:stale".to_string()),
            start_line: 3,
            end_line: 5,
            start_byte: Some(20),
            end_byte: Some(60),
            signature: Some("fn stale_test()".to_string()),
            modifiers: "[]".to_string(),
            docstring: None,
            content_hash: None,
            discovery_source: "static_analysis".to_string(),
            revision_kind: "commit".to_string(),
            revision_id: COMMIT_SHA.to_string(),
        },
    ]
}

fn stale_test_edges() -> Vec<TestArtefactEdgeCurrentRecord> {
    vec![TestArtefactEdgeCurrentRecord {
        edge_id: "link:stale".to_string(),
        repo_id: REPO_ID.to_string(),
        commit_sha: COMMIT_SHA.to_string(),
        blob_sha: "blob:test:stale".to_string(),
        path: "tests/stale.rs".to_string(),
        from_artefact_id: "test-artefact:scenario:stale".to_string(),
        from_symbol_id: "scenario:stale".to_string(),
        to_artefact_id: Some("artefact:function:create_user".to_string()),
        to_symbol_id: Some(PRODUCTION_SYMBOL_ID.to_string()),
        to_symbol_ref: None,
        edge_kind: "tests".to_string(),
        language: "rust".to_string(),
        start_line: Some(3),
        end_line: Some(5),
        metadata: "{\"imports\":[\"create_user\"]}".to_string(),
        revision_kind: "commit".to_string(),
        revision_id: COMMIT_SHA.to_string(),
    }]
}

fn stale_discovery_run() -> TestDiscoveryRunRecord {
    TestDiscoveryRunRecord {
        discovery_run_id: "discovery:stale".to_string(),
        repo_id: REPO_ID.to_string(),
        commit_sha: COMMIT_SHA.to_string(),
        language: Some("rust".to_string()),
        started_at: "2026-03-24T00:00:00Z".to_string(),
        finished_at: Some("2026-03-24T00:00:01Z".to_string()),
        status: "complete".to_string(),
        enumeration_status: Some("static_only".to_string()),
        notes_json: Some("{\"note\":\"stale\"}".to_string()),
        stats_json: Some("{\"files\":1}".to_string()),
    }
}

fn stale_diagnostic() -> TestDiscoveryDiagnosticRecord {
    TestDiscoveryDiagnosticRecord {
        diagnostic_id: "diag:stale".to_string(),
        discovery_run_id: "discovery:stale".to_string(),
        repo_id: REPO_ID.to_string(),
        commit_sha: COMMIT_SHA.to_string(),
        path: Some("tests/stale.rs".to_string()),
        line: Some(1),
        severity: "warning".to_string(),
        code: "stale".to_string(),
        message: "stale diagnostic".to_string(),
        metadata_json: Some("{\"stale\":true}".to_string()),
    }
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

fn test_edges() -> Vec<TestArtefactEdgeCurrentRecord> {
    vec![TestArtefactEdgeCurrentRecord {
        edge_id: TEST_LINK_ID.to_string(),
        repo_id: REPO_ID.to_string(),
        commit_sha: COMMIT_SHA.to_string(),
        blob_sha: "blob:test:user-service".to_string(),
        path: "tests/user_service.rs".to_string(),
        from_artefact_id: SCENARIO_ARTEFACT_ID.to_string(),
        from_symbol_id: SCENARIO_ID.to_string(),
        to_artefact_id: Some("artefact:function:create_user".to_string()),
        to_symbol_id: Some(PRODUCTION_SYMBOL_ID.to_string()),
        to_symbol_ref: None,
        edge_kind: "tests".to_string(),
        language: "rust".to_string(),
        start_line: Some(8),
        end_line: Some(14),
        metadata: "{\"calls\":[\"create_user\"]}".to_string(),
        revision_kind: "commit".to_string(),
        revision_id: COMMIT_SHA.to_string(),
    }]
}

fn test_run_record() -> TestRunRecord {
    TestRunRecord {
        run_id: RUN_ID.to_string(),
        repo_id: REPO_ID.to_string(),
        commit_sha: COMMIT_SHA.to_string(),
        test_symbol_id: SCENARIO_ID.to_string(),
        status: "failed".to_string(),
        duration_ms: Some(73),
        ran_at: "2026-03-24T00:00:02Z".to_string(),
    }
}

fn coverage_capture_record() -> CoverageCaptureRecord {
    CoverageCaptureRecord {
        capture_id: CAPTURE_ID.to_string(),
        repo_id: REPO_ID.to_string(),
        commit_sha: COMMIT_SHA.to_string(),
        tool: "llvm-cov".to_string(),
        format: CoverageFormat::Lcov,
        scope_kind: ScopeKind::TestScenario,
        subject_test_symbol_id: Some(SCENARIO_ID.to_string()),
        line_truth: true,
        branch_truth: false,
        captured_at: "2026-03-24T00:00:03Z".to_string(),
        status: "complete".to_string(),
        metadata_json: Some("{\"runner\":\"cargo test\"}".to_string()),
    }
}

fn coverage_hits() -> Vec<CoverageHitRecord> {
    vec![
        CoverageHitRecord {
            capture_id: CAPTURE_ID.to_string(),
            production_symbol_id: PRODUCTION_SYMBOL_ID.to_string(),
            file_path: FILE_USER.to_string(),
            line: 10,
            branch_id: -1,
            covered: true,
            hit_count: 3,
        },
        CoverageHitRecord {
            capture_id: CAPTURE_ID.to_string(),
            production_symbol_id: PRODUCTION_SYMBOL_ID.to_string(),
            file_path: FILE_USER.to_string(),
            line: 11,
            branch_id: -1,
            covered: false,
            hit_count: 0,
        },
    ]
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

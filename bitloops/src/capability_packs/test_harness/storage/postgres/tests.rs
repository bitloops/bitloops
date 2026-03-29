use std::env;
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, anyhow, bail};
use tempfile::TempDir;

use super::PostgresTestHarnessRepository;
use crate::capability_packs::test_harness::storage::{
    TestHarnessQueryRepository, TestHarnessRepository,
};
use crate::models::{
    CoverageCaptureRecord, CoverageFormat, CoverageHitRecord, ScopeKind, TestArtefactCurrentRecord,
    TestArtefactEdgeCurrentRecord, TestDiscoveryDiagnosticRecord, TestDiscoveryRunRecord,
    TestRunRecord,
};

#[allow(clippy::items_after_test_module)]
mod devql_schema {
    include!("../../../../host/devql/ingestion/schema/relational_postgres_schema.rs");

    pub(crate) fn sql() -> &'static str {
        postgres_schema_sql()
    }
}

const REPO_ID: &str = "repo-postgres-test-harness";
const COMMIT_SHA: &str = "commit-postgres-test-harness";
const FILE_USER: &str = "src/services/user_service.rs";
const FILE_EMAIL: &str = "src/services/email.rs";
const ARTEFACT_CREATE_USER: &str = "artefact:function:create_user";
const SYMBOL_CREATE_USER: &str = "symbol:function:create_user";
const SUITE_ID: &str = "suite:user-service";
const SCENARIO_ID: &str = "scenario:checks-email-domain";
const TEST_LINK_ID: &str = "link:checks-email-domain:create-user";
const SUITE_ARTEFACT_ID: &str = "test-artefact:suite:user-service";
const SCENARIO_ARTEFACT_ID: &str = "test-artefact:scenario:checks-email-domain";
const DISCOVERY_RUN_ID: &str = "discovery:user-service";
const DIAGNOSTIC_ID: &str = "diag:user-service";
const RUN_ID: &str = "run:checks-email-domain";
const CAPTURE_ID: &str = "capture:checks-email-domain";

#[test]
fn postgres_repository_round_trips_test_harness_flow() -> Result<()> {
    let Some(postgres) = TempPostgres::start()? else {
        eprintln!(
            "skipping Postgres test-harness coverage test; local Postgres binaries not found"
        );
        return Ok(());
    };
    let mut repository = PostgresTestHarnessRepository::connect(postgres.dsn())?;
    initialise_postgres_repository(&repository)?;
    seed_production_state(&repository)?;

    repository.replace_test_discovery(
        COMMIT_SHA,
        &stale_test_artefacts(),
        &stale_test_edges(),
        &stale_discovery_run(),
        &[stale_diagnostic()],
    )?;
    repository.replace_test_discovery(
        COMMIT_SHA,
        &test_artefacts(),
        &test_edges(),
        &discovery_run_record(),
        &[diagnostic_record()],
    )?;

    assert_eq!(table_count(&repository, "test_artefacts_current")?, 2);
    assert_eq!(table_count(&repository, "test_artefact_edges_current")?, 1);
    assert_eq!(table_count(&repository, "test_discovery_runs")?, 1);
    assert_eq!(table_count(&repository, "test_discovery_diagnostics")?, 1);

    let scenarios = repository.load_test_scenarios(COMMIT_SHA)?;
    assert_eq!(scenarios.len(), 1);
    assert_eq!(scenarios[0].scenario_id, SCENARIO_ID);
    assert_eq!(scenarios[0].suite_name, "UserService");
    assert_eq!(scenarios[0].test_name, "checks_email_domain");

    let fan_out = repository.load_linked_fan_out_by_test(COMMIT_SHA)?;
    assert_eq!(fan_out.get(SCENARIO_ID), Some(&1));
    assert!(!repository.coverage_exists_for_commit(COMMIT_SHA)?);
    assert!(
        repository
            .load_coverage_summary(COMMIT_SHA, SYMBOL_CREATE_USER)?
            .is_none()
    );
    assert!(
        repository
            .load_latest_test_run(COMMIT_SHA, SCENARIO_ID)?
            .is_none()
    );

    repository.replace_test_runs(COMMIT_SHA, &[test_run_record()])?;
    let latest_run = repository
        .load_latest_test_run(COMMIT_SHA, SCENARIO_ID)?
        .expect("latest run should exist");
    assert_eq!(latest_run.status, "failed");
    assert_eq!(latest_run.duration_ms, Some(73));

    repository.insert_coverage_capture(&coverage_capture_record())?;
    repository.insert_coverage_hits(&coverage_hits())?;
    assert!(repository.coverage_exists_for_commit(COMMIT_SHA)?);

    let pair_stats =
        repository.load_coverage_pair_stats(COMMIT_SHA, SCENARIO_ID, SYMBOL_CREATE_USER)?;
    assert_eq!(pair_stats.total_rows, 4);
    assert_eq!(pair_stats.covered_rows, 2);

    let classifications = repository.rebuild_classifications_from_coverage(COMMIT_SHA)?;
    assert_eq!(classifications, 1);
    assert_eq!(table_count(&repository, "test_classifications")?, 1);

    let covering_tests = repository.load_covering_tests(COMMIT_SHA, SYMBOL_CREATE_USER)?;
    assert_eq!(covering_tests.len(), 1);
    assert_eq!(covering_tests[0].test_id, SCENARIO_ID);
    assert_eq!(covering_tests[0].suite_name.as_deref(), Some("UserService"));
    assert_eq!(covering_tests[0].classification.as_deref(), Some("unit"));
    assert_eq!(covering_tests[0].fan_out, Some(2));

    let summary = repository
        .load_coverage_summary(COMMIT_SHA, SYMBOL_CREATE_USER)?
        .expect("coverage summary should exist");
    assert_eq!(summary.line_total, 2);
    assert_eq!(summary.line_covered, 1);
    assert_eq!(summary.branch_total, 2);
    assert_eq!(summary.branch_covered, 1);
    assert_eq!(summary.branches.len(), 2);
    assert_eq!(
        summary
            .branches
            .iter()
            .filter(|branch| branch.covered)
            .count(),
        1
    );

    Ok(())
}

#[test]
fn postgres_repository_replace_test_discovery_clears_stale_runs_coverage_and_classifications()
-> Result<()> {
    let Some(postgres) = TempPostgres::start()? else {
        eprintln!("skipping Postgres test-harness test; local Postgres binaries not found");
        return Ok(());
    };

    let mut repository = PostgresTestHarnessRepository::connect(postgres.dsn())?;
    initialise_postgres_repository(&repository)?;
    seed_production_state(&repository)?;

    repository.replace_test_discovery(
        COMMIT_SHA,
        &stale_test_artefacts(),
        &stale_test_edges(),
        &stale_discovery_run(),
        &[stale_diagnostic()],
    )?;
    repository.replace_test_runs(COMMIT_SHA, &[test_run_record()])?;
    repository.insert_coverage_capture(&coverage_capture_record())?;
    repository.insert_coverage_hits(&coverage_hits())?;
    assert_eq!(
        repository.rebuild_classifications_from_coverage(COMMIT_SHA)?,
        1
    );

    assert_eq!(table_count(&repository, "test_runs")?, 1);
    assert_eq!(table_count(&repository, "coverage_captures")?, 1);
    assert_eq!(table_count(&repository, "coverage_hits")?, 6);
    assert_eq!(table_count(&repository, "test_classifications")?, 1);

    repository.replace_test_discovery(
        COMMIT_SHA,
        &test_artefacts(),
        &test_edges(),
        &discovery_run_record(),
        &[diagnostic_record()],
    )?;

    assert_eq!(table_count(&repository, "test_runs")?, 0);
    assert_eq!(table_count(&repository, "coverage_captures")?, 0);
    assert_eq!(table_count(&repository, "coverage_hits")?, 0);
    assert_eq!(table_count(&repository, "test_classifications")?, 0);
    assert_eq!(table_count(&repository, "test_artefacts_current")?, 2);
    assert_eq!(table_count(&repository, "test_artefact_edges_current")?, 1);

    let scenarios = repository.load_test_scenarios(COMMIT_SHA)?;
    assert_eq!(scenarios.len(), 1);
    assert_eq!(scenarios[0].scenario_id, SCENARIO_ID);
    Ok(())
}

#[test]
fn postgres_repository_insert_coverage_diagnostics_empty_slice_is_noop() -> Result<()> {
    let Some(postgres) = TempPostgres::start()? else {
        eprintln!("skipping Postgres test-harness test; local Postgres binaries not found");
        return Ok(());
    };

    let mut repository = PostgresTestHarnessRepository::connect(postgres.dsn())?;
    initialise_postgres_repository(&repository)?;
    seed_production_state(&repository)?;

    repository.insert_coverage_diagnostics(&[])?;
    assert_eq!(table_count(&repository, "coverage_diagnostics")?, 0);
    Ok(())
}

#[test]
fn postgres_repository_rebuild_classifications_returns_zero_without_covered_hits() -> Result<()> {
    let Some(postgres) = TempPostgres::start()? else {
        eprintln!("skipping Postgres test-harness test; local Postgres binaries not found");
        return Ok(());
    };

    let mut repository = PostgresTestHarnessRepository::connect(postgres.dsn())?;
    initialise_postgres_repository(&repository)?;
    seed_production_state(&repository)?;

    let inserted = repository.rebuild_classifications_from_coverage(COMMIT_SHA)?;
    assert_eq!(inserted, 0);
    assert_eq!(table_count(&repository, "test_classifications")?, 0);
    Ok(())
}

fn initialise_postgres_repository(repository: &PostgresTestHarnessRepository) -> Result<()> {
    repository.postgres.execute_batch(devql_schema::sql())?;
    repository.initialise_schema()?;
    Ok(())
}

fn seed_production_state(repository: &PostgresTestHarnessRepository) -> Result<()> {
    repository
        .postgres
        .execute_batch(
            r#"
INSERT INTO repositories (repo_id, provider, organization, name, default_branch)
VALUES ('repo-postgres-test-harness', 'local', 'local', 'repo', 'main');

INSERT INTO commits (commit_sha, repo_id, author_name, author_email, commit_message, committed_at)
VALUES ('commit-postgres-test-harness', 'repo-postgres-test-harness', 'Markos', 'markos@example.com', 'seed', '2026-03-19T12:00:00Z');

INSERT INTO file_state (repo_id, commit_sha, path, blob_sha)
VALUES
  ('repo-postgres-test-harness', 'commit-postgres-test-harness', 'src/services/user_service.rs', 'blob-user'),
  ('repo-postgres-test-harness', 'commit-postgres-test-harness', 'src/services/email.rs', 'blob-email');

INSERT INTO current_file_state (repo_id, path, commit_sha, blob_sha, committed_at)
VALUES
  ('repo-postgres-test-harness', 'src/services/user_service.rs', 'commit-postgres-test-harness', 'blob-user', '2026-03-19T12:00:00Z'),
  ('repo-postgres-test-harness', 'src/services/email.rs', 'commit-postgres-test-harness', 'blob-email', '2026-03-19T12:00:00Z');

INSERT INTO artefacts (
  artefact_id, symbol_id, repo_id, blob_sha, path, language, canonical_kind, language_kind,
  symbol_fqn, parent_artefact_id, start_line, end_line, start_byte, end_byte, signature,
  modifiers, docstring, content_hash
) VALUES
  ('artefact:file:user_service', 'symbol:file:user_service', 'repo-postgres-test-harness', 'blob-user', 'src/services/user_service.rs', 'rust', 'file', 'source_file', 'src/services/user_service.rs', NULL, 1, 40, 0, 800, NULL, '[]'::jsonb, NULL, 'hash-file-user'),
  ('artefact:file:email', 'symbol:file:email', 'repo-postgres-test-harness', 'blob-email', 'src/services/email.rs', 'rust', 'file', 'source_file', 'src/services/email.rs', NULL, 1, 30, 0, 600, NULL, '[]'::jsonb, NULL, 'hash-file-email'),
  ('artefact:struct:user', 'symbol:struct:user', 'repo-postgres-test-harness', 'blob-user', 'src/services/user_service.rs', 'rust', NULL, 'Struct', 'src/services/user_service.rs::User', 'artefact:file:user_service', 3, 8, 24, 96, NULL, '[]'::jsonb, NULL, 'hash-user-struct'),
  ('artefact:function:create_user', 'symbol:function:create_user', 'repo-postgres-test-harness', 'blob-user', 'src/services/user_service.rs', 'rust', 'function', 'function_item', 'src/services/user_service.rs::create_user', 'artefact:file:user_service', 10, 20, 100, 350, 'pub fn create_user(name: &str) -> User', '[]'::jsonb, NULL, 'hash-create-user'),
  ('artefact:function:normalize_email', 'symbol:function:normalize_email', 'repo-postgres-test-harness', 'blob-email', 'src/services/email.rs', 'rust', 'function', 'function_item', 'src/services/email.rs::normalize_email', 'artefact:file:email', 5, 12, 50, 200, 'pub fn normalize_email(raw: &str) -> String', '[]'::jsonb, NULL, 'hash-normalize-email');

INSERT INTO artefacts_current (
  repo_id, symbol_id, artefact_id, commit_sha, blob_sha, path, language, canonical_kind,
  language_kind, symbol_fqn, parent_symbol_id, parent_artefact_id, start_line, end_line,
  start_byte, end_byte, signature, modifiers, docstring, content_hash
) VALUES
  ('repo-postgres-test-harness', 'symbol:file:user_service', 'artefact:file:user_service', 'commit-postgres-test-harness', 'blob-user', 'src/services/user_service.rs', 'rust', 'file', 'source_file', 'src/services/user_service.rs', NULL, NULL, 1, 40, 0, 800, NULL, '[]'::jsonb, NULL, 'hash-file-user'),
  ('repo-postgres-test-harness', 'symbol:file:email', 'artefact:file:email', 'commit-postgres-test-harness', 'blob-email', 'src/services/email.rs', 'rust', 'file', 'source_file', 'src/services/email.rs', NULL, NULL, 1, 30, 0, 600, NULL, '[]'::jsonb, NULL, 'hash-file-email'),
  ('repo-postgres-test-harness', 'symbol:struct:user', 'artefact:struct:user', 'commit-postgres-test-harness', 'blob-user', 'src/services/user_service.rs', 'rust', NULL, 'Struct', 'src/services/user_service.rs::User', 'symbol:file:user_service', 'artefact:file:user_service', 3, 8, 24, 96, NULL, '[]'::jsonb, NULL, 'hash-user-struct'),
  ('repo-postgres-test-harness', 'symbol:function:create_user', 'artefact:function:create_user', 'commit-postgres-test-harness', 'blob-user', 'src/services/user_service.rs', 'rust', 'function', 'function_item', 'src/services/user_service.rs::create_user', 'symbol:file:user_service', 'artefact:file:user_service', 10, 20, 100, 350, 'pub fn create_user(name: &str) -> User', '[]'::jsonb, NULL, 'hash-create-user'),
  ('repo-postgres-test-harness', 'symbol:function:normalize_email', 'artefact:function:normalize_email', 'commit-postgres-test-harness', 'blob-email', 'src/services/email.rs', 'rust', 'function', 'function_item', 'src/services/email.rs::normalize_email', 'symbol:file:email', 'artefact:file:email', 5, 12, 50, 200, 'pub fn normalize_email(raw: &str) -> String', '[]'::jsonb, NULL, 'hash-normalize-email');
"#,
        )
        .context("seeding production state")?;
    Ok(())
}

fn table_count(repository: &PostgresTestHarnessRepository, table: &str) -> Result<i64> {
    let table_name = table.to_string();
    let query = format!("SELECT COUNT(*) FROM {table}");
    repository.postgres.with_client(move |client| {
        Box::pin(async move {
            let row = client
                .query_one(&query, &[])
                .await
                .with_context(|| format!("counting rows in {table_name}"))?;
            let count: i64 = row.get(0);
            Ok(count)
        })
    })
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
        to_artefact_id: Some(ARTEFACT_CREATE_USER.to_string()),
        to_symbol_id: Some(SYMBOL_CREATE_USER.to_string()),
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
        started_at: "2026-03-19T12:01:00Z".to_string(),
        finished_at: Some("2026-03-19T12:01:01Z".to_string()),
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

fn test_edges() -> Vec<TestArtefactEdgeCurrentRecord> {
    vec![TestArtefactEdgeCurrentRecord {
        edge_id: TEST_LINK_ID.to_string(),
        repo_id: REPO_ID.to_string(),
        commit_sha: COMMIT_SHA.to_string(),
        blob_sha: "blob:test:user-service".to_string(),
        path: "tests/user_service.rs".to_string(),
        from_artefact_id: SCENARIO_ARTEFACT_ID.to_string(),
        from_symbol_id: SCENARIO_ID.to_string(),
        to_artefact_id: Some(ARTEFACT_CREATE_USER.to_string()),
        to_symbol_id: Some(SYMBOL_CREATE_USER.to_string()),
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

fn discovery_run_record() -> TestDiscoveryRunRecord {
    TestDiscoveryRunRecord {
        discovery_run_id: DISCOVERY_RUN_ID.to_string(),
        repo_id: REPO_ID.to_string(),
        commit_sha: COMMIT_SHA.to_string(),
        language: Some("rust".to_string()),
        started_at: "2026-03-19T12:02:00Z".to_string(),
        finished_at: Some("2026-03-19T12:02:03Z".to_string()),
        status: "complete".to_string(),
        enumeration_status: Some("hybrid_full".to_string()),
        notes_json: Some("{\"mode\":\"hybrid\"}".to_string()),
        stats_json: Some("{\"files\":2,\"scenarios\":1}".to_string()),
    }
}

fn diagnostic_record() -> TestDiscoveryDiagnosticRecord {
    TestDiscoveryDiagnosticRecord {
        diagnostic_id: DIAGNOSTIC_ID.to_string(),
        discovery_run_id: DISCOVERY_RUN_ID.to_string(),
        repo_id: REPO_ID.to_string(),
        commit_sha: COMMIT_SHA.to_string(),
        path: Some("tests/user_service.rs".to_string()),
        line: Some(8),
        severity: "info".to_string(),
        code: "enumeration".to_string(),
        message: "hybrid enumeration used cargo-backed discovery".to_string(),
        metadata_json: Some("{\"enumerated\":1}".to_string()),
    }
}

fn test_run_record() -> TestRunRecord {
    TestRunRecord {
        run_id: RUN_ID.to_string(),
        repo_id: REPO_ID.to_string(),
        commit_sha: COMMIT_SHA.to_string(),
        test_symbol_id: SCENARIO_ID.to_string(),
        status: "failed".to_string(),
        duration_ms: Some(73),
        ran_at: "2026-03-19T12:03:00Z".to_string(),
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
        branch_truth: true,
        captured_at: "2026-03-19T12:04:00Z".to_string(),
        status: "complete".to_string(),
        metadata_json: Some("{\"runner\":\"cargo test\"}".to_string()),
    }
}

fn coverage_hits() -> Vec<CoverageHitRecord> {
    vec![
        CoverageHitRecord {
            capture_id: CAPTURE_ID.to_string(),
            production_symbol_id: SYMBOL_CREATE_USER.to_string(),
            file_path: FILE_USER.to_string(),
            line: 10,
            branch_id: -1,
            covered: true,
            hit_count: 3,
        },
        CoverageHitRecord {
            capture_id: CAPTURE_ID.to_string(),
            production_symbol_id: SYMBOL_CREATE_USER.to_string(),
            file_path: FILE_USER.to_string(),
            line: 11,
            branch_id: -1,
            covered: false,
            hit_count: 0,
        },
        CoverageHitRecord {
            capture_id: CAPTURE_ID.to_string(),
            production_symbol_id: SYMBOL_CREATE_USER.to_string(),
            file_path: FILE_USER.to_string(),
            line: 12,
            branch_id: 0,
            covered: true,
            hit_count: 1,
        },
        CoverageHitRecord {
            capture_id: CAPTURE_ID.to_string(),
            production_symbol_id: SYMBOL_CREATE_USER.to_string(),
            file_path: FILE_USER.to_string(),
            line: 12,
            branch_id: 1,
            covered: false,
            hit_count: 0,
        },
        CoverageHitRecord {
            capture_id: CAPTURE_ID.to_string(),
            production_symbol_id: "symbol:function:normalize_email".to_string(),
            file_path: FILE_EMAIL.to_string(),
            line: 5,
            branch_id: -1,
            covered: true,
            hit_count: 2,
        },
        CoverageHitRecord {
            capture_id: CAPTURE_ID.to_string(),
            production_symbol_id: "symbol:function:normalize_email".to_string(),
            file_path: FILE_EMAIL.to_string(),
            line: 6,
            branch_id: -1,
            covered: false,
            hit_count: 0,
        },
    ]
}

struct TempPostgres {
    _root: TempDir,
    data_dir: PathBuf,
    socket_dir: PathBuf,
    pg_ctl_path: PathBuf,
    dsn: String,
}

impl TempPostgres {
    fn start() -> Result<Option<Self>> {
        let Some(initdb_path) = find_postgres_binary("initdb") else {
            return Ok(None);
        };
        let Some(pg_ctl_path) = find_postgres_binary("pg_ctl") else {
            return Ok(None);
        };

        // Retry up to 3 times: free_port() has a race window between dropping the
        // listener and pg_ctl binding the port — under parallel test load another
        // process can steal the port and cause pg_ctl to fail.
        for _ in 0..3 {
            match Self::try_start(&initdb_path, &pg_ctl_path) {
                Ok(pg) => return Ok(Some(pg)),
                Err(e) => eprintln!("TempPostgres startup attempt failed: {e:#}"),
            }
        }
        eprintln!("skipping Postgres test: all startup attempts failed under parallel load");
        Ok(None)
    }

    fn try_start(initdb_path: &Path, pg_ctl_path: &Path) -> Result<Self> {
        let root = TempDir::new().context("creating temporary postgres root")?;
        let data_dir = root.path().join("data");
        let socket_dir = root.path().join("socket");
        fs::create_dir_all(&socket_dir).context("creating postgres socket directory")?;

        run_command(
            Command::new(initdb_path).args([
                "-D",
                data_dir
                    .to_str()
                    .ok_or_else(|| anyhow!("postgres data path is not valid UTF-8"))?,
                "-A",
                "trust",
                "-U",
                "postgres",
                "--no-locale",
            ]),
            "initdb",
        )?;

        let port = free_port()?;
        let postgres_options = format!(
            "-k {} -p {} -F -c listen_addresses=''",
            socket_dir.display(),
            port
        );
        run_status_command(
            Command::new(pg_ctl_path).args([
                "-D",
                data_dir
                    .to_str()
                    .ok_or_else(|| anyhow!("postgres data path is not valid UTF-8"))?,
                "-o",
                &postgres_options,
                "-w",
                "start",
            ]),
            "pg_ctl start",
        )?;

        let dsn = format!(
            "host={} port={} user=postgres dbname=postgres",
            socket_dir.display(),
            port
        );

        Ok(Self {
            _root: root,
            data_dir,
            socket_dir,
            pg_ctl_path: pg_ctl_path.to_path_buf(),
            dsn,
        })
    }

    fn dsn(&self) -> &str {
        &self.dsn
    }
}

impl Drop for TempPostgres {
    fn drop(&mut self) {
        let _ = Command::new(&self.pg_ctl_path)
            .args([
                "-D",
                self.data_dir.to_string_lossy().as_ref(),
                "-o",
                &format!(
                    "-k {} -c listen_addresses=''",
                    self.socket_dir.to_string_lossy()
                ),
                "-m",
                "immediate",
                "stop",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn find_postgres_binary(name: &str) -> Option<PathBuf> {
    for prefix in [
        "/opt/homebrew/opt/postgresql@16/bin",
        "/opt/homebrew/opt/postgresql@15/bin",
        "/opt/homebrew/opt/postgresql@14/bin",
        "/usr/local/opt/postgresql@16/bin",
        "/usr/local/opt/postgresql@15/bin",
        "/usr/local/opt/postgresql@14/bin",
        "/opt/homebrew/bin",
        "/usr/local/bin",
    ] {
        let candidate = Path::new(prefix).join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

fn run_command(command: &mut Command, label: &str) -> Result<()> {
    let output = command
        .output()
        .with_context(|| format!("running {label}"))?;
    if output.status.success() {
        return Ok(());
    }

    bail!(
        "{label} failed with status {}:\nstdout:\n{}\nstderr:\n{}",
        output
            .status
            .code()
            .map_or_else(|| "signal".to_string(), |code| code.to_string()),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_status_command(command: &mut Command, label: &str) -> Result<()> {
    let status = command
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .with_context(|| format!("running {label}"))?;
    if status.success() {
        return Ok(());
    }

    bail!(
        "{label} failed with status {}",
        status
            .code()
            .map_or_else(|| "signal".to_string(), |code| code.to_string())
    );
}

fn free_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").context("binding an ephemeral tcp port")?;
    let port = listener
        .local_addr()
        .context("reading ephemeral tcp port")?
        .port();
    drop(listener);
    Ok(port)
}

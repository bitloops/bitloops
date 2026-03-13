use std::path::{Path, PathBuf};

use crate::support::cli::{project_root, run_cargo_in_dir_or_panic, run_testlens_or_panic};
use crate::support::rust_journey::{copy_real_rust_fixture, generate_rust_lcov};
use cucumber::{World as _, given, then, when};
use rusqlite::{Connection, params};
use serde_json::Value;
use tempfile::TempDir;

#[derive(Debug, Default, cucumber::World)]
struct RustQuickstartWorld {
    temp_dir: Option<TempDir>,
    db_path: Option<PathBuf>,
    lcov_path: Option<PathBuf>,
    source_repo_dir: Option<PathBuf>,
    repo_dir: Option<PathBuf>,
    commit_sha: String,
    last_query_json: Option<Value>,
}

#[given("a temporary sqlite database for the Rust fixture quickstart")]
fn given_temporary_sqlite_database(world: &mut RustQuickstartWorld) {
    let temp_dir = TempDir::new().expect("failed to create temp dir for rust quickstart e2e");
    let db_path = temp_dir.path().join("testlens-rust-e2e.db");
    let lcov_path = temp_dir.path().join("rust-coverage.lcov");
    let source_repo_dir = project_root().join("testlens-fixture-rust");
    let repo_dir = copy_real_rust_fixture(temp_dir.path());

    assert!(
        source_repo_dir.exists(),
        "expected rust fixture repo at {}",
        source_repo_dir.display()
    );

    world.temp_dir = Some(temp_dir);
    world.db_path = Some(db_path);
    world.lcov_path = Some(lcov_path);
    world.source_repo_dir = Some(source_repo_dir);
    world.repo_dir = Some(repo_dir);
    world.commit_sha = "fixture-rust-e2e".to_string();
}

#[given("the real Rust fixture repository passes its own tests")]
fn given_rust_fixture_repo_passes_its_tests(world: &mut RustQuickstartWorld) {
    run_cargo_in_dir_or_panic(world.source_repo_dir(), &["test"]);
}

#[when("I run the Rust quickstart ingestion commands")]
fn when_i_run_the_rust_quickstart_ingestion_commands(world: &mut RustQuickstartWorld) {
    let db = world.db_path().to_string_lossy().to_string();
    let repo_dir = world.repo_dir().to_string_lossy().to_string();

    let init_args = vec!["init".to_string(), "--db".to_string(), db.clone()];
    let init_refs: Vec<&str> = init_args.iter().map(String::as_str).collect();
    run_testlens_or_panic(&init_refs);

    let prod_args = vec![
        "ingest-production-artefacts".to_string(),
        "--db".to_string(),
        db.clone(),
        "--repo-dir".to_string(),
        repo_dir.clone(),
        "--commit".to_string(),
        world.commit_sha.clone(),
    ];
    let prod_refs: Vec<&str> = prod_args.iter().map(String::as_str).collect();
    run_testlens_or_panic(&prod_refs);

    let test_args = vec![
        "ingest-tests".to_string(),
        "--db".to_string(),
        db,
        "--repo-dir".to_string(),
        repo_dir,
        "--commit".to_string(),
        world.commit_sha.clone(),
    ];
    let test_refs: Vec<&str> = test_args.iter().map(String::as_str).collect();
    run_testlens_or_panic(&test_refs);
}

#[then("Rust production artefacts are materialized for the commit")]
fn then_rust_production_artefacts_are_materialized(world: &mut RustQuickstartWorld) {
    let conn = Connection::open(world.db_path()).expect("failed to open sqlite db");

    let production_count: i64 = conn
        .query_row(
            r#"
SELECT COUNT(*)
FROM artefacts
WHERE commit_sha = ?1
  AND language = 'rust'
  AND path LIKE 'src/%'
  AND canonical_kind IN ('file', 'function', 'method', 'type', 'interface')
"#,
            params![world.commit_sha],
            |row| row.get(0),
        )
        .expect("failed counting rust production artefacts");

    let has_find_by_id: i64 = conn
        .query_row(
            r#"
SELECT COUNT(*)
FROM artefacts
WHERE commit_sha = ?1
  AND symbol_fqn = 'UserRepository.find_by_id'
"#,
            params![world.commit_sha],
            |row| row.get(0),
        )
        .expect("failed checking find_by_id production artefact");

    assert!(
        production_count > 0,
        "expected rust production artefacts for commit {}",
        world.commit_sha
    );
    assert_eq!(
        has_find_by_id, 1,
        "expected UserRepository.find_by_id production artefact"
    );
}

#[then("Rust test suites and scenarios are materialized for the commit")]
fn then_rust_test_suites_and_scenarios_are_materialized(world: &mut RustQuickstartWorld) {
    let conn = Connection::open(world.db_path()).expect("failed to open sqlite db");

    let suite_count: i64 = conn
        .query_row(
            r#"
SELECT COUNT(*)
FROM artefacts
WHERE commit_sha = ?1
  AND language = 'rust'
  AND canonical_kind = 'test_suite'
"#,
            params![world.commit_sha],
            |row| row.get(0),
        )
        .expect("failed counting rust test suites");

    let scenario_count: i64 = conn
        .query_row(
            r#"
SELECT COUNT(*)
FROM artefacts
WHERE commit_sha = ?1
  AND language = 'rust'
  AND canonical_kind = 'test_scenario'
"#,
            params![world.commit_sha],
            |row| row.get(0),
        )
        .expect("failed counting rust test scenarios");

    assert!(suite_count > 0, "expected rust test suites");
    assert!(scenario_count > 0, "expected rust test scenarios");
}

#[then("static test links are created for the commit")]
fn then_static_test_links_are_created(world: &mut RustQuickstartWorld) {
    let conn = Connection::open(world.db_path()).expect("failed to open sqlite db");

    let link_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM test_links WHERE commit_sha = ?1",
            params![world.commit_sha],
            |row| row.get(0),
        )
        .expect("failed counting test links");

    let has_find_by_id_link: i64 = conn
        .query_row(
            r#"
SELECT COUNT(*)
FROM test_links tl
JOIN artefacts t ON t.artefact_id = tl.test_artefact_id
JOIN artefacts p ON p.artefact_id = tl.production_artefact_id
WHERE tl.commit_sha = ?1
  AND t.commit_sha = ?1
  AND p.commit_sha = ?1
  AND t.canonical_kind = 'test_scenario'
  AND p.symbol_fqn = 'UserRepository.find_by_id'
"#,
            params![world.commit_sha],
            |row| row.get(0),
        )
        .expect("failed checking find_by_id static links");

    assert!(link_count > 0, "expected static test links");
    assert!(
        has_find_by_id_link > 0,
        "expected at least one static link to UserRepository.find_by_id"
    );
}

#[then(expr = "querying {string} returns covering tests before coverage ingestion")]
fn then_querying_returns_covering_tests_before_coverage_ingestion(
    world: &mut RustQuickstartWorld,
    artefact: String,
) {
    let db = world.db_path().to_string_lossy().to_string();
    let output = run_testlens_or_panic(&[
        "query",
        "--db",
        &db,
        "--artefact",
        &artefact,
        "--commit",
        &world.commit_sha,
        "--min-strength",
        "0.0",
    ]);

    let json: Value = serde_json::from_str(&output).expect("failed to parse query JSON");
    let covering_tests = json["covering_tests"]
        .as_array()
        .expect("covering_tests should be an array");

    assert!(
        !covering_tests.is_empty(),
        "expected covering_tests for artefact {}",
        artefact
    );

    let includes_repo_test = covering_tests.iter().any(|row| {
        row["file_path"]
            .as_str()
            .is_some_and(|path| path == "tests/user_repository_test.rs")
    });
    assert!(
        includes_repo_test,
        "expected user_repository_test.rs to cover {}",
        artefact
    );

    world.last_query_json = Some(json);
}

#[then("the query coverage payload is null before coverage ingestion")]
fn then_the_query_coverage_payload_is_null_before_coverage_ingestion(
    world: &mut RustQuickstartWorld,
) {
    let json = world
        .last_query_json
        .as_ref()
        .expect("expected query JSON to be available");
    assert!(
        json["coverage"].is_null(),
        "expected coverage to be null before ingest-coverage"
    );
}

#[when("I generate and ingest Rust LCOV coverage for the same commit")]
fn when_i_generate_and_ingest_rust_lcov_coverage(world: &mut RustQuickstartWorld) {
    generate_rust_lcov(world.repo_dir(), world.lcov_path());

    let db = world.db_path().to_string_lossy().to_string();
    let lcov = world.lcov_path().to_string_lossy().to_string();
    run_testlens_or_panic(&[
        "ingest-coverage",
        "--db",
        &db,
        "--lcov",
        &lcov,
        "--commit",
        &world.commit_sha,
    ]);
}

#[then(expr = "querying {string} with coverage view returns a non-null coverage payload")]
fn then_querying_with_coverage_view_returns_non_null_payload(
    world: &mut RustQuickstartWorld,
    artefact: String,
) {
    let db = world.db_path().to_string_lossy().to_string();
    let output = run_testlens_or_panic(&[
        "query",
        "--db",
        &db,
        "--artefact",
        &artefact,
        "--commit",
        &world.commit_sha,
        "--view",
        "coverage",
    ]);

    let json: Value = serde_json::from_str(&output).expect("failed to parse coverage query JSON");
    assert!(
        json["coverage"].is_object(),
        "expected non-null coverage payload for artefact {}",
        artefact
    );
    world.last_query_json = Some(json);
}

#[then("the coverage payload reports positive line coverage")]
fn then_coverage_payload_reports_positive_line_coverage(world: &mut RustQuickstartWorld) {
    let json = world
        .last_query_json
        .as_ref()
        .expect("expected query JSON to be available");
    let line_coverage_pct = json["coverage"]["line_coverage_pct"]
        .as_f64()
        .expect("expected line_coverage_pct");
    assert!(
        line_coverage_pct > 0.0,
        "expected positive line coverage percentage"
    );
}

#[tokio::test]
async fn rust_quickstart_e2e_gherkin() {
    RustQuickstartWorld::run("features/rust_quickstart_e2e.feature").await;
}

impl RustQuickstartWorld {
    fn db_path(&self) -> &Path {
        self.db_path
            .as_deref()
            .expect("db path should be initialized for this step")
    }

    fn lcov_path(&self) -> &Path {
        self.lcov_path
            .as_deref()
            .expect("lcov path should be initialized for this step")
    }

    fn source_repo_dir(&self) -> &Path {
        self.source_repo_dir
            .as_deref()
            .expect("source repo dir should be initialized for this step")
    }

    fn repo_dir(&self) -> &Path {
        self.repo_dir
            .as_deref()
            .expect("repo dir should be initialized for this step")
    }
}

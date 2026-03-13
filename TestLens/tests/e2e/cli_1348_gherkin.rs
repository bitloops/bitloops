use std::path::{Path, PathBuf};

use crate::support::cli::{project_root, run_testlens_or_panic};
use crate::support::rust_journey::{copy_real_rust_fixture, generate_rust_lcov};
use cucumber::{World as _, given, then, when};
use rusqlite::{Connection, params};
use serde_json::Value;
use tempfile::TempDir;

#[derive(Debug, Default, cucumber::World)]
struct RustCoverageWorld {
    temp_dir: Option<TempDir>,
    db_path: Option<PathBuf>,
    lcov_path: Option<PathBuf>,
    repo_dir: Option<PathBuf>,
    commit_c0: String,
    commit_c1: String,
    c0_query_json: Option<Value>,
    c1_query_json: Option<Value>,
}

#[given("a temporary sqlite database for Rust coverage ingestion validation")]
fn given_temporary_sqlite_database(world: &mut RustCoverageWorld) {
    let temp_dir = TempDir::new().expect("failed to create temp dir for rust coverage validation");
    let db_path = temp_dir.path().join("rust-coverage.db");
    let lcov_path = temp_dir.path().join("rust-coverage.lcov");
    let source_repo = project_root().join("testlens-fixture-rust");
    let repo_dir = copy_real_rust_fixture(temp_dir.path());

    assert!(
        source_repo.exists(),
        "expected rust fixture repo at {}",
        source_repo.display()
    );

    world.temp_dir = Some(temp_dir);
    world.db_path = Some(db_path);
    world.lcov_path = Some(lcov_path);
    world.repo_dir = Some(repo_dir);
    world.commit_c0 = "rust-cov-c0".to_string();
    world.commit_c1 = "rust-cov-c1".to_string();
}

#[given("the real Rust fixture repository is indexed for commits C0 and C1")]
fn given_real_rust_fixture_is_indexed_for_two_commits(world: &mut RustCoverageWorld) {
    let db = world.db_path().to_string_lossy().to_string();
    let repo_dir = world.repo_dir().to_string_lossy().to_string();

    run_testlens_or_panic(&["init", "--db", &db]);
    for commit in [&world.commit_c0, &world.commit_c1] {
        run_testlens_or_panic(&[
            "ingest-production-artefacts",
            "--db",
            &db,
            "--repo-dir",
            &repo_dir,
            "--commit",
            commit,
        ]);
        run_testlens_or_panic(&[
            "ingest-tests",
            "--db",
            &db,
            "--repo-dir",
            &repo_dir,
            "--commit",
            commit,
        ]);
    }
}

#[when("I generate Rust LCOV and ingest it for commit C1")]
fn when_i_generate_rust_lcov_and_ingest_it_for_c1(world: &mut RustCoverageWorld) {
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
        &world.commit_c1,
    ]);

    world.c0_query_json = Some(query_coverage_json(
        world.db_path(),
        &world.commit_c0,
        "UserRepository.find_by_id",
    ));
    world.c1_query_json = Some(query_coverage_json(
        world.db_path(),
        &world.commit_c1,
        "UserRepository.find_by_id",
    ));
}

#[then(expr = "querying {string} with coverage view at commit C0 returns null coverage")]
fn then_querying_coverage_at_c0_returns_null(world: &mut RustCoverageWorld, _artefact: String) {
    let json = world
        .c0_query_json
        .as_ref()
        .expect("expected C0 coverage query JSON");
    assert!(
        json["coverage"].is_null(),
        "expected null coverage payload for commit C0"
    );
}

#[then(expr = "querying {string} with coverage view at commit C1 returns non-null coverage")]
fn then_querying_coverage_at_c1_returns_non_null(world: &mut RustCoverageWorld, _artefact: String) {
    let json = world
        .c1_query_json
        .as_ref()
        .expect("expected C1 coverage query JSON");
    assert!(
        json["coverage"].is_object(),
        "expected non-null coverage payload for commit C1"
    );
}

#[then(expr = "coverage rows exist only for commit C1")]
fn then_coverage_rows_exist_only_for_commit_c1(world: &mut RustCoverageWorld) {
    let conn = Connection::open(world.db_path()).expect("failed opening sqlite db");
    let c0_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM test_coverage WHERE commit_sha = ?1",
            params![world.commit_c0],
            |row| row.get(0),
        )
        .expect("failed counting C0 coverage");
    let c1_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM test_coverage WHERE commit_sha = ?1",
            params![world.commit_c1],
            |row| row.get(0),
        )
        .expect("failed counting C1 coverage");

    assert_eq!(c0_count, 0, "expected no coverage rows for C0");
    assert!(c1_count > 0, "expected coverage rows for C1");
}

#[tokio::test]
async fn cli_1348_gherkin() {
    RustCoverageWorld::run("features/cli_1348.feature").await;
}

fn query_coverage_json(db_path: &Path, commit_sha: &str, artefact: &str) -> Value {
    let db = db_path.to_string_lossy().to_string();
    let output = run_testlens_or_panic(&[
        "query",
        "--db",
        &db,
        "--artefact",
        artefact,
        "--commit",
        commit_sha,
        "--view",
        "coverage",
    ]);
    serde_json::from_str(&output).expect("failed to parse query JSON")
}

impl RustCoverageWorld {
    fn db_path(&self) -> &Path {
        self.db_path
            .as_deref()
            .expect("db path should be initialized")
    }

    fn lcov_path(&self) -> &Path {
        self.lcov_path
            .as_deref()
            .expect("lcov path should be initialized")
    }

    fn repo_dir(&self) -> &Path {
        self.repo_dir
            .as_deref()
            .expect("repo dir should be initialized")
    }
}

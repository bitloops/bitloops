use std::path::Path;

use crate::support::cli::run_testlens_or_panic;
use crate::support::fixture::{BddWorkspace, write_cli_1381_base_fixture};
use cucumber::{World as _, given, then, when};
use rusqlite::{Connection, params};
use serde_json::Value;

#[derive(Debug, Default, cucumber::World)]
struct RustHybridWorld {
    workspace: Option<BddWorkspace>,
    commit_sha: String,
}

#[given("a temporary sqlite database for Rust hybrid detection")]
fn given_temporary_sqlite_database(world: &mut RustHybridWorld) {
    initialize_world(world);
}

#[given("a Cargo-backed Rust fixture repository with rstest, proptest, and doctests")]
fn given_cargo_backed_rust_fixture(world: &mut RustHybridWorld) {
    initialize_world(world);
}

#[when("I run the Rust hybrid detection journey")]
fn when_i_run_the_rust_hybrid_detection_journey(world: &mut RustHybridWorld) {
    let db = world.db_path().to_string_lossy().to_string();
    let repo_dir = world.repo_dir().to_string_lossy().to_string();

    run_testlens_or_panic(&["init", "--db", &db]);
    run_testlens_or_panic(&[
        "ingest-production-artefacts",
        "--db",
        &db,
        "--repo-dir",
        &repo_dir,
        "--commit",
        &world.commit_sha,
    ]);
    run_testlens_or_panic(&[
        "ingest-tests",
        "--db",
        &db,
        "--repo-dir",
        &repo_dir,
        "--commit",
        &world.commit_sha,
    ]);
}

#[then("listing Rust test scenarios returns rstest, proptest, and doctest cases")]
fn then_listing_rust_test_scenarios_returns_hybrid_cases(world: &mut RustHybridWorld) {
    let conn = Connection::open(world.db_path()).expect("failed to open sqlite db");
    let mut stmt = conn
        .prepare(
            r#"
SELECT symbol_fqn
FROM artefacts
WHERE commit_sha = ?1
  AND canonical_kind = 'test_scenario'
ORDER BY symbol_fqn ASC
"#,
        )
        .expect("failed preparing test scenario query");

    let names: Vec<String> = stmt
        .query_map(params![&world.commit_sha], |row| row.get(0))
        .expect("failed querying test scenarios")
        .map(|row| row.expect("failed reading test scenario"))
        .collect();

    for expected in [
        "hybrid_tests.double_is_even",
        "hybrid_tests.doubles_case_values[2, 4]",
        "hybrid_tests.doubles_case_values[3, 6]",
        "hybrid_tests.doubles_values[input=1]",
        "hybrid_tests.doubles_values[input=2]",
        "hybrid_tests.files_fallback",
        "hybrid_tests.triples_from_template[2, 6]",
        "hybrid_tests.triples_from_template[3, 9]",
    ] {
        assert!(
            names.contains(&expected.to_string()),
            "expected {} in {:?}",
            expected,
            names
        );
    }
    assert!(
        names
            .iter()
            .any(|name| name.starts_with("docs::doctests.documented_increment[doctest:")),
        "expected doctest scenario in {:?}",
        names
    );
}

#[then(expr = "querying {string} returns the covering test {string}")]
fn then_querying_returns_covering_test(
    world: &mut RustHybridWorld,
    artefact: String,
    test_name: String,
) {
    let output = run_query(world, &artefact);
    let json: Value = serde_json::from_str(&output).expect("failed to parse query JSON");
    let covering_tests = json["covering_tests"]
        .as_array()
        .expect("covering_tests should be an array");

    let found = covering_tests.iter().any(|row| {
        row["test_name"]
            .as_str()
            .is_some_and(|value| value == test_name)
    });
    assert!(
        found,
        "expected query for {} to include covering test {}, got {}",
        artefact, test_name, output
    );
}

#[then("querying \"documented_increment\" returns a doctest covering test")]
fn then_querying_documented_increment_returns_doctest(world: &mut RustHybridWorld) {
    let output = run_query(world, "documented_increment");
    let json: Value = serde_json::from_str(&output).expect("failed to parse query JSON");
    let covering_tests = json["covering_tests"]
        .as_array()
        .expect("covering_tests should be an array");

    let found = covering_tests.iter().any(|row| {
        row["test_name"]
            .as_str()
            .is_some_and(|value| value.starts_with("documented_increment[doctest:"))
    });
    assert!(
        found,
        "expected doctest covering test for documented_increment, got {}",
        output
    );
}

#[tokio::test]
async fn rust_hybrid_detection_e2e_gherkin() {
    RustHybridWorld::run("features/rust_hybrid_detection_e2e.feature").await;
}

fn initialize_world(world: &mut RustHybridWorld) {
    if world.workspace.is_some() {
        return;
    }

    let workspace = BddWorkspace::new();
    write_cli_1381_base_fixture(&workspace);
    world.workspace = Some(workspace);
    world.commit_sha = "fixture-rust-hybrid".to_string();
}

fn run_query(world: &RustHybridWorld, artefact: &str) -> String {
    let db = world.db_path().to_string_lossy().to_string();
    run_testlens_or_panic(&[
        "query",
        "--db",
        &db,
        "--artefact",
        artefact,
        "--commit",
        &world.commit_sha,
        "--view",
        "tests",
        "--min-strength",
        "0.0",
    ])
}

impl RustHybridWorld {
    fn workspace(&self) -> &BddWorkspace {
        self.workspace.as_ref().expect("expected workspace")
    }

    fn db_path(&self) -> &Path {
        self.workspace().db_path()
    }

    fn repo_dir(&self) -> &Path {
        self.workspace().repo_dir()
    }
}

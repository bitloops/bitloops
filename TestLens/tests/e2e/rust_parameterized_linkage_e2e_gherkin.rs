use std::path::Path;

use crate::support::cli::run_testlens_or_panic;
use crate::support::fixture::{BddWorkspace, write_cli_1369_base_fixture};
use cucumber::{World as _, given, then, when};
use rusqlite::{Connection, params};
use serde_json::Value;

#[derive(Debug, Default, cucumber::World)]
struct RustParameterizedWorld {
    workspace: Option<BddWorkspace>,
    commit_sha: String,
    last_query_json: Option<Value>,
}

#[given("a temporary sqlite database for Rust parameterized linkage")]
fn given_temporary_sqlite_database(world: &mut RustParameterizedWorld) {
    initialize_world(world);
}

#[given("a Rust fixture repository with inline parameterized src tests and Ruff-style additional declarations")]
fn given_rust_fixture_repository(world: &mut RustParameterizedWorld) {
    initialize_world(world);
}

#[when("I run the Rust parameterized linkage journey")]
fn when_i_run_the_rust_parameterized_linkage_journey(world: &mut RustParameterizedWorld) {
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

#[then("listing Rust test scenarios returns the parameterized and Ruff-style cases")]
fn then_listing_rust_test_scenarios_returns_parameterized_and_ruff_style_cases(
    world: &mut RustParameterizedWorld,
) {
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

    assert_eq!(
        names,
        vec![
            "api.empty_config".to_string(),
            "stable.equivalent_to_is_reflexive".to_string(),
            "stable.subtype_of_is_reflexive".to_string(),
            "tests.rules[StringDotFormatExtraNamedArguments, F522.py]".to_string(),
            "tests.rules[StringDotFormatExtraPositionalArguments, F523.py]".to_string(),
        ]
    );
}

#[then(expr = "querying {string} returns the covering test {string}")]
fn then_querying_returns_covering_test(
    world: &mut RustParameterizedWorld,
    artefact: String,
    test_name: String,
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
        "tests",
        "--min-strength",
        "0.0",
    ]);

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
    world.last_query_json = Some(json);
}

#[tokio::test]
async fn rust_parameterized_linkage_e2e_gherkin() {
    RustParameterizedWorld::run("features/rust_parameterized_linkage_e2e.feature").await;
}

fn initialize_world(world: &mut RustParameterizedWorld) {
    if world.workspace.is_some() {
        return;
    }

    let workspace = BddWorkspace::new();
    write_cli_1369_base_fixture(&workspace);
    world.workspace = Some(workspace);
    world.commit_sha = "fixture-rust-parameterized".to_string();
}

impl RustParameterizedWorld {
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

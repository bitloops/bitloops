use std::collections::HashSet;
use std::path::Path;

use crate::support::cli::run_testlens_or_panic;
use crate::support::fixture::{BddWorkspace, write_cli_1381_base_fixture};
use cucumber::{World as _, given, then, when};
use rusqlite::{Connection, params};
use serde_json::Value;

#[derive(Debug, Default, cucumber::World)]
struct Cli1381World {
    workspace: Option<BddWorkspace>,
    commit_c1: String,
    ingest_output: Option<String>,
}

#[given("a Cargo-backed Rust fixture repository with rstest, proptest, and doctests at commit C1")]
fn given_cargo_backed_rust_fixture(world: &mut Cli1381World) {
    initialize_world(world);
}

#[when("static linkage is ingested for C1 in the cargo-backed fixture")]
fn when_static_linkage_is_ingested(world: &mut Cli1381World) {
    initialize_world(world);

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
        &world.commit_c1,
    ]);
    let output = run_testlens_or_panic(&[
        "ingest-tests",
        "--db",
        &db,
        "--repo-dir",
        &repo_dir,
        "--commit",
        &world.commit_c1,
    ]);
    world.ingest_output = Some(output);
}

#[then("ingest-tests reports hybrid enumeration")]
fn then_ingest_tests_reports_hybrid_enumeration(world: &mut Cli1381World) {
    let output = world
        .ingest_output
        .as_deref()
        .expect("expected ingest-tests output");
    assert!(
        output.contains("enumeration: hybrid-full")
            || output.contains("enumeration: hybrid-partial"),
        "expected hybrid enumeration status, got {}",
        output
    );
}

#[then("rstest, proptest, and doctest scenarios are materialized")]
fn then_rust_patterns_are_materialized(world: &mut Cli1381World) {
    let conn = Connection::open(world.db_path()).expect("failed to open sqlite db");
    let mut stmt = conn
        .prepare(
            r#"
SELECT signature
FROM artefacts
WHERE commit_sha = ?1
  AND canonical_kind = 'test_scenario'
ORDER BY signature ASC
"#,
        )
        .expect("failed preparing scenario query");

    let scenario_names: HashSet<String> = stmt
        .query_map(params![&world.commit_c1], |row| row.get(0))
        .expect("failed querying scenario names")
        .map(|row| row.expect("failed reading scenario name"))
        .collect();

    for expected in [
        "doubles_case_values[2, 4]",
        "doubles_case_values[3, 6]",
        "doubles_values[input=1]",
        "doubles_values[input=2]",
        "triples_from_template[2, 6]",
        "triples_from_template[3, 9]",
        "files_fallback",
        "double_is_even",
    ] {
        assert!(
            scenario_names.contains(expected),
            "expected scenario {} in {:?}",
            expected,
            scenario_names
        );
    }
    assert!(
        scenario_names
            .iter()
            .any(|name| name.starts_with("documented_increment[doctest:")),
        "expected doctest scenario in {:?}",
        scenario_names
    );
}

#[then(expr = "querying double returns the covering tests {string} and {string}")]
fn then_querying_double_returns_covering_tests(
    world: &mut Cli1381World,
    first_test: String,
    second_test: String,
) {
    let json = query_json(world, "double");
    let covering_tests = json["covering_tests"]
        .as_array()
        .expect("covering_tests should be an array");

    for expected in [first_test, second_test] {
        let found = covering_tests.iter().any(|row| {
            row["test_name"]
                .as_str()
                .is_some_and(|value| value == expected)
        });
        assert!(
            found,
            "expected query for double to include covering test {}, got {}",
            expected,
            json
        );
    }
}

#[then(expr = "querying triple returns the covering test {string}")]
fn then_querying_triple_returns_covering_test(world: &mut Cli1381World, test_name: String) {
    assert_covering_test(world, "triple", &test_name);
}

#[then("querying documented_increment returns a doctest covering test")]
fn then_querying_documented_increment_returns_doctest(world: &mut Cli1381World) {
    let json = query_json(world, "documented_increment");
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
        "expected documented_increment query to include a doctest covering test, got {}",
        json
    );
}

#[tokio::test]
async fn cli_1381_gherkin() {
    Cli1381World::run("features/cli_1381.feature").await;
}

fn initialize_world(world: &mut Cli1381World) {
    if world.workspace.is_some() {
        return;
    }

    world.commit_c1 = "C1".to_string();
    let workspace = BddWorkspace::new();
    write_cli_1381_base_fixture(&workspace);
    world.workspace = Some(workspace);
}

fn assert_covering_test(world: &Cli1381World, artefact: &str, expected_test_name: &str) {
    let json = query_json(world, artefact);
    let covering_tests = json["covering_tests"]
        .as_array()
        .expect("covering_tests should be an array");

    let found = covering_tests.iter().any(|row| {
        row["test_name"]
            .as_str()
            .is_some_and(|value| value == expected_test_name)
    });
    assert!(
        found,
        "expected query for {} to include covering test {}, got {}",
        artefact, expected_test_name, json
    );
}

fn query_json(world: &Cli1381World, artefact: &str) -> Value {
    let db = world.db_path().to_string_lossy().to_string();
    let output = run_testlens_or_panic(&[
        "query",
        "--db",
        &db,
        "--artefact",
        artefact,
        "--commit",
        &world.commit_c1,
        "--view",
        "tests",
        "--min-strength",
        "0.0",
    ]);
    serde_json::from_str(&output).expect("failed to parse query JSON")
}

impl Cli1381World {
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

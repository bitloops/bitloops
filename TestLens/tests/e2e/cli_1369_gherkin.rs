use crate::support::cli::run_testlens_or_panic;
use crate::support::fixture::{BddWorkspace, write_cli_1369_base_fixture};
use crate::support::sqlite::initialize_schema;
use cucumber::{World as _, given, then, when};
use rusqlite::{Connection, params};
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, Default, cucumber::World)]
struct CliWorld {
    workspace: Option<BddWorkspace>,
    commit_c1: String,
}

#[given("a Rust fixture repository with Ruff-style additional test declarations at commit C1")]
fn given_fixture_repository_with_ruff_style_additional_tests(world: &mut CliWorld) {
    initialize_world(world);
}

#[when("static linkage is ingested for C1")]
fn when_static_linkage_is_ingested_for_c1(world: &mut CliWorld) {
    initialize_world(world);
    ingest_for_commit(world, &world.commit_c1);
}

#[then("Ruff-style Rust test scenarios are materialized")]
fn then_ruff_style_rust_test_scenarios_are_materialized(world: &mut CliWorld) {
    let conn = Connection::open(world.workspace().db_path()).expect("failed to open sqlite db");
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

    let expected: HashSet<String> = [
        "empty_config".to_string(),
        "equivalent_to_is_reflexive".to_string(),
        "rules[StringDotFormatExtraNamedArguments, F522.py]".to_string(),
        "rules[StringDotFormatExtraPositionalArguments, F523.py]".to_string(),
        "subtype_of_is_reflexive".to_string(),
    ]
    .into_iter()
    .collect();

    assert_eq!(scenario_names, expected);
}

#[then("querying render_message returns the wasm harness case")]
fn then_querying_render_message_returns_the_wasm_case(world: &mut CliWorld) {
    assert_covering_test(world, "render_message", "empty_config");
}

#[then("querying is_equivalent_to returns the quickcheck property test")]
fn then_querying_is_equivalent_to_returns_the_property_test(world: &mut CliWorld) {
    assert_covering_test(world, "is_equivalent_to", "equivalent_to_is_reflexive");
}

#[tokio::test]
async fn cli_1369_gherkin() {
    CliWorld::run("features/cli_1369.feature").await;
}

fn initialize_world(world: &mut CliWorld) {
    if world.workspace.is_some() {
        return;
    }

    world.commit_c1 = "C1".to_string();

    let workspace = BddWorkspace::new();
    write_cli_1369_base_fixture(&workspace);
    initialize_schema(workspace.db_path());
    world.workspace = Some(workspace);
}

fn ingest_for_commit(world: &CliWorld, commit: &str) {
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
        commit.to_string(),
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
        commit.to_string(),
    ];
    let test_refs: Vec<&str> = test_args.iter().map(String::as_str).collect();
    run_testlens_or_panic(&test_refs);
}

fn assert_covering_test(world: &CliWorld, artefact: &str, expected_test_name: &str) {
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
    let json: Value = serde_json::from_str(&output).expect("failed to parse query JSON");
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
        artefact, expected_test_name, output
    );
}

impl CliWorld {
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

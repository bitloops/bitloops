mod test_harness_support;

use std::collections::BTreeSet;

use cucumber::{World as _, given, then, when, writer::Stats as _};
use rusqlite::{Connection, params};
use serde_json::Value;
use test_harness_support::{
    ListedArtefact, Workspace, discovered_languages, load_symbol_fqn,
    load_test_scenario_signatures, run_bitloops_or_panic, scenario_link_exists,
    seed_production_artefacts, write_rust_additional_declarations_fixture,
    write_rust_hybrid_fixture, write_rust_parameterized_fixture, write_rust_static_link_fixture,
    write_typescript_static_link_fixture,
};

#[derive(Debug, Default, cucumber::World)]
struct TestHarnessWorld {
    workspace: Option<Workspace>,
    commit_sha: Option<String>,
    listed_scenarios: Vec<ListedArtefact>,
    query_json: Option<Value>,
    ingest_output: Option<String>,
}

#[given(
    expr = "an initialized TypeScript repository with production artefacts for commit {string}"
)]
fn given_initialized_typescript_repository(world: &mut TestHarnessWorld, commit_sha: String) {
    let workspace = Workspace::new("gherkin-typescript-static-links");
    write_typescript_static_link_fixture(&workspace);

    initialize_repository_with_production(world, workspace, commit_sha);
}

#[given(expr = "an initialized Rust repository with production artefacts for commit {string}")]
fn given_initialized_rust_repository(world: &mut TestHarnessWorld, commit_sha: String) {
    let workspace = Workspace::new("gherkin-rust-static-links");
    write_rust_static_link_fixture(&workspace);

    initialize_repository_with_production(world, workspace, commit_sha);
}

#[given(
    expr = "an initialized Rust repository with inline parameterized tests for commit {string}"
)]
fn given_initialized_rust_parameterized_repository(
    world: &mut TestHarnessWorld,
    commit_sha: String,
) {
    let workspace = Workspace::new("gherkin-rust-parameterized");
    write_rust_parameterized_fixture(&workspace);

    initialize_repository_with_production(world, workspace, commit_sha);
}

#[given(
    expr = "an initialized Rust repository with additional test declarations for commit {string}"
)]
fn given_initialized_rust_additional_declarations_repository(
    world: &mut TestHarnessWorld,
    commit_sha: String,
) {
    let workspace = Workspace::new("gherkin-rust-additional-declarations");
    write_rust_additional_declarations_fixture(&workspace);

    initialize_repository_with_production(world, workspace, commit_sha);
}

#[given(
    expr = "an initialized Cargo-backed Rust repository with rstest, proptest, and doctests for commit {string}"
)]
fn given_initialized_rust_hybrid_repository(world: &mut TestHarnessWorld, commit_sha: String) {
    let workspace = Workspace::new("gherkin-rust-hybrid");
    write_rust_hybrid_fixture(&workspace);

    initialize_repository_with_production(world, workspace, commit_sha);
}

fn initialize_repository_with_production(
    world: &mut TestHarnessWorld,
    workspace: Workspace,
    commit_sha: String,
) {
    run_bitloops_or_panic(
        workspace.repo_dir(),
        &["init", "--agent", "codex", "--telemetry", "false"],
    );
    run_bitloops_or_panic(workspace.repo_dir(), &["devql", "init"]);
    seed_production_artefacts(&workspace, &commit_sha);

    world.workspace = Some(workspace);
    world.commit_sha = Some(commit_sha);
    world.listed_scenarios.clear();
    world.query_json = None;
    world.ingest_output = None;
}

#[when(expr = "I ingest tests for commit {string}")]
fn when_ingest_tests(world: &mut TestHarnessWorld, commit_sha: String) {
    let output = run_bitloops_or_panic(
        world.workspace().repo_dir(),
        &["testlens", "ingest-tests", "--commit", &commit_sha],
    );
    world.ingest_output = Some(output);
}

#[then(expr = "test suites, test scenarios, and test links are created for commit {string}")]
fn then_test_rows_are_created(world: &mut TestHarnessWorld, commit_sha: String) {
    let conn = Connection::open(world.workspace().db_path()).expect("open sqlite db");
    let suite_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM test_suites WHERE commit_sha = ?1",
            params![commit_sha],
            |row| row.get(0),
        )
        .expect("count test_suites");
    let scenario_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM test_scenarios WHERE commit_sha = ?1",
            params![commit_sha],
            |row| row.get(0),
        )
        .expect("count test_scenarios");
    let link_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM test_links WHERE commit_sha = ?1",
            params![commit_sha],
            |row| row.get(0),
        )
        .expect("count test_links");

    assert!(suite_count > 0, "expected discovered test suites");
    assert!(scenario_count > 0, "expected discovered test scenarios");
    assert!(link_count > 0, "expected discovered test links");
}

#[then(expr = "{string} test artefacts are discoverable for commit {string}")]
fn then_test_artefacts_are_discoverable(
    world: &mut TestHarnessWorld,
    language_label: String,
    commit_sha: String,
) {
    let output = run_bitloops_or_panic(
        world.workspace().repo_dir(),
        &[
            "testlens",
            "list",
            "--commit",
            &commit_sha,
            "--kind",
            "test_scenario",
        ],
    );
    let scenarios: Vec<ListedArtefact> =
        serde_json::from_str(&output).expect("parse test scenario list");
    assert!(!scenarios.is_empty(), "expected discovered test scenarios");
    match language_label.as_str() {
        "TypeScript" => assert!(
            scenarios
                .iter()
                .all(|artefact| artefact.file_path.contains("userRepository.test.ts")),
            "expected only TypeScript scenarios"
        ),
        "Rust" => assert!(
            scenarios
                .iter()
                .all(|artefact| artefact.file_path.contains("rust_repo_test.rs")),
            "expected only Rust scenarios"
        ),
        other => panic!("unsupported language label `{other}`"),
    }

    let conn = Connection::open(world.workspace().db_path()).expect("open sqlite db");
    let languages = discovered_languages(&conn, &commit_sha);
    match language_label.as_str() {
        "TypeScript" => assert!(
            languages.contains("typescript") && !languages.contains("rust"),
            "expected only typescript test artefacts"
        ),
        "Rust" => assert!(
            languages.contains("rust") && !languages.contains("typescript"),
            "expected only rust test artefacts"
        ),
        other => panic!("unsupported language label `{other}`"),
    }

    world.listed_scenarios = scenarios;
}

#[then(
    expr = "production artefact matching {string} can be queried with covering tests for commit {string}"
)]
fn then_production_artefact_is_queryable(
    world: &mut TestHarnessWorld,
    artefact_pattern: String,
    commit_sha: String,
) {
    let query_json = query_covering_tests(world, &commit_sha, &artefact_pattern);
    let covering_tests = query_json["covering_tests"]
        .as_array()
        .expect("covering_tests should be an array");
    assert!(
        !covering_tests.is_empty(),
        "expected linked test scenarios in query output"
    );
    assert!(
        query_json["coverage"].is_null(),
        "expected coverage to be null before ingest-coverage"
    );

    world.query_json = Some(query_json);
}

#[then(
    expr = "scenario {string} links to symbol matching {string} but not {string} for commit {string}"
)]
fn then_scenario_links_match_expectations(
    world: &mut TestHarnessWorld,
    scenario_name: String,
    expected_symbol: String,
    absent_symbol: String,
    commit_sha: String,
) {
    let conn = Connection::open(world.workspace().db_path()).expect("open sqlite db");
    assert!(
        scenario_link_exists(
            &conn,
            &commit_sha,
            &scenario_name,
            &format!("%{expected_symbol}")
        ),
        "expected linkage to `{expected_symbol}` for scenario `{scenario_name}`"
    );
    assert!(
        !scenario_link_exists(
            &conn,
            &commit_sha,
            &scenario_name,
            &format!("%{absent_symbol}")
        ),
        "did not expect linkage to `{absent_symbol}` for scenario `{scenario_name}`"
    );
}

#[then(expr = "case-specific Rust test scenarios are materialized for commit {string}")]
fn then_case_specific_rust_scenarios_are_materialized(
    world: &mut TestHarnessWorld,
    commit_sha: String,
) {
    let conn = Connection::open(world.workspace().db_path()).expect("open sqlite db");
    let scenario_names = load_test_scenario_signatures(&conn, &commit_sha);

    assert_eq!(
        scenario_names,
        vec![
            "rules[StringDotFormatExtraNamedArguments, F522.py]".to_string(),
            "rules[StringDotFormatExtraPositionalArguments, F523.py]".to_string(),
        ]
    );
}

#[then(expr = "Ruff-style additional Rust test scenarios are materialized for commit {string}")]
fn then_additional_rust_scenarios_are_materialized(
    world: &mut TestHarnessWorld,
    commit_sha: String,
) {
    let conn = Connection::open(world.workspace().db_path()).expect("open sqlite db");
    let scenario_names: BTreeSet<String> = load_test_scenario_signatures(&conn, &commit_sha)
        .into_iter()
        .collect();

    let expected: BTreeSet<String> = [
        "empty_config",
        "equivalent_to_is_reflexive",
        "rules[StringDotFormatExtraNamedArguments, F522.py]",
        "rules[StringDotFormatExtraPositionalArguments, F523.py]",
        "subtype_of_is_reflexive",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();

    assert_eq!(scenario_names, expected);
}

#[then(expr = "ingest-tests reports hybrid enumeration for commit {string}")]
fn then_ingest_tests_reports_hybrid_enumeration(world: &mut TestHarnessWorld, _commit_sha: String) {
    let output = world
        .ingest_output
        .as_deref()
        .expect("expected ingest-tests output");
    assert!(
        output.contains("enumeration: hybrid-full")
            || output.contains("enumeration: hybrid-partial"),
        "expected hybrid enumeration status, got {output}"
    );
}

#[then(expr = "rstest, proptest, and doctest scenarios are materialized for commit {string}")]
fn then_hybrid_rust_scenarios_are_materialized(world: &mut TestHarnessWorld, commit_sha: String) {
    let conn = Connection::open(world.workspace().db_path()).expect("open sqlite db");
    let scenario_names: BTreeSet<String> = load_test_scenario_signatures(&conn, &commit_sha)
        .into_iter()
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
            "expected scenario {expected} in {:?}",
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

#[then(
    expr = "querying production artefact matching {string} returns covering test {string} for commit {string}"
)]
fn then_query_returns_covering_test(
    world: &mut TestHarnessWorld,
    artefact_pattern: String,
    expected_test_name: String,
    commit_sha: String,
) {
    let query_json = query_covering_tests(world, &commit_sha, &artefact_pattern);
    assert_covering_test_names(&query_json, &[expected_test_name.as_str()]);
    world.query_json = Some(query_json);
}

#[then(
    expr = "querying production artefact matching {string} returns covering tests {string} and {string} for commit {string}"
)]
fn then_query_returns_covering_tests(
    world: &mut TestHarnessWorld,
    artefact_pattern: String,
    first_test_name: String,
    second_test_name: String,
    commit_sha: String,
) {
    let query_json = query_covering_tests(world, &commit_sha, &artefact_pattern);
    assert_covering_test_names(&query_json, &[&first_test_name, &second_test_name]);
    world.query_json = Some(query_json);
}

#[then(
    expr = "querying production artefact matching {string} returns a doctest covering test for commit {string}"
)]
fn then_query_returns_doctest_covering_test(
    world: &mut TestHarnessWorld,
    artefact_pattern: String,
    commit_sha: String,
) {
    let query_json = query_covering_tests(world, &commit_sha, &artefact_pattern);
    let covering_tests = query_json["covering_tests"]
        .as_array()
        .expect("covering_tests should be an array");
    let found = covering_tests.iter().any(|row| {
        row["test_name"]
            .as_str()
            .is_some_and(|value| value.starts_with("documented_increment[doctest:"))
    });
    assert!(found, "expected doctest covering test, got {query_json}");
    world.query_json = Some(query_json);
}

#[tokio::test]
async fn testlens_bdd_features_pass() {
    let feature_dir = format!("{}/tests/features/test_harness", env!("CARGO_MANIFEST_DIR"));

    let result = TestHarnessWorld::cucumber()
        .before(|_, _, _, world| {
            Box::pin(async move {
                world.workspace = None;
                world.commit_sha = None;
                world.listed_scenarios.clear();
                world.query_json = None;
                world.ingest_output = None;
            })
        })
        .with_default_cli()
        .fail_on_skipped()
        .run(feature_dir)
        .await;

    assert!(
        !result.execution_has_failed(),
        "cucumber suite reported failures"
    );
    assert_eq!(result.skipped_steps(), 0, "cucumber suite skipped steps");
    assert_eq!(
        result.parsing_errors(),
        0,
        "cucumber suite had parse errors"
    );
}

impl TestHarnessWorld {
    fn workspace(&self) -> &Workspace {
        self.workspace
            .as_ref()
            .expect("workspace should be initialized for this step")
    }
}

fn query_covering_tests(
    world: &TestHarnessWorld,
    commit_sha: &str,
    artefact_pattern: &str,
) -> Value {
    query_artefact(
        world,
        commit_sha,
        artefact_pattern,
        &["--view", "tests", "--min-strength", "0.0"],
    )
}

fn query_artefact(
    world: &TestHarnessWorld,
    commit_sha: &str,
    artefact_pattern: &str,
    extra_args: &[&str],
) -> Value {
    let conn = Connection::open(world.workspace().db_path()).expect("open sqlite db");
    let symbol_fqn = load_symbol_fqn(&conn, commit_sha, &format!("%{artefact_pattern}"));
    let mut args = vec![
        "testlens",
        "query",
        "--artefact",
        &symbol_fqn,
        "--commit",
        commit_sha,
    ];
    args.extend_from_slice(extra_args);
    let output = run_bitloops_or_panic(world.workspace().repo_dir(), &args);
    serde_json::from_str(&output).expect("parse query json")
}

fn assert_covering_test_names(query_json: &Value, expected_test_names: &[&str]) {
    let covering_tests = query_json["covering_tests"]
        .as_array()
        .expect("covering_tests should be an array");

    for expected in expected_test_names {
        let found = covering_tests.iter().any(|row| {
            row["test_name"]
                .as_str()
                .is_some_and(|value| value == *expected)
        });
        assert!(
            found,
            "expected query to include covering test `{expected}`, got {query_json}"
        );
    }
}

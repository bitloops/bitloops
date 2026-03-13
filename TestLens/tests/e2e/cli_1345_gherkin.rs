use std::collections::BTreeSet;

use crate::support::cli::{list_artefacts_by_kind, run_testlens_or_panic};
use crate::support::fixture::{
    BddWorkspace, write_cli_1345_base_fixture, write_cli_1345_c1_extra_test,
};
use crate::support::sqlite::{initialize_schema, seed_source_file_for_commits};
use crate::support::types::ListedArtefact;
use cucumber::{World as _, given, then, when};
use rusqlite::{Connection, params};

#[derive(Debug, Default, cucumber::World)]
struct CliWorld {
    workspace: Option<BddWorkspace>,
    commit_c0: String,
    commit_c1: String,
    c0_scenarios: Vec<ListedArtefact>,
    c1_scenarios: Vec<ListedArtefact>,
}

#[given("a fixture repository containing TypeScript and Rust production files and test files")]
fn given_fixture_repository(world: &mut CliWorld) {
    initialize_world_fixture(world);
}

#[given("a commit C1 where new test files are added")]
fn given_commit_c1_with_new_tests(world: &mut CliWorld) {
    write_cli_1345_c1_extra_test(world.workspace());
}

#[when("the user runs testlens ingest-tests for C1")]
fn when_user_runs_ingest_for_c1(world: &mut CliWorld) {
    ingest_tests_for_commit(world, &world.commit_c1);
}

#[then("test_suite and test_scenario artefacts are created for C1")]
fn then_test_artefacts_created_for_c1(world: &mut CliWorld) {
    let conn = Connection::open(world.workspace().db_path()).expect("failed to open sqlite db");
    let suites: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM artefacts WHERE commit_sha = ?1 AND canonical_kind = 'test_suite'",
            params![world.commit_c1],
            |row| row.get(0),
        )
        .expect("failed counting test suites");
    let scenarios: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM artefacts WHERE commit_sha = ?1 AND canonical_kind = 'test_scenario'",
            params![world.commit_c1],
            |row| row.get(0),
        )
        .expect("failed counting test scenarios");

    assert!(suites > 0, "expected at least one test_suite artefact");
    assert!(
        scenarios > 0,
        "expected at least one test_scenario artefact"
    );
}

#[then("both Rust and TypeScript test artefacts are discovered for C1")]
fn then_both_rust_and_typescript_test_artefacts_are_discovered(world: &mut CliWorld) {
    let conn = Connection::open(world.workspace().db_path()).expect("failed to open sqlite db");
    let mut stmt = conn
        .prepare(
            r#"
SELECT DISTINCT language
FROM artefacts
WHERE commit_sha = ?1
  AND canonical_kind IN ('test_suite', 'test_scenario')
"#,
        )
        .expect("failed preparing language query");

    let rows = stmt
        .query_map(params![world.commit_c1], |row| row.get::<_, String>(0))
        .expect("failed querying test artefact languages");

    let mut languages = BTreeSet::new();
    for row in rows {
        languages.insert(row.expect("failed decoding language row"));
    }

    assert!(
        languages.contains("rust"),
        "expected rust test artefacts for C1"
    );
    assert!(
        languages.contains("typescript"),
        "expected typescript test artefacts for C1"
    );
}

#[then("each C1 test artefact includes language, file path, and source span metadata")]
fn then_c1_artefacts_have_required_metadata(world: &mut CliWorld) {
    let conn = Connection::open(world.workspace().db_path()).expect("failed to open sqlite db");
    let mut stmt = conn
        .prepare(
            r#"
SELECT language, path, start_line, end_line
FROM artefacts
WHERE commit_sha = ?1
  AND canonical_kind IN ('test_suite', 'test_scenario')
"#,
        )
        .expect("failed preparing metadata query");

    let rows = stmt
        .query_map(params![world.commit_c1], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .expect("failed querying artefact metadata");

    let mut seen = 0usize;
    for row in rows {
        let (language, path, start_line, end_line) = row.expect("failed mapping row");
        assert!(!language.trim().is_empty(), "language should not be empty");
        assert!(!path.trim().is_empty(), "path should not be empty");
        assert!(start_line > 0, "start_line should be positive");
        assert!(end_line >= start_line, "end_line should be >= start_line");
        seen += 1;
    }

    assert!(seen > 0, "expected at least one test artefact row");
}

#[then("querying C1 test scenarios is reproducible")]
fn then_c1_queries_are_reproducible(world: &mut CliWorld) {
    let first = list_test_scenarios(world, &world.commit_c1);
    let second = list_test_scenarios(world, &world.commit_c1);
    assert!(!first.is_empty(), "expected at least one C1 scenario");
    assert_eq!(
        logical_scenario_keys(&first),
        logical_scenario_keys(&second),
        "expected reproducible C1 query results"
    );
}

#[given("commits C0 and C1 where C1 introduces additional tests")]
fn given_commits_c0_and_c1(world: &mut CliWorld) {
    initialize_world_fixture(world);

    ingest_tests_for_commit(world, &world.commit_c0);
    write_cli_1345_c1_extra_test(world.workspace());
    ingest_tests_for_commit(world, &world.commit_c1);
}

#[when("test artefacts are queried at C0 and C1")]
fn when_test_artefacts_are_queried(world: &mut CliWorld) {
    world.c0_scenarios = list_test_scenarios(world, &world.commit_c0);
    world.c1_scenarios = list_test_scenarios(world, &world.commit_c1);
}

#[then("test artefacts introduced in C1 are absent at C0")]
fn then_new_c1_artefacts_absent_at_c0(world: &mut CliWorld) {
    let c0_keys = logical_scenario_keys(&world.c0_scenarios);
    let c1_keys = logical_scenario_keys(&world.c1_scenarios);
    assert!(!c0_keys.is_empty(), "expected C0 scenarios to be present");
    assert!(!c1_keys.is_empty(), "expected C1 scenarios to be present");

    let introduced: BTreeSet<_> = c1_keys.difference(&c0_keys).cloned().collect();
    assert!(
        !introduced.is_empty(),
        "expected at least one scenario introduced in C1"
    );
}

#[then("commit-addressed query results are reproducible for both C0 and C1")]
fn then_commit_addressed_results_are_reproducible(world: &mut CliWorld) {
    let c0_again = list_test_scenarios(world, &world.commit_c0);
    let c1_again = list_test_scenarios(world, &world.commit_c1);

    assert_eq!(
        logical_scenario_keys(&world.c0_scenarios),
        logical_scenario_keys(&c0_again),
        "expected reproducible C0 query results"
    );
    assert_eq!(
        logical_scenario_keys(&world.c1_scenarios),
        logical_scenario_keys(&c1_again),
        "expected reproducible C1 query results"
    );
}

#[tokio::test]
async fn cli_1345_gherkin() {
    CliWorld::run("features/cli_1345.feature").await;
}

fn initialize_world_fixture(world: &mut CliWorld) {
    world.commit_c0 = "C0".to_string();
    world.commit_c1 = "C1".to_string();

    let workspace = BddWorkspace::new();
    write_cli_1345_base_fixture(&workspace);

    initialize_schema(workspace.db_path());
    seed_source_file_for_commits(
        workspace.db_path(),
        "gherkin-fixture",
        &[&world.commit_c0, &world.commit_c1],
        "src/service.ts",
        "typescript",
    );

    world.workspace = Some(workspace);
}

fn ingest_tests_for_commit(world: &CliWorld, commit: &str) {
    let db = world.workspace().db_path().to_string_lossy().to_string();
    let repo_dir = world.workspace().repo_dir().to_string_lossy().to_string();

    let args = vec![
        "ingest-tests".to_string(),
        "--db".to_string(),
        db,
        "--repo-dir".to_string(),
        repo_dir,
        "--commit".to_string(),
        commit.to_string(),
    ];
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run_testlens_or_panic(&arg_refs);
}

fn list_test_scenarios(world: &CliWorld, commit: &str) -> Vec<ListedArtefact> {
    list_artefacts_by_kind(world.workspace().db_path(), commit, "test_scenario")
}

fn logical_scenario_keys(rows: &[ListedArtefact]) -> BTreeSet<String> {
    rows.iter()
        .map(|item| format!("{}:{}:{}", item.file_path, item.start_line, item.end_line))
        .collect()
}

impl CliWorld {
    fn workspace(&self) -> &BddWorkspace {
        self.workspace
            .as_ref()
            .expect("workspace should be initialized for this step")
    }
}

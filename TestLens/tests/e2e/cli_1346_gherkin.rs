use std::collections::BTreeSet;

use crate::support::cli::run_testlens_or_panic;
use crate::support::fixture::{
    BddWorkspace, write_cli_1346_base_fixture, write_cli_1346_c1_updated_tests,
};
use crate::support::sqlite::initialize_schema;
use cucumber::{World as _, given, then, when};
use rusqlite::{Connection, params};
use serde_json::Value;

#[derive(Debug, Default, cucumber::World)]
struct CliWorld {
    workspace: Option<BddWorkspace>,
    commit_c0: String,
    commit_c1: String,
    c0_edges: BTreeSet<String>,
    c1_edges: BTreeSet<String>,
}

#[given("a fixture repository with production artefacts and tests at commit C1")]
fn given_fixture_repository_at_c1(world: &mut CliWorld) {
    initialize_world(world);
    write_cli_1346_c1_updated_tests(world.workspace());
}

#[given("a test scenario that calls UserRepository.findById")]
fn given_test_scenario_calls_find_by_id(world: &mut CliWorld) {
    if world.workspace.is_none() {
        initialize_world(world);
        write_cli_1346_c1_updated_tests(world.workspace());
    }
}

#[when("static linkage is ingested for C1")]
fn when_static_linkage_ingested_for_c1(world: &mut CliWorld) {
    if world.workspace.is_none() {
        initialize_world(world);
        write_cli_1346_c1_updated_tests(world.workspace());
    }
    ingest_for_commit(world, &world.commit_c1);
}

#[then("a linkage edge is created to UserRepository.findById")]
fn then_linkage_edge_created_find_by_id(world: &mut CliWorld) {
    let exists = scenario_link_exists(
        world.workspace().db_path(),
        &world.commit_c1,
        "finds by id",
        "UserRepository.findById",
    );
    assert!(
        exists,
        "expected linkage edge to UserRepository.findById for scenario 'finds by id'"
    );
}

#[then("the linkage is queryable before coverage ingestion")]
fn then_linkage_is_queryable_before_coverage(world: &mut CliWorld) {
    let output = run_testlens_or_panic(&[
        "query",
        "--db",
        &world.workspace().db_path().to_string_lossy(),
        "--artefact",
        "UserRepository.findById",
        "--commit",
        &world.commit_c1,
    ]);
    let json: Value = serde_json::from_str(&output).expect("failed to parse query JSON");

    let covering_tests = json["covering_tests"]
        .as_array()
        .expect("covering_tests should be an array");
    assert!(
        !covering_tests.is_empty(),
        "expected covering_tests to include static links before coverage ingestion"
    );
    assert!(
        json["coverage"].is_null(),
        "coverage should be null before ingest-coverage"
    );
}

#[given("a fixture test scenario that imports UserRepository but only calls findByEmail")]
fn given_email_only_scenario(world: &mut CliWorld) {
    initialize_world(world);
    write_cli_1346_c1_updated_tests(world.workspace());
}

#[then("linkage exists to UserRepository.findByEmail for that scenario")]
fn then_linkage_exists_to_find_by_email(world: &mut CliWorld) {
    let exists = scenario_link_exists(
        world.workspace().db_path(),
        &world.commit_c1,
        "calls email lookup only",
        "UserRepository.findByEmail",
    );
    assert!(
        exists,
        "expected findByEmail linkage for 'calls email lookup only' scenario"
    );
}

#[then("linkage is not created to UserRepository.findById for that scenario")]
fn then_linkage_not_created_to_find_by_id(world: &mut CliWorld) {
    let exists = scenario_link_exists(
        world.workspace().db_path(),
        &world.commit_c1,
        "calls email lookup only",
        "UserRepository.findById",
    );
    assert!(
        !exists,
        "did not expect findById linkage for 'calls email lookup only' scenario"
    );
}

#[given("commits C0 and C1 where a scenario references a new production artefact only in C1")]
fn given_commits_c0_and_c1_with_new_edge(world: &mut CliWorld) {
    initialize_world(world);
    ingest_for_commit(world, &world.commit_c0);

    write_cli_1346_c1_updated_tests(world.workspace());
    ingest_for_commit(world, &world.commit_c1);
}

#[when("linkage is queried for C0 and C1")]
fn when_linkage_queried_for_c0_and_c1(world: &mut CliWorld) {
    world.c0_edges =
        scenario_linked_symbols(world.workspace().db_path(), &world.commit_c0, "finds by id");
    world.c1_edges =
        scenario_linked_symbols(world.workspace().db_path(), &world.commit_c1, "finds by id");
}

#[then("the new linkage edge appears only in C1")]
fn then_new_edge_appears_only_in_c1(world: &mut CliWorld) {
    assert!(
        !world.c0_edges.contains("UserRepository.findByEmail"),
        "did not expect findByEmail edge in C0"
    );
    assert!(
        world.c1_edges.contains("UserRepository.findByEmail"),
        "expected findByEmail edge in C1"
    );
}

#[then("linkage query results are reproducible for both commits")]
fn then_linkage_results_are_reproducible(world: &mut CliWorld) {
    let c0_again =
        scenario_linked_symbols(world.workspace().db_path(), &world.commit_c0, "finds by id");
    let c1_again =
        scenario_linked_symbols(world.workspace().db_path(), &world.commit_c1, "finds by id");

    assert_eq!(
        world.c0_edges, c0_again,
        "expected reproducible linkage for C0"
    );
    assert_eq!(
        world.c1_edges, c1_again,
        "expected reproducible linkage for C1"
    );
}

#[tokio::test]
async fn cli_1346_gherkin() {
    CliWorld::run("features/cli_1346.feature").await;
}

fn initialize_world(world: &mut CliWorld) {
    world.commit_c0 = "C0".to_string();
    world.commit_c1 = "C1".to_string();

    let workspace = BddWorkspace::new();
    write_cli_1346_base_fixture(&workspace);
    initialize_schema(workspace.db_path());
    world.workspace = Some(workspace);
}

fn ingest_for_commit(world: &CliWorld, commit: &str) {
    let db = world.workspace().db_path().to_string_lossy().to_string();
    let repo_dir = world.workspace().repo_dir().to_string_lossy().to_string();

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

fn scenario_link_exists(
    db_path: &std::path::Path,
    commit: &str,
    scenario_name: &str,
    symbol_fqn: &str,
) -> bool {
    let conn = Connection::open(db_path).expect("failed opening sqlite db");
    let count: i64 = conn
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
  AND t.signature = ?2
  AND p.symbol_fqn = ?3
"#,
            params![commit, scenario_name, symbol_fqn],
            |row| row.get(0),
        )
        .expect("failed querying linkage edge");
    count > 0
}

fn scenario_linked_symbols(
    db_path: &std::path::Path,
    commit: &str,
    scenario_name: &str,
) -> BTreeSet<String> {
    let conn = Connection::open(db_path).expect("failed opening sqlite db");
    let mut stmt = conn
        .prepare(
            r#"
SELECT p.symbol_fqn
FROM test_links tl
JOIN artefacts t ON t.artefact_id = tl.test_artefact_id
JOIN artefacts p ON p.artefact_id = tl.production_artefact_id
WHERE tl.commit_sha = ?1
  AND t.commit_sha = ?1
  AND p.commit_sha = ?1
  AND t.canonical_kind = 'test_scenario'
  AND t.signature = ?2
ORDER BY p.symbol_fqn ASC
"#,
        )
        .expect("failed preparing linkage symbol query");

    let rows = stmt
        .query_map(params![commit, scenario_name], |row| {
            row.get::<_, String>(0)
        })
        .expect("failed querying linkage symbols");

    let mut symbols = BTreeSet::new();
    for row in rows {
        symbols.insert(row.expect("failed decoding linkage symbol row"));
    }
    symbols
}

impl CliWorld {
    fn workspace(&self) -> &BddWorkspace {
        self.workspace
            .as_ref()
            .expect("workspace should be initialized for this step")
    }
}

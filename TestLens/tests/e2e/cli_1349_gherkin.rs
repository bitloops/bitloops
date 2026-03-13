use std::path::{Path, PathBuf};

use crate::support::cli::run_testlens_or_panic;
use crate::support::typescript_journey::{
    copy_real_typescript_fixture, index_real_typescript_fixture,
};
use cucumber::{World as _, given, then, when};
use serde_json::Value;
use tempfile::TempDir;

#[derive(Debug, Default, cucumber::World)]
struct CoverageViewWorld {
    temp_dir: Option<TempDir>,
    db_path: Option<PathBuf>,
    repo_dir: Option<PathBuf>,
    jest_json_path: Option<PathBuf>,
    commit_sha: String,
    first_query_json: Option<Value>,
    second_query_json: Option<Value>,
}

#[given("a temporary sqlite database and copied TypeScript fixture for coverage-view validation")]
fn given_temp_db_and_copied_typescript_fixture(world: &mut CoverageViewWorld) {
    let temp_dir = TempDir::new().expect("failed to create temp dir for coverage-view validation");
    let repo_dir = copy_real_typescript_fixture(temp_dir.path());
    let db_path = temp_dir.path().join("coverage-view.db");
    let jest_json_path = temp_dir.path().join("test-results.json");

    world.temp_dir = Some(temp_dir);
    world.repo_dir = Some(repo_dir);
    world.db_path = Some(db_path);
    world.jest_json_path = Some(jest_json_path);
    world.commit_sha = "fixture-coverage-view".to_string();
}

#[given("the copied TypeScript fixture has been fully indexed for coverage-view validation")]
fn given_typescript_fixture_is_fully_indexed(world: &mut CoverageViewWorld) {
    index_real_typescript_fixture(
        world.db_path(),
        world.repo_dir(),
        &world.commit_sha,
        world.jest_json_path(),
    );
}

#[when(expr = "I query coverage view for {string} and {string}")]
fn when_i_query_coverage_view_for_two_artefacts(
    world: &mut CoverageViewWorld,
    first: String,
    second: String,
) {
    world.first_query_json = Some(query_coverage_json(
        world.db_path(),
        &world.commit_sha,
        &first,
    ));
    world.second_query_json = Some(query_coverage_json(
        world.db_path(),
        &world.commit_sha,
        &second,
    ));
}

#[then("both coverage queries refer to the same source file")]
fn then_both_coverage_queries_refer_to_the_same_source_file(world: &mut CoverageViewWorld) {
    let first = world
        .first_query_json
        .as_ref()
        .expect("expected first coverage query");
    let second = world
        .second_query_json
        .as_ref()
        .expect("expected second coverage query");

    assert_eq!(
        first["artefact"]["file_path"].as_str(),
        second["artefact"]["file_path"].as_str(),
        "expected both artefacts to come from the same file"
    );
}

#[then("the coverage payloads are different for the two artefacts")]
fn then_coverage_payloads_are_different(world: &mut CoverageViewWorld) {
    let first = world
        .first_query_json
        .as_ref()
        .expect("expected first coverage query");
    let second = world
        .second_query_json
        .as_ref()
        .expect("expected second coverage query");

    assert_ne!(
        first["coverage"], second["coverage"],
        "expected different coverage payloads for same-file artefacts"
    );
}

#[when(expr = "I query coverage view for {string}")]
fn when_i_query_coverage_view_for_one_artefact(world: &mut CoverageViewWorld, artefact: String) {
    world.first_query_json = Some(query_coverage_json(
        world.db_path(),
        &world.commit_sha,
        &artefact,
    ));
}

#[then("the coverage payload contains uncovered branches")]
fn then_coverage_payload_contains_uncovered_branches(world: &mut CoverageViewWorld) {
    let json = world
        .first_query_json
        .as_ref()
        .expect("expected first coverage query");
    let branches = json["coverage"]["branches"]
        .as_array()
        .expect("expected branches array");

    let has_uncovered = branches
        .iter()
        .any(|branch| branch["covered"].as_bool().is_some_and(|covered| !covered));
    assert!(has_uncovered, "expected at least one uncovered branch");
}

#[tokio::test]
async fn cli_1349_gherkin() {
    CoverageViewWorld::run("features/cli_1349.feature").await;
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

impl CoverageViewWorld {
    fn db_path(&self) -> &Path {
        self.db_path
            .as_deref()
            .expect("db path should be initialized")
    }

    fn repo_dir(&self) -> &Path {
        self.repo_dir
            .as_deref()
            .expect("repo dir should be initialized")
    }

    fn jest_json_path(&self) -> &Path {
        self.jest_json_path
            .as_deref()
            .expect("jest json path should be initialized")
    }
}

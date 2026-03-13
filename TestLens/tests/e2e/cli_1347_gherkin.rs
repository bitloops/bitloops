use std::path::{Path, PathBuf};

use crate::support::cli::{run_testlens_allow_failure, run_testlens_or_panic};
use crate::support::typescript_journey::{
    copy_real_typescript_fixture, index_real_typescript_fixture,
};
use cucumber::{World as _, given, then, when};
use serde_json::Value;
use tempfile::TempDir;

#[derive(Debug, Default, cucumber::World)]
struct QueryLayerWorld {
    temp_dir: Option<TempDir>,
    db_path: Option<PathBuf>,
    repo_dir: Option<PathBuf>,
    jest_json_path: Option<PathBuf>,
    commit_sha: String,
    last_query_json: Option<Value>,
    default_tests_json: Option<Value>,
    override_tests_json: Option<Value>,
    last_failed_query_output: Option<String>,
}

#[given("a temporary sqlite database and copied TypeScript fixture for query-layer validation")]
fn given_temporary_db_and_copied_typescript_fixture(world: &mut QueryLayerWorld) {
    let temp_dir = TempDir::new().expect("failed to create temp dir for query-layer validation");
    let repo_dir = copy_real_typescript_fixture(temp_dir.path());
    let db_path = temp_dir.path().join("query-layer.db");
    let jest_json_path = temp_dir.path().join("test-results.json");

    world.temp_dir = Some(temp_dir);
    world.repo_dir = Some(repo_dir);
    world.db_path = Some(db_path);
    world.jest_json_path = Some(jest_json_path);
    world.commit_sha = "fixture-query-layer".to_string();
}

#[given("the copied TypeScript fixture has been fully indexed for the commit")]
fn given_typescript_fixture_fully_indexed(world: &mut QueryLayerWorld) {
    index_real_typescript_fixture(
        world.db_path(),
        world.repo_dir(),
        &world.commit_sha,
        world.jest_json_path(),
    );
}

#[when(expr = "I query {string} with summary view")]
fn when_i_query_with_summary_view(world: &mut QueryLayerWorld, artefact: String) {
    world.last_query_json = Some(run_query_json(
        world.db_path(),
        &world.commit_sha,
        &artefact,
        &["--view", "summary"],
    ));
}

#[then("the response contains only artefact and summary data")]
fn then_response_contains_only_artefact_and_summary_data(world: &mut QueryLayerWorld) {
    let json = world
        .last_query_json
        .as_ref()
        .expect("expected last query JSON");
    assert!(json.get("artefact").is_some(), "expected artefact payload");
    assert!(json.get("summary").is_some(), "expected summary payload");
    assert!(
        json.get("covering_tests").is_none(),
        "did not expect covering_tests in summary view"
    );
    assert!(
        json.get("coverage").is_none(),
        "did not expect coverage in summary view"
    );
}

#[then("the summary includes coverage percentages")]
fn then_summary_includes_coverage_percentages(world: &mut QueryLayerWorld) {
    let json = world
        .last_query_json
        .as_ref()
        .expect("expected last query JSON");
    assert!(
        json["summary"]["line_coverage_pct"].is_number(),
        "expected line_coverage_pct in summary payload"
    );
    assert!(
        json["summary"]["branch_coverage_pct"].is_number(),
        "expected branch_coverage_pct in summary payload"
    );
}

#[when(expr = "I query {string} with tests view using the default strength filter")]
fn when_i_query_with_tests_view_default_filter(world: &mut QueryLayerWorld, artefact: String) {
    world.default_tests_json = Some(run_query_json(
        world.db_path(),
        &world.commit_sha,
        &artefact,
        &["--view", "tests"],
    ));
}

#[when(expr = "I query {string} with tests view and min_strength 0.0")]
fn when_i_query_with_tests_view_override(world: &mut QueryLayerWorld, artefact: String) {
    world.override_tests_json = Some(run_query_json(
        world.db_path(),
        &world.commit_sha,
        &artefact,
        &["--view", "tests", "--min-strength", "0.0"],
    ));
}

#[then("the override returns more covering tests than the default query")]
fn then_override_returns_more_covering_tests(world: &mut QueryLayerWorld) {
    let default_count = covering_tests_len(
        world
            .default_tests_json
            .as_ref()
            .expect("expected default tests JSON"),
    );
    let override_count = covering_tests_len(
        world
            .override_tests_json
            .as_ref()
            .expect("expected override tests JSON"),
    );

    assert!(
        override_count > default_count,
        "expected override to return more tests (default: {}, override: {})",
        default_count,
        override_count
    );
}

#[then("the summary still reports the full covering-test count")]
fn then_summary_reports_full_covering_test_count(world: &mut QueryLayerWorld) {
    let default_json = world
        .default_tests_json
        .as_ref()
        .expect("expected default tests JSON");
    let override_json = world
        .override_tests_json
        .as_ref()
        .expect("expected override tests JSON");

    let default_summary_total = default_json["summary"]["total_covering_tests"]
        .as_u64()
        .expect("expected total_covering_tests in default summary");
    let override_visible_count = covering_tests_len(override_json) as u64;

    assert_eq!(
        default_summary_total, override_visible_count,
        "expected summary total to describe the full unfiltered match set"
    );
}

#[when(expr = "I query {string} with coverage view")]
fn when_i_query_with_coverage_view(world: &mut QueryLayerWorld, artefact: String) {
    world.last_query_json = Some(run_query_json(
        world.db_path(),
        &world.commit_sha,
        &artefact,
        &["--view", "coverage"],
    ));
}

#[then("the response contains only artefact and coverage data")]
fn then_response_contains_only_artefact_and_coverage_data(world: &mut QueryLayerWorld) {
    let json = world
        .last_query_json
        .as_ref()
        .expect("expected last query JSON");
    assert!(json.get("artefact").is_some(), "expected artefact payload");
    assert!(json.get("coverage").is_some(), "expected coverage payload");
    assert!(
        json.get("covering_tests").is_none(),
        "did not expect covering_tests in coverage view"
    );
    assert!(
        json.get("summary").is_none(),
        "did not expect summary in coverage view"
    );
}

#[then("the coverage payload includes branch entries")]
fn then_coverage_payload_includes_branch_entries(world: &mut QueryLayerWorld) {
    let json = world
        .last_query_json
        .as_ref()
        .expect("expected last query JSON");
    let branches = json["coverage"]["branches"]
        .as_array()
        .expect("expected branches array");
    assert!(!branches.is_empty(), "expected at least one branch entry");
}

#[then("the summary reports the artefact as untested")]
fn then_summary_reports_artefact_as_untested(world: &mut QueryLayerWorld) {
    let json = world
        .last_query_json
        .as_ref()
        .expect("expected last query JSON");
    assert_eq!(
        json["summary"]["verification_level"].as_str(),
        Some("untested"),
        "expected untested verification level"
    );
    assert_eq!(
        json["summary"]["total_covering_tests"].as_u64(),
        Some(0),
        "expected zero covering tests"
    );
}

#[when(expr = "I query {string} on an unindexed commit")]
fn when_i_query_on_an_unindexed_commit(world: &mut QueryLayerWorld, artefact: String) {
    initialize_db(world.db_path());
    world.last_failed_query_output = Some(run_query_failure(
        world.db_path(),
        "unindexed-commit",
        &artefact,
        &["--view", "summary"],
    ));
}

#[when(expr = "I query {string} with summary view expecting failure")]
fn when_i_query_with_summary_view_expect_failure(
    world: &mut QueryLayerWorld,
    artefact: String,
) {
    world.last_failed_query_output = Some(run_query_failure(
        world.db_path(),
        &world.commit_sha,
        &artefact,
        &["--view", "summary"],
    ));
}

#[then(expr = "the query fails with {string}")]
fn then_query_fails_with(world: &mut QueryLayerWorld, message: String) {
    let output = world
        .last_failed_query_output
        .as_ref()
        .expect("expected failed query output");
    assert!(
        output.contains(&message),
        "expected query failure output to contain {:?}, got:\n{}",
        message,
        output
    );
}

#[tokio::test]
async fn cli_1347_gherkin() {
    QueryLayerWorld::run("features/cli_1347.feature").await;
}

fn run_query_json(db_path: &Path, commit_sha: &str, artefact: &str, extra_args: &[&str]) -> Value {
    let db = db_path.to_string_lossy().to_string();

    let mut args = vec![
        "query".to_string(),
        "--db".to_string(),
        db,
        "--artefact".to_string(),
        artefact.to_string(),
        "--commit".to_string(),
        commit_sha.to_string(),
    ];
    args.extend(extra_args.iter().map(|item| item.to_string()));
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();

    let output = run_testlens_or_panic(&arg_refs);
    serde_json::from_str(&output).expect("failed to parse query JSON")
}

fn covering_tests_len(json: &Value) -> usize {
    json["covering_tests"]
        .as_array()
        .expect("expected covering_tests array")
        .len()
}

fn initialize_db(db_path: &Path) {
    let db = db_path.to_string_lossy().to_string();
    run_testlens_or_panic(&["init", "--db", &db]);
}

fn run_query_failure(db_path: &Path, commit_sha: &str, artefact: &str, extra_args: &[&str]) -> String {
    let db = db_path.to_string_lossy().to_string();

    let mut args = vec![
        "query".to_string(),
        "--db".to_string(),
        db,
        "--artefact".to_string(),
        artefact.to_string(),
        "--commit".to_string(),
        commit_sha.to_string(),
    ];
    args.extend(extra_args.iter().map(|item| item.to_string()));
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();

    let output = run_testlens_allow_failure(&arg_refs);
    assert!(
        !output.status.success(),
        "expected query to fail, stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

impl QueryLayerWorld {
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
